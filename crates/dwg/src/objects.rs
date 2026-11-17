//! Object framing: common object/entity data, the three per-object streams
//! (data, strings, handles) and dispatch to type decoders.

use crate::bits::{BitReader, Color, Point3};
use crate::classes::Class;
use crate::error::{Error, Result};
use crate::model::*;
use crate::strings::string_stream;
use crate::version::Version;
use crate::{entities, tables};

/// Reader over one object: data stream, string stream (R2007+), handle stream.
pub struct ObjReader<'a> {
    pub d: BitReader<'a>,
    pub s: Option<BitReader<'a>>,
    pub h: BitReader<'a>,
    pub handle: u64,
    pub ver: Version,
}

impl<'a> ObjReader<'a> {
    /// TV: text, from the string stream on R2007+.
    pub fn tv(&mut self) -> Result<String> {
        if self.ver >= Version::R2007 {
            // objects with text fields always carry a string stream; without
            // one the data is corrupt (and loops over counts must not spin)
            match self.s.as_mut() {
                Some(s) => s.tu(),
                None => Err(Error::Eof),
            }
        } else {
            self.d.t8()
        }
    }

    /// Next handle from the handle stream, resolved to absolute.
    #[inline]
    pub fn href(&mut self) -> Result<u64> {
        self.h.ha(self.handle)
    }

    /// Like `href` but returns 0 if the handle stream is exhausted.
    pub fn href_opt(&mut self) -> u64 {
        self.href().unwrap_or(0)
    }

    /// CMC colour (table entries, MTEXT background, ...).
    pub fn cmc(&mut self) -> Result<Color> {
        let index = self.d.bs_i()?;
        let mut c = Color { index, ..Default::default() };
        if self.ver < Version::R2004 {
            return Ok(c);
        }
        let rgb = self.d.bl()?;
        let flag = self.d.rc()?;
        match rgb >> 24 {
            0xC2 => c.rgb = Some(rgb & 0xFF_FFFF),
            0xC3 => c.index = (rgb & 0xFF) as i16,
            0xC0 => c.index = 256,
            0xC1 => c.index = 0,
            _ => {}
        }
        let mut name = String::new();
        if flag & 1 != 0 {
            name = self.tv()?;
        }
        if flag & 2 != 0 {
            let book = self.tv()?;
            name = format!("{book}${name}");
        }
        if !name.is_empty() {
            c.name = Some(name);
        }
        Ok(c)
    }

    /// ENC: entity colour (R2004+), or BS index before.
    fn enc(&mut self) -> Result<Color> {
        if self.ver < Version::R2004 {
            return Ok(Color { index: self.d.bs_i()?, ..Default::default() });
        }
        let v = self.d.bs()?;
        let flags = v & 0xFF00;
        let mut c = Color { index: (v & 0x1FF) as i16, ..Default::default() };
        if flags & 0x8000 != 0 {
            let rgb = self.d.bl()?;
            c.rgb = Some(rgb & 0xFF_FFFF);
            c.index = match rgb >> 24 {
                0xC3 => (rgb & 0xFF) as i16,
                0xC0 => 256,
                0xC1 => 0,
                _ => c.index & 0xFF,
            };
            if rgb >> 24 != 0xC2 {
                c.rgb = None;
            }
        }
        if flags & 0x4000 != 0 {
            c.has_book_ref = true;
        }
        if flags & 0x2000 != 0 {
            c.transparency = Some(self.d.bl()?);
        }
        Ok(c)
    }
}

/// What kind of object a type code denotes.
pub struct TypeInfo {
    pub dxf_name: String,
    pub is_entity: bool,
}

pub fn type_info(code: u16, classes: &[Class]) -> TypeInfo {
    let (name, ent) = match code {
        1 => ("TEXT", true),
        2 => ("ATTRIB", true),
        3 => ("ATTDEF", true),
        4 => ("BLOCK", true),
        5 => ("ENDBLK", true),
        6 => ("SEQEND", true),
        7 => ("INSERT", true),
        8 => ("INSERT", true), // MINSERT is an INSERT in DXF
        10..=14 => ("VERTEX", true),
        15 | 16 | 29 | 30 => ("POLYLINE", true),
        17 => ("ARC", true),
        18 => ("CIRCLE", true),
        19 => ("LINE", true),
        20..=26 => ("DIMENSION", true),
        27 => ("POINT", true),
        28 => ("3DFACE", true),
        31 => ("SOLID", true),
        32 => ("TRACE", true),
        33 => ("SHAPE", true),
        34 => ("VIEWPORT", true),
        35 => ("ELLIPSE", true),
        36 => ("SPLINE", true),
        37 => ("REGION", true),
        38 => ("3DSOLID", true),
        39 => ("BODY", true),
        40 => ("RAY", true),
        41 => ("XLINE", true),
        42 => ("DICTIONARY", false),
        43 => ("OLEFRAME", true),
        44 => ("MTEXT", true),
        45 => ("LEADER", true),
        46 => ("TOLERANCE", true),
        47 => ("MLINE", true),
        48 => ("BLOCK_CONTROL", false),
        49 => ("BLOCK_RECORD", false),
        50 => ("LAYER_CONTROL", false),
        51 => ("LAYER", false),
        52 => ("STYLE_CONTROL", false),
        53 => ("STYLE", false),
        56 => ("LTYPE_CONTROL", false),
        57 => ("LTYPE", false),
        60 => ("VIEW_CONTROL", false),
        61 => ("VIEW", false),
        62 => ("UCS_CONTROL", false),
        63 => ("UCS", false),
        64 => ("VPORT_CONTROL", false),
        65 => ("VPORT", false),
        66 => ("APPID_CONTROL", false),
        67 => ("APPID", false),
        68 => ("DIMSTYLE_CONTROL", false),
        69 => ("DIMSTYLE", false),
        70 => ("VX_CONTROL", false),
        71 => ("VX_TABLE_RECORD", false),
        72 => ("GROUP", false),
        73 => ("MLINESTYLE", false),
        74 => ("OLE2FRAME", true),
        75 => ("DUMMY", false),
        76 => ("LONG_TRANSACTION", false),
        77 => ("LWPOLYLINE", true),
        78 => ("HATCH", true),
        79 => ("XRECORD", false),
        80 => ("ACDBPLACEHOLDER", false),
        81 => ("VBA_PROJECT", false),
        82 => ("LAYOUT", false),
        0x1F2 => ("ACAD_PROXY_ENTITY", true),
        0x1F3 => ("ACAD_PROXY_OBJECT", false),
        _ => {
            if code >= 500 {
                if let Some(c) = classes.iter().find(|c| c.number == code) {
                    return TypeInfo { dxf_name: c.dxf_name.clone(), is_entity: c.is_entity() };
                }
            }
            return TypeInfo { dxf_name: format!("UNKNOWN_{code}"), is_entity: false };
        }
    };
    TypeInfo { dxf_name: name.to_string(), is_entity: ent }
}

/// DXF 370 lineweight from the DWG lineweight index.
pub fn lineweight(raw: u8) -> i16 {
    const LW: [i16; 24] =
        [0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158, 200, 211];
    match raw {
        0..=23 => LW[raw as usize],
        29 => -1,
        30 => -2,
        31 => -3,
        _ => -3,
    }
}

fn read_eed(r: &mut ObjReader) -> Result<Vec<XData>> {
    let mut out = Vec::new();
    loop {
        let size = r.d.bs()? as usize;
        if size == 0 {
            break;
        }
        let app = r.d.h()?.value;
        let raw = r.d.bytes(size)?;
        out.push(XData { app, items: parse_eed(&raw, r.ver) });
    }
    Ok(out)
}

fn parse_eed(raw: &[u8], ver: Version) -> Vec<XValue> {
    let mut items = Vec::new();
    let mut i = 0usize;
    let rd = |i: usize| raw.get(i..i + 8).map(|b| f64::from_le_bytes(b.try_into().unwrap()));
    while i < raw.len() {
        let code = raw[i] as i16;
        i += 1;
        let item = match code {
            0 => {
                if ver >= Version::R2007 {
                    let Some(b) = raw.get(i..i + 2) else { break };
                    let n = u16::from_le_bytes([b[0], b[1]]) as usize;
                    i += 2;
                    let Some(s) = raw.get(i..i + 2 * n) else { break };
                    i += 2 * n;
                    let u: Vec<u16> =
                        s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                    XValue::Str(String::from_utf16_lossy(&u))
                } else {
                    let Some(&n) = raw.get(i) else { break };
                    i += 3;
                    let Some(s) = raw.get(i..i + n as usize) else { break };
                    i += n as usize;
                    XValue::Str(crate::bits::decode_8bit(s))
                }
            }
            2 => {
                let Some(&b) = raw.get(i) else { break };
                i += 1;
                XValue::Brace(b == 1)
            }
            3 | 5 => {
                let Some(b) = raw.get(i..i + 8) else { break };
                i += 8;
                // stored most significant byte first
                let h = u64::from_be_bytes(b.try_into().unwrap());
                if code == 3 {
                    XValue::LayerRef(h)
                } else {
                    XValue::Handle(h)
                }
            }
            4 => {
                let Some(&n) = raw.get(i) else { break };
                i += 1;
                let Some(s) = raw.get(i..i + n as usize) else { break };
                i += n as usize;
                XValue::Binary(s.to_vec())
            }
            10..=13 => {
                let (Some(x), Some(y), Some(z)) = (rd(i), rd(i + 8), rd(i + 16)) else { break };
                i += 24;
                XValue::Point(1000 + code, Point3::new(x, y, z))
            }
            40..=42 => {
                let Some(v) = rd(i) else { break };
                i += 8;
                XValue::Real(1000 + code, v)
            }
            70 => {
                let Some(b) = raw.get(i..i + 2) else { break };
                i += 2;
                XValue::Short(i16::from_le_bytes([b[0], b[1]]))
            }
            71 => {
                let Some(b) = raw.get(i..i + 4) else { break };
                i += 4;
                XValue::Long(i32::from_le_bytes(b.try_into().unwrap()))
            }
            _ => break,
        };
        items.push(item);
    }
    items
}

/// Decode the object starting at byte `offset` of `buf`.
pub fn read_object(buf: &[u8], offset: usize, ver: Version, classes: &[Class]) -> Result<Object> {
    let mut r0 = BitReader::new(buf, ver);
    r0.set_pos(offset.checked_mul(8).ok_or(Error::Eof)?);
    let size = r0.ms()? as usize;
    // R2010+: handle stream size, then `size` bytes of object data
    let hsize = if ver >= Version::R2010 { r0.umc()? as usize } else { 0 };
    let start = r0.pos();
    let end = start.saturating_add(size.saturating_mul(8));
    if end > buf.len() * 8 {
        return Err(Error::Corrupt("object extends past end of data".into()));
    }
    r0.set_end(end);
    let type_code = r0.ot()?;
    let info = type_info(type_code, classes);

    // start of the handle stream (absolute bit position), if known now
    let mut handle_start = if ver >= Version::R2010 {
        Some(end.saturating_sub(hsize))
    } else if ver >= Version::R2000 {
        Some(start + r0.rl()? as usize)
    } else {
        None
    };

    let handle = r0.h()?.value;
    let mut r = ObjReader {
        d: r0,
        s: None,
        h: BitReader::with_end(buf, ver, end),
        handle,
        ver,
    };
    let xdata = read_eed(&mut r)?;

    let mut obj = Object {
        handle,
        type_code,
        dxf_name: info.dxf_name,
        owner: 0,
        reactors: vec![],
        xdict: 0,
        xdata,
        entity: None,
        kind: Kind::Plain,
        groups: Groups::default(),
        error: None,
        graphics: None,
        sab: None,
    };

    let mut ent = EntityCommon::default();
    let numreactors;
    let xdic_missing;
    let mut isbylayerlt = false;
    let mut nolinks = true;
    let mut graphics = None;
    if info.is_entity {
        if r.d.b()? {
            let gsize = if ver >= Version::R2010 { r.d.bll()? } else { r.d.rl()? as u64 };
            graphics = Some(r.d.bytes(gsize as usize)?);
        }
        if ver < Version::R2000 {
            handle_start = Some(start + r.d.rl()? as usize);
        }
    } else if ver < Version::R2000 {
        handle_start = Some(start + r.d.rl()? as usize);
    }
    let hstart = handle_start.unwrap();
    if hstart > end {
        return Err(Error::Corrupt("handle stream outside object".into()));
    }
    r.d.set_end(hstart);
    r.h.set_pos(hstart);
    if ver >= Version::R2007 {
        r.s = string_stream(buf, ver, hstart);
        // the string stream sits at the end of the data stream
        if let Some(s) = &r.s {
            r.d.set_end(s.pos());
        }
    }

    if info.is_entity {
        ent.entmode = r.d.bb()?;
        numreactors = r.d.bl()? as usize;
        xdic_missing = ver >= Version::R2004 && r.d.b()?;
        if ver >= Version::R2013 {
            let _has_ds = r.d.b()?;
        }
        if ver < Version::R2000 {
            isbylayerlt = r.d.b()?;
        }
        if ver < Version::R2004 {
            nolinks = r.d.b()?;
        }
        ent.color = r.enc()?;
        ent.ltype_scale = r.d.bd()?;
        if ver >= Version::R2000 {
            ent.ltype_flags = r.d.bb()?;
            ent.plotstyle_flags = r.d.bb()?;
        } else {
            ent.ltype_flags = if isbylayerlt { 0 } else { 3 };
        }
        let mut vis = (false, false, false);
        if ver >= Version::R2007 {
            ent.material_flags = r.d.bb()?;
            let _shadow = r.d.rc()?;
        }
        if ver >= Version::R2010 {
            vis = (r.d.b()?, r.d.b()?, r.d.b()?);
        }
        ent.invisible = r.d.bs()? & 1 != 0;
        ent.lineweight = if ver >= Version::R2000 { lineweight(r.d.rc()?) } else { -1 };

        // common entity handles
        if ent.entmode == 0 {
            obj.owner = r.href()?;
        }
        for _ in 0..numreactors {
            obj.reactors.push(r.href()?);
        }
        if !xdic_missing {
            obj.xdict = r.href()?;
        }
        if ver < Version::R2000 {
            ent.layer = r.href()?;
            if !isbylayerlt {
                ent.ltype = r.href()?;
            }
        }
        if ver < Version::R2004 && !nolinks {
            ent.prev = r.href()?;
            ent.next = r.href()?;
        } else if nolinks {
            ent.prev = handle.wrapping_sub(1);
            ent.next = handle + 1;
        }
        if ver >= Version::R2004 && ent.color.has_book_ref {
            let _book = r.href()?;
        }
        if ver >= Version::R2000 {
            ent.layer = r.href()?;
            if ent.ltype_flags == 3 {
                ent.ltype = r.href()?;
            }
        }
        if ver >= Version::R2007 && ent.material_flags == 3 {
            ent.material = r.href()?;
        }
        if ver >= Version::R2000 && ent.plotstyle_flags == 3 {
            ent.plotstyle = r.href()?;
        }
        if ver >= Version::R2010 {
            if vis.0 {
                r.href()?;
            }
            if vis.1 {
                r.href()?;
            }
            if vis.2 {
                r.href()?;
            }
        }
        obj.entity = Some(ent);
    } else {
        numreactors = r.d.bl()? as usize;
        xdic_missing = ver >= Version::R2004 && r.d.b()?;
        if ver >= Version::R2013 {
            let _has_ds = r.d.b()?;
        }
        obj.owner = r.href()?;
        for _ in 0..numreactors {
            obj.reactors.push(r.href()?);
        }
        if !xdic_missing {
            obj.xdict = r.href()?;
        }
    }

    obj.graphics = graphics;
    let res = if info.is_entity {
        entities::decode(&mut r, &mut obj)
    } else {
        tables::decode(&mut r, &mut obj)
    };
    if let Err(e) = res {
        obj.error = Some(e.to_string());
    }
    Ok(obj)
}
