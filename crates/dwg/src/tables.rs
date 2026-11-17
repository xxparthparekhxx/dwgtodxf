//! Non-entity objects: symbol tables and their records, dictionaries,
//! XRECORD, placeholders.

use crate::error::Result;
use crate::model::*;
use crate::objects::ObjReader;
use crate::version::Version;

pub fn decode(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    match o.type_code {
        48 => control(r, o, TableKind::Block),
        50 => control(r, o, TableKind::Layer),
        52 => control(r, o, TableKind::Style),
        56 => control(r, o, TableKind::Ltype),
        60 => control(r, o, TableKind::View),
        62 => control(r, o, TableKind::Ucs),
        64 => control(r, o, TableKind::Vport),
        66 => control(r, o, TableKind::Appid),
        68 => control(r, o, TableKind::Dimstyle),
        70 => control(r, o, TableKind::VxTable),
        49 => block_header(r, o),
        51 => layer(r, o),
        53 => style(r, o),
        57 => ltype(r, o),
        61 => name_only(r, o, TableKind::View),
        63 => name_only(r, o, TableKind::Ucs),
        65 => name_only(r, o, TableKind::Vport),
        67 => appid(r, o),
        69 => name_only(r, o, TableKind::Dimstyle),
        71 => name_only(r, o, TableKind::VxTable),
        42 => dictionary(r, o, false),
        79 => xrecord(r, o),
        80 => Ok(()),
        82 => layout(r, o),
        73 => mlinestyle(r, o),
        _ => match o.dxf_name.as_str() {
            "ACDBDICTIONARYWDFLT" => dictionary(r, o, true),
            "MLINESTYLE" => mlinestyle(r, o),
            "LAYOUT" => layout(r, o),
            "PLOTSETTINGS" => {
                plot_settings(r, &mut o.groups)?;
                let _ = r.href_opt(); // plot view
                Ok(())
            }
            "IMAGEDEF" => {
                let g = &mut o.groups;
                let cls = r.d.bl()?;
                let size = r.d.rd2()?;
                let path = r.tv()?;
                let loaded = r.d.b()?;
                let units = r.d.rc()?;
                let px = r.d.rd2()?;
                g.subclass("AcDbRasterImageDef");
                g.int(90, cls as i32);
                g.str(1, path);
                g.pt2(10, size);
                g.pt2(11, px);
                g.int(280, loaded as i16);
                g.int(281, units);
                Ok(())
            }
            "IMAGEDEF_REACTOR" => {
                let cls = r.d.bl()?;
                o.groups.subclass("AcDbRasterImageDefReactor");
                o.groups.int(90, cls as i32);
                Ok(())
            }
            "RASTERVARIABLES" => {
                let cls = r.d.bl()?;
                let frame = r.d.bs()?;
                let quality = r.d.bs()?;
                let units = r.d.bs()?;
                let g = &mut o.groups;
                g.subclass("AcDbRasterVariables");
                g.int(90, cls as i32);
                g.int(70, frame as i16);
                g.int(71, quality as i16);
                g.int(72, units as i16);
                Ok(())
            }
            "WIPEOUTVARIABLES" => {
                let frame = r.d.bs()?;
                o.groups.subclass("AcDbWipeoutVariables");
                o.groups.int(70, frame as i16);
                Ok(())
            }
            "DICTIONARYVAR" => {
                let schema = r.d.rc()?;
                let value = r.tv()?;
                o.groups.subclass("DictionaryVariables");
                o.groups.int(280, schema);
                o.groups.str(1, value);
                Ok(())
            }
            "XRECORD" => xrecord(r, o),
            "ACDBPLACEHOLDER" => Ok(()),
            _ => {
                o.kind = Kind::Unsupported;
                Ok(())
            }
        },
    }
}

fn control(r: &mut ObjReader, o: &mut Object, kind: TableKind) -> Result<()> {
    let n = r.d.bs()? as usize;
    let mut entries = Vec::with_capacity(n);
    for _ in 0..n {
        entries.push(r.href()?);
    }
    let mut extra = vec![];
    match kind {
        // *MODEL_SPACE and *PAPER_SPACE
        TableKind::Block => {
            extra.push(r.href_opt());
            extra.push(r.href_opt());
        }
        // BYBLOCK and BYLAYER linetypes
        TableKind::Ltype => {
            extra.push(r.href_opt());
            extra.push(r.href_opt());
        }
        _ => {}
    }
    o.kind = Kind::Table { kind, entries, extra };
    Ok(())
}

/// Common table record prefix: name, 64-flag, xref index, xdep.
/// Returns (name, DXF 70 flags so far).
fn record_prefix(r: &mut ObjReader) -> Result<(String, i16)> {
    let name = r.tv()?;
    let mut flags = 0i16;
    if r.ver >= Version::R2007 {
        // R2007+: a single BS; 0x100 marks an xref-dependent record
        if r.d.bs()? & 0x100 != 0 {
            flags |= 16;
        }
        return Ok((name, flags));
    }
    if r.d.b()? {
        flags |= 64;
    }
    let _xrefindex = r.d.bs()?;
    if r.d.b()? {
        flags |= 16;
    }
    Ok((name, flags))
}

fn name_only(r: &mut ObjReader, o: &mut Object, kind: TableKind) -> Result<()> {
    let (name, flags) = record_prefix(r)?;
    o.groups.int(70, flags);
    o.kind = Kind::TableEntry { kind, name };
    Ok(())
}

fn appid(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let (name, flags) = record_prefix(r)?;
    o.groups.subclass("AcDbRegAppTableRecord");
    o.groups.str(2, name.clone());
    o.groups.int(70, flags);
    o.kind = Kind::TableEntry { kind: TableKind::Appid, name };
    Ok(())
}

fn block_header(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let ver = r.ver;
    let mut b = BlockHeader::default();
    let (name, mut flags) = record_prefix(r)?;
    b.name = name;
    let anonymous = r.d.b()?;
    let has_attrs = r.d.b()?;
    let is_xref = r.d.b()?;
    let overlaid = r.d.b()?;
    if anonymous {
        flags |= 1;
    }
    if has_attrs {
        flags |= 2;
    }
    if is_xref {
        flags |= 4;
    }
    if overlaid {
        flags |= 8;
    }
    if ver >= Version::R2000 {
        let _loaded = r.d.b()?;
    }
    let mut owned_count = 0;
    if ver >= Version::R2004 && !is_xref && !overlaid {
        owned_count = r.d.bl()? as usize;
    }
    b.base = r.d.bd3()?;
    b.xref_path = r.tv()?;
    let mut insert_count = 0;
    if ver >= Version::R2000 {
        loop {
            if r.d.rc()? == 0 {
                break;
            }
            insert_count += 1;
        }
        b.description = r.tv()?;
        let psize = r.d.bl()? as usize;
        r.d.skip_bits(psize * 8)?;
    }
    if ver >= Version::R2007 {
        b.insert_units = r.d.bs_i()?;
        b.explodable = r.d.b()?;
        b.scaling = r.d.rc()?;
    }
    b.flags = flags;

    let _xref = r.href()?;
    b.block_entity = r.href()?;
    if ver < Version::R2004 {
        if !is_xref && !overlaid {
            b.first_entity = r.href()?;
            b.last_entity = r.href()?;
        }
    } else {
        b.entities.reserve(crate::error::cap(owned_count));
        for _ in 0..owned_count {
            b.entities.push(r.href()?);
        }
    }
    b.endblk_entity = r.href()?;
    if ver >= Version::R2000 {
        for _ in 0..insert_count {
            r.href()?;
        }
        b.layout = r.href_opt();
    }
    o.kind = Kind::BlockHeader(Box::new(b));
    Ok(())
}

fn layer(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let ver = r.ver;
    let (name, mut flags) = record_prefix(r)?;
    let off;
    let (mut plot, mut lw) = (true, -3i16);
    if ver < Version::R2000 {
        if r.d.b()? {
            flags |= 1;
        }
        off = !r.d.b()?;
        if r.d.b()? {
            flags |= 2;
        }
        if r.d.b()? {
            flags |= 4;
        }
    } else {
        let v = r.d.bs()?;
        if v & 1 != 0 {
            flags |= 1;
        }
        off = v & 2 != 0;
        if v & 4 != 0 {
            flags |= 2;
        }
        if v & 8 != 0 {
            flags |= 4;
        }
        plot = v & 16 != 0;
        lw = crate::objects::lineweight(((v & 0x3E0) >> 5) as u8);
    }
    let color = r.cmc()?;
    let _xref = r.href()?;
    let mut plotstyle = 0;
    if ver >= Version::R2000 {
        plotstyle = r.href_opt();
    }
    if ver >= Version::R2007 {
        let _material = r.href_opt();
    }
    let ltype = r.href_opt();

    let g = &mut o.groups;
    g.subclass("AcDbLayerTableRecord");
    g.str(2, name.clone());
    g.int(70, flags);
    let idx = if color.index == 0 || color.index == 256 { 7 } else { color.index.abs() };
    g.int(62, if off { -idx } else { idx });
    if let Some(rgb) = color.rgb {
        g.int(420, rgb as i32);
    }
    g.name(6, ltype, "Continuous");
    if ver >= Version::R2000 {
        if !plot {
            g.int(290, 0);
        }
        g.int(370, lw);
        if plotstyle != 0 {
            g.handle(390, plotstyle);
        }
    }
    o.kind = Kind::TableEntry { kind: TableKind::Layer, name };
    Ok(())
}

fn style(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let (name, mut flags) = record_prefix(r)?;
    if r.d.b()? {
        flags |= 4;
    }
    if r.d.b()? {
        flags |= 1;
    }
    let fixed_h = r.d.bd()?;
    let width = r.d.bd()?;
    let oblique = r.d.bd()?;
    let gen = r.d.rc()?;
    let last_h = r.d.bd()?;
    let font = r.tv()?;
    let bigfont = r.tv()?;
    let g = &mut o.groups;
    g.subclass("AcDbTextStyleTableRecord");
    g.str(2, name.clone());
    g.int(70, flags);
    g.real(40, fixed_h);
    g.real(41, width);
    g.angle_deg(50, oblique);
    g.int(71, gen);
    g.real(42, last_h);
    g.str(3, font);
    g.str(4, bigfont);
    o.kind = Kind::TableEntry { kind: TableKind::Style, name };
    Ok(())
}

fn ltype(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let (name, flags) = record_prefix(r)?;
    let desc = r.tv()?;
    let len = r.d.bd()?;
    let align = r.d.rc()?;
    let n = r.d.rc()? as usize;
    let mut dashes = Vec::with_capacity(n);
    for _ in 0..n {
        let length = r.d.bd()?;
        let shape = r.d.bs()?;
        let xo = r.d.rd()?;
        let yo = r.d.rd()?;
        let scale = r.d.bd()?;
        let rot = r.d.bd()?;
        let sflag = r.d.bs()?;
        dashes.push((length, shape, xo, yo, scale, rot, sflag));
    }
    let g = &mut o.groups;
    g.subclass("AcDbLinetypeTableRecord");
    g.str(2, name.clone());
    g.int(70, flags);
    g.str(3, desc);
    g.int(72, if align == 0 { 65 } else { align as i16 });
    g.int(73, n as i16);
    g.real(40, len);
    // Complex dash elements (shapes, text) need the shape file / string
    // area; we keep the dash lengths, which is what renderers use.
    for d in &dashes {
        g.real(49, d.0);
        g.int(74, 0);
    }
    o.kind = Kind::TableEntry { kind: TableKind::Ltype, name };
    Ok(())
}

fn dictionary(r: &mut ObjReader, o: &mut Object, wdflt: bool) -> Result<()> {
    let ver = r.ver;
    let n = r.d.bl()? as usize;
    if ver == Version::R14 {
        let _ = r.d.rc()?;
    }
    let (mut cloning, mut hard_owner) = (1i16, false);
    if ver >= Version::R2000 {
        cloning = r.d.bs_i()?;
        hard_owner = r.d.rc()? != 0;
    }
    let mut names = Vec::with_capacity(n.min(1 << 20));
    for _ in 0..n {
        names.push(r.tv()?);
    }
    let mut entries = Vec::with_capacity(names.len());
    for name in names {
        entries.push((name, r.href()?));
    }
    let default = if wdflt { Some(r.href_opt()) } else { None };
    o.kind = Kind::Dictionary { entries, hard_owner, cloning, default };
    Ok(())
}

fn xrecord(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let ver = r.ver;
    let nbytes = r.d.bl()? as usize;
    let raw = r.d.bytes(nbytes)?;
    let mut cloning = 1i16;
    if ver >= Version::R2000 {
        cloning = r.d.bs_i()?;
    }
    let g = &mut o.groups;
    g.subclass("AcDbXrecord");
    g.int(280, cloning);
    let mut i = 0usize;
    let rd = |i: usize| raw.get(i..i + 8).map(|b| f64::from_le_bytes(b.try_into().unwrap()));
    while i + 2 <= raw.len() {
        let code = i16::from_le_bytes([raw[i], raw[i + 1]]);
        i += 2;
        match dxf_value_type(code) {
            VT::Str => {
                if ver >= Version::R2007 {
                    let Some(b) = raw.get(i..i + 2) else { break };
                    let n = u16::from_le_bytes([b[0], b[1]]) as usize;
                    i += 2;
                    let Some(s) = raw.get(i..i + 2 * n) else { break };
                    i += 2 * n;
                    let u: Vec<u16> =
                        s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                    g.str(code, String::from_utf16_lossy(&u));
                } else {
                    let Some(b) = raw.get(i..i + 3) else { break };
                    let n = u16::from_le_bytes([b[0], b[1]]) as usize;
                    i += 3;
                    let Some(s) = raw.get(i..i + n) else { break };
                    i += n;
                    g.str(code, crate::bits::decode_8bit(s));
                }
            }
            VT::Real => {
                let Some(v) = rd(i) else { break };
                i += 8;
                g.real(code, v);
            }
            VT::Point => {
                let (Some(x), Some(y), Some(z)) = (rd(i), rd(i + 8), rd(i + 16)) else { break };
                i += 24;
                g.pt(code, crate::bits::Point3::new(x, y, z));
            }
            VT::I8 => {
                let Some(&b) = raw.get(i) else { break };
                i += 1;
                g.int(code, b as i8);
            }
            VT::I16 => {
                let Some(b) = raw.get(i..i + 2) else { break };
                i += 2;
                g.int(code, i16::from_le_bytes([b[0], b[1]]));
            }
            VT::I32 => {
                let Some(b) = raw.get(i..i + 4) else { break };
                i += 4;
                g.int(code, i32::from_le_bytes(b.try_into().unwrap()));
            }
            VT::I64 => {
                let Some(b) = raw.get(i..i + 8) else { break };
                i += 8;
                g.int(code, i64::from_le_bytes(b.try_into().unwrap()));
            }
            VT::Handle => {
                let Some(b) = raw.get(i..i + 8) else { break };
                i += 8;
                g.handle(code, u64::from_le_bytes(b.try_into().unwrap()));
            }
            VT::Binary => {
                let Some(&n) = raw.get(i) else { break };
                i += 1;
                let Some(s) = raw.get(i..i + n as usize) else { break };
                i += n as usize;
                g.str(code, s.iter().map(|b| format!("{b:02X}")).collect::<String>());
            }
            VT::Bool => {
                let Some(&b) = raw.get(i) else { break };
                i += 1;
                g.int(code, b as i16);
            }
        }
    }
    Ok(())
}

enum VT {
    Str,
    Real,
    Point,
    I8,
    I16,
    I32,
    I64,
    Handle,
    Binary,
    Bool,
}

/// Value type of a DXF group code (as used in XRECORD data).
fn dxf_value_type(code: i16) -> VT {
    match code {
        0..=9 | 100 | 101 | 102 | 300..=309 | 410..=419 | 430..=439 | 470..=479 | 999..=1003 | 1006..=1009 => {
            VT::Str
        }
        10..=18 | 110..=112 | 1010..=1013 => VT::Point,
        20..=59 | 140..=149 | 210..=239 | 460..=469 | 1014..=1059 => VT::Real,
        60..=79 | 170..=179 | 270..=279 | 400..=409 | 1060..=1070 => VT::I16,
        90..=99 | 420..=429 | 440..=449 | 450..=459 | 1071 => VT::I32,
        160..=169 => VT::I64,
        280..=289 => VT::I8,
        290..=299 => VT::Bool,
        310..=319 | 1004 => VT::Binary,
        320..=369 | 390..=399 | 480..=481 | 1005 => VT::Handle,
        370..=389 => VT::I16,
        _ => VT::Str,
    }
}

fn mlinestyle(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let name = r.tv()?;
    let desc = r.tv()?;
    let f = r.d.bs()?;
    let fill = r.cmc()?;
    let a0 = r.d.bd()?;
    let a1 = r.d.bd()?;
    let n = r.d.rc()? as usize;
    let mut lines = Vec::with_capacity(n);
    for _ in 0..n {
        let off = r.d.bd()?;
        let color = r.cmc()?;
        let lt = if r.ver < Version::R2018 { Some(r.d.bs()?) } else { None };
        lines.push((off, color, lt));
    }
    // DWG -> DXF flag bits
    let map = [(1, 2), (2, 1), (16, 16), (32, 64), (64, 32), (256, 256), (512, 1024), (1024, 512)];
    let mut flags = 0i16;
    for (d, x) in map {
        if f & d != 0 {
            flags |= x;
        }
    }
    let mut lt_handles = vec![];
    if r.ver >= Version::R2018 {
        for _ in 0..n {
            lt_handles.push(r.href_opt());
        }
    }
    let g = &mut o.groups;
    g.subclass("AcDbMlineStyle");
    g.str(2, name);
    g.int(70, flags);
    g.str(3, desc);
    g.int(62, if fill.index == 0 || fill.index == 256 { 256 } else { fill.index });
    g.angle_deg(51, a0);
    g.angle_deg(52, a1);
    g.int(71, n as i16);
    for (i, (off, color, lt)) in lines.into_iter().enumerate() {
        g.real(49, off);
        g.int(62, color.index);
        match lt {
            Some(0x7FFF) | None if lt_handles.get(i).copied().unwrap_or(0) == 0 => g.str(6, "BYLAYER"),
            Some(0x7FFE) => g.str(6, "BYBLOCK"),
            Some(idx) => g.0.push((6, Val::LtypeIndex(idx))),
            None => g.name(6, lt_handles[i], "BYLAYER"),
        }
    }
    Ok(())
}

/// Plot settings shared by PLOTSETTINGS and LAYOUT (AcDbPlotSettings).
fn plot_settings(r: &mut ObjReader, g: &mut Groups) -> Result<()> {
    let ver = r.ver;
    let page_setup = r.tv()?;
    let printer = r.tv()?;
    let flags = r.d.bs()?;
    let (ml, mb, mr, mt) = (r.d.bd()?, r.d.bd()?, r.d.bd()?, r.d.bd()?);
    let (pw, ph) = (r.d.bd()?, r.d.bd()?);
    let paper = r.tv()?;
    let origin = r.d.bd2()?;
    let units = r.d.bs()?;
    let rotation = r.d.bs()?;
    let ptype = r.d.bs()?;
    let wmin = r.d.bd2()?;
    let wmax = r.d.bd2()?;
    let mut view_name = String::new();
    if ver < Version::R2004 {
        view_name = r.tv()?;
    }
    let num = r.d.bd()?;
    let den = r.d.bd()?;
    let style_sheet = r.tv()?;
    let scale_type = r.d.bs()?;
    let scale = r.d.bd()?;
    let img_origin = r.d.bd2()?;
    let mut shade = None;
    if ver >= Version::R2004 {
        shade = Some((r.d.bs()?, r.d.bs()?, r.d.bs()?));
    }
    g.subclass("AcDbPlotSettings");
    g.str(1, page_setup);
    g.str(2, printer);
    g.str(4, paper);
    g.str(6, view_name);
    g.real(40, ml);
    g.real(41, mb);
    g.real(42, mr);
    g.real(43, mt);
    g.real(44, pw);
    g.real(45, ph);
    g.real(46, origin.x);
    g.real(47, origin.y);
    g.real(48, wmin.x);
    g.real(49, wmin.y);
    g.real(140, wmax.x);
    g.real(141, wmax.y);
    g.real(142, num);
    g.real(143, den);
    g.int(70, flags as i16);
    g.int(72, units as i16);
    g.int(73, rotation as i16);
    g.int(74, ptype as i16);
    g.str(7, style_sheet);
    g.int(75, scale_type as i16);
    if let Some((mode, res, dpi)) = shade {
        g.int(76, mode as i16);
        g.int(77, res as i16);
        g.int(78, dpi as i16);
    }
    g.real(147, scale);
    g.real(148, img_origin.x);
    g.real(149, img_origin.y);
    Ok(())
}

fn layout(r: &mut ObjReader, o: &mut Object) -> Result<()> {
    let ver = r.ver;
    let mut g = Groups::default();
    plot_settings(r, &mut g)?;
    let name = r.tv()?;
    let tab = r.d.bl()?;
    let flags = r.d.bs()?;
    let ucs_org = r.d.bd3()?;
    let limmin = r.d.rd2()?;
    let limmax = r.d.rd2()?;
    let ins = r.d.bd3()?;
    let ucs_x = r.d.bd3()?;
    let ucs_y = r.d.bd3()?;
    let elev = r.d.bd()?;
    let ortho = r.d.bs()?;
    let extmin = r.d.bd3()?;
    let extmax = r.d.bd3()?;
    let mut nvp = 0usize;
    if ver >= Version::R2004 {
        // the spec says RL; files store a BL
        nvp = r.d.bl()? as usize;
    }
    if ver >= Version::R2004 {
        let _plot_view = r.href_opt();
    }
    if ver >= Version::R2007 {
        let _visual_style = r.href_opt();
    }
    let block = r.href_opt();
    let last_vp = r.href_opt();
    let base_ucs = r.href_opt();
    let named_ucs = r.href_opt();
    for _ in 0..nvp.min(1 << 12) {
        let _ = r.href_opt();
    }
    g.subclass("AcDbLayout");
    g.str(1, name);
    g.int(70, flags as i16);
    g.int(71, tab as i32);
    g.pt2(10, limmin);
    g.pt2(11, limmax);
    g.pt(12, ins);
    g.pt(14, extmin);
    g.pt(15, extmax);
    g.real(146, elev);
    g.pt(13, ucs_org);
    g.pt(16, ucs_x);
    g.pt(17, ucs_y);
    g.int(76, ortho as i16);
    g.handle(330, block);
    if last_vp != 0 {
        g.handle(331, last_vp);
    }
    if named_ucs != 0 {
        g.handle(345, named_ucs);
    }
    if base_ucs != 0 {
        g.handle(346, base_ucs);
    }
    o.groups = g;
    Ok(())
}
