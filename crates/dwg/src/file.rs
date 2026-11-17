//! File containers: locate the header, classes, object map and object data
//! for each DWG generation.

use std::collections::HashMap;

use crate::decompress::decompress_r18;
use crate::error::{cap, check_section_size, corrupt, Error, Result};
use crate::version::Version;

/// The logical sections we need, decoded and decompressed.
pub struct Sections {
    pub version: Version,
    pub maint: u8,
    pub codepage: u16,
    /// Header variables section (starting at its sentinel). May be empty.
    pub header: Vec<u8>,
    /// Classes section (starting at its sentinel).
    pub classes: Vec<u8>,
    /// Object map (handle -> offset), starting at the first block size.
    pub handles: Vec<u8>,
    /// Buffer the object map offsets point into.
    pub objects: Vec<u8>,
    /// R2013+ data storage section (binary ACIS of solids), may be empty.
    pub datastorage: Vec<u8>,
}

fn u16le(d: &[u8], o: usize) -> Result<u16> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or(Error::Eof)
}
fn u32le(d: &[u8], o: usize) -> Result<u32> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or(Error::Eof)
}
fn u64le(d: &[u8], o: usize) -> Result<u64> {
    d.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).ok_or(Error::Eof)
}
fn slice(d: &[u8], o: usize, n: usize) -> Result<&[u8]> {
    d.get(o..o.checked_add(n).ok_or(Error::Eof)?).ok_or(Error::Eof)
}

pub fn read_sections(data: &[u8]) -> Result<Sections> {
    let version = Version::from_magic(data)?;
    match version {
        Version::R13 | Version::R14 | Version::R2000 => read_r13(data, version),
        Version::R2007 => crate::r2007::read_r2007(data),
        _ => read_r2004(data, version),
    }
}

// ---------------------------------------------------------------------------
// R13 - R2000

fn read_r13(data: &[u8], version: Version) -> Result<Sections> {
    let codepage = u16le(data, 0x13)?;
    let n = u32le(data, 0x15)? as usize;
    if n > 16 {
        return corrupt("too many section locators");
    }
    let mut loc: HashMap<u8, (usize, usize)> = HashMap::new();
    for i in 0..n {
        let o = 0x19 + i * 9;
        let rec = *data.get(o).ok_or(Error::Eof)?;
        let seeker = u32le(data, o + 1)? as usize;
        let size = u32le(data, o + 5)? as usize;
        loc.insert(rec, (seeker, size));
    }
    let get = |rec: u8| -> Result<Vec<u8>> {
        let (s, n) = *loc.get(&rec).ok_or_else(|| Error::Corrupt(format!("missing section {rec}")))?;
        Ok(slice(data, s, n)?.to_vec())
    };
    let header = get(0).unwrap_or_default();
    let classes = get(1)?;
    let handles = get(2)?;
    Ok(Sections {
        version,
        maint: *data.get(0x0B).ok_or(Error::Eof)?,
        codepage,
        header,
        classes,
        handles,
        objects: data.to_vec(),
        datastorage: vec![],
    })
}

// ---------------------------------------------------------------------------
// R2004 / R2010 / R2013 / R2018

fn magic_sequence() -> [u8; 0x6C] {
    let mut out = [0u8; 0x6C];
    let mut seed: u32 = 1;
    for b in out.iter_mut() {
        seed = seed.wrapping_mul(0x343FD).wrapping_add(0x269EC3);
        *b = (seed >> 16) as u8;
    }
    out
}

struct SectionDesc {
    size: u64,
    max_size: u32,
    compressed: u32,
    encrypted: u32,
    name: String,
    /// (page number, data size, start offset)
    pages: Vec<(i32, u32, u64)>,
}

fn read_system_page(data: &[u8], addr: usize, expect: u32) -> Result<Vec<u8>> {
    let kind = u32le(data, addr)?;
    if kind != expect {
        return corrupt(format!("system page type {kind:#x} at {addr:#x}, expected {expect:#x}"));
    }
    let decomp = u32le(data, addr + 4)? as usize;
    let comp = u32le(data, addr + 8)? as usize;
    let ctype = u32le(data, addr + 12)?;
    let raw = slice(data, addr + 0x14, comp)?;
    if ctype == 2 {
        decompress_r18(raw, decomp)
    } else {
        Ok(raw.to_vec())
    }
}

fn read_r2004(data: &[u8], version: Version) -> Result<Sections> {
    let maint = *data.get(0x0B).ok_or(Error::Eof)?;
    let codepage = u16le(data, 0x13)?;
    let mut hdr = slice(data, 0x80, 0x6C)?.to_vec();
    for (b, m) in hdr.iter_mut().zip(magic_sequence()) {
        *b ^= m;
    }
    if &hdr[0..11] != b"AcFssFcAJMB" {
        return corrupt("bad R2004 file header id");
    }
    let page_map_addr = u64le(&hdr, 0x54)? as usize + 0x100;
    let section_map_id = u32le(&hdr, 0x5C)? as i32;

    // global section page map: page number -> file address
    let pm = read_system_page(data, page_map_addr, 0x4163_0E3B)?;
    let mut page_addr: HashMap<i32, usize> = HashMap::new();
    let mut addr = 0x100usize;
    let mut p = 0;
    while p + 8 <= pm.len() {
        let num = u32le(&pm, p)? as i32;
        let size = u32le(&pm, p + 4)? as usize;
        p += 8;
        if num < 0 {
            p += 16;
        } else {
            page_addr.insert(num, addr);
        }
        addr += size;
    }

    let sm_addr = *page_addr
        .get(&section_map_id)
        .ok_or_else(|| Error::Corrupt("section map page not found".into()))?;
    let sm = read_system_page(data, sm_addr, 0x4163_003B)?;
    let ndesc = u32le(&sm, 0)? as usize;
    let mut descs = Vec::with_capacity(cap(ndesc));
    let mut p = 0x14;
    for _ in 0..ndesc {
        let size = u64le(&sm, p)?;
        let page_count = u32le(&sm, p + 8)? as usize;
        let max_size = u32le(&sm, p + 12)?;
        let compressed = u32le(&sm, p + 20)?;
        let encrypted = u32le(&sm, p + 28)?;
        let name_raw = slice(&sm, p + 32, 64)?;
        let name = crate::bits::decode_8bit(name_raw);
        p += 96;
        let mut pages = Vec::with_capacity(cap(page_count));
        for _ in 0..page_count {
            pages.push((u32le(&sm, p)? as i32, u32le(&sm, p + 4)?, u64le(&sm, p + 8)?));
            p += 16;
        }
        descs.push(SectionDesc { size, max_size, compressed, encrypted, name, pages });
    }

    let find = |name: &str| descs.iter().find(|d| d.name == name);
    let load = |name: &str, required: bool| -> Result<Vec<u8>> {
        let Some(d) = find(name) else {
            return if required { corrupt(format!("missing section {name}")) } else { Ok(vec![]) };
        };
        if d.encrypted == 1 {
            return Err(Error::Unsupported("password-protected DWG".into()));
        }
        assemble(data, &page_addr, d)
    };

    Ok(Sections {
        version,
        maint,
        codepage,
        header: load("AcDb:Header", false)?,
        classes: load("AcDb:Classes", true)?,
        handles: load("AcDb:Handles", true)?,
        objects: load("AcDb:AcDbObjects", true)?,
        datastorage: load("AcDb:AcDsPrototype_1b", false).unwrap_or_default(),
    })
}

fn assemble(data: &[u8], page_addr: &HashMap<i32, usize>, d: &SectionDesc) -> Result<Vec<u8>> {
    let total = check_section_size(d.size, data.len())?;
    let mut buf = vec![0u8; total];
    for &(num, _size, start) in &d.pages {
        let addr = *page_addr
            .get(&num)
            .ok_or_else(|| Error::Corrupt(format!("page {num} of {} not found", d.name)))?;
        // 32-byte masked data page header
        let mut h = [0u8; 32];
        h.copy_from_slice(slice(data, addr, 32)?);
        let mask = (0x4164_536Bu32 ^ addr as u32).to_le_bytes();
        for (i, b) in h.iter_mut().enumerate() {
            *b ^= mask[i & 3];
        }
        let kind = u32::from_le_bytes(h[0..4].try_into().unwrap());
        if kind != 0x4163_043B {
            return corrupt(format!("bad data page header {kind:#x} in {}", d.name));
        }
        let data_size = u32::from_le_bytes(h[8..12].try_into().unwrap()) as usize;
        let raw = slice(data, addr + 32, data_size)?;
        let start = start as usize;
        if start >= total {
            continue;
        }
        let want = (total - start).min(d.max_size as usize);
        let page = if d.compressed == 2 {
            decompress_r18(raw, (d.max_size as usize).min(1 << 24))?
        } else {
            raw.to_vec()
        };
        let n = want.min(page.len());
        buf[start..start + n].copy_from_slice(&page[..n]);
    }
    Ok(buf)
}
