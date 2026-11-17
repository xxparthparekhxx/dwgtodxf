//! AcDb:AcDsPrototype_1b — the R2013+ data storage section (spec chapter 24).
//! We only need the binary ACIS (SAB) record of each 3DSOLID/REGION/BODY,
//! keyed by the owning entity's handle.

use std::collections::HashMap;

const SEG_HEADER: usize = 48;
const BLOB_REF: u32 = 0xBB10_6BB1;

fn u32at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}
fn u64at(d: &[u8], o: usize) -> Option<u64> {
    d.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
}

fn is_sab(b: &[u8]) -> bool {
    b.starts_with(b"ACIS BinaryFile") || b.starts_with(b"ASM BinaryFile")
}

/// handle -> SAB bytes. Malformed input yields whatever could be read.
pub fn sab_records(d: &[u8]) -> HashMap<u64, Vec<u8>> {
    read(d).unwrap_or_default()
}

fn read(d: &[u8]) -> Option<HashMap<u64, Vec<u8>>> {
    let mut out = HashMap::new();
    let segidx_off = u32at(d, 24)? as usize;
    let segidx_count = u32at(d, 32)? as usize;
    let datidx_seg = u32at(d, 40)? as usize;

    // segment index: (offset, size) per segment
    let mut segs = Vec::with_capacity(segidx_count.min(1 << 16));
    let mut p = segidx_off + SEG_HEADER;
    for _ in 0..segidx_count.min(1 << 20) {
        segs.push((u64at(d, p)? as usize, u32at(d, p + 8)? as usize));
        p += 12;
    }
    let (dat_off, _) = *segs.get(datidx_seg)?;

    // data index: (segment, local offset of the record header)
    let mut p = dat_off + SEG_HEADER;
    let n = u32at(d, p)? as usize;
    p += 8;
    let mut by_seg: HashMap<usize, Vec<usize>> = HashMap::new();
    for _ in 0..n.min(1 << 20) {
        let seg = u32at(d, p)? as usize;
        let local = u32at(d, p + 4)? as usize;
        p += 12;
        if seg != 0 {
            by_seg.entry(seg).or_default().push(local);
        }
    }

    for (seg, mut locals) in by_seg {
        let Some(&(start, size)) = segs.get(seg) else { continue };
        locals.sort_unstable();
        // record headers: size u32, unknown u32, handle u64, local offset u32
        let mut hdrs = vec![];
        for &l in &locals {
            // header offsets are relative to the end of the segment header
            let h = start + SEG_HEADER + l;
            if let (Some(handle), Some(off)) = (u64at(d, h + 8), u32at(d, h + 16)) {
                hdrs.push((handle, off as usize));
            }
        }
        // records start at the "object data alignment" offset (in 16-byte units)
        let Some(align) = u32at(d, start + 36) else { continue };
        let data_start = start + ((align as usize) << 4);
        let seg_end = (start + size).min(d.len());
        let mut offs: Vec<usize> = hdrs.iter().map(|h| h.1).collect();
        offs.sort_unstable();
        for &(handle, off) in &hdrs {
            let rec = data_start + off;
            let next = offs.iter().copied().find(|&o| o > off).map(|o| data_start + o).unwrap_or(seg_end);
            let max = next.saturating_sub(rec);
            let Some(dsize) = u32at(d, rec) else { continue };
            let bytes = if (dsize as usize) + 4 <= max {
                d.get(rec + 4..rec + 4 + dsize as usize).map(|b| b.to_vec())
            } else if dsize == BLOB_REF {
                blob(d, rec + 4, &segs)
            } else {
                None
            };
            if let Some(b) = bytes {
                if is_sab(&b) {
                    out.insert(handle, b);
                }
            }
        }
    }
    Some(out)
}

fn blob(d: &[u8], p: usize, segs: &[(usize, usize)]) -> Option<Vec<u8>> {
    let total = u64at(d, p)? as usize;
    let pages = u32at(d, p + 8)? as usize;
    let mut out = Vec::with_capacity(total.min(1 << 26));
    let mut q = p + 28;
    for _ in 0..pages.min(1 << 16) {
        let seg = u32at(d, q)? as usize;
        q += 8;
        let (start, _) = *segs.get(seg)?;
        let h = start + SEG_HEADER;
        let len = u64at(d, h + 24)? as usize;
        out.extend_from_slice(d.get(h + 32..h + 32 + len)?);
    }
    out.truncate(total);
    Some(out)
}
