//! R2007 (AC1021) container: Reed-Solomon framed pages and the R21
//! compression (spec chapter 5).
//!
//! Reading does not need error correction: when the data is intact, decoding
//! a Reed-Solomon code word is just taking its data bytes (systematic code),
//! after undoing the interleaving.

use std::collections::HashMap;

use crate::error::{cap, check_section_size, corrupt, Error, Result};
use crate::file::Sections;
use crate::version::Version;

fn u64le(d: &[u8], o: usize) -> Result<u64> {
    d.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).ok_or(Error::Eof)
}

/// Take the data bytes of `blocks` interleaved RS(255,k) code words.
fn rs_deinterleave(raw: &[u8], k: usize, blocks: usize) -> Result<Vec<u8>> {
    if blocks.checked_mul(255).is_none_or(|n| raw.len() < n) {
        return Err(Error::Eof);
    }
    let mut out = Vec::with_capacity(blocks * k);
    for b in 0..blocks {
        for i in 0..k {
            out.push(raw[i * blocks + b]);
        }
    }
    Ok(out)
}

/// Non-interleaved: data blocks first, parity after.
fn rs_plain(raw: &[u8], k: usize, blocks: usize) -> Result<Vec<u8>> {
    raw.get(..blocks.checked_mul(k).ok_or(Error::Eof)?).map(|s| s.to_vec()).ok_or(Error::Eof)
}

// ---------------------------------------------------------------------------
// R21 decompression (spec 5.10)

/// Byte order of literal copies: for n = 1..=32 bytes, a list of
/// (block length, source index); blocks of length > 1 recurse.
const LITERAL_ORDER: [&[(u8, u8)]; 33] = [
    &[],
    &[(1, 0)],
    &[(1, 1), (1, 0)],
    &[(1, 2), (1, 1), (1, 0)],
    &[(1, 0), (1, 1), (1, 2), (1, 3)],
    &[(1, 4), (4, 0)],
    &[(1, 5), (4, 1), (1, 0)],
    &[(2, 5), (4, 1), (1, 0)],
    &[(4, 0), (4, 4)],
    &[(1, 8), (8, 0)],
    &[(1, 9), (8, 1), (1, 0)],
    &[(2, 9), (8, 1), (1, 0)],
    &[(4, 8), (8, 0)],
    &[(1, 12), (4, 8), (8, 0)],
    &[(1, 13), (4, 9), (8, 1), (1, 0)],
    &[(2, 13), (4, 9), (8, 1), (1, 0)],
    &[(8, 8), (8, 0)],
    &[(8, 9), (1, 8), (8, 0)],
    &[(1, 17), (16, 1), (1, 0)],
    &[(3, 16), (16, 0)],
    &[(4, 16), (16, 0)],
    &[(1, 20), (4, 16), (16, 0)],
    &[(2, 20), (4, 16), (16, 0)],
    &[(3, 20), (4, 16), (16, 0)],
    &[(8, 16), (16, 0)],
    &[(8, 17), (1, 16), (16, 0)],
    &[(1, 25), (8, 17), (1, 16), (16, 0)],
    &[(2, 25), (8, 17), (1, 16), (16, 0)],
    &[(4, 24), (8, 16), (16, 0)],
    &[(1, 28), (4, 24), (8, 16), (16, 0)],
    &[(2, 28), (4, 24), (8, 16), (16, 0)],
    &[(1, 30), (4, 26), (8, 18), (16, 2), (2, 0)],
    &[(16, 16), (16, 0)],
];

fn emit_literal(n: usize, src: &[u8], out: &mut Vec<u8>) {
    for &(len, idx) in LITERAL_ORDER[n] {
        if len == 1 {
            out.push(src[idx as usize]);
        } else {
            emit_literal(len as usize, &src[idx as usize..], out);
        }
    }
}

fn copy_literal(src: &[u8], i: &mut usize, mut n: usize, out: &mut Vec<u8>) -> Result<()> {
    if *i + n > src.len() {
        return Err(Error::Eof);
    }
    while n >= 32 {
        emit_literal(32, &src[*i..], out);
        *i += 32;
        n -= 32;
    }
    if n > 0 {
        emit_literal(n, &src[*i..], out);
        *i += n;
    }
    Ok(())
}

fn byte(src: &[u8], i: &mut usize) -> Result<u8> {
    let b = *src.get(*i).ok_or(Error::Eof)?;
    *i += 1;
    Ok(b)
}

fn literal_length(src: &[u8], i: &mut usize, op: u8) -> Result<usize> {
    let mut len = op as usize + 8;
    if len == 0x17 {
        let n = byte(src, i)? as usize;
        len += n;
        if n == 0xFF {
            loop {
                let n = byte(src, i)? as usize | (byte(src, i)? as usize) << 8;
                len += n;
                if n != 0xFFFF {
                    break;
                }
            }
        }
    }
    Ok(len)
}

/// Returns (offset, length) and updates `op`.
fn instructions(src: &[u8], i: &mut usize, op: &mut u8) -> Result<(usize, usize)> {
    let (offset, length);
    match *op >> 4 {
        0 => {
            let l = (*op & 0xF) as usize + 0x13;
            let o = byte(src, i)? as usize;
            *op = byte(src, i)?;
            length = ((*op as usize >> 3) & 0x10) + l;
            offset = ((*op as usize & 0x78) << 5) + 1 + o;
        }
        1 => {
            length = (*op & 0xF) as usize + 3;
            let o = byte(src, i)? as usize;
            *op = byte(src, i)?;
            offset = ((*op as usize & 0xF8) << 5) + 1 + o;
        }
        2 => {
            let mut o = byte(src, i)? as usize;
            o |= (byte(src, i)? as usize) << 8;
            let mut l = (*op & 7) as usize;
            if *op & 8 == 0 {
                *op = byte(src, i)?;
                l += (*op & 0xF8) as usize;
            } else {
                o += 1;
                l += (byte(src, i)? as usize) << 3;
                *op = byte(src, i)?;
                l += ((*op as usize & 0xF8) << 8) + 0x100;
            }
            offset = o;
            length = l;
        }
        _ => {
            length = (*op >> 4) as usize;
            let o = (*op & 15) as usize;
            *op = byte(src, i)?;
            offset = ((*op as usize & 0xF8) << 1) + o + 1;
        }
    }
    Ok((offset, length))
}

pub fn decompress_r21(src: &[u8], size: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(size.min(1 << 24));
    let mut i = 0usize;
    let mut length = 0usize;
    let mut op = byte(src, &mut i)?;
    if op & 0xF0 == 0x20 {
        i += 2;
        length = (byte(src, &mut i)? & 7) as usize;
    }
    while i < src.len() {
        if length == 0 {
            length = literal_length(src, &mut i, op)?;
        }
        if out.len() + length > size + 64 {
            return Err(Error::Corrupt("R21 data larger than declared".into()));
        }
        copy_literal(src, &mut i, length, &mut out)?;
        if i >= src.len() {
            break;
        }
        op = byte(src, &mut i)?;
        let (mut offset, mut len) = instructions(src, &mut i, &mut op)?;
        loop {
            let start = out
                .len()
                .checked_sub(offset)
                .ok_or_else(|| Error::Corrupt("R21 back-reference before start".into()))?;
            if out.len() + len > size + 64 {
                return Err(Error::Corrupt("R21 data larger than declared".into()));
            }
            for k in 0..len {
                let b = out[start + k];
                out.push(b);
            }
            length = (op & 7) as usize;
            if length != 0 || i >= src.len() {
                break;
            }
            op = byte(src, &mut i)?;
            if op >> 4 == 0 {
                break;
            }
            if op >> 4 == 0xF {
                op &= 0xF;
            }
            (offset, len) = instructions(src, &mut i, &mut op)?;
        }
        if out.len() >= size {
            break;
        }
    }
    out.truncate(size);
    if out.len() < size {
        // short stream: zero-fill small gaps, reject implausible sizes
        if size - out.len() > 1 << 20 {
            return Err(Error::Corrupt("R21 data shorter than declared".into()));
        }
        out.resize(size, 0);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------

/// Decode a system page (page map, section map).
fn system_page(data: &[u8], pos: usize, comp: usize, uncomp: usize, factor: usize) -> Result<Vec<u8>> {
    let total = comp
        .checked_add(7)
        .and_then(|a| (a & !7).checked_mul(factor.max(1)))
        .filter(|t| *t <= data.len() * 2)
        .ok_or(Error::Eof)?;
    let blocks = total.div_ceil(239);
    let raw = data.get(pos..pos.saturating_add(blocks * 255)).ok_or(Error::Eof)?;
    let dec = rs_deinterleave(raw, 239, blocks)?;
    let payload = dec.get(..comp).ok_or(Error::Eof)?;
    if comp < uncomp {
        decompress_r21(payload, uncomp)
    } else {
        Ok(payload[..uncomp.min(payload.len())].to_vec())
    }
}

struct Page {
    offset: u64,
    size: u64,
}

pub fn read_r2007(data: &[u8]) -> Result<Sections> {
    let maint = *data.get(0x0B).ok_or(Error::Eof)?;
    let codepage = data.get(0x13..0x15).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or(Error::Eof)?;

    // file header: RS(255,239) x3 at 0x80
    let raw = data.get(0x80..0x80 + 0x3D8).ok_or(Error::Eof)?;
    let dec = rs_deinterleave(raw, 239, 3)?;
    let comp_len = i32::from_le_bytes(dec[0x18..0x1C].try_into().unwrap());
    let hdr = if comp_len < 0 {
        let n = (-comp_len) as usize;
        dec.get(0x20..0x20 + n).ok_or(Error::Eof)?.to_vec()
    } else {
        decompress_r21(dec.get(0x20..0x20 + comp_len as usize).ok_or(Error::Eof)?, 0x110)?
    };
    let pm_factor = u64le(&hdr, 0x18)? as usize;
    let pm_offset = u64le(&hdr, 0x38)? as usize + 0x480;
    let pm_comp = u64le(&hdr, 0x50)? as usize;
    let pm_uncomp = u64le(&hdr, 0x58)? as usize;
    let sm_comp = u64le(&hdr, 0xB0)? as usize;
    let sm_id = u64le(&hdr, 0xC0)?;
    let sm_uncomp = u64le(&hdr, 0xC8)? as usize;
    let sm_factor = u64le(&hdr, 0xD8)? as usize;

    // page map
    let pm = system_page(data, pm_offset, pm_comp, pm_uncomp, pm_factor)?;
    let mut pages: HashMap<u64, Page> = HashMap::new();
    let mut offset = 0u64;
    let mut p = 0;
    while p + 16 <= pm.len() {
        let size = u64le(&pm, p)?;
        let id = u64le(&pm, p + 8)? as i64;
        pages.insert(id.unsigned_abs(), Page { offset, size });
        offset += size;
        p += 16;
    }

    // section map
    let smp = pages.get(&sm_id).ok_or_else(|| Error::Corrupt("section map page missing".into()))?;
    let sm = system_page(data, smp.offset as usize + 0x480, sm_comp, sm_uncomp, sm_factor)?;

    let mut out = Sections {
        version: Version::R2007,
        maint,
        codepage,
        header: vec![],
        classes: vec![],
        handles: vec![],
        objects: vec![],
        datastorage: vec![],
    };
    let mut p = 0usize;
    while p + 64 <= sm.len() {
        let data_size = u64le(&sm, p)? as usize;
        let max_size = u64le(&sm, p + 8)? as usize;
        let encryption = u64le(&sm, p + 16)?;
        let name_len = u64le(&sm, p + 32)? as usize;
        let encoding = u64le(&sm, p + 48)?;
        let num_pages = u64le(&sm, p + 56)? as usize;
        p += 64;
        let mut name = String::new();
        if name_len > 0 {
            // name length is in bytes, including the UTF-16 terminator
            let raw = sm.get(p..p + name_len).ok_or(Error::Eof)?;
            let u: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            name = String::from_utf16_lossy(&u).trim_end_matches('\0').to_string();
            p += name_len;
        }
        let mut page_list = Vec::with_capacity(cap(num_pages));
        for _ in 0..num_pages {
            let off = u64le(&sm, p)? as usize;
            let id = u64le(&sm, p + 16)?;
            let uncomp = u64le(&sm, p + 24)? as usize;
            let comp = u64le(&sm, p + 32)? as usize;
            page_list.push((off, id, uncomp, comp));
            p += 56;
        }
        let target = match name.as_str() {
            "AcDb:Header" => &mut out.header,
            "AcDb:Classes" => &mut out.classes,
            "AcDb:Handles" => &mut out.handles,
            "AcDb:AcDbObjects" => &mut out.objects,
            _ => continue,
        };
        if encryption != 0 {
            return Err(Error::Unsupported("password-protected DWG".into()));
        }
        let data_size = check_section_size(data_size as u64, data.len())?;
        let mut buf = vec![0u8; data_size];
        for (off, id, uncomp, comp) in page_list {
            let pg = pages.get(&id).ok_or_else(|| Error::Corrupt(format!("page {id} missing")))?;
            let pos = pg.offset as usize + 0x480;
            let aligned = comp.checked_add(7).ok_or(Error::Eof)? & !7;
            if aligned > data.len() {
                return Err(Error::Eof);
            }
            let blocks = aligned.div_ceil(251);
            let end = pos.saturating_add((pg.size as usize).max(blocks * 255));
            let raw = data.get(pos..end).ok_or(Error::Eof)?;
            let dec = if encoding == 4 {
                rs_deinterleave(raw, 251, blocks)?
            } else {
                rs_plain(raw, 251, blocks)?
            };
            let payload = &dec[..comp.min(dec.len())];
            let page = if comp < uncomp { decompress_r21(payload, uncomp)? } else { payload.to_vec() };
            if off >= buf.len() {
                continue;
            }
            let n = page.len().min(buf.len() - off).min(max_size.max(uncomp));
            buf[off..off + n].copy_from_slice(&page[..n]);
        }
        *target = buf;
    }
    if out.classes.is_empty() || out.handles.is_empty() || out.objects.is_empty() {
        return corrupt("R2007: required sections missing");
    }
    Ok(out)
}
