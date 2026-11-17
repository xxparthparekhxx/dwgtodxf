//! Entity decoders (spec 20.4.3 - 20.4.85). Each one reads the type-specific
//! data and handles and emits the DXF groups that follow the AcDbEntity part.

use crate::bits::Point3;
use crate::error::Result;
use crate::model::*;
use crate::objects::ObjReader;
use crate::version::Version;

pub fn decode(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let g = &mut o.groups;
    match o.type_code {
        1 => text(r, g, TextKind::Text),
        2 => text(r, g, TextKind::Attrib),
        3 => text(r, g, TextKind::Attdef),
        4 => {
            let name = r.tv()?;
            o.kind = Kind::Block { name };
            Ok(())
        }
        5 => {
            o.kind = Kind::EndBlk;
            Ok(())
        }
        6 => {
            o.kind = Kind::Seqend;
            Ok(())
        }
        7 | 8 => insert(r, o),
        10 => vertex_2d(r, g),
        11..=13 => vertex_3d(r, g, o.type_code),
        14 => pface_face(r, g),
        15 | 16 | 29 | 30 => polyline(r, o),
        17 => arc(r, g),
        18 => circle(r, g),
        19 => line(r, g),
        20..=26 => dimension(r, g, o.type_code),
        27 => point(r, g),
        28 => face3d(r, g),
        31 | 32 => solid(r, g),
        34 => viewport(r, g),
        37..=39 => acis(r, o),
        35 => ellipse(r, g),
        36 => spline(r, g),
        40 | 41 => ray(r, g, o.type_code),
        44 => mtext(r, g),
        45 => leader(r, g),
        46 => tolerance(r, g),
        47 => mline(r, g),
        _ => match o.dxf_name.as_str() {
            "LWPOLYLINE" => lwpolyline(r, g),
            "3DSOLID" | "REGION" | "BODY" => acis(r, o),
            "HATCH" => hatch(r, g),
            "IMAGE" => image(r, g, false),
            "WIPEOUT" => image(r, g, true),
            _ => {
                o.kind = Kind::Unsupported;
                Ok(())
            }
        },
    }
}

#[derive(PartialEq, Clone, Copy)]
enum TextKind {
    Text,
    Attrib,
    Attdef,
}

fn text(r: &mut ObjReader, g: &mut Groups, kind: TextKind) -> Result<()> {
    let ver = r.ver;
    let (elev, ins, align, extr, thick, oblique, rot, height, width, value, gen, halign, valign);
    if ver < Version::R2000 {
        elev = r.d.bd()?;
        ins = r.d.rd2()?;
        align = r.d.rd2()?;
        extr = r.d.bd3()?;
        thick = r.d.bd()?;
        oblique = r.d.bd()?;
        rot = r.d.bd()?;
        height = r.d.bd()?;
        width = r.d.bd()?;
        value = r.tv()?;
        gen = r.d.bs()?;
        halign = r.d.bs()?;
        valign = r.d.bs()?;
    } else {
        let f = r.d.rc()?;
        elev = if f & 0x01 == 0 { r.d.rd()? } else { 0.0 };
        ins = r.d.rd2()?;
        align = if f & 0x02 == 0 { r.d.dd2(ins)? } else { ins };
        extr = r.d.be()?;
        thick = r.d.bt()?;
        oblique = if f & 0x04 == 0 { r.d.rd()? } else { 0.0 };
        rot = if f & 0x08 == 0 { r.d.rd()? } else { 0.0 };
        height = r.d.rd()?;
        width = if f & 0x10 == 0 { r.d.rd()? } else { 1.0 };
        value = r.tv()?;
        gen = if f & 0x20 == 0 { r.d.bs()? } else { 0 };
        halign = if f & 0x40 == 0 { r.d.bs()? } else { 0 };
        valign = if f & 0x80 == 0 { r.d.bs()? } else { 0 };
    }

    // attribute part
    let mut attr = None;
    if kind != TextKind::Text {
        let mut version = None;
        if ver >= Version::R2010 {
            version = Some(r.d.rc()?);
        }
        let mut atype = 1u8;
        if ver >= Version::R2018 {
            atype = r.d.rc()?;
        }
        if atype > 1 {
            // multi-line attribute: embedded MTEXT, not decoded
            return Err(crate::error::Error::Unsupported("multi-line attribute".into()));
        }
        let tag = r.tv()?;
        let field_len = r.d.bs()?;
        let flags = r.d.rc()?;
        let lock = if ver >= Version::R2007 { r.d.b()? } else { false };
        let mut prompt = String::new();
        if kind == TextKind::Attdef {
            if ver >= Version::R2010 {
                let _v = r.d.rc()?;
            }
            prompt = r.tv()?;
        }
        attr = Some((version, tag, field_len, flags, lock, prompt));
    }
    let style = r.href_opt();

    g.subclass("AcDbText");
    if thick != 0.0 {
        g.real(39, thick);
    }
    g.pt(10, Point3::new(ins.x, ins.y, elev));
    g.real(40, height);
    g.str(1, value);
    if rot != 0.0 {
        g.angle_deg(50, rot);
    }
    if width != 1.0 {
        g.real(41, width);
    }
    if oblique != 0.0 {
        g.angle_deg(51, oblique);
    }
    g.name(7, style, "Standard");
    if gen != 0 {
        g.int(71, gen as i16);
    }
    if halign != 0 {
        g.int(72, halign as i16);
    }
    if halign != 0 || valign != 0 {
        g.pt(11, Point3::new(align.x, align.y, elev));
    }
    g.extrusion(extr);
    match (kind, attr) {
        (TextKind::Text, _) => {
            g.subclass("AcDbText");
            if valign != 0 {
                g.int(73, valign as i16);
            }
        }
        (TextKind::Attrib, Some((version, tag, field_len, flags, lock, _))) => {
            g.subclass("AcDbAttribute");
            if let Some(v) = version {
                g.int(280, v);
            }
            g.str(2, tag);
            g.int(70, flags);
            if field_len != 0 {
                g.int(73, field_len as i16);
            }
            if valign != 0 {
                g.int(74, valign as i16);
            }
            if lock {
                g.int(280, 1);
            }
        }
        (TextKind::Attdef, Some((version, tag, field_len, flags, lock, prompt))) => {
            g.subclass("AcDbAttributeDefinition");
            if let Some(v) = version {
                g.int(280, v);
            }
            g.str(3, prompt);
            g.str(2, tag);
            g.int(70, flags);
            if field_len != 0 {
                g.int(73, field_len as i16);
            }
            if valign != 0 {
                g.int(74, valign as i16);
            }
            if lock {
                g.int(280, 1);
            }
        }
        _ => {}
    }
    Ok(())
}

fn insert(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let ver = r.ver;
    let ins = r.d.bd3()?;
    let scale = if ver < Version::R2000 {
        r.d.bd3()?
    } else {
        match r.d.bb()? {
            3 => Point3::new(1.0, 1.0, 1.0),
            1 => {
                let y = r.d.dd(1.0)?;
                let z = r.d.dd(1.0)?;
                Point3::new(1.0, y, z)
            }
            2 => {
                let x = r.d.rd()?;
                Point3::new(x, x, x)
            }
            _ => {
                let x = r.d.rd()?;
                let y = r.d.dd(x)?;
                let z = r.d.dd(x)?;
                Point3::new(x, y, z)
            }
        }
    };
    let rot = r.d.bd()?;
    let extr = r.d.bd3()?;
    let has_attribs = r.d.b()?;
    let mut owned_count = 0;
    if ver >= Version::R2004 && has_attribs {
        owned_count = r.d.bl()? as usize;
    }
    let minsert = o.type_code == 8;
    let (mut cols, mut rows, mut cs, mut rs) = (1u16, 1u16, 0.0, 0.0);
    if minsert {
        cols = r.d.bs()?;
        rows = r.d.bs()?;
        cs = r.d.bd()?;
        rs = r.d.bd()?;
    }
    let block = r.href()?;
    let (mut first, mut last, mut owned) = (0, 0, vec![]);
    if has_attribs {
        if ver < Version::R2004 {
            first = r.href()?;
            last = r.href()?;
        } else {
            for _ in 0..owned_count {
                owned.push(r.href()?);
            }
        }
        let seqend = r.href()?;
        o.kind = Kind::Complex { first, last, owned, seqend };
    }

    let g = &mut o.groups;
    g.subclass(if minsert { "AcDbMInsertBlock" } else { "AcDbBlockReference" });
    if has_attribs {
        g.int(66, 1);
    }
    g.name(2, block, "");
    g.pt(10, ins);
    if scale.x != 1.0 {
        g.real(41, scale.x);
    }
    if scale.y != 1.0 {
        g.real(42, scale.y);
    }
    if scale.z != 1.0 {
        g.real(43, scale.z);
    }
    if rot != 0.0 {
        g.angle_deg(50, rot);
    }
    if minsert {
        g.int(70, cols as i16);
        g.int(71, rows as i16);
        g.real(44, cs);
        g.real(45, rs);
    }
    g.extrusion(extr);
    Ok(())
}

fn vertex_2d(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let flags = r.d.rc()?;
    let p = r.d.bd3()?;
    let mut sw = r.d.bd()?;
    let ew = if sw < 0.0 {
        sw = -sw;
        sw
    } else {
        r.d.bd()?
    };
    let bulge = r.d.bd()?;
    if r.ver >= Version::R2010 {
        let _id = r.d.bl()?;
    }
    let tangent = r.d.bd()?;
    g.subclass("AcDbVertex");
    g.subclass("AcDb2dVertex");
    g.pt(10, p);
    if sw != 0.0 {
        g.real(40, sw);
    }
    if ew != 0.0 {
        g.real(41, ew);
    }
    if bulge != 0.0 {
        g.real(42, bulge);
    }
    g.int(70, flags);
    if tangent != 0.0 {
        g.angle_deg(50, tangent);
    }
    Ok(())
}

fn vertex_3d(r: &mut ObjReader, g: &mut Groups, code: u16) -> Result<()> {
    let flags = r.d.rc()?;
    let p = r.d.bd3()?;
    g.subclass("AcDbVertex");
    let (sub, f) = match code {
        11 => ("AcDb3dPolylineVertex", flags | 32),
        12 => ("AcDbPolygonMeshVertex", flags | 64),
        _ => ("AcDbPolyFaceMeshVertex", flags | 64 | 128),
    };
    g.subclass(sub);
    g.pt(10, p);
    g.int(70, f);
    Ok(())
}

fn pface_face(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let idx = [r.d.bs_i()?, r.d.bs_i()?, r.d.bs_i()?, r.d.bs_i()?];
    g.subclass("AcDbVertex");
    g.subclass("AcDbFaceRecord");
    g.pt(10, Point3::default());
    g.int(70, 128);
    for (i, v) in idx.iter().enumerate() {
        if *v != 0 {
            g.int(71 + i as i16, *v);
        }
    }
    Ok(())
}

fn polyline(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let ver = r.ver;
    let g = &mut o.groups;
    match o.type_code {
        15 => {
            let flags = r.d.bs()?;
            let curve = r.d.bs()?;
            let sw = r.d.bd()?;
            let ew = r.d.bd()?;
            let thick = r.d.bt()?;
            let elev = r.d.bd()?;
            let extr = r.d.be()?;
            g.subclass("AcDb2dPolyline");
            g.int(66, 1);
            g.pt(10, Point3::new(0.0, 0.0, elev));
            if thick != 0.0 {
                g.real(39, thick);
            }
            g.int(70, flags as i16);
            if sw != 0.0 {
                g.real(40, sw);
            }
            if ew != 0.0 {
                g.real(41, ew);
            }
            if curve != 0 {
                g.int(75, curve as i16);
            }
            g.extrusion(extr);
        }
        16 => {
            let sflags = r.d.rc()?;
            let cflags = r.d.rc()?;
            let mut flags = 8i16;
            if cflags & 1 != 0 {
                flags |= 1;
            }
            let mut curve = 0;
            if sflags & 3 != 0 {
                flags |= 4;
                curve = if sflags & 1 != 0 { 5 } else { 6 };
            }
            g.subclass("AcDb3dPolyline");
            g.int(66, 1);
            g.pt(10, Point3::default());
            g.int(70, flags);
            if curve != 0 {
                g.int(75, curve);
            }
        }
        29 => {
            let nv = r.d.bs_i()?;
            let nf = r.d.bs_i()?;
            g.subclass("AcDbPolyFaceMesh");
            g.int(66, 1);
            g.pt(10, Point3::default());
            g.int(70, 64);
            g.int(71, nv);
            g.int(72, nf);
        }
        _ => {
            let flags = r.d.bs()?;
            let curve = r.d.bs()?;
            let (m, n, md, nd) = (r.d.bs_i()?, r.d.bs_i()?, r.d.bs_i()?, r.d.bs_i()?);
            g.subclass("AcDbPolygonMesh");
            g.int(66, 1);
            g.pt(10, Point3::default());
            g.int(70, (flags | 16) as i16);
            g.int(71, m);
            g.int(72, n);
            g.int(73, md);
            g.int(74, nd);
            g.int(75, curve as i16);
        }
    }
    let mut owned = vec![];
    let (mut first, mut last) = (0, 0);
    if ver >= Version::R2004 {
        let n = r.d.bl()? as usize;
        for _ in 0..n {
            owned.push(r.href()?);
        }
    } else {
        first = r.href()?;
        last = r.href()?;
    }
    let seqend = r.href()?;
    o.kind = Kind::Complex { first, last, owned, seqend };
    Ok(())
}

fn arc(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let c = r.d.bd3()?;
    let radius = r.d.bd()?;
    let thick = r.d.bt()?;
    let extr = r.d.be()?;
    let a0 = r.d.bd()?;
    let a1 = r.d.bd()?;
    g.subclass("AcDbCircle");
    if thick != 0.0 {
        g.real(39, thick);
    }
    g.pt(10, c);
    g.real(40, radius);
    g.extrusion(extr);
    g.subclass("AcDbArc");
    g.angle_deg(50, a0);
    g.angle_deg(51, a1);
    Ok(())
}

fn circle(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let c = r.d.bd3()?;
    let radius = r.d.bd()?;
    let thick = r.d.bt()?;
    let extr = r.d.be()?;
    g.subclass("AcDbCircle");
    if thick != 0.0 {
        g.real(39, thick);
    }
    g.pt(10, c);
    g.real(40, radius);
    g.extrusion(extr);
    Ok(())
}

fn line(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let (p0, p1);
    if r.ver < Version::R2000 {
        p0 = r.d.bd3()?;
        p1 = r.d.bd3()?;
    } else {
        let zzero = r.d.b()?;
        let sx = r.d.rd()?;
        let ex = r.d.dd(sx)?;
        let sy = r.d.rd()?;
        let ey = r.d.dd(sy)?;
        let (mut sz, mut ez) = (0.0, 0.0);
        if !zzero {
            sz = r.d.rd()?;
            ez = r.d.dd(sz)?;
        }
        p0 = Point3::new(sx, sy, sz);
        p1 = Point3::new(ex, ey, ez);
    }
    let thick = r.d.bt()?;
    let extr = r.d.be()?;
    g.subclass("AcDbLine");
    if thick != 0.0 {
        g.real(39, thick);
    }
    g.pt(10, p0);
    g.pt(11, p1);
    g.extrusion(extr);
    Ok(())
}

fn point(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let p = r.d.bd3()?;
    let thick = r.d.bt()?;
    let extr = r.d.be()?;
    let xang = r.d.bd()?;
    g.subclass("AcDbPoint");
    g.pt(10, p);
    if thick != 0.0 {
        g.real(39, thick);
    }
    g.extrusion(extr);
    if xang != 0.0 {
        g.angle_deg(50, xang);
    }
    Ok(())
}

fn face3d(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let (c, invis);
    if r.ver < Version::R2000 {
        c = [r.d.bd3()?, r.d.bd3()?, r.d.bd3()?, r.d.bd3()?];
        invis = r.d.bs()?;
    } else {
        let no_flags = r.d.b()?;
        let zzero = r.d.b()?;
        let x = r.d.rd()?;
        let y = r.d.rd()?;
        let z = if zzero { 0.0 } else { r.d.rd()? };
        let c0 = Point3::new(x, y, z);
        let c1 = r.d.dd3(c0)?;
        let c2 = r.d.dd3(c1)?;
        let c3 = r.d.dd3(c2)?;
        c = [c0, c1, c2, c3];
        invis = if no_flags { 0 } else { r.d.bs()? };
    }
    g.subclass("AcDbFace");
    for (i, p) in c.iter().enumerate() {
        g.pt(10 + i as i16, *p);
    }
    if invis != 0 {
        g.int(70, invis as i16);
    }
    Ok(())
}

fn solid(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let thick = r.d.bt()?;
    let elev = r.d.bd()?;
    let c = [r.d.rd2()?, r.d.rd2()?, r.d.rd2()?, r.d.rd2()?];
    let extr = r.d.be()?;
    g.subclass("AcDbTrace");
    for (i, p) in c.iter().enumerate() {
        g.pt(10 + i as i16, Point3::new(p.x, p.y, elev));
    }
    if thick != 0.0 {
        g.real(39, thick);
    }
    g.extrusion(extr);
    Ok(())
}

fn ellipse(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let c = r.d.bd3()?;
    let major = r.d.bd3()?;
    let extr = r.d.bd3()?;
    let ratio = r.d.bd()?;
    let a0 = r.d.bd()?;
    let a1 = r.d.bd()?;
    g.subclass("AcDbEllipse");
    g.pt(10, c);
    g.pt(11, major);
    g.extrusion(extr);
    g.real(40, ratio);
    g.real(41, a0);
    g.real(42, a1);
    Ok(())
}

fn spline(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let mut scenario = r.d.bl()?;
    let mut flags: i16 = 0;
    let mut knot_param = None;
    if r.ver >= Version::R2013 {
        let f1 = r.d.bl()?;
        let kp = r.d.bl()?;
        knot_param = Some(kp);
        if f1 & 4 != 0 {
            flags |= 1;
        }
        // flags1 bit 1 means "method fit points"
        scenario = if f1 & 1 != 0 { 2 } else { 1 };
        let _ = kp;
    }
    let degree = r.d.bl()?;
    let (mut fit_tol, mut knot_tol, mut ctrl_tol) = (0.0, 0.0, 0.0);
    let (mut beg_tan, mut end_tan) = (Point3::default(), Point3::default());
    let (mut nfit, mut nknots, mut nctrl, mut weighted) = (0usize, 0usize, 0usize, false);
    if scenario == 2 {
        fit_tol = r.d.bd()?;
        beg_tan = r.d.bd3()?;
        end_tan = r.d.bd3()?;
        nfit = r.d.bl()? as usize;
    } else {
        let rational = r.d.b()?;
        let closed = r.d.b()?;
        let periodic = r.d.b()?;
        knot_tol = r.d.bd()?;
        ctrl_tol = r.d.bd()?;
        nknots = r.d.bl()? as usize;
        nctrl = r.d.bl()? as usize;
        weighted = r.d.b()?;
        if rational {
            flags |= 4;
        }
        if closed {
            flags |= 1;
        }
        if periodic {
            flags |= 2;
        }
    }
    let mut knots = Vec::with_capacity(nknots.min(1 << 20));
    for _ in 0..nknots {
        knots.push(r.d.bd()?);
    }
    let mut ctrl = Vec::with_capacity(nctrl.min(1 << 20));
    for _ in 0..nctrl {
        let p = r.d.bd3()?;
        let w = if weighted { r.d.bd()? } else { 1.0 };
        ctrl.push((p, w));
    }
    let mut fit = Vec::with_capacity(nfit.min(1 << 20));
    for _ in 0..nfit {
        fit.push(r.d.bd3()?);
    }
    let _ = knot_param;

    // fit-point splines: compute the control points AutoCAD would write
    let mut degree = degree;
    if knots.is_empty() && ctrl.is_empty() && fit.len() >= 2 {
        if let Some((k, c)) = crate::spline::fit_to_control(&fit, beg_tan, end_tan) {
            knots = k;
            ctrl = c.into_iter().map(|p| (p, 1.0)).collect();
            degree = 3;
        }
    }

    g.subclass("AcDbSpline");
    g.int(70, flags | 8);
    g.int(71, degree as i32);
    g.int(72, knots.len() as i32);
    g.int(73, ctrl.len() as i32);
    g.int(74, fit.len() as i32);
    if knot_tol != 0.0 {
        g.real(42, knot_tol);
    }
    if ctrl_tol != 0.0 {
        g.real(43, ctrl_tol);
    }
    if scenario == 2 {
        g.real(44, fit_tol);
        if beg_tan != Point3::default() {
            g.pt(12, beg_tan);
        }
        if end_tan != Point3::default() {
            g.pt(13, end_tan);
        }
    }
    for k in &knots {
        g.real(40, *k);
    }
    if weighted {
        for (_, w) in &ctrl {
            g.real(41, *w);
        }
    }
    for (p, _) in &ctrl {
        g.pt(10, *p);
    }
    for p in &fit {
        g.pt(11, *p);
    }
    Ok(())
}

fn ray(r: &mut ObjReader, g: &mut Groups, code: u16) -> Result<()> {
    let p = r.d.bd3()?;
    let v = r.d.bd3()?;
    g.subclass(if code == 40 { "AcDbRay" } else { "AcDbXline" });
    g.pt(10, p);
    g.pt(11, v);
    Ok(())
}

fn mtext(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let ver = r.ver;
    let ins = r.d.bd3()?;
    let extr = r.d.bd3()?;
    let xdir = r.d.bd3()?;
    let rect_w = r.d.bd()?;
    let rect_h = if ver >= Version::R2007 { r.d.bd()? } else { 0.0 };
    let height = r.d.bd()?;
    let attach = r.d.bs()?;
    let dir = r.d.bs()?;
    let _ext_h = r.d.bd()?;
    let _ext_w = r.d.bd()?;
    let text = r.tv()?;
    let (mut ls_style, mut ls_factor) = (1u16, 1.0);
    if ver >= Version::R2000 {
        ls_style = r.d.bs()?;
        ls_factor = r.d.bd()?;
        let _ = r.d.b()?;
    }
    let mut bg = None;
    if ver >= Version::R2004 {
        let flags = r.d.bl()?;
        if flags & 1 != 0 || (ver >= Version::R2018 && flags & 0x10 != 0) {
            let scale = r.d.bd()?;
            let color = r.cmc()?;
            let transp = r.d.bl()?;
            bg = Some((flags, scale, color, transp));
        } else if flags != 0 {
            bg = Some((flags, 1.5, Default::default(), 0));
        }
    }
    let style = r.href_opt();

    g.subclass("AcDbMText");
    g.pt(10, ins);
    g.real(40, height);
    g.real(41, rect_w);
    if rect_h != 0.0 {
        g.real(46, rect_h);
    }
    g.int(71, attach as i16);
    g.int(72, dir as i16);
    // text in 250-char chunks: 3 ... 3, then 1
    let chars: Vec<char> = text.chars().collect();
    let chunks: Vec<String> = chars.chunks(250).map(|c| c.iter().collect()).collect();
    if chunks.is_empty() {
        g.str(1, "");
    }
    for (i, c) in chunks.iter().enumerate() {
        g.str(if i + 1 == chunks.len() { 1 } else { 3 }, c.clone());
    }
    g.name(7, style, "Standard");
    g.extrusion(extr);
    g.pt(11, xdir);
    g.int(73, ls_style as i16);
    g.real(44, ls_factor);
    if let Some((flags, scale, color, transp)) = bg {
        g.int(90, flags as i32);
        if flags & 1 != 0 {
            g.int(63, color.index);
            if let Some(rgb) = color.rgb {
                g.int(421, rgb as i32);
            }
            g.real(45, scale);
            g.int(441, transp as i32);
        }
    }
    Ok(())
}

fn dimension(r: &mut ObjReader, g: &mut Groups, code: u16) -> Result<()> {
    let ver = r.ver;
    let mut version = None;
    if ver >= Version::R2010 {
        version = Some(r.d.rc()?);
    }
    let extr = r.d.bd3()?;
    let text_mid = r.d.rd2()?;
    let elev = r.d.bd()?;
    let flags1 = r.d.rc()?;
    let user_text = r.tv()?;
    let text_rot = r.d.bd()?;
    let horiz_dir = r.d.bd()?;
    let _ins_scale = r.d.bd3()?;
    let _ins_rot = r.d.bd()?;
    let (mut attach, mut ls_style, mut ls_factor, mut measurement) = (0u16, 0u16, 1.0, 0.0);
    if ver >= Version::R2000 {
        attach = r.d.bs()?;
        ls_style = r.d.bs()?;
        ls_factor = r.d.bd()?;
        measurement = r.d.bd()?;
    }
    if ver >= Version::R2007 {
        let _ = r.d.b()?;
        let _flip1 = r.d.b()?;
        let _flip2 = r.d.b()?;
    }
    let pt12 = r.d.rd2()?;

    let mut dimtype: i16 = 32;
    if flags1 & 1 == 0 {
        dimtype |= 128;
    }
    let mut sub = Groups::default();
    let def10;
    match code {
        20 => {
            def10 = r.d.bd3()?;
            let p13 = r.d.bd3()?;
            let p14 = r.d.bd3()?;
            let flags2 = r.d.rc()?;
            dimtype |= 6;
            if flags2 & 1 != 0 {
                dimtype |= 64;
            }
            sub.subclass("AcDbOrdinateDimension");
            sub.pt(13, p13);
            sub.pt(14, p14);
        }
        21 | 22 => {
            let p13 = r.d.bd3()?;
            let p14 = r.d.bd3()?;
            def10 = r.d.bd3()?;
            let ext_rot = r.d.bd()?;
            sub.subclass("AcDbAlignedDimension");
            sub.pt(13, p13);
            sub.pt(14, p14);
            if code == 21 {
                let dim_rot = r.d.bd()?;
                sub.angle_deg(50, dim_rot);
                if ext_rot != 0.0 {
                    sub.angle_deg(52, ext_rot);
                }
                sub.subclass("AcDbRotatedDimension");
            } else {
                dimtype |= 1;
                if ext_rot != 0.0 {
                    sub.angle_deg(52, ext_rot);
                }
            }
        }
        23 => {
            def10 = r.d.bd3()?;
            let p13 = r.d.bd3()?;
            let p14 = r.d.bd3()?;
            let p15 = r.d.bd3()?;
            dimtype |= 5;
            sub.subclass("AcDb3PointAngularDimension");
            sub.pt(13, p13);
            sub.pt(14, p14);
            sub.pt(15, p15);
        }
        24 => {
            let p16 = r.d.rd2()?;
            let p13 = r.d.bd3()?;
            let p14 = r.d.bd3()?;
            let p15 = r.d.bd3()?;
            def10 = r.d.bd3()?;
            dimtype |= 2;
            sub.subclass("AcDb2LineAngularDimension");
            sub.pt(13, p13);
            sub.pt(14, p14);
            sub.pt(15, p15);
            sub.pt(16, Point3::new(p16.x, p16.y, elev));
        }
        25 | 26 => {
            let (p15, leader);
            if code == 25 {
                def10 = r.d.bd3()?;
                p15 = r.d.bd3()?;
            } else {
                p15 = r.d.bd3()?;
                def10 = r.d.bd3()?;
            }
            leader = r.d.bd()?;
            dimtype |= if code == 25 { 4 } else { 3 };
            sub.subclass(if code == 25 { "AcDbRadialDimension" } else { "AcDbDiametricDimension" });
            sub.pt(15, p15);
            sub.real(40, leader);
        }
        _ => unreachable!(),
    }
    let dimstyle = r.href_opt();
    let block = r.href_opt();

    g.subclass("AcDbDimension");
    if let Some(v) = version {
        g.int(280, v);
    }
    g.name(2, block, "");
    g.pt(10, def10);
    g.pt(11, Point3::new(text_mid.x, text_mid.y, elev));
    if pt12 != Point3::default() {
        g.pt(12, Point3::new(pt12.x, pt12.y, elev));
    }
    g.int(70, dimtype);
    if ver >= Version::R2000 {
        g.int(71, attach as i16);
        g.int(72, ls_style as i16);
        g.real(41, ls_factor);
        g.real(42, measurement);
    }
    if !user_text.is_empty() {
        g.str(1, user_text);
    }
    if text_rot != 0.0 {
        g.angle_deg(53, text_rot);
    }
    if horiz_dir != 0.0 {
        g.angle_deg(51, horiz_dir);
    }
    g.extrusion(extr);
    g.name(3, dimstyle, "Standard");
    g.0.extend(sub.0);
    Ok(())
}

/// LWPOLYLINE data decoder, also used for proxy graphics chunks.
pub(crate) fn lwpolyline_pub(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    lwpolyline(r, g)
}

fn lwpolyline(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let ver = r.ver;
    let flag = r.d.bs()?;
    let const_w = if flag & 4 != 0 { r.d.bd()? } else { 0.0 };
    let elev = if flag & 8 != 0 { r.d.bd()? } else { 0.0 };
    let thick = if flag & 2 != 0 { r.d.bd()? } else { 0.0 };
    let normal = if flag & 1 != 0 { r.d.bd3()? } else { Point3::Z };
    let n = r.d.bl()? as usize;
    let nbulges = if flag & 16 != 0 { r.d.bl()? as usize } else { 0 };
    let nids = if ver >= Version::R2010 && flag & 1024 != 0 { r.d.bl()? as usize } else { 0 };
    let nwidths = if flag & 32 != 0 { r.d.bl()? as usize } else { 0 };
    let mut pts = Vec::with_capacity(n.min(1 << 20));
    if ver < Version::R2000 {
        for _ in 0..n {
            pts.push(r.d.rd2()?);
        }
    } else if n > 0 {
        let mut p = r.d.rd2()?;
        pts.push(p);
        for _ in 1..n {
            p = r.d.dd2(p)?;
            pts.push(p);
        }
    }
    let mut bulges = Vec::with_capacity(nbulges.min(1 << 20));
    for _ in 0..nbulges {
        bulges.push(r.d.bd()?);
    }
    for _ in 0..nids {
        r.d.bl()?;
    }
    let mut widths = Vec::with_capacity(nwidths.min(1 << 20));
    for _ in 0..nwidths {
        widths.push((r.d.bd()?, r.d.bd()?));
    }

    let mut f70 = 0i16;
    if flag & 512 != 0 {
        f70 |= 1;
    }
    if flag & 256 != 0 {
        f70 |= 128;
    }
    g.subclass("AcDbPolyline");
    g.int(90, pts.len() as i32);
    g.int(70, f70);
    if flag & 4 != 0 {
        g.real(43, const_w);
    }
    if elev != 0.0 {
        g.real(38, elev);
    }
    if thick != 0.0 {
        g.real(39, thick);
    }
    for (i, p) in pts.iter().enumerate() {
        g.pt2(10, *p);
        if let Some((s, e)) = widths.get(i) {
            if *s != 0.0 || *e != 0.0 {
                g.real(40, *s);
                g.real(41, *e);
            }
        }
        if let Some(b) = bulges.get(i) {
            if *b != 0.0 {
                g.real(42, *b);
            }
        }
    }
    g.extrusion(normal);
    Ok(())
}

fn hatch(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let ver = r.ver;
    let mut gradient = Groups::default();
    if ver >= Version::R2004 {
        let is_gradient = r.d.bl()?;
        let reserved = r.d.bl()?;
        let angle = r.d.bd()?;
        let shift = r.d.bd()?;
        let single = r.d.bl()?;
        let tint = r.d.bd()?;
        let ncolors = r.d.bl()? as usize;
        let mut colors = vec![];
        for _ in 0..ncolors.min(1 << 16) {
            let _unk_d = r.d.bd()?;
            let _unk_s = r.d.bs()?;
            let rgb = r.d.bl()?;
            let _ = r.d.rc()?;
            colors.push(rgb);
        }
        let name = r.tv()?;
        if is_gradient != 0 {
            gradient.int(450, is_gradient as i32);
            gradient.int(451, reserved as i32);
            gradient.int(452, single as i32);
            gradient.int(453, colors.len() as i32);
            for (i, c) in colors.iter().enumerate() {
                gradient.real(463, i as f64);
                gradient.int(63, 256);
                gradient.int(421, (*c & 0xFF_FFFF) as i32);
            }
            gradient.angle_deg(460, angle);
            gradient.real(461, shift);
            gradient.real(462, tint);
            gradient.str(470, name);
        }
    }
    let z = r.d.bd()?;
    let extr = r.d.bd3()?;
    let name = r.tv()?;
    let solid = r.d.b()?;
    let assoc = r.d.b()?;
    let npaths = r.d.bl()? as usize;

    let mut paths = Groups::default();
    let mut nbound_total = 0usize;
    let mut any_pixel = false;
    for _ in 0..npaths {
        let pflag = r.d.bl()?;
        if pflag & 4 != 0 {
            any_pixel = true;
        }
        paths.int(92, pflag as i32);
        if pflag & 2 == 0 {
            let nsegs = r.d.bl()? as usize;
            paths.int(93, nsegs as i32);
            for _ in 0..nsegs {
                let t = r.d.rc()?;
                paths.int(72, t);
                match t {
                    1 => {
                        paths.pt2(10, r.d.rd2()?);
                        paths.pt2(11, r.d.rd2()?);
                    }
                    2 => {
                        paths.pt2(10, r.d.rd2()?);
                        paths.real(40, r.d.bd()?);
                        paths.angle_deg(50, r.d.bd()?);
                        paths.angle_deg(51, r.d.bd()?);
                        paths.int(73, r.d.b()? as i16);
                    }
                    3 => {
                        paths.pt2(10, r.d.rd2()?);
                        paths.pt2(11, r.d.rd2()?);
                        paths.real(40, r.d.bd()?);
                        paths.angle_deg(50, r.d.bd()?);
                        paths.angle_deg(51, r.d.bd()?);
                        paths.int(73, r.d.b()? as i16);
                    }
                    4 => {
                        let degree = r.d.bl()?;
                        let rational = r.d.b()?;
                        let periodic = r.d.b()?;
                        let nk = r.d.bl()? as usize;
                        let nc = r.d.bl()? as usize;
                        paths.int(94, degree as i32);
                        paths.int(73, rational as i16);
                        paths.int(74, periodic as i16);
                        paths.int(95, nk as i32);
                        paths.int(96, nc as i32);
                        for _ in 0..nk {
                            paths.real(40, r.d.bd()?);
                        }
                        for _ in 0..nc {
                            paths.pt2(10, r.d.rd2()?);
                            if rational {
                                paths.real(42, r.d.bd()?);
                            }
                        }
                        if ver >= Version::R2010 {
                            let nfit = r.d.bl()? as usize;
                            paths.int(97, nfit as i32);
                            if nfit > 0 {
                                for _ in 0..nfit {
                                    paths.pt2(11, r.d.rd2()?);
                                }
                                paths.pt2(12, r.d.rd2()?);
                                paths.pt2(13, r.d.rd2()?);
                            }
                        }
                    }
                    _ => {
                        return Err(crate::error::Error::Corrupt(format!("hatch edge type {t}")))
                    }
                }
            }
        } else {
            let bulges = r.d.b()?;
            let closed = r.d.b()?;
            let n = r.d.bl()? as usize;
            paths.int(72, bulges as i16);
            paths.int(73, closed as i16);
            paths.int(93, n as i32);
            for _ in 0..n {
                paths.pt2(10, r.d.rd2()?);
                if bulges {
                    paths.real(42, r.d.bd()?);
                }
            }
        }
        let nb = r.d.bl()? as usize;
        // boundary handles are in the handle stream; placeholder for count
        paths.int(97, nb as i32);
        paths.0.push((-1, Val::Int(nb as i64)));
        nbound_total += nb;
    }
    let style = r.d.bs()?;
    let ptype = r.d.bs()?;
    let mut pattern = Groups::default();
    if !solid {
        let angle = r.d.bd()?;
        let scale = r.d.bd()?;
        let double = r.d.b()?;
        let nlines = r.d.bs()? as usize;
        pattern.angle_deg(52, angle);
        pattern.real(41, scale);
        pattern.int(77, double as i16);
        pattern.int(78, nlines as i16);
        for _ in 0..nlines {
            pattern.angle_deg(53, r.d.bd()?);
            let base = r.d.bd2()?;
            let off = r.d.bd2()?;
            pattern.real(43, base.x);
            pattern.real(44, base.y);
            pattern.real(45, off.x);
            pattern.real(46, off.y);
            let nd = r.d.bs()? as usize;
            pattern.int(79, nd as i16);
            for _ in 0..nd {
                pattern.real(49, r.d.bd()?);
            }
        }
    }
    let pixel = if any_pixel { Some(r.d.bd()?) } else { None };
    let nseeds = r.d.bl()? as usize;
    let mut seeds = vec![];
    for _ in 0..nseeds.min(1 << 16) {
        seeds.push(r.d.rd2()?);
    }

    // boundary object handles
    let mut handles = Vec::with_capacity(crate::error::cap(nbound_total));
    // real files always store every boundary handle; running out means
    // corrupt counts (and must not expand into a huge group list)
    for _ in 0..nbound_total {
        handles.push(r.href()?);
    }
    let mut hi = handles.into_iter();
    let mut resolved = Groups::default();
    for (code, v) in paths.0 {
        if code == -1 {
            if let Val::Int(n) = v {
                for _ in 0..n {
                    resolved.handle(330, hi.next().unwrap_or(0));
                }
            }
        } else {
            resolved.0.push((code, v));
        }
    }

    g.subclass("AcDbHatch");
    g.pt(10, Point3::new(0.0, 0.0, z));
    g.pt(210, extr);
    g.str(2, name);
    g.int(70, solid as i16);
    g.int(71, assoc as i16);
    g.int(91, npaths as i32);
    g.0.extend(resolved.0);
    g.int(75, style as i16);
    g.int(76, ptype as i16);
    g.0.extend(pattern.0);
    if let Some(p) = pixel {
        g.real(47, p);
    }
    g.int(98, seeds.len() as i32);
    for s in seeds {
        g.pt2(10, s);
    }
    g.0.extend(gradient.0);
    Ok(())
}

fn leader(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let ver = r.ver;
    let _ = r.d.b()?;
    let annot_type = r.d.bs()?;
    let path_type = r.d.bs()?;
    let n = r.d.bl()? as usize;
    let mut pts = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        pts.push(r.d.bd3()?);
    }
    let _origin = r.d.bd3()?;
    let extr = r.d.bd3()?;
    let xdir = r.d.bd3()?;
    let off_block = r.d.bd3()?;
    let mut endproj = Point3::default();
    if ver >= Version::R14 {
        endproj = r.d.bd3()?;
    }
    if ver < Version::R2000 {
        let _dimgap = r.d.bd()?;
    }
    // R2010+ no longer stores the annotation box size
    let (box_h, box_w) = if ver < Version::R2010 { (r.d.bd()?, r.d.bd()?) } else { (0.0, 0.0) };
    let hook_xdir = r.d.b()?;
    let arrow = r.d.b()?;
    // followed by unknown BS, B, B (not needed; handles have their own stream)

    let annotation = r.href_opt();
    let dimstyle = r.href_opt();
    g.subclass("AcDbLeader");
    g.name(3, dimstyle, "Standard");
    g.int(71, arrow as i16);
    g.int(72, path_type as i16);
    g.int(73, annot_type as i16);
    g.int(74, hook_xdir as i16);
    g.int(75, 0);
    g.real(40, box_h);
    g.real(41, box_w);
    g.int(76, pts.len() as i16);
    for p in &pts {
        g.pt(10, *p);
    }
    if annotation != 0 {
        g.handle(340, annotation);
    }
    g.pt(210, extr);
    g.pt(211, xdir);
    g.pt(212, off_block);
    g.pt(213, endproj);
    Ok(())
}

fn tolerance(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    if r.ver < Version::R2000 {
        let _ = r.d.bs()?;
        let _height = r.d.bd()?;
        let _dimgap = r.d.bd()?;
    }
    let ins = r.d.bd3()?;
    let xdir = r.d.bd3()?;
    let extr = r.d.bd3()?;
    let text = r.tv()?;
    let dimstyle = r.href_opt();
    g.subclass("AcDbFcf");
    g.name(3, dimstyle, "Standard");
    g.pt(10, ins);
    g.str(1, text);
    g.extrusion(extr);
    g.pt(11, xdir);
    Ok(())
}

fn mline(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let scale = r.d.bd()?;
    let just = r.d.rc()?;
    let base = r.d.bd3()?;
    let extr = r.d.bd3()?;
    let openclosed = r.d.bs()?;
    let nlines = r.d.rc()? as usize;
    let nverts = r.d.bs()? as usize;
    let mut verts = Groups::default();
    for _ in 0..nverts {
        verts.pt(11, r.d.bd3()?);
        verts.pt(12, r.d.bd3()?);
        verts.pt(13, r.d.bd3()?);
        for _ in 0..nlines {
            let np = r.d.bs()? as usize;
            verts.int(74, np as i16);
            for _ in 0..np {
                verts.real(41, r.d.bd()?);
            }
            let nf = r.d.bs()? as usize;
            verts.int(75, nf as i16);
            for _ in 0..nf {
                verts.real(42, r.d.bd()?);
            }
        }
    }
    let style = r.href_opt();
    g.subclass("AcDbMline");
    g.name(2, style, "Standard");
    g.handle(340, style);
    g.real(40, scale);
    g.int(70, just as i16);
    g.int(71, openclosed as i16);
    g.int(72, nverts as i16);
    g.int(73, nlines as i16);
    g.pt(10, base);
    g.pt(210, extr);
    g.0.extend(verts.0);
    Ok(())
}

fn image(r: &mut ObjReader, g: &mut Groups, wipeout: bool) -> Result<()> {
    let clsver = r.d.bl()?;
    let p0 = r.d.bd3()?;
    let u = r.d.bd3()?;
    let v = r.d.bd3()?;
    let size = r.d.rd2()?;
    let props = r.d.bs()?;
    let clipping = r.d.b()?;
    let brightness = r.d.rc()?;
    let contrast = r.d.rc()?;
    let fade = r.d.rc()?;
    let mut clip_mode = None;
    if r.ver >= Version::R2010 {
        clip_mode = Some(r.d.b()?);
    }
    let clip_type = r.d.bs()?;
    let mut verts = vec![];
    if clip_type == 1 {
        verts.push(r.d.rd2()?);
        verts.push(r.d.rd2()?);
    } else {
        let n = r.d.bl()? as usize;
        for _ in 0..n.min(1 << 16) {
            verts.push(r.d.rd2()?);
        }
    }
    let imagedef = r.href_opt();
    let reactor = r.href_opt();
    g.subclass(if wipeout { "AcDbWipeout" } else { "AcDbRasterImage" });
    g.int(90, clsver as i32);
    g.pt(10, p0);
    g.pt(11, u);
    g.pt(12, v);
    g.real(13, size.x);
    g.real(23, size.y);
    if imagedef != 0 {
        g.handle(340, imagedef);
    }
    g.int(70, props as i16);
    g.int(280, clipping as i16);
    g.int(281, brightness);
    g.int(282, contrast);
    g.int(283, fade);
    if reactor != 0 {
        g.handle(360, reactor);
    }
    g.int(71, clip_type as i16);
    g.int(91, verts.len() as i32);
    for p in verts {
        g.pt2(14, p);
    }
    if let Some(m) = clip_mode {
        g.int(290, m as i16);
    }
    Ok(())
}

/// Placeholder values for VIEWPORT 68/69 (status, id); the DXF writer
/// assigns real ones per layout.
pub(crate) const VP_STATUS: i64 = -0x7FFF_0001;
pub(crate) const VP_ID: i64 = -0x7FFF_0002;

fn viewport(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let ver = r.ver;
    let center = r.d.bd3()?;
    let width = r.d.bd()?;
    let height = r.d.bd()?;
    g.subclass("AcDbViewport");
    g.pt(10, center);
    g.real(40, width);
    g.real(41, height);
    g.0.push((68, Val::Int(VP_STATUS)));
    g.0.push((69, Val::Int(VP_ID)));
    if ver < Version::R2000 {
        return Ok(());
    }
    let target = r.d.bd3()?;
    let dir = r.d.bd3()?;
    let twist = r.d.bd()?;
    let view_h = r.d.bd()?;
    let lens = r.d.bd()?;
    let front = r.d.bd()?;
    let back = r.d.bd()?;
    let snap_ang = r.d.bd()?;
    let view_center = r.d.rd2()?;
    let snap_base = r.d.rd2()?;
    let snap_sp = r.d.rd2()?;
    let grid_sp = r.d.rd2()?;
    let zoom = r.d.bs()?;
    let mut grid_major = None;
    if ver >= Version::R2007 {
        grid_major = Some(r.d.bs()?);
    }
    let nfrozen = r.d.bl()? as usize;
    let status = r.d.bl()?;
    let sheet = r.tv()?;
    let render = r.d.rc()?;
    let ucs_at_origin = r.d.b()?;
    let ucs_per_vp = r.d.b()?;
    let ucs_org = r.d.bd3()?;
    let ucs_x = r.d.bd3()?;
    let ucs_y = r.d.bd3()?;
    let ucs_elev = r.d.bd()?;
    let ortho = r.d.bs()?;
    let mut shade = None;
    if ver >= Version::R2004 {
        shade = Some(r.d.bs()?);
    }
    if ver >= Version::R2007 {
        let _def_lights = r.d.b()?;
        let _light_type = r.d.rc()?;
        let _brightness = r.d.bd()?;
        let _contrast = r.d.bd()?;
        let _ambient = r.cmc()?;
    }
    let mut frozen = Vec::with_capacity(nfrozen.min(1 << 12));
    for _ in 0..nfrozen.min(1 << 16) {
        frozen.push(r.href_opt());
    }
    let clip = r.href_opt();
    if ver == Version::R2000 {
        let _vpenthdr = r.href_opt();
    }
    let named_ucs = r.href_opt();
    let base_ucs = r.href_opt();

    g.pt2(12, view_center);
    g.pt2(13, snap_base);
    g.pt2(14, snap_sp);
    g.pt2(15, grid_sp);
    g.pt(16, dir);
    g.pt(17, target);
    g.real(42, lens);
    g.real(43, front);
    g.real(44, back);
    g.real(45, view_h);
    g.angle_deg(50, snap_ang);
    g.angle_deg(51, twist);
    g.int(72, zoom as i16);
    for h in frozen.into_iter().filter(|h| *h != 0) {
        g.handle(331, h);
    }
    g.int(90, status as i32);
    if clip != 0 {
        g.handle(340, clip);
    }
    g.str(1, sheet);
    g.int(281, render);
    g.int(71, ucs_per_vp as i16);
    g.int(74, ucs_at_origin as i16);
    g.pt(110, ucs_org);
    g.pt(111, ucs_x);
    g.pt(112, ucs_y);
    g.int(79, ortho as i16);
    g.real(146, ucs_elev);
    if named_ucs != 0 {
        g.handle(345, named_ucs);
    }
    if base_ucs != 0 {
        g.handle(346, base_ucs);
    }
    if let Some(s) = shade {
        g.int(170, s as i16);
    }
    if let Some(m) = grid_major {
        g.int(61, m as i16);
    }
    Ok(())
}

/// ACIS SAT text as DXF expects it: every printable non-space character
/// mapped to 0x9F - c (the same cipher DWG uses for version 1 data).
fn sat_crypt(line: &str) -> String {
    line.chars()
        .map(|c| match c as u32 {
            0x21..=0x7E => char::from_u32(0x9F - c as u32).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// One wireframe wire: points (with the optional per-wire transform applied).
fn acis_wire(r: &mut ObjReader) -> Result<Vec<Point3>> {
    let _ty = r.d.rc()?;
    let _sel = r.d.bl()?;
    let _color = r.d.bs()?;
    let _idx = r.d.bl()?;
    let n = r.d.bl()? as usize;
    let mut pts = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        pts.push(r.d.bd3()?);
    }
    if r.d.b()? {
        let x = r.d.bd3()?;
        let y = r.d.bd3()?;
        let z = r.d.bd3()?;
        let t = r.d.bd3()?;
        let s = r.d.bd()?;
        let _ = (r.d.b()?, r.d.b()?, r.d.b()?);
        for p in pts.iter_mut() {
            *p = Point3::new(
                (p.x * x.x + p.y * y.x + p.z * z.x) * s + t.x,
                (p.x * x.y + p.y * y.y + p.z * z.y) * s + t.y,
                (p.x * x.z + p.y * y.z + p.z * z.z) * s + t.z,
            );
        }
    }
    Ok(pts)
}

enum Acis {
    /// SAT text lines, already in the DXF cipher
    Sat(Vec<String>),
    /// binary SAB file
    Sab(Vec<u8>),
}

/// Read ACIS data in DWG format.
fn acis_data(r: &mut ObjReader) -> Result<Option<Acis>> {
    if r.d.b()? {
        return Ok(None); // empty (R2013+: data lives in the data storage section)
    }
    let _ = r.d.b()?;
    let version = r.d.bs()?;
    if version == 1 {
        let mut raw = Vec::new();
        loop {
            let n = r.d.bl()? as usize;
            if n == 0 {
                break;
            }
            raw.extend(r.d.bytes(n)?);
        }
        // already in the DXF cipher; just split into lines
        let text = crate::bits::decode_8bit(&raw.iter().map(|&b| if b == 0 { b'\n' } else { b }).collect::<Vec<_>>());
        let lines = text.split('\n').map(|l| l.trim_end_matches('\r').to_string()).filter(|l| !l.is_empty()).collect();
        return Ok(Some(Acis::Sat(lines)));
    }
    // version 2: plain SAT text or SAB binary, no length given
    let start = r.d.pos();
    let head = r.d.clone().bytes(15).unwrap_or_default();
    if head.starts_with(b"ACIS BinaryFile") {
        // find the SAB end marker
        let marker = b"End\x0E\x02of\x0E\x04ACIS\x0D\x04data";
        let mut buf = Vec::new();
        while r.d.remaining() >= 8 {
            buf.push(r.d.rc()?);
            if buf.ends_with(marker) {
                return Ok(Some(Acis::Sab(buf)));
            }
        }
        r.d.set_pos(start);
        return Ok(None);
    }
    let mut text = Vec::new();
    while r.d.remaining() >= 8 {
        let b = r.d.rc()?;
        text.push(b);
        if text.ends_with(b"End-of-ACIS-data") {
            // rest of the line
            while r.d.remaining() >= 8 {
                let c = r.d.rc()?;
                if c == b'\n' {
                    break;
                }
            }
            break;
        }
    }
    let s = crate::bits::decode_8bit(&text);
    Ok(Some(Acis::Sat(
        s.lines().map(|l| sat_crypt(l.trim_end_matches('\r'))).filter(|l| !l.is_empty()).collect(),
    )))
}

fn acis(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let sat = acis_data(r)?;
    let mut wires: Vec<Vec<Point3>> = vec![];
    // wireframe and silhouettes (for display when the ACIS data is binary)
    let wire_res = (|| -> Result<()> {
        if r.d.b()? {
            if r.d.b()? {
                let _pt = r.d.bd3()?;
            }
            let _iso = r.d.bl()?;
            if r.d.b()? {
                let n = r.d.bl()? as usize;
                for _ in 0..n.min(1 << 16) {
                    wires.push(acis_wire(r)?);
                }
                let nsil = r.d.bl()? as usize;
                for _ in 0..nsil.min(1 << 12) {
                    let _ = (r.d.bl()?, r.d.bd3()?, r.d.bd3()?, r.d.bd3()?, r.d.b()?);
                    let nw = r.d.bl()? as usize;
                    for _ in 0..nw.min(1 << 16) {
                        acis_wire(r)?;
                    }
                }
            }
        }
        Ok(())
    })();
    let wire_gfx = if wire_res.is_ok() && !wires.is_empty() { Some(wires_to_graphics(&wires)) } else { None };
    match sat {
        Some(Acis::Sat(lines)) if !lines.is_empty() => {
            let g = &mut o.groups;
            g.subclass("AcDbModelerGeometry");
            g.int(70, 1);
            for l in lines {
                let chars: Vec<char> = l.chars().collect();
                for (i, chunk) in chars.chunks(255).enumerate() {
                    g.str(if i == 0 { 1 } else { 3 }, chunk.iter().collect::<String>());
                }
            }
            if o.dxf_name == "3DSOLID" && r.ver >= Version::R2007 {
                g.subclass("AcDb3dSolid");
            }
        }
        Some(Acis::Sab(data)) => {
            // written to the ACDSDATA section (DXF R2013+); the wireframe is
            // the fallback for older DXF versions
            o.sab = Some(data);
            o.graphics = wire_gfx;
        }
        _ => {
            // no convertible ACIS: draw the stored wireframe instead
            o.kind = Kind::Unsupported;
            o.graphics = wire_gfx;
        }
    }
    Ok(())
}

/// Encode polylines as proxy graphics so the proxy converter draws them.
fn wires_to_graphics(wires: &[Vec<Point3>]) -> Vec<u8> {
    let mut out = vec![0u8; 8];
    let mut count = 0u32;
    for w in wires.iter().filter(|w| w.len() >= 2) {
        let size = 8 + 4 + 24 * w.len();
        out.extend((size as u32).to_le_bytes());
        out.extend(6u32.to_le_bytes());
        out.extend((w.len() as u32).to_le_bytes());
        for p in w {
            out.extend(p.x.to_le_bytes());
            out.extend(p.y.to_le_bytes());
            out.extend(p.z.to_le_bytes());
        }
        count += 1;
    }
    let total = out.len() as u32;
    out[0..4].copy_from_slice(&total.to_le_bytes());
    out[4..8].copy_from_slice(&count.to_le_bytes());
    out
}
