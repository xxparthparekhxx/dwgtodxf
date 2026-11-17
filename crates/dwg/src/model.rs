//! In-memory representation of decoded DWG objects.
//!
//! Object-specific data is kept as DXF group codes (`groups`), which is what
//! the DXF writer emits and what most consumers want anyway. Structural data
//! needed to rebuild the drawing (owners, block contents, table entries,
//! dictionaries) is kept in typed fields.

use crate::bits::{Color, Point3};
use crate::classes::Class;
use crate::version::Version;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Int(i64),
    Real(f64),
    Str(String),
    /// A handle, written as hex.
    Handle(u64),
    /// The name of the table record with this handle, or the default if unknown.
    NameOf(u64, &'static str),
    /// A linetype referenced by its index in the LTYPE table.
    LtypeIndex(u16),
}

pub type Group = (i16, Val);

#[derive(Debug, Clone, Default)]
pub struct Groups(pub Vec<Group>);

impl Groups {
    #[inline]
    pub fn int(&mut self, code: i16, v: impl Into<i64>) {
        self.0.push((code, Val::Int(v.into())));
    }
    #[inline]
    pub fn real(&mut self, code: i16, v: f64) {
        self.0.push((code, Val::Real(v)));
    }
    #[inline]
    pub fn str(&mut self, code: i16, v: impl Into<String>) {
        self.0.push((code, Val::Str(v.into())));
    }
    #[inline]
    pub fn handle(&mut self, code: i16, h: u64) {
        self.0.push((code, Val::Handle(h)));
    }
    #[inline]
    pub fn name(&mut self, code: i16, h: u64, default: &'static str) {
        self.0.push((code, Val::NameOf(h, default)));
    }
    pub fn subclass(&mut self, name: &str) {
        self.str(100, name);
    }
    pub fn pt(&mut self, code: i16, p: Point3) {
        self.real(code, p.x);
        self.real(code + 10, p.y);
        self.real(code + 20, p.z);
    }
    pub fn pt2(&mut self, code: i16, p: Point3) {
        self.real(code, p.x);
        self.real(code + 10, p.y);
    }
    /// Extrusion (210) only if it isn't the default (0,0,1).
    pub fn extrusion(&mut self, p: Point3) {
        if p != Point3::Z {
            self.pt(210, p);
        }
    }
    pub fn angle_deg(&mut self, code: i16, rad: f64) {
        self.real(code, rad.to_degrees());
    }
}

/// Common entity properties (spec 20.4.1 / 20.4.2).
#[derive(Debug, Clone, Default)]
pub struct EntityCommon {
    /// 0 = owner handle present, 1 = paper space, 2 = model space.
    pub entmode: u8,
    pub color: Color,
    pub ltype_scale: f64,
    /// 0 bylayer, 1 byblock, 2 continuous, 3 handle
    pub ltype_flags: u8,
    pub plotstyle_flags: u8,
    pub material_flags: u8,
    pub invisible: bool,
    /// DXF 370 value (-1 bylayer, -2 byblock, -3 default, or 1/100 mm)
    pub lineweight: i16,
    pub layer: u64,
    pub ltype: u64,
    pub plotstyle: u64,
    pub material: u64,
    /// R13-R2000 entity chain
    pub prev: u64,
    pub next: u64,
}

#[derive(Debug, Clone, Default)]
pub struct BlockHeader {
    pub name: String,
    pub flags: i16,
    pub base: Point3,
    pub xref_path: String,
    pub description: String,
    pub block_entity: u64,
    pub endblk_entity: u64,
    pub first_entity: u64,
    pub last_entity: u64,
    /// R2004+: explicit list of owned entities
    pub entities: Vec<u64>,
    pub layout: u64,
    pub insert_units: i16,
    pub explodable: bool,
    pub scaling: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableKind {
    Block,
    Layer,
    Style,
    Ltype,
    View,
    Ucs,
    Vport,
    Appid,
    Dimstyle,
    VxTable,
}

impl TableKind {
    pub fn dxf_table(self) -> &'static str {
        match self {
            TableKind::Block => "BLOCK_RECORD",
            TableKind::Layer => "LAYER",
            TableKind::Style => "STYLE",
            TableKind::Ltype => "LTYPE",
            TableKind::View => "VIEW",
            TableKind::Ucs => "UCS",
            TableKind::Vport => "VPORT",
            TableKind::Appid => "APPID",
            TableKind::Dimstyle => "DIMSTYLE",
            TableKind::VxTable => "VX_TABLE_RECORD",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Kind {
    /// An entity or object fully described by `groups`.
    Plain,
    Block { name: String },
    EndBlk,
    Seqend,
    /// INSERT/MINSERT with attributes, or a POLYLINE: owns sub-entities
    /// terminated by a SEQEND.
    Complex { first: u64, last: u64, owned: Vec<u64>, seqend: u64 },
    BlockHeader(Box<BlockHeader>),
    /// Symbol table (control object).
    Table { kind: TableKind, entries: Vec<u64>, extra: Vec<u64> },
    /// Symbol table record with a name.
    TableEntry { kind: TableKind, name: String },
    Dictionary { entries: Vec<(String, u64)>, hard_owner: bool, cloning: i16, default: Option<u64> },
    /// Decoding not implemented for this type.
    Unsupported,
}

#[derive(Debug, Clone)]
pub enum XValue {
    Str(String),
    Brace(bool),
    LayerRef(u64),
    Binary(Vec<u8>),
    Handle(u64),
    Point(i16, Point3),
    Real(i16, f64),
    Short(i16),
    Long(i32),
}

#[derive(Debug, Clone)]
pub struct XData {
    pub app: u64,
    pub items: Vec<XValue>,
}

#[derive(Debug, Clone)]
pub struct Object {
    pub handle: u64,
    pub type_code: u16,
    /// DXF type name, e.g. "LINE", "VERTEX", "DICTIONARY"
    pub dxf_name: String,
    pub owner: u64,
    pub reactors: Vec<u64>,
    pub xdict: u64,
    pub xdata: Vec<XData>,
    pub entity: Option<EntityCommon>,
    pub kind: Kind,
    pub groups: Groups,
    /// Set if decoding stopped early; `groups` may be incomplete.
    pub error: Option<String>,
    /// Proxy graphics (pre-rendered primitives), if the entity has them.
    pub graphics: Option<Vec<u8>>,
    /// Binary ACIS (SAB) data of 3DSOLID/REGION/BODY.
    pub sab: Option<Vec<u8>>,
}

impl Object {
    pub fn is_entity(&self) -> bool {
        self.entity.is_some()
    }
}

/// A decoded DWG drawing.
#[derive(Debug, Default)]
pub struct Drawing {
    pub version: Option<Version>,
    pub maint: u8,
    pub codepage: u16,
    pub classes: Vec<Class>,
    pub header: crate::header::Header,
    /// All objects in handle order.
    pub objects: Vec<Object>,
    pub index: HashMap<u64, usize>,
    /// Objects that could not be decoded at all: (handle, type, error)
    pub failed: Vec<(u64, u16, String)>,
}

impl Drawing {
    pub fn get(&self, h: u64) -> Option<&Object> {
        self.index.get(&h).map(|&i| &self.objects[i])
    }

    pub fn version(&self) -> Version {
        self.version.unwrap_or(Version::R2000)
    }

    /// Count entities by DXF type.
    pub fn entity_counts(&self) -> Vec<(String, usize)> {
        let mut m: HashMap<&str, usize> = HashMap::new();
        for o in &self.objects {
            if o.is_entity() {
                *m.entry(o.dxf_name.as_str()).or_default() += 1;
            }
        }
        let mut v: Vec<_> = m.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        v.sort();
        v
    }
}
