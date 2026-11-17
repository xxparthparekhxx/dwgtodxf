//! ASCII DXF writer.
//!
//! Rebuilds the drawing structure from the decoded objects, keeping the
//! original DWG handles and ownership: HEADER, CLASSES, TABLES, BLOCKS,
//! ENTITIES and OBJECTS.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::bits::Point3;
use crate::model::*;
use crate::proxy::Prim;
use crate::version::Version;

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Output DXF version. Default: the DWG's own version (R2000 at least).
    pub version: Option<Version>,
}

/// Statistics about what was written.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub entities_written: usize,
    pub objects_written: usize,
    /// DXF type -> count of entities/objects that were skipped.
    pub skipped: Vec<(String, usize)>,
}

pub fn write(d: &Drawing, o: &Options) -> String {
    write_with_report(d, o).0
}

pub fn write_with_report(d: &Drawing, o: &Options) -> (String, Report) {
    let src = d.version();
    // binary ACIS solids can only be stored in DXF R2013+
    let has_sab = d.objects.iter().any(|o| o.sab.is_some());
    let default = if has_sab { Version::R2018 } else { src };
    let ver = o.version.unwrap_or(default).max(Version::R2000).min(Version::R2018);
    let mut w = Writer::new(d, ver);
    w.header();
    w.classes();
    w.tables();
    w.blocks();
    w.entities();
    w.objects();
    w.acdsdata();
    w.out.push_str("  0\nEOF\n");
    let mut skipped: Vec<(String, usize)> = w.skipped.into_iter().collect();
    skipped.sort();
    let report = Report {
        entities_written: w.n_entities,
        objects_written: w.n_objects,
        skipped,
    };
    (w.out, report)
}

struct Writer<'a> {
    d: &'a Drawing,
    ver: Version,
    out: String,
    names: HashMap<u64, String>,
    /// objects that will be written to OBJECTS
    alive: HashSet<u64>,
    /// children of each owner, in handle order (fallback ordering)
    children: HashMap<u64, Vec<u64>>,
    model_space: u64,
    paper_space: u64,
    written: HashSet<u64>,
    skipped: HashMap<String, usize>,
    n_entities: usize,
    n_objects: usize,
    utf8: bool,
    /// proxy-graphics replacements for entities we cannot decode natively
    proxies: HashMap<u64, Vec<(u64, Prim)>>,
    layer_by_index: Vec<u64>,
    ltype_by_index: Vec<u64>,
    seed: u64,
    /// next VIEWPORT id per owning block record
    vp_ids: HashMap<u64, i64>,
    /// solids whose SAB data goes to the ACDSDATA section
    sab_written: Vec<u64>,
}

fn fmt_real(v: f64) -> String {
    if !v.is_finite() {
        return "0.0".into();
    }
    if v == 0.0 {
        return "0.0".into();
    }
    let a = v.abs();
    if (1e-10..1e16).contains(&a) {
        let s = format!("{v}");
        if s.contains('.') {
            s
        } else {
            s + ".0"
        }
    } else {
        format!("{v:E}")
    }
}

impl<'a> Writer<'a> {
    fn new(d: &'a Drawing, ver: Version) -> Self {
        let mut names: HashMap<u64, String> = HashMap::new();
        let mut model_space = 0;
        let mut paper_space = 0;
        // anonymous blocks are stored as "*D", "*U", ...; number them uniquely
        let mut used: HashSet<String> = d
            .objects
            .iter()
            .filter_map(|o| match &o.kind {
                Kind::BlockHeader(b) => Some(b.name.to_ascii_uppercase()),
                _ => None,
            })
            .collect();
        let mut counters: HashMap<String, usize> = HashMap::new();
        let active_ps = d
            .objects
            .iter()
            .find_map(|o| match &o.kind {
                Kind::Table { kind: TableKind::Block, extra, .. } => extra.get(1).copied(),
                _ => None,
            })
            .unwrap_or(0);
        for o in &d.objects {
            match &o.kind {
                Kind::TableEntry { name, .. } => {
                    names.insert(o.handle, name.clone());
                }
                Kind::BlockHeader(b) => {
                    let n = &b.name;
                    if n.eq_ignore_ascii_case("*Paper_Space") && o.handle != active_ps {
                        // DWG stores every paper space block as "*Paper_Space";
                        // the inactive ones are numbered on load
                        let mut k = 0;
                        let name = loop {
                            let cand = format!("*Paper_Space{k}");
                            if used.insert(cand.to_ascii_uppercase()) {
                                break cand;
                            }
                            k += 1;
                        };
                        names.insert(o.handle, name);
                        continue;
                    }
                    let anon = n.len() == 2 && n.starts_with('*') && n.as_bytes()[1].is_ascii_alphabetic();
                    let name = if anon {
                        let k = n.to_ascii_uppercase();
                        let c = counters.entry(k.clone()).or_insert(0);
                        loop {
                            *c += 1;
                            let cand = format!("{k}{c}");
                            if used.insert(cand.clone()) {
                                break cand;
                            }
                        }
                    } else {
                        n.clone()
                    };
                    names.insert(o.handle, name);
                }
                Kind::Table { kind: TableKind::Block, extra, .. } => {
                    model_space = extra.first().copied().unwrap_or(0);
                    paper_space = extra.get(1).copied().unwrap_or(0);
                }
                _ => {}
            }
        }
        // fall back to names if the block control was not decoded
        for o in &d.objects {
            if let Kind::BlockHeader(b) = &o.kind {
                let n = b.name.to_ascii_uppercase();
                if model_space == 0 && n == "*MODEL_SPACE" {
                    model_space = o.handle;
                }
                if paper_space == 0 && n == "*PAPER_SPACE" {
                    paper_space = o.handle;
                }
            }
        }
        let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
        for o in &d.objects {
            if let Some(e) = &o.entity {
                let owner = match e.entmode {
                    1 => paper_space,
                    2 => model_space,
                    _ => o.owner,
                };
                children.entry(owner).or_default().push(o.handle);
            }
        }
        // OBJECTS: keep only decodable objects whose owner chain is written
        let mut alive: HashSet<u64> = d
            .objects
            .iter()
            .filter(|o| writable_object(o))
            .map(|o| o.handle)
            .collect();
        loop {
            let before = alive.len();
            let snapshot = alive.clone();
            alive.retain(|h| {
                let o = d.get(*h).unwrap();
                o.owner == 0
                    || snapshot.contains(&o.owner)
                    || d.get(o.owner).map(|w| owner_written(w)).unwrap_or(false)
            });
            if alive.len() == before {
                break;
            }
        }
        let mut seed = d.objects.iter().map(|o| o.handle).max().unwrap_or(0x20) + 1;
        let mut proxies: HashMap<u64, Vec<(u64, Prim)>> = HashMap::new();
        for o in &d.objects {
            let sab_fallback = o.sab.is_some() && ver < Version::R2013;
            if !o.is_entity() || !(matches!(o.kind, Kind::Unsupported) || o.error.is_some() || sab_fallback) {
                continue;
            }
            if let Some(gr) = &o.graphics {
                let prims = crate::proxy::convert(gr, d.version());
                if !prims.is_empty() {
                    let v = prims
                        .into_iter()
                        .map(|p| {
                            seed += 1;
                            (seed - 1, p)
                        })
                        .collect();
                    proxies.insert(o.handle, v);
                }
            }
        }
        let layer_by_index = d
            .objects
            .iter()
            .find_map(|o| match &o.kind {
                Kind::Table { kind: TableKind::Layer, entries, .. } => Some(entries.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let ltype_by_index = d
            .objects
            .iter()
            .find_map(|o| match &o.kind {
                Kind::Table { kind: TableKind::Ltype, entries, .. } => Some(entries.clone()),
                _ => None,
            })
            .unwrap_or_default();
        Writer {
            d,
            ver,
            proxies,
            layer_by_index,
            ltype_by_index,
            vp_ids: HashMap::new(),
            sab_written: vec![],
            seed,
            out: String::with_capacity(d.objects.len() * 300),
            names,
            alive,
            children,
            model_space,
            paper_space,
            written: HashSet::new(),
            skipped: HashMap::new(),
            n_entities: 0,
            n_objects: 0,
            utf8: ver >= Version::R2007,
        }
    }

    // ---- primitive output ---------------------------------------------

    #[inline]
    fn code(&mut self, c: i16) {
        let _ = writeln!(self.out, "{c:>3}");
    }
    fn s(&mut self, c: i16, v: &str) {
        self.code(c);
        if self.utf8 || v.is_ascii() {
            for ch in v.chars() {
                // group values are single lines
                self.out.push(if ch == '\n' || ch == '\r' { ' ' } else { ch });
            }
        } else {
            for ch in v.chars() {
                if ch.is_ascii() {
                    self.out.push(if ch == '\n' || ch == '\r' { ' ' } else { ch });
                } else {
                    let mut buf = [0u16; 2];
                    for u in ch.encode_utf16(&mut buf) {
                        let _ = write!(self.out, "\\U+{u:04X}");
                    }
                }
            }
        }
        self.out.push('\n');
    }
    fn i(&mut self, c: i16, v: i64) {
        self.code(c);
        let _ = writeln!(self.out, "{v}");
    }
    fn f(&mut self, c: i16, v: f64) {
        self.code(c);
        self.out.push_str(&fmt_real(v));
        self.out.push('\n');
    }
    fn h(&mut self, c: i16, v: u64) {
        self.code(c);
        let _ = writeln!(self.out, "{v:X}");
    }
    fn pt(&mut self, c: i16, p: Point3) {
        self.f(c, p.x);
        self.f(c + 10, p.y);
        self.f(c + 20, p.z);
    }

    fn name_of(&self, h: u64) -> Option<&str> {
        self.names.get(&h).map(|s| s.as_str())
    }

    fn groups(&mut self, g: &Groups) {
        for (c, v) in &g.0 {
            let c = *c;
            // skip group codes the target DXF version does not know
            if self.ver < Version::R2004 && matches!(c, 420..=429 | 430..=439 | 440..=449) {
                continue;
            }
            match v {
                Val::Int(i) => self.i(c, *i),
                Val::Real(f) => self.f(c, *f),
                Val::Str(s) => self.s(c, s),
                Val::Handle(h) => self.h(c, *h),
                Val::NameOf(h, def) => {
                    let n = self.name_of(*h).unwrap_or(def).to_string();
                    self.s(c, &n);
                }
                Val::LtypeIndex(i) => {
                    let n = self
                        .ltype_by_index
                        .get(*i as usize)
                        .and_then(|h| self.name_of(*h))
                        .unwrap_or("BYLAYER")
                        .to_string();
                    self.s(c, &n);
                }
            }
        }
    }

    fn xdata(&mut self, o: &Object) {
        for x in &o.xdata {
            let app = self.name_of(x.app).unwrap_or("ACAD").to_string();
            self.s(1001, &app);
            for it in &x.items {
                match it {
                    XValue::Str(s) => self.s(1000, s),
                    XValue::Brace(close) => self.s(1002, if *close { "}" } else { "{" }),
                    XValue::LayerRef(h) => {
                        let n = self.name_of(*h).unwrap_or("0").to_string();
                        self.s(1003, &n);
                    }
                    XValue::Binary(b) => {
                        let hex: String = b.iter().map(|x| format!("{x:02X}")).collect();
                        self.s(1004, &hex);
                    }
                    XValue::Handle(h) => self.h(1005, *h),
                    XValue::Point(c, p) => self.pt(*c, *p),
                    XValue::Real(c, v) => self.f(*c, *v),
                    XValue::Short(v) => self.i(1070, *v as i64),
                    XValue::Long(v) => self.i(1071, *v as i64),
                }
            }
        }
    }

    /// 102 groups for reactors and extension dictionary.
    fn app_groups(&mut self, o: &Object) {
        let reactors: Vec<u64> =
            o.reactors.iter().copied().filter(|h| *h != 0 && self.d.get(*h).is_some()).collect();
        if !reactors.is_empty() {
            self.s(102, "{ACAD_REACTORS");
            for r in reactors {
                self.h(330, r);
            }
            self.s(102, "}");
        }
        if o.xdict != 0 && self.alive.contains(&o.xdict) {
            self.s(102, "{ACAD_XDICTIONARY");
            self.h(360, o.xdict);
            self.s(102, "}");
        }
    }

    fn skip(&mut self, o: &Object) {
        *self.skipped.entry(o.dxf_name.clone()).or_default() += 1;
    }

    // ---- HEADER --------------------------------------------------------

    fn header(&mut self) {
        use crate::header::HVal;
        let seed = self.seed;
        self.s(0, "SECTION");
        self.s(2, "HEADER");
        self.s(9, "$ACADVER");
        self.s(1, self.ver.acadver());
        self.s(9, "$DWGCODEPAGE");
        self.s(3, "ANSI_1252");
        let hdr = &self.d.header;
        for &(name, code, min) in HEADER_VARS {
            if self.ver < min {
                continue;
            }
            let Some(v) = hdr.vars.get(name) else { continue };
            let v = v.clone();
            self.s(9, &format!("${name}"));
            match (v, code) {
                (HVal::Int(i), c) => self.i(c, i),
                (HVal::Real(f), c) => self.f(c, f),
                (HVal::Str(s), c) => self.s(c, &s),
                (HVal::Point(p), 10) if name.ends_with("LIMMIN") || name.ends_with("LIMMAX") => {
                    self.f(10, p.x);
                    self.f(20, p.y);
                }
                (HVal::Point(p), c) => self.pt(c, p),
                (HVal::Color(col), c) => self.i(c, col.index as i64),
                (HVal::Handle(h), c) => {
                    let def = match c {
                        8 => "0",
                        7 => "Standard",
                        6 => "ByLayer",
                        _ => "",
                    };
                    let n = self.name_of(h).unwrap_or(def).to_string();
                    self.s(c, &n);
                }
            }
        }
        self.s(9, "$HANDSEED");
        self.h(5, seed);
        self.s(0, "ENDSEC");
    }

    // ---- CLASSES -------------------------------------------------------

    fn classes(&mut self) {
        self.s(0, "SECTION");
        self.s(2, "CLASSES");
        for c in &self.d.classes {
            self.s(0, "CLASS");
            self.s(1, &c.dxf_name);
            self.s(2, &c.cpp_name);
            self.s(3, &c.app_name);
            self.i(90, c.proxy_flags as i64);
            if self.ver >= Version::R2004 {
                self.i(91, c.instance_count as i64);
            }
            self.i(280, c.was_zombie as i64);
            self.i(281, c.is_entity() as i64);
        }
        self.s(0, "ENDSEC");
    }

    // ---- TABLES --------------------------------------------------------

    fn tables(&mut self) {
        self.s(0, "SECTION");
        self.s(2, "TABLES");
        for kind in [
            TableKind::Vport,
            TableKind::Ltype,
            TableKind::Layer,
            TableKind::Style,
            TableKind::View,
            TableKind::Ucs,
            TableKind::Appid,
            TableKind::Dimstyle,
            TableKind::Block,
        ] {
            let ctrl = self.d.objects.iter().find(|o| match &o.kind {
                Kind::Table { kind: k, .. } => *k == kind,
                _ => false,
            });
            let Some(ctrl) = ctrl else { continue };
            let Kind::Table { entries, extra, .. } = &ctrl.kind else { continue };
            let mut list: Vec<u64> = Vec::with_capacity(entries.len() + 2);
            if matches!(kind, TableKind::Block | TableKind::Ltype) {
                list.extend(extra.iter().copied().filter(|h| *h != 0));
            }
            list.extend(entries.iter().copied());
            let mut seen = HashSet::new();
            list.retain(|h| seen.insert(*h));
            let recs: Vec<&Object> = list
                .iter()
                .filter_map(|h| self.d.get(*h))
                .filter(|o| o.error.is_none())
                .filter(|o| matches!(o.kind, Kind::TableEntry { .. } | Kind::BlockHeader(_)))
                .collect();

            self.s(0, "TABLE");
            self.s(2, kind.dxf_table());
            self.h(5, ctrl.handle);
            self.h(330, 0);
            self.s(100, "AcDbSymbolTable");
            self.i(70, recs.len() as i64);
            if kind == TableKind::Dimstyle {
                self.s(100, "AcDbDimStyleTable");
            }
            for r in recs {
                self.table_record(r, kind, ctrl.handle);
            }
            self.s(0, "ENDTAB");
        }
        self.s(0, "ENDSEC");
    }

    fn table_record(&mut self, o: &Object, kind: TableKind, ctrl: u64) {
        self.s(0, kind.dxf_table());
        self.h(if kind == TableKind::Dimstyle { 105 } else { 5 }, o.handle);
        self.app_groups(o);
        self.h(330, ctrl);
        self.s(100, "AcDbSymbolTableRecord");
        match (&o.kind, kind) {
            (Kind::BlockHeader(b), _) => {
                self.s(100, "AcDbBlockTableRecord");
                let n = self.name_of(o.handle).unwrap_or(&b.name).to_string();
                self.s(2, &n);
                if b.layout != 0 && self.alive.contains(&b.layout)
                {
                    self.h(340, b.layout);
                }
                if self.ver >= Version::R2007 {
                    self.i(70, b.insert_units as i64);
                    self.i(280, b.explodable as i64);
                    self.i(281, b.scaling as i64);
                }
            }
            (Kind::TableEntry { name, .. }, TableKind::Vport | TableKind::View | TableKind::Ucs | TableKind::Dimstyle) => {
                let sub = match kind {
                    TableKind::Vport => "AcDbViewportTableRecord",
                    TableKind::View => "AcDbViewTableRecord",
                    TableKind::Ucs => "AcDbUCSTableRecord",
                    _ => "AcDbDimStyleTableRecord",
                };
                self.s(100, sub);
                self.s(2, name);
                self.groups(&o.groups);
            }
            _ => self.groups(&o.groups),
        }
        self.xdata(o);
        self.n_objects += 1;
    }

    // ---- BLOCKS / ENTITIES ---------------------------------------------

    /// Entities owned by a block record, in drawing order.
    fn block_entities(&self, blk: &Object) -> Vec<u64> {
        let Kind::BlockHeader(b) = &blk.kind else { return vec![] };
        let skip = |h: &u64| *h == b.block_entity || *h == b.endblk_entity;
        if !b.entities.is_empty() {
            return b.entities.iter().copied().filter(|h| !skip(h)).collect();
        }
        if b.first_entity != 0 {
            let chain = self.follow_chain(b.first_entity, b.last_entity);
            if !chain.is_empty() {
                return chain.into_iter().filter(|h| !skip(h)).collect();
            }
        }
        let mut v = self.children.get(&blk.handle).cloned().unwrap_or_default();
        v.retain(|h| !skip(h));
        v.retain(|h| {
            self.d
                .get(*h)
                .map(|o| !matches!(o.kind, Kind::Block { .. } | Kind::EndBlk))
                .unwrap_or(false)
        });
        v
    }

    /// R13-R2000: follow next-entity links from `first` to `last`.
    fn follow_chain(&self, first: u64, last: u64) -> Vec<u64> {
        let mut out = vec![];
        let mut h = first;
        let mut guard = HashSet::new();
        while h != 0 && guard.insert(h) {
            let Some(o) = self.d.get(h) else { break };
            out.push(h);
            if h == last {
                break;
            }
            h = o.entity.as_ref().map(|e| e.next).unwrap_or(0);
        }
        out
    }

    /// Sub-entities of a complex entity (vertices, attributes).
    fn sub_entities(&self, o: &Object) -> (Vec<u64>, u64) {
        let Kind::Complex { first, last, owned, seqend } = &o.kind else { return (vec![], 0) };
        if !owned.is_empty() {
            return (owned.clone(), *seqend);
        }
        if *first != 0 {
            let c = self.follow_chain(*first, *last);
            if !c.is_empty() {
                return (c, *seqend);
            }
        }
        let mut v = self.children.get(&o.handle).cloned().unwrap_or_default();
        v.retain(|h| *h != *seqend);
        (v, *seqend)
    }

    fn blocks(&mut self) {
        self.s(0, "SECTION");
        self.s(2, "BLOCKS");
        let mut order: Vec<&Object> = vec![];
        for h in [self.model_space, self.paper_space] {
            if let Some(o) = self.d.get(h) {
                order.push(o);
            }
        }
        for o in &self.d.objects {
            if matches!(o.kind, Kind::BlockHeader(_)) && o.handle != self.model_space && o.handle != self.paper_space {
                order.push(o);
            }
        }
        for blk in order {
            if blk.error.is_some() {
                continue;
            }
            let Kind::BlockHeader(b) = &blk.kind else { continue };
            let is_layout_space = blk.handle == self.model_space || blk.handle == self.paper_space;
            let ps = blk.handle == self.paper_space;
            let begin = self.d.get(b.block_entity);
            self.s(0, "BLOCK");
            self.h(5, if b.block_entity != 0 { b.block_entity } else { blk.handle + 0x10_0000 });
            if let Some(be) = begin {
                self.app_groups(be);
            }
            self.h(330, blk.handle);
            self.s(100, "AcDbEntity");
            if ps {
                self.i(67, 1);
            }
            let layer = begin.and_then(|e| e.entity.as_ref()).map(|e| e.layer).unwrap_or(0);
            let ln = self.name_of(layer).unwrap_or("0").to_string();
            self.s(8, &ln);
            self.s(100, "AcDbBlockBegin");
            let bname = self.name_of(blk.handle).unwrap_or(&b.name).to_string();
            self.s(2, &bname);
            self.i(70, b.flags as i64 & 0x1F);
            self.pt(10, b.base);
            self.s(3, &bname);
            self.s(1, &b.xref_path);
            if !b.description.is_empty() {
                self.s(4, &b.description);
            }
            if !is_layout_space {
                for h in self.block_entities(blk) {
                    if let Some(e) = self.d.get(h) {
                        self.entity(e, blk.handle, false);
                    }
                }
            }
            let end = self.d.get(b.endblk_entity);
            self.s(0, "ENDBLK");
            self.h(5, if b.endblk_entity != 0 { b.endblk_entity } else { blk.handle + 0x20_0000 });
            if let Some(ee) = end {
                self.app_groups(ee);
            }
            self.h(330, blk.handle);
            self.s(100, "AcDbEntity");
            if ps {
                self.i(67, 1);
            }
            self.s(8, &ln);
            self.s(100, "AcDbBlockEnd");
        }
        self.s(0, "ENDSEC");
    }

    fn entities(&mut self) {
        self.s(0, "SECTION");
        self.s(2, "ENTITIES");
        for (space, ps) in [(self.model_space, false), (self.paper_space, true)] {
            let Some(blk) = self.d.get(space) else { continue };
            for h in self.block_entities(blk) {
                if let Some(e) = self.d.get(h) {
                    self.entity(e, space, ps);
                }
            }
        }
        self.s(0, "ENDSEC");
    }

    fn entity(&mut self, o: &Object, owner: u64, paper: bool) {
        let Some(e) = &o.entity else { return };
        if let Some(prims) = self.proxies.get(&o.handle) {
            if self.written.insert(o.handle) {
                for (h, p) in prims.clone() {
                    self.proxy_prim(&p, h, e, owner, paper);
                }
            }
            return;
        }
        if o.error.is_some()
            || matches!(o.kind, Kind::Unsupported | Kind::Block { .. } | Kind::EndBlk | Kind::Seqend)
        {
            if !matches!(o.kind, Kind::Block { .. } | Kind::EndBlk | Kind::Seqend) {
                self.skip(o);
            }
            return;
        }
        // binary ACIS cannot be stored before DXF R2013 (and had no wireframe)
        if o.sab.is_some() && self.ver < Version::R2013 {
            self.skip(o);
            return;
        }
        // dimensions without a geometry block (e.g. constraint parameters)
        if o.dxf_name == "DIMENSION"
            && o.groups.0.iter().any(|(c, v)| *c == 2 && matches!(v, Val::NameOf(0, _)))
        {
            self.skip(o);
            return;
        }
        if !self.written.insert(o.handle) {
            return;
        }
        self.s(0, &o.dxf_name);
        self.h(5, o.handle);
        self.app_groups(o);
        self.h(330, owner);
        self.common(e, paper);
        if o.sab.is_some() {
            self.s(100, "AcDbModelerGeometry");
            self.i(290, 1);
            self.s(2, "{00000000-0000-0000-0000-000000000000}");
            if o.dxf_name == "3DSOLID" {
                self.s(100, "AcDb3dSolid");
            }
            self.sab_written.push(o.handle);
        } else if o.dxf_name == "VIEWPORT" {
            let id = {
                let n = self.vp_ids.entry(owner).or_insert(0);
                *n += 1;
                *n
            };
            let mut g = o.groups.clone();
            for (_, v) in g.0.iter_mut() {
                if *v == Val::Int(crate::entities::VP_STATUS) || *v == Val::Int(crate::entities::VP_ID) {
                    *v = Val::Int(id);
                }
            }
            self.groups(&g);
        } else {
            self.groups(&o.groups);
        }
        self.xdata(o);
        self.n_entities += 1;

        if let Kind::Complex { .. } = &o.kind {
            let (subs, seqend) = self.sub_entities(o);
            for h in subs {
                if let Some(s) = self.d.get(h) {
                    if s.is_entity() && !matches!(s.kind, Kind::Seqend) {
                        self.entity(s, o.handle, paper);
                    }
                }
            }
            // SEQEND
            let se = self.d.get(seqend);
            self.s(0, "SEQEND");
            self.h(5, if seqend != 0 { seqend } else { o.handle + 0x30_0000 });
            if let Some(se) = se {
                self.app_groups(se);
            }
            self.h(330, o.handle);
            self.s(100, "AcDbEntity");
            if paper {
                self.i(67, 1);
            }
            let ln = self.name_of(e.layer).unwrap_or("0").to_string();
            self.s(8, &ln);
        }
    }

    /// Write one primitive converted from proxy graphics.
    fn proxy_prim(&mut self, p: &Prim, handle: u64, e: &EntityCommon, owner: u64, paper: bool) {
        let mut ce = e.clone();
        if let Some(i) = p.layer_index {
            if let Some(h) = self.layer_by_index.get(i as usize) {
                ce.layer = *h;
            }
        }
        match p.color {
            // BYBLOCK inside proxy graphics means "the proxy entity's colour"
            Some(0) | None => {}
            Some(c) => {
                ce.color.index = c;
                ce.color.rgb = None;
            }
        }
        if p.rgb.is_some() {
            ce.color.rgb = p.rgb;
        }
        if let Some(lw) = p.lineweight {
            ce.lineweight = lw;
        }
        ce.invisible = false;
        self.s(0, p.dxf_name);
        self.h(5, handle);
        self.h(330, owner);
        self.common(&ce, paper);
        self.groups(&p.groups);
        self.n_entities += 1;
    }

    fn common(&mut self, e: &EntityCommon, paper: bool) {
        self.s(100, "AcDbEntity");
        if paper {
            self.i(67, 1);
        }
        let ln = self.name_of(e.layer).unwrap_or("0").to_string();
        self.s(8, &ln);
        match e.ltype_flags {
            1 => self.s(6, "BYBLOCK"),
            2 => self.s(6, "Continuous"),
            3 => {
                if let Some(n) = self.name_of(e.ltype).map(|s| s.to_string()) {
                    if !n.eq_ignore_ascii_case("BYLAYER") {
                        self.s(6, &n);
                    }
                }
            }
            _ => {}
        }
        if e.color.index != 256 {
            self.i(62, e.color.index as i64);
        }
        if self.ver >= Version::R2004 {
            if let Some(rgb) = e.color.rgb {
                self.i(420, rgb as i64);
            }
            if let Some(t) = e.color.transparency {
                if t >> 24 == 3 {
                    self.i(440, (t | 0x0200_0000) as i64 & 0xFFFF_FFFF);
                }
            }
        }
        if e.lineweight != -1 {
            self.i(370, e.lineweight as i64);
        }
        if e.ltype_scale != 1.0 && e.ltype_scale != 0.0 {
            self.f(48, e.ltype_scale);
        }
        if e.invisible {
            self.i(60, 1);
        }
    }

    // ---- ACDSDATA (binary ACIS of R2013+ DXF) ----------------------------

    fn acdsdata(&mut self) {
        if self.sab_written.is_empty() || self.ver < Version::R2013 {
            return;
        }
        self.s(0, "SECTION");
        self.s(2, "ACDSDATA");
        self.i(70, 2);
        self.i(71, 8);
        // schema definitions, as AutoCAD writes them
        self.out.push_str(ACDS_SCHEMAS);
        for h in std::mem::take(&mut self.sab_written) {
            let Some(data) = self.d.get(h).and_then(|o| o.sab.as_ref()) else { continue };
            self.s(0, "ACDSRECORD");
            self.i(90, 0);
            self.s(2, "AcDbDs::ID");
            self.i(280, 10);
            self.h(320, h);
            self.s(2, "ASM_Data");
            self.i(280, 15);
            self.i(94, data.len() as i64);
            for chunk in data.chunks(127) {
                self.code(310);
                for b in chunk {
                    let _ = write!(self.out, "{b:02X}");
                }
                self.out.push('\n');
            }
        }
        self.s(0, "ENDSEC");
    }

    // ---- OBJECTS -------------------------------------------------------

    fn objects(&mut self) {
        self.s(0, "SECTION");
        self.s(2, "OBJECTS");
        // root dictionary first
        let root = self
            .d
            .objects
            .iter()
            .find(|o| o.owner == 0 && matches!(o.kind, Kind::Dictionary { .. }));
        if let Some(r) = root {
            self.object(r);
        }
        for o in &self.d.objects {
            if Some(o.handle) == root.map(|r| r.handle) {
                continue;
            }
            if o.is_entity() || matches!(o.kind, Kind::Table { .. } | Kind::TableEntry { .. } | Kind::BlockHeader(_)) {
                continue;
            }
            self.object(o);
        }
        self.s(0, "ENDSEC");
    }

    fn object(&mut self, o: &Object) {
        if !self.alive.contains(&o.handle) {
            self.skip(o);
            return;
        }
        self.s(0, &o.dxf_name);
        self.h(5, o.handle);
        self.app_groups(o);
        self.h(330, o.owner);
        match &o.kind {
            Kind::Dictionary { entries, hard_owner, cloning, default } => {
                self.s(100, "AcDbDictionary");
                if *hard_owner {
                    self.i(280, 1);
                }
                self.i(281, *cloning as i64);
                for (name, h) in entries {
                    if self.alive.contains(h) {
                        self.s(3, name);
                        self.h(if *hard_owner { 360 } else { 350 }, *h);
                    }
                }
                if let Some(def) = default {
                    self.s(100, "AcDbDictionaryWithDefault");
                    self.h(340, *def);
                }
            }
            _ => self.groups(&o.groups),
        }
        self.xdata(o);
        self.n_objects += 1;
    }
}

/// ACDSDATA schema records (fixed content).
const ACDS_SCHEMAS: &str = "  0\nACDSSCHEMA\n 90\n0\n  1\nAcDb3DSolid_ASM_Data\n  2\nAcDbDs::ID\n280\n10\n 91\n8\n  2\nASM_Data\n280\n15\n 91\n0\n101\nACDSRECORD\n 95\n0\n 90\n2\n  2\nAcDbDs::TreatedAsObjectData\n280\n1\n291\n1\n101\nACDSRECORD\n 95\n0\n 90\n3\n  2\nAcDbDs::Legacy\n280\n1\n291\n1\n101\nACDSRECORD\n  1\nAcDbDs::ID\n 90\n4\n  2\nAcDs:Indexable\n280\n1\n291\n1\n101\nACDSRECORD\n  1\nAcDbDs::ID\n 90\n5\n  2\nAcDbDs::HandleAttribute\n280\n7\n282\n1\n  0\nACDSSCHEMA\n 90\n1\n  1\nAcDb_Thumbnail_Schema\n  2\nAcDbDs::ID\n280\n10\n 91\n8\n  2\nThumbnail_Data\n280\n15\n 91\n0\n101\nACDSRECORD\n 95\n1\n 90\n2\n  2\nAcDbDs::TreatedAsObjectData\n280\n1\n291\n1\n101\nACDSRECORD\n 95\n1\n 90\n3\n  2\nAcDbDs::Legacy\n280\n1\n291\n1\n101\nACDSRECORD\n  1\nAcDbDs::ID\n 90\n4\n  2\nAcDs:Indexable\n280\n1\n291\n1\n101\nACDSRECORD\n  1\nAcDbDs::ID\n 90\n5\n  2\nAcDbDs::HandleAttribute\n280\n7\n282\n1\n  0\nACDSSCHEMA\n 90\n2\n  1\nAcDbDs::TreatedAsObjectDataSchema\n  2\nAcDbDs::TreatedAsObjectData\n280\n1\n 91\n0\n  0\nACDSSCHEMA\n 90\n3\n  1\nAcDbDs::LegacySchema\n  2\nAcDbDs::Legacy\n280\n1\n 91\n0\n  0\nACDSSCHEMA\n 90\n4\n  1\nAcDbDs::IndexedPropertySchema\n  2\nAcDs:Indexable\n280\n1\n 91\n0\n  0\nACDSSCHEMA\n 90\n5\n  1\nAcDbDs::HandleAttributeSchema\n  2\nAcDbDs::HandleAttribute\n280\n7\n 91\n1\n284\n1\n";

/// Header variables written to DXF: (name, group code, first DXF version).
/// Handles are written as the referenced record's name.
#[rustfmt::skip]
const HEADER_VARS: &[(&str, i16, Version)] = {
    use Version::*;
    &[
    ("INSBASE", 10, R2000), ("EXTMIN", 10, R2000), ("EXTMAX", 10, R2000), ("LIMMIN", 10, R2000),
    ("LIMMAX", 10, R2000), ("ORTHOMODE", 70, R2000), ("REGENMODE", 70, R2000), ("FILLMODE", 70, R2000),
    ("QTEXTMODE", 70, R2000), ("MIRRTEXT", 70, R2000), ("LTSCALE", 40, R2000), ("ATTMODE", 70, R2000),
    ("TEXTSIZE", 40, R2000), ("TRACEWID", 40, R2000), ("TEXTSTYLE", 7, R2000), ("CLAYER", 8, R2000),
    ("CELTYPE", 6, R2000), ("CECOLOR", 62, R2000), ("CELTSCALE", 40, R2000), ("DISPSILH", 70, R2000),
    ("DIMSCALE", 40, R2000), ("DIMASZ", 40, R2000), ("DIMEXO", 40, R2000), ("DIMDLI", 40, R2000),
    ("DIMRND", 40, R2000), ("DIMDLE", 40, R2000), ("DIMEXE", 40, R2000), ("DIMTP", 40, R2000),
    ("DIMTM", 40, R2000), ("DIMTXT", 40, R2000), ("DIMCEN", 40, R2000), ("DIMTSZ", 40, R2000),
    ("DIMTOL", 70, R2000), ("DIMLIM", 70, R2000), ("DIMTIH", 70, R2000), ("DIMTOH", 70, R2000),
    ("DIMSE1", 70, R2000), ("DIMSE2", 70, R2000), ("DIMTAD", 70, R2000), ("DIMZIN", 70, R2000),
    ("DIMBLK", 1, R2000), ("DIMASO", 70, R2000), ("DIMSHO", 70, R2000), ("DIMPOST", 1, R2000),
    ("DIMAPOST", 1, R2000), ("DIMALT", 70, R2000), ("DIMALTD", 70, R2000), ("DIMALTF", 40, R2000),
    ("DIMLFAC", 40, R2000), ("DIMTOFL", 70, R2000), ("DIMTVP", 40, R2000), ("DIMTIX", 70, R2000),
    ("DIMSOXD", 70, R2000), ("DIMSAH", 70, R2000), ("DIMBLK1", 1, R2000), ("DIMBLK2", 1, R2000),
    ("DIMSTYLE", 2, R2000), ("DIMCLRD", 70, R2000), ("DIMCLRE", 70, R2000), ("DIMCLRT", 70, R2000),
    ("DIMTFAC", 40, R2000), ("DIMGAP", 40, R2000), ("DIMJUST", 70, R2000), ("DIMSD1", 70, R2000),
    ("DIMSD2", 70, R2000), ("DIMTOLJ", 70, R2000), ("DIMTZIN", 70, R2000), ("DIMALTZ", 70, R2000),
    ("DIMALTTZ", 70, R2000), ("DIMUPT", 70, R2000), ("DIMDEC", 70, R2000), ("DIMTDEC", 70, R2000),
    ("DIMALTU", 70, R2000), ("DIMALTTD", 70, R2000), ("DIMTXSTY", 7, R2000), ("DIMAUNIT", 70, R2000),
    ("DIMADEC", 70, R2000), ("DIMALTRND", 40, R2000), ("DIMAZIN", 70, R2000), ("DIMDSEP", 70, R2000),
    ("DIMATFIT", 70, R2000), ("DIMFRAC", 70, R2000), ("DIMLDRBLK", 1, R2000), ("DIMLUNIT", 70, R2000),
    ("DIMLWD", 70, R2000), ("DIMLWE", 70, R2000), ("DIMTMOVE", 70, R2000),
    ("DIMFXL", 40, R2007), ("DIMFXLON", 70, R2007), ("DIMJOGANG", 40, R2007), ("DIMTFILL", 70, R2007),
    ("DIMTFILLCLR", 70, R2007), ("DIMARCSYM", 70, R2007), ("DIMLTYPE", 6, R2007), ("DIMLTEX1", 6, R2007),
    ("DIMLTEX2", 6, R2007), ("DIMTXTDIRECTION", 70, R2010),
    ("LUNITS", 70, R2000), ("LUPREC", 70, R2000), ("SKETCHINC", 40, R2000), ("FILLETRAD", 40, R2000),
    ("AUNITS", 70, R2000), ("AUPREC", 70, R2000), ("MENU", 1, R2000), ("ELEVATION", 40, R2000),
    ("PELEVATION", 40, R2000), ("THICKNESS", 40, R2000), ("LIMCHECK", 70, R2000), ("CHAMFERA", 40, R2000),
    ("CHAMFERB", 40, R2000), ("CHAMFERC", 40, R2000), ("CHAMFERD", 40, R2000), ("SKPOLY", 70, R2000),
    ("TDCREATE", 40, R2000), ("TDUPDATE", 40, R2000), ("TDINDWG", 40, R2000), ("TDUSRTIMER", 40, R2000),
    ("USRTIMER", 70, R2000), ("ANGBASE", 50, R2000), ("ANGDIR", 70, R2000), ("PDMODE", 70, R2000),
    ("PDSIZE", 40, R2000), ("PLINEWID", 40, R2000), ("SPLFRAME", 70, R2000), ("SPLINETYPE", 70, R2000),
    ("SPLINESEGS", 70, R2000), ("SURFTAB1", 70, R2000), ("SURFTAB2", 70, R2000), ("SURFTYPE", 70, R2000),
    ("SURFU", 70, R2000), ("SURFV", 70, R2000), ("UCSNAME", 2, R2000), ("UCSORG", 10, R2000),
    ("UCSXDIR", 10, R2000), ("UCSYDIR", 10, R2000), ("PUCSNAME", 2, R2000), ("PUCSORG", 10, R2000),
    ("PUCSXDIR", 10, R2000), ("PUCSYDIR", 10, R2000), ("USERI1", 70, R2000), ("USERI2", 70, R2000),
    ("USERI3", 70, R2000), ("USERI4", 70, R2000), ("USERI5", 70, R2000), ("USERR1", 40, R2000),
    ("USERR2", 40, R2000), ("USERR3", 40, R2000), ("USERR4", 40, R2000), ("USERR5", 40, R2000),
    ("WORLDVIEW", 70, R2000), ("SHADEDGE", 70, R2000), ("SHADEDIF", 70, R2000), ("TILEMODE", 70, R2000),
    ("MAXACTVP", 70, R2000), ("PINSBASE", 10, R2000), ("PLIMCHECK", 70, R2000), ("PEXTMIN", 10, R2000),
    ("PEXTMAX", 10, R2000), ("PLIMMIN", 10, R2000), ("PLIMMAX", 10, R2000), ("UNITMODE", 70, R2000),
    ("VISRETAIN", 70, R2000), ("PLINEGEN", 70, R2000), ("PSLTSCALE", 70, R2000), ("TREEDEPTH", 70, R2000),
    ("CMLSTYLE", 2, R2000), ("CMLJUST", 70, R2000), ("CMLSCALE", 40, R2000), ("PROXYGRAPHICS", 70, R2000),
    ("CELWEIGHT", 370, R2000), ("ENDCAPS", 280, R2000), ("JOINSTYLE", 280, R2000),
    ("LWDISPLAY", 290, R2000), ("INSUNITS", 70, R2000), ("HYPERLINKBASE", 1, R2000),
    ("STYLESHEET", 1, R2000), ("XEDIT", 290, R2000), ("CEPSNTYPE", 380, R2000),
    ("PSTYLEMODE", 290, R2000), ("FINGERPRINTGUID", 2, R2000), ("VERSIONGUID", 2, R2000),
    ("EXTNAMES", 290, R2000), ("PSVPSCALE", 40, R2000), ("OLESTARTUP", 290, R2000),
    ("SORTENTS", 280, R2004), ("INDEXCTL", 280, R2004), ("HIDETEXT", 280, R2004),
    ("XCLIPFRAME", 280, R2004), ("HALOGAP", 280, R2004), ("OBSCUREDCOLOR", 70, R2004),
    ("OBSCUREDLTYPE", 280, R2004), ("INTERSECTIONDISPLAY", 280, R2004),
    ("INTERSECTIONCOLOR", 70, R2004), ("DIMASSOC", 280, R2004), ("PROJECTNAME", 1, R2004),
    ("CAMERADISPLAY", 290, R2007), ("LENSLENGTH", 40, R2007), ("CAMERAHEIGHT", 40, R2007),
    ("STEPSPERSEC", 40, R2007), ("STEPSIZE", 40, R2007), ("3DDWFPREC", 40, R2007),
    ("PSOLWIDTH", 40, R2007), ("PSOLHEIGHT", 40, R2007), ("LOFTANG1", 40, R2007),
    ("LOFTANG2", 40, R2007), ("LOFTMAG1", 40, R2007), ("LOFTMAG2", 40, R2007), ("LOFTPARAM", 70, R2007),
    ("LOFTNORMALS", 280, R2007), ("LATITUDE", 40, R2007), ("LONGITUDE", 40, R2007),
    ("NORTHDIRECTION", 40, R2007), ("TIMEZONE", 70, R2007), ("LIGHTGLYPHDISPLAY", 280, R2007),
    ("TILEMODELIGHTSYNCH", 280, R2007), ("SOLIDHIST", 280, R2007), ("SHOWHIST", 280, R2007),
    ("DWFFRAME", 280, R2007), ("DGNFRAME", 280, R2007), ("CSHADOW", 280, R2007),
    ("INTERFERECOLOR", 62, R2007),
    ]
};

/// Whether an object outside the OBJECTS section (entity, table record,
/// block record) will be written, for owner checks.
fn owner_written(o: &Object) -> bool {
    if o.error.is_some() {
        return false;
    }
    if o.is_entity() {
        return !matches!(o.kind, Kind::Unsupported);
    }
    matches!(o.kind, Kind::Table { .. } | Kind::TableEntry { .. } | Kind::BlockHeader(_))
}

/// Whether a non-entity object is fully decoded and can be written to OBJECTS.
fn writable_object(o: &Object) -> bool {
    if o.error.is_some() || o.is_entity() {
        return false;
    }
    match &o.kind {
        Kind::Dictionary { .. } => true,
        Kind::Plain => matches!(
            o.dxf_name.as_str(),
            "XRECORD"
                | "ACDBPLACEHOLDER"
                | "LAYOUT"
                | "PLOTSETTINGS"
                | "MLINESTYLE"
                | "IMAGEDEF"
                | "IMAGEDEF_REACTOR"
                | "RASTERVARIABLES"
                | "WIPEOUTVARIABLES"
                | "DICTIONARYVAR"
        ),
        _ => false,
    }
}
