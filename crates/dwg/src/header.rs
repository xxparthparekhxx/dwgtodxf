//! AcDb:Header — drawing header variables (spec chapter 9).
//!
//! The variables are one long, version-dependent bit stream; the table below
//! lists them in file order. Names starting with `_` are read but not kept.

use std::collections::HashMap;

use crate::bits::{BitReader, Color, Point3};
use crate::error::{corrupt, Result};
use crate::objects::ObjReader;
use crate::strings::string_stream;
use crate::version::Version;

const HEADER_SN: [u8; 16] = [
    0xCF, 0x7B, 0x1F, 0x23, 0xFD, 0xDE, 0x38, 0xA9, 0x5F, 0x7C, 0x68, 0xB8, 0x4E, 0x6D, 0x33, 0x5F,
];

#[derive(Debug, Clone, PartialEq)]
pub enum HVal {
    Int(i64),
    Real(f64),
    Point(Point3),
    Str(String),
    Handle(u64),
    Color(Color),
}

/// Decoded header variables, by name (without the `$`).
#[derive(Debug, Clone, Default)]
pub struct Header {
    pub vars: HashMap<&'static str, HVal>,
}

impl Header {
    pub fn int(&self, k: &str) -> Option<i64> {
        match self.vars.get(k)? {
            HVal::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn real(&self, k: &str) -> Option<f64> {
        match self.vars.get(k)? {
            HVal::Real(f) => Some(*f),
            _ => None,
        }
    }
    pub fn point(&self, k: &str) -> Option<Point3> {
        match self.vars.get(k)? {
            HVal::Point(p) => Some(*p),
            _ => None,
        }
    }
    pub fn handle(&self, k: &str) -> Option<u64> {
        match self.vars.get(k)? {
            HVal::Handle(h) => Some(*h),
            _ => None,
        }
    }
    pub fn str(&self, k: &str) -> Option<&str> {
        match self.vars.get(k)? {
            HVal::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum C {
    All,
    R13_14,
    Pre2004,
    R14p,
    R2000p,
    R2004p,
    R2007p,
    R2010p,
    R2013p,
}

impl C {
    fn ok(self, v: Version) -> bool {
        match self {
            C::All => true,
            C::R13_14 => v <= Version::R14,
            C::Pre2004 => v < Version::R2004,
            C::R14p => v >= Version::R14,
            C::R2000p => v >= Version::R2000,
            C::R2004p => v >= Version::R2004,
            C::R2007p => v >= Version::R2007,
            C::R2010p => v >= Version::R2010,
            C::R2013p => v >= Version::R2013,
        }
    }
}

#[derive(Clone, Copy)]
enum T {
    B,
    BS,
    BL,
    BLL,
    BD,
    RD2,
    BD3,
    RC,
    /// 8-bit text before R2007, else from the string stream
    TV,
    /// handle reference (handle stream on R2007+)
    H,
    /// HANDSEED: a handle that always sits in the data stream
    HSeed,
    CMC,
    /// two BLs: Julian day + milliseconds -> real Julian date
    Date,
}

use C::*;
use T::*;

#[rustfmt::skip]
const VARS: &[(C, T, &str)] = &[
    (R2013p, BLL, "_REQUIREDVERSIONS"),
    (All, BD, "_u1"), (All, BD, "_u2"), (All, BD, "_u3"), (All, BD, "_u4"),
    (All, TV, "_s1"), (All, TV, "_s2"), (All, TV, "_s3"), (All, TV, "_s4"),
    (All, BL, "_l1"), (All, BL, "_l2"),
    (R13_14, BS, "_s5"),
    (Pre2004, H, "_VPENTHDR"),
    (All, B, "DIMASO"), (All, B, "DIMSHO"),
    (R13_14, B, "_DIMSAV"),
    (All, B, "PLINEGEN"), (All, B, "ORTHOMODE"), (All, B, "REGENMODE"), (All, B, "FILLMODE"),
    (All, B, "QTEXTMODE"), (All, B, "PSLTSCALE"), (All, B, "LIMCHECK"),
    (R13_14, B, "BLIPMODE"),
    (R2004p, B, "_b1"),
    (All, B, "USRTIMER"), (All, B, "SKPOLY"), (All, B, "ANGDIR"), (All, B, "SPLFRAME"),
    (R13_14, B, "ATTREQ"), (R13_14, B, "ATTDIA"),
    (All, B, "MIRRTEXT"), (All, B, "WORLDVIEW"),
    (R13_14, B, "_WIREFRAME"),
    (All, B, "TILEMODE"), (All, B, "PLIMCHECK"), (All, B, "VISRETAIN"),
    (R13_14, B, "DELOBJ"),
    (All, B, "DISPSILH"), (All, B, "_PELLIPSE"), (All, BS, "PROXYGRAPHICS"),
    (R13_14, BS, "DRAGMODE"),
    (All, BS, "TREEDEPTH"), (All, BS, "LUNITS"), (All, BS, "LUPREC"), (All, BS, "AUNITS"), (All, BS, "AUPREC"),
    (R13_14, BS, "OSMODE"),
    (All, BS, "ATTMODE"),
    (R13_14, BS, "COORDS"),
    (All, BS, "PDMODE"),
    (R13_14, BS, "PICKSTYLE"),
    (R2004p, BL, "_l3"), (R2004p, BL, "_l4"), (R2004p, BL, "_l5"),
    (All, BS, "USERI1"), (All, BS, "USERI2"), (All, BS, "USERI3"), (All, BS, "USERI4"), (All, BS, "USERI5"),
    (All, BS, "SPLINESEGS"), (All, BS, "SURFU"), (All, BS, "SURFV"), (All, BS, "SURFTYPE"),
    (All, BS, "SURFTAB1"), (All, BS, "SURFTAB2"), (All, BS, "SPLINETYPE"), (All, BS, "SHADEDGE"),
    (All, BS, "SHADEDIF"), (All, BS, "UNITMODE"), (All, BS, "MAXACTVP"), (All, BS, "ISOLINES"),
    (All, BS, "CMLJUST"), (All, BS, "TEXTQLTY"),
    (All, BD, "LTSCALE"), (All, BD, "TEXTSIZE"), (All, BD, "TRACEWID"), (All, BD, "SKETCHINC"),
    (All, BD, "FILLETRAD"), (All, BD, "THICKNESS"), (All, BD, "ANGBASE"), (All, BD, "PDSIZE"),
    (All, BD, "PLINEWID"), (All, BD, "USERR1"), (All, BD, "USERR2"), (All, BD, "USERR3"),
    (All, BD, "USERR4"), (All, BD, "USERR5"), (All, BD, "CHAMFERA"), (All, BD, "CHAMFERB"),
    (All, BD, "CHAMFERC"), (All, BD, "CHAMFERD"), (All, BD, "FACETRES"), (All, BD, "CMLSCALE"),
    (All, BD, "CELTSCALE"),
    // the spec says R13-R18 only, but R2007+ files still store it
    (All, TV, "MENU"),
    (All, Date, "TDCREATE"), (All, Date, "TDUPDATE"),
    (R2004p, BL, "_l6"), (R2004p, BL, "_l7"), (R2004p, BL, "_l8"),
    (All, Date, "TDINDWG"), (All, Date, "TDUSRTIMER"),
    (All, CMC, "CECOLOR"),
    (All, HSeed, "HANDSEED"),
    (All, H, "CLAYER"), (All, H, "TEXTSTYLE"), (All, H, "CELTYPE"),
    (R2007p, H, "CMATERIAL"),
    (All, H, "DIMSTYLE"), (All, H, "CMLSTYLE"),
    (R2000p, BD, "PSVPSCALE"),
    (All, BD3, "PINSBASE"), (All, BD3, "PEXTMIN"), (All, BD3, "PEXTMAX"),
    (All, RD2, "PLIMMIN"), (All, RD2, "PLIMMAX"), (All, BD, "PELEVATION"),
    (All, BD3, "PUCSORG"), (All, BD3, "PUCSXDIR"), (All, BD3, "PUCSYDIR"), (All, H, "PUCSNAME"),
    (R2000p, H, "PUCSORTHOREF"), (R2000p, BS, "PUCSORTHOVIEW"), (R2000p, H, "PUCSBASE"),
    (R2000p, BD3, "PUCSORGTOP"), (R2000p, BD3, "PUCSORGBOTTOM"), (R2000p, BD3, "PUCSORGLEFT"),
    (R2000p, BD3, "PUCSORGRIGHT"), (R2000p, BD3, "PUCSORGFRONT"), (R2000p, BD3, "PUCSORGBACK"),
    (All, BD3, "INSBASE"), (All, BD3, "EXTMIN"), (All, BD3, "EXTMAX"),
    (All, RD2, "LIMMIN"), (All, RD2, "LIMMAX"), (All, BD, "ELEVATION"),
    (All, BD3, "UCSORG"), (All, BD3, "UCSXDIR"), (All, BD3, "UCSYDIR"), (All, H, "UCSNAME"),
    (R2000p, H, "UCSORTHOREF"), (R2000p, BS, "UCSORTHOVIEW"), (R2000p, H, "UCSBASE"),
    (R2000p, BD3, "UCSORGTOP"), (R2000p, BD3, "UCSORGBOTTOM"), (R2000p, BD3, "UCSORGLEFT"),
    (R2000p, BD3, "UCSORGRIGHT"), (R2000p, BD3, "UCSORGFRONT"), (R2000p, BD3, "UCSORGBACK"),
    (R2000p, TV, "DIMPOST"), (R2000p, TV, "DIMAPOST"),
    (R13_14, B, "DIMTOL"), (R13_14, B, "DIMLIM"), (R13_14, B, "DIMTIH"), (R13_14, B, "DIMTOH"),
    (R13_14, B, "DIMSE1"), (R13_14, B, "DIMSE2"), (R13_14, B, "DIMALT"), (R13_14, B, "DIMTOFL"),
    (R13_14, B, "DIMSAH"), (R13_14, B, "DIMTIX"), (R13_14, B, "DIMSOXD"), (R13_14, RC, "DIMALTD"),
    (R13_14, RC, "DIMZIN"), (R13_14, B, "DIMSD1"), (R13_14, B, "DIMSD2"), (R13_14, RC, "DIMTOLJ"),
    (R13_14, RC, "DIMJUST"), (R13_14, RC, "DIMFIT"), (R13_14, B, "DIMUPT"), (R13_14, RC, "DIMTZIN"),
    (R13_14, RC, "DIMALTZ"), (R13_14, RC, "DIMALTTZ"), (R13_14, RC, "DIMTAD"), (R13_14, BS, "DIMUNIT"),
    (R13_14, BS, "DIMAUNIT"), (R13_14, BS, "DIMDEC"), (R13_14, BS, "DIMTDEC"), (R13_14, BS, "DIMALTU"),
    (R13_14, BS, "DIMALTTD"), (R13_14, H, "DIMTXSTY"),
    (All, BD, "DIMSCALE"), (All, BD, "DIMASZ"), (All, BD, "DIMEXO"), (All, BD, "DIMDLI"),
    (All, BD, "DIMEXE"), (All, BD, "DIMRND"), (All, BD, "DIMDLE"), (All, BD, "DIMTP"), (All, BD, "DIMTM"),
    (R2007p, BD, "DIMFXL"), (R2007p, BD, "DIMJOGANG"), (R2007p, BS, "DIMTFILL"), (R2007p, CMC, "DIMTFILLCLR"),
    (R2000p, B, "DIMTOL"), (R2000p, B, "DIMLIM"), (R2000p, B, "DIMTIH"), (R2000p, B, "DIMTOH"),
    (R2000p, B, "DIMSE1"), (R2000p, B, "DIMSE2"), (R2000p, BS, "DIMTAD"), (R2000p, BS, "DIMZIN"),
    (R2000p, BS, "DIMAZIN"),
    (R2007p, BS, "DIMARCSYM"),
    (All, BD, "DIMTXT"), (All, BD, "DIMCEN"), (All, BD, "DIMTSZ"), (All, BD, "DIMALTF"),
    (All, BD, "DIMLFAC"), (All, BD, "DIMTVP"), (All, BD, "DIMTFAC"), (All, BD, "DIMGAP"),
    (R13_14, TV, "DIMPOST"), (R13_14, TV, "DIMAPOST"), (R13_14, TV, "_DIMBLK"),
    (R13_14, TV, "_DIMBLK1"), (R13_14, TV, "_DIMBLK2"),
    (R2000p, BD, "DIMALTRND"), (R2000p, B, "DIMALT"), (R2000p, BS, "DIMALTD"), (R2000p, B, "DIMTOFL"),
    (R2000p, B, "DIMSAH"), (R2000p, B, "DIMTIX"), (R2000p, B, "DIMSOXD"),
    (All, CMC, "DIMCLRD"), (All, CMC, "DIMCLRE"), (All, CMC, "DIMCLRT"),
    (R2000p, BS, "DIMADEC"), (R2000p, BS, "DIMDEC"), (R2000p, BS, "DIMTDEC"), (R2000p, BS, "DIMALTU"),
    (R2000p, BS, "DIMALTTD"), (R2000p, BS, "DIMAUNIT"), (R2000p, BS, "DIMFRAC"), (R2000p, BS, "DIMLUNIT"),
    (R2000p, BS, "DIMDSEP"), (R2000p, BS, "DIMTMOVE"), (R2000p, BS, "DIMJUST"), (R2000p, B, "DIMSD1"),
    (R2000p, B, "DIMSD2"), (R2000p, BS, "DIMTOLJ"), (R2000p, BS, "DIMTZIN"), (R2000p, BS, "DIMALTZ"),
    (R2000p, BS, "DIMALTTZ"), (R2000p, B, "DIMUPT"), (R2000p, BS, "DIMATFIT"),
    (R2007p, B, "DIMFXLON"),
    (R2010p, B, "DIMTXTDIRECTION"), (R2010p, BD, "DIMALTMZF"), (R2010p, TV, "DIMALTMZS"),
    (R2010p, BD, "DIMMZF"), (R2010p, TV, "DIMMZS"),
    (R2000p, H, "DIMTXSTY"), (R2000p, H, "DIMLDRBLK"), (R2000p, H, "DIMBLK"), (R2000p, H, "DIMBLK1"),
    (R2000p, H, "DIMBLK2"),
    (R2007p, H, "DIMLTYPE"), (R2007p, H, "DIMLTEX1"), (R2007p, H, "DIMLTEX2"),
    (R2000p, BS, "DIMLWD"), (R2000p, BS, "DIMLWE"),
    (All, H, "_BLOCK_CONTROL"), (All, H, "_LAYER_CONTROL"), (All, H, "_STYLE_CONTROL"),
    (All, H, "_LTYPE_CONTROL"), (All, H, "_VIEW_CONTROL"), (All, H, "_UCS_CONTROL"),
    (All, H, "_VPORT_CONTROL"), (All, H, "_APPID_CONTROL"), (All, H, "_DIMSTYLE_CONTROL"),
    (Pre2004, H, "_VPENTHDR_CONTROL"),
    (All, H, "_DICT_GROUP"), (All, H, "_DICT_MLINESTYLE"), (All, H, "_DICT_NOD"),
    (R2000p, BS, "_TSTACKALIGN"), (R2000p, BS, "_TSTACKSIZE"),
    (R2000p, TV, "HYPERLINKBASE"), (R2000p, TV, "STYLESHEET"),
    (R2000p, H, "_DICT_LAYOUTS"), (R2000p, H, "_DICT_PLOTSETTINGS"), (R2000p, H, "_DICT_PLOTSTYLES"),
    (R2004p, H, "_DICT_MATERIALS"), (R2004p, H, "_DICT_COLORS"),
    (R2007p, H, "_DICT_VISUALSTYLE"),
    (R2013p, H, "_u5"),
    (R2000p, BL, "_FLAGS"), (R2000p, BS, "INSUNITS"), (R2000p, BS, "CEPSNTYPE"),
    // CPSNID follows here when CEPSNTYPE == 3 (handled in code)
    (R2000p, TV, "FINGERPRINTGUID"), (R2000p, TV, "VERSIONGUID"),
    (R2004p, RC, "SORTENTS"), (R2004p, RC, "INDEXCTL"), (R2004p, RC, "HIDETEXT"),
    (R2004p, RC, "XCLIPFRAME"), (R2004p, RC, "DIMASSOC"), (R2004p, RC, "HALOGAP"),
    (R2004p, BS, "OBSCUREDCOLOR"), (R2004p, BS, "INTERSECTIONCOLOR"), (R2004p, RC, "OBSCUREDLTYPE"),
    (R2004p, RC, "INTERSECTIONDISPLAY"), (R2004p, TV, "PROJECTNAME"),
    (All, H, "_PAPER_SPACE"), (All, H, "_MODEL_SPACE"), (All, H, "_BYLAYER"), (All, H, "_BYBLOCK"),
    (All, H, "_CONTINUOUS"),
    (R2007p, B, "CAMERADISPLAY"), (R2007p, BL, "_l9"), (R2007p, BL, "_l10"), (R2007p, BD, "_d1"),
    (R2007p, BD, "STEPSPERSEC"), (R2007p, BD, "STEPSIZE"), (R2007p, BD, "3DDWFPREC"),
    (R2007p, BD, "LENSLENGTH"), (R2007p, BD, "CAMERAHEIGHT"), (R2007p, RC, "SOLIDHIST"),
    (R2007p, RC, "SHOWHIST"), (R2007p, BD, "PSOLWIDTH"), (R2007p, BD, "PSOLHEIGHT"),
    (R2007p, BD, "LOFTANG1"), (R2007p, BD, "LOFTANG2"), (R2007p, BD, "LOFTMAG1"),
    (R2007p, BD, "LOFTMAG2"), (R2007p, BS, "LOFTPARAM"), (R2007p, RC, "LOFTNORMALS"),
    (R2007p, BD, "LATITUDE"), (R2007p, BD, "LONGITUDE"), (R2007p, BD, "NORTHDIRECTION"),
    (R2007p, BL, "TIMEZONE"), (R2007p, RC, "LIGHTGLYPHDISPLAY"), (R2007p, RC, "TILEMODELIGHTSYNCH"),
    (R2007p, RC, "DWFFRAME"), (R2007p, RC, "DGNFRAME"), (R2007p, B, "_b2"),
    (R2007p, CMC, "INTERFERECOLOR"), (R2007p, H, "_INTERFEREOBJVS"), (R2007p, H, "_INTERFEREVPVS"),
    (R2007p, H, "_DRAGVS"), (R2007p, RC, "CSHADOW"), (R2007p, BD, "_d2"),
    (R14p, BS, "_s6"), (R14p, BS, "_s7"), (R14p, BS, "_s8"), (R14p, BS, "_s9"),
];

/// Parse the header section. Unknown or truncated data yields the variables
/// read so far (an incomplete header is not fatal).
pub fn read_header(data: &[u8], ver: Version, maint: u8) -> Result<Header> {
    let mut h = Header::default();
    if data.len() < 24 {
        return Ok(h);
    }
    let mut r = BitReader::new(data, ver);
    if !r.sentinel(&HEADER_SN)? {
        return corrupt("header sentinel not found");
    }
    let size = r.rl()? as usize;
    if (ver >= Version::R2010 && maint > 3) || ver >= Version::R2018 {
        let _hi = r.rl()?;
    }
    let data_start = r.pos();
    let mut end = (data_start + size * 8).min(data.len() * 8);
    let mut strings = None;
    let mut handles_at = None;
    if ver >= Version::R2007 {
        let bitsize = r.rl()? as usize;
        let endbit = data_start + bitsize;
        strings = string_stream(data, ver, endbit);
        handles_at = Some(endbit);
        end = end.min(endbit);
    }
    r.set_end(end);
    let mut hr = BitReader::new(data, ver);
    if let Some(p) = handles_at {
        hr.set_pos(p);
        hr.set_end((data_start + size * 8).min(data.len() * 8));
    }
    let mut o = ObjReader { d: r, s: strings, h: hr, handle: 0, ver };
    let _ = parse(&mut o, ver, handles_at.is_some(), &mut h);
    Ok(h)
}

fn parse(o: &mut ObjReader, ver: Version, split: bool, h: &mut Header) -> Result<()> {
    for &(cond, ty, name) in VARS {
        if !cond.ok(ver) {
            continue;
        }
        let v = match ty {
            B => HVal::Int(o.d.b()? as i64),
            BS => HVal::Int(o.d.bs_i()? as i64),
            BL => HVal::Int(o.d.bl()? as i64),
            BLL => HVal::Int(o.d.bll()? as i64),
            BD => HVal::Real(o.d.bd()?),
            RD2 => HVal::Point(o.d.rd2()?),
            BD3 => HVal::Point(o.d.bd3()?),
            RC => HVal::Int(o.d.rc()? as i64),
            TV => HVal::Str(o.tv()?),
            H => HVal::Handle(if split { o.h.h()?.value } else { o.d.h()?.value }),
            HSeed => HVal::Handle(o.d.h()?.value),
            CMC => HVal::Color(o.cmc()?),
            Date => {
                let day = o.d.bl()? as f64;
                let ms = o.d.bl()? as f64;
                HVal::Real(day + ms / 86_400_000.0)
            }
        };
        if name == "CEPSNTYPE" && v == HVal::Int(3) {
            let ps = if split { o.h.h()?.value } else { o.d.h()?.value };
            h.vars.insert("CPSNID", HVal::Handle(ps));
        }
        if name == "_FLAGS" {
            if let HVal::Int(f) = v {
                h.vars.insert("CELWEIGHT", HVal::Int(crate::objects::lineweight((f & 0x1F) as u8) as i64));
                h.vars.insert("ENDCAPS", HVal::Int((f & 0x60) >> 5));
                h.vars.insert("JOINSTYLE", HVal::Int((f & 0x180) >> 7));
                h.vars.insert("LWDISPLAY", HVal::Int((f & 0x200 == 0) as i64));
                h.vars.insert("XEDIT", HVal::Int((f & 0x400 == 0) as i64));
                h.vars.insert("EXTNAMES", HVal::Int((f & 0x800 != 0) as i64));
                h.vars.insert("PSTYLEMODE", HVal::Int((f & 0x2000 != 0) as i64));
                h.vars.insert("OLESTARTUP", HVal::Int((f & 0x4000 != 0) as i64));
            }
        }
        if !name.starts_with('_') {
            h.vars.insert(name, v);
        }
    }
    Ok(())
}
