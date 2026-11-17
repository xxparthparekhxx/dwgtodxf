//! Proxy entity graphics (spec chapter 29): the pre-rendered primitives
//! AutoCAD stores for custom entities. Converting them to plain DXF entities
//! lets us show entities we cannot decode natively (multileaders, tables,
//! wipeouts, third-party objects, ...).

use crate::bits::{BitReader, Point3};
use crate::model::Groups;
use crate::version::Version;

/// One converted primitive: DXF type, entity-specific groups, and the
/// attribute overrides in effect when it was drawn.
#[derive(Debug, Clone)]
pub struct Prim {
    pub dxf_name: &'static str,
    pub groups: Groups,
    /// ACI colour override (None = inherit from the proxy entity)
    pub color: Option<i16>,
    pub rgb: Option<u32>,
    /// index into the layer table, if overridden
    pub layer_index: Option<u32>,
    pub lineweight: Option<i16>,
}

struct Rd<'a> {
    d: &'a [u8],
    i: usize,
    end: usize,
}

impl Rd<'_> {
    fn u32(&mut self) -> Option<u32> {
        let b = self.d.get(self.i..self.i + 4)?;
        self.i += 4;
        Some(u32::from_le_bytes(b.try_into().unwrap()))
    }
    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|v| v as i32)
    }
    fn f64(&mut self) -> Option<f64> {
        let b = self.d.get(self.i..self.i + 8)?;
        self.i += 8;
        Some(f64::from_le_bytes(b.try_into().unwrap()))
    }
    fn pt(&mut self) -> Option<Point3> {
        Some(Point3::new(self.f64()?, self.f64()?, self.f64()?))
    }
    fn u8(&mut self) -> Option<u8> {
        let b = *self.d.get(self.i)?;
        self.i += 1;
        Some(b)
    }
    fn align4(&mut self, start: usize) {
        let n = self.i - start;
        self.i = start + n.div_ceil(4) * 4;
    }
    /// zero-terminated 8-bit string padded to 4 bytes
    fn ps(&mut self) -> Option<String> {
        let start = self.i;
        let rest = self.d.get(self.i..self.end)?;
        let n = rest.iter().position(|&b| b == 0)?;
        let s = crate::bits::decode_8bit(&rest[..n]);
        self.i += n + 1;
        self.align4(start);
        Some(s)
    }
    /// zero-terminated UTF-16 string padded to 4 bytes
    fn pus(&mut self) -> Option<String> {
        let start = self.i;
        let mut u = vec![];
        loop {
            let b = self.d.get(self.i..self.i + 2)?;
            self.i += 2;
            let c = u16::from_le_bytes([b[0], b[1]]);
            if c == 0 {
                break;
            }
            u.push(c);
        }
        self.align4(start);
        Some(String::from_utf16_lossy(&u))
    }
}

type M = [f64; 16];
const IDENT: M = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];

fn mul(a: &M, b: &M) -> M {
    let mut r = [0.0; 16];
    for i in 0..4 {
        for j in 0..4 {
            r[i * 4 + j] = (0..4).map(|k| a[i * 4 + k] * b[k * 4 + j]).sum();
        }
    }
    r
}

fn xf(m: &M, p: Point3) -> Point3 {
    Point3::new(
        m[0] * p.x + m[1] * p.y + m[2] * p.z + m[3],
        m[4] * p.x + m[5] * p.y + m[6] * p.z + m[7],
        m[8] * p.x + m[9] * p.y + m[10] * p.z + m[11],
    )
}

fn xf_dir(m: &M, p: Point3) -> Point3 {
    Point3::new(
        m[0] * p.x + m[1] * p.y + m[2] * p.z,
        m[4] * p.x + m[5] * p.y + m[6] * p.z,
        m[8] * p.x + m[9] * p.y + m[10] * p.z,
    )
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}
fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
fn cross(a: Point3, b: Point3) -> Point3 {
    Point3::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}
fn len(a: Point3) -> f64 {
    dot(a, a).sqrt()
}
fn norm(a: Point3) -> Point3 {
    let l = len(a);
    if l == 0.0 {
        Point3::Z
    } else {
        Point3::new(a.x / l, a.y / l, a.z / l)
    }
}
fn scale(a: Point3, s: f64) -> Point3 {
    Point3::new(a.x * s, a.y * s, a.z * s)
}

/// Arbitrary axis algorithm: OCS x/y axes for a normal.
pub fn ocs_axes(n: Point3) -> (Point3, Point3) {
    let n = norm(n);
    let ax = if n.x.abs() < 1.0 / 64.0 && n.y.abs() < 1.0 / 64.0 {
        norm(cross(Point3::new(0., 1., 0.), n))
    } else {
        norm(cross(Point3::Z, n))
    };
    (ax, norm(cross(n, ax)))
}

fn to_ocs(p: Point3, n: Point3) -> Point3 {
    if n == Point3::Z {
        return p;
    }
    let (ax, ay) = ocs_axes(n);
    Point3::new(dot(p, ax), dot(p, ay), dot(p, norm(n)))
}

fn angle_in_ocs(v: Point3, n: Point3) -> f64 {
    let (ax, ay) = ocs_axes(n);
    dot(v, ay).atan2(dot(v, ax))
}

/// Circle through three points: (center, radius, normal), with the normal
/// oriented so that p1 -> p2 -> p3 runs counter-clockwise.
fn circle3(p1: Point3, p2: Point3, p3: Point3) -> Option<(Point3, f64, Point3)> {
    let a = sub(p2, p1);
    let b = sub(p3, p1);
    let n = cross(a, b);
    let nn = dot(n, n);
    if nn < 1e-24 {
        return None;
    }
    let u = scale(cross(b, n), dot(a, a));
    let v = scale(cross(n, a), dot(b, b));
    let c = Point3::new(
        p1.x + (u.x + v.x) / (2.0 * nn),
        p1.y + (u.y + v.y) / (2.0 * nn),
        p1.z + (u.z + v.z) / (2.0 * nn),
    );
    Some((c, len(sub(p1, c)), norm(n)))
}

struct State {
    color: Option<i16>,
    rgb: Option<u32>,
    layer: Option<u32>,
    lw: Option<i16>,
    fill: bool,
    stack: Vec<M>,
}

impl State {
    fn m(&self) -> &M {
        self.stack.last().unwrap_or(&IDENT)
    }
    fn prim(&self, name: &'static str, g: Groups) -> Prim {
        Prim { dxf_name: name, groups: g, color: self.color, rgb: self.rgb, layer_index: self.layer, lineweight: self.lw }
    }
}

fn polyline(st: &State, pts: &[Point3], closed: bool, out: &mut Vec<Prim>) {
    if pts.len() < 2 {
        return;
    }
    let m = st.m();
    let pts: Vec<Point3> = pts.iter().map(|p| xf(m, *p)).collect();
    let z = pts[0].z;
    if pts.iter().all(|p| (p.z - z).abs() < 1e-9) {
        if st.fill && closed && pts.len() >= 3 {
            let mut g = Groups::default();
            g.subclass("AcDbHatch");
            g.pt(10, Point3::new(0., 0., z));
            g.pt(210, Point3::Z);
            g.str(2, "SOLID");
            g.int(70, 1);
            g.int(71, 0);
            g.int(91, 1);
            g.int(92, 3);
            g.int(72, 0);
            g.int(73, 1);
            g.int(93, pts.len() as i32);
            for p in &pts {
                g.pt2(10, *p);
            }
            g.int(97, 0);
            g.int(75, 0);
            g.int(76, 1);
            g.int(98, 0);
            out.push(st.prim("HATCH", g));
            return;
        }
        let mut g = Groups::default();
        g.subclass("AcDbPolyline");
        g.int(90, pts.len() as i32);
        g.int(70, closed as i16);
        if z != 0.0 {
            g.real(38, z);
        }
        for p in &pts {
            g.pt2(10, *p);
        }
        out.push(st.prim("LWPOLYLINE", g));
    } else {
        let n = pts.len();
        let segs = if closed { n } else { n - 1 };
        for i in 0..segs {
            let mut g = Groups::default();
            g.subclass("AcDbLine");
            g.pt(10, pts[i]);
            g.pt(11, pts[(i + 1) % n]);
            out.push(st.prim("LINE", g));
        }
    }
}

fn circle(st: &State, c: Point3, r: f64, n: Point3, arc: Option<(Point3, f64)>, out: &mut Vec<Prim>) {
    let m = st.m();
    let c = xf(m, c);
    let n = norm(xf_dir(m, n));
    // uniform scale of the transform
    let s = len(xf_dir(m, Point3::new(1., 0., 0.)));
    let r = r * if s > 0.0 { s } else { 1.0 };
    let mut g = Groups::default();
    g.subclass("AcDbCircle");
    g.pt(10, to_ocs(c, n));
    g.real(40, r);
    g.extrusion(n);
    match arc {
        None => out.push(st.prim("CIRCLE", g)),
        Some((start_vec, sweep)) => {
            let sv = xf_dir(m, start_vec);
            let a0 = angle_in_ocs(sv, n);
            let (s0, s1) = if sweep >= 0.0 { (a0, a0 + sweep) } else { (a0 + sweep, a0) };
            g.subclass("AcDbArc");
            g.angle_deg(50, s0);
            g.angle_deg(51, s1);
            out.push(st.prim("ARC", g));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn text(st: &State, p: Point3, n: Point3, dir: Point3, h: f64, wf: f64, obl: f64, s: String, out: &mut Vec<Prim>) {
    if s.is_empty() {
        return;
    }
    let m = st.m();
    let p = xf(m, p);
    let n = norm(xf_dir(m, n));
    let dir = xf_dir(m, dir);
    let sc = len(xf_dir(m, Point3::new(0., 1., 0.)));
    let mut g = Groups::default();
    g.subclass("AcDbText");
    g.pt(10, to_ocs(p, n));
    g.real(40, h * if sc > 0.0 { sc } else { 1.0 });
    g.str(1, s);
    let rot = angle_in_ocs(dir, n);
    if rot != 0.0 {
        g.angle_deg(50, rot);
    }
    if wf != 1.0 && wf > 0.0 {
        g.real(41, wf);
    }
    if obl != 0.0 {
        g.angle_deg(51, obl);
    }
    g.extrusion(n);
    g.subclass("AcDbText");
    out.push(st.prim("TEXT", g));
}

fn face(st: &State, pts: &[Point3], out: &mut Vec<Prim>) {
    let m = st.m();
    let v: Vec<Point3> = pts.iter().map(|p| xf(m, *p)).collect();
    let tris: Vec<[Point3; 4]> = match v.len() {
        3 => vec![[v[0], v[1], v[2], v[2]]],
        4 => vec![[v[0], v[1], v[2], v[3]]],
        n if n > 4 => (1..n - 1).map(|i| [v[0], v[i], v[i + 1], v[i + 1]]).collect(),
        _ => return,
    };
    for t in tris {
        let mut g = Groups::default();
        g.subclass("AcDbFace");
        for (i, p) in t.iter().enumerate() {
            g.pt(10 + i as i16, *p);
        }
        out.push(st.prim("3DFACE", g));
    }
}

fn color_of(v: u32, st: &mut State) {
    match v >> 24 {
        0xC2 => st.rgb = Some(v & 0xFF_FFFF),
        0xC3 => {
            st.color = Some((v & 0xFF) as i16);
            st.rgb = None
        }
        0xC0 => {
            st.color = Some(256);
            st.rgb = None
        }
        0xC1 => {
            st.color = Some(0);
            st.rgb = None
        }
        _ => {
            st.color = Some(v as i16);
            st.rgb = None
        }
    }
}

/// Convert proxy graphics to primitives. Unknown chunk types are skipped.
pub fn convert(data: &[u8], ver: Version) -> Vec<Prim> {
    let mut out = vec![];
    let mut st = State { color: None, rgb: None, layer: None, lw: None, fill: false, stack: vec![] };
    if data.len() < 8 {
        return out;
    }
    let mut pos = 8usize;
    while pos + 8 <= data.len() {
        let size = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap());
        if size < 8 || pos + size > data.len() {
            break;
        }
        let mut r = Rd { d: data, i: pos + 8, end: pos + size };
        let _ = chunk(kind, &mut r, &mut st, ver, &mut out);
        pos += size;
    }
    out
}

fn chunk(kind: u32, r: &mut Rd, st: &mut State, ver: Version, out: &mut Vec<Prim>) -> Option<()> {
    match kind {
        2 => {
            let c = r.pt()?;
            let rad = r.f64()?;
            let n = r.pt()?;
            circle(st, c, rad, n, None, out);
        }
        3 => {
            let (p1, p2, p3) = (r.pt()?, r.pt()?, r.pt()?);
            if let Some((c, rad, n)) = circle3(p1, p2, p3) {
                circle(st, c, rad, n, None, out);
            }
        }
        4 => {
            let c = r.pt()?;
            let rad = r.f64()?;
            let n = r.pt()?;
            let sv = r.pt()?;
            let sweep = r.f64()?;
            circle(st, c, rad, n, Some((sv, sweep)), out);
        }
        5 => {
            let (p1, p2, p3) = (r.pt()?, r.pt()?, r.pt()?);
            if let Some((c, rad, n)) = circle3(p1, p2, p3) {
                let (ax, ay) = ocs_axes(n);
                let ang = |p: Point3| {
                    let v = sub(p, c);
                    dot(v, ay).atan2(dot(v, ax))
                };
                let a0 = ang(p1);
                let mut a1 = ang(p3);
                while a1 <= a0 {
                    a1 += std::f64::consts::TAU;
                }
                let sv = sub(p1, c);
                circle(st, c, rad, n, Some((sv, a1 - a0)), out);
            }
        }
        6 | 7 | 32 => {
            let n = r.u32()? as usize;
            let mut pts = Vec::with_capacity(n.min(1 << 16));
            for _ in 0..n {
                pts.push(r.pt()?);
            }
            polyline(st, &pts, kind == 7, out);
        }
        8 => {
            let rows = r.u32()? as usize;
            let cols = r.u32()? as usize;
            let mut v = Vec::with_capacity((rows * cols).min(1 << 20));
            for _ in 0..rows * cols {
                v.push(r.pt()?);
            }
            for i in 0..rows.saturating_sub(1) {
                for j in 0..cols.saturating_sub(1) {
                    let q = [v[i * cols + j], v[i * cols + j + 1], v[(i + 1) * cols + j + 1], v[(i + 1) * cols + j]];
                    face(st, &q, out);
                }
            }
        }
        9 => {
            let n = r.u32()? as usize;
            let mut v = Vec::with_capacity(n.min(1 << 20));
            for _ in 0..n {
                v.push(r.pt()?);
            }
            let ne = r.u32()? as usize;
            let mut k = 0;
            while k < ne {
                let c = r.i32()?;
                k += 1;
                let cnt = c.unsigned_abs() as usize;
                let mut f = Vec::with_capacity(cnt.min(1 << 16));
                for _ in 0..cnt {
                    let idx = r.i32()?;
                    k += 1;
                    if let Some(p) = v.get(idx.unsigned_abs() as usize) {
                        f.push(*p);
                    }
                }
                if c > 0 {
                    face(st, &f, out);
                }
            }
        }
        10 | 36 => {
            let p = r.pt()?;
            let n = r.pt()?;
            let dir = r.pt()?;
            let h = r.f64()?;
            let wf = r.f64()?;
            let obl = r.f64()?;
            let s = if kind == 10 { r.ps()? } else { r.pus()? };
            text(st, p, n, dir, h, wf, obl, s, out);
        }
        11 | 38 => {
            let p = r.pt()?;
            let n = r.pt()?;
            let dir = r.pt()?;
            let s = if kind == 11 { r.ps()? } else { r.pus()? };
            let _len = r.i32()?;
            let _raw = r.u32()?;
            let h = r.f64()?;
            let wf = r.f64()?;
            let obl = r.f64()?;
            text(st, p, n, dir, h, wf, obl, s, out);
        }
        12 | 13 => {
            let m = *st.m();
            let p1 = xf(&m, r.pt()?);
            let p2 = xf(&m, r.pt()?);
            let mut g = Groups::default();
            g.subclass(if kind == 12 { "AcDbXline" } else { "AcDbRay" });
            g.pt(10, p1);
            g.pt(11, norm(sub(p2, p1)));
            out.push(st.prim(if kind == 12 { "XLINE" } else { "RAY" }, g));
        }
        14 => color_of(r.u32()?, st),
        16 => st.layer = Some(r.u32()?),
        20 => st.fill = r.u32()? != 0,
        22 => {
            let (rr, gg, bb) = (r.u8()?, r.u8()?, r.u8()?);
            st.rgb = Some((rr as u32) << 16 | (gg as u32) << 8 | bb as u32);
        }
        23 => {
            let v = r.i32()?;
            st.lw = Some(match v {
                -1 | -2 | -3 => v as i16,
                v if (0..=211).contains(&v) => v as i16,
                _ => crate::objects::lineweight(v as u8),
            });
        }
        29 | 30 => {
            let mut m = [0.0; 16];
            for x in m.iter_mut() {
                *x = r.f64()?;
            }
            let top = *st.m();
            st.stack.push(mul(&top, &m));
        }
        31 => {
            st.stack.pop();
        }
        33 => {
            let n = r.u32()? as usize;
            let raw = r.d.get(r.i..r.i + n)?;
            let mut o = crate::objects::ObjReader {
                d: BitReader::new(raw, ver),
                s: None,
                h: BitReader::new(&[], ver),
                handle: 0,
                ver,
            };
            let mut g = Groups::default();
            if crate::entities::lwpolyline_pub(&mut o, &mut g).is_ok() {
                if st.stack.is_empty() {
                    out.push(st.prim("LWPOLYLINE", g));
                }
            }
        }
        _ => {}
    }
    Some(())
}
