//! R2007+ separated string streams (spec 20.1).

use crate::bits::BitReader;
use crate::version::Version;

/// Locate the string stream that ends just before bit `endbit`.
///
/// `endbit` is the first bit *after* the data stream (where the handle stream
/// starts for objects); the "has strings" flag is the bit right before it.
/// Returns a reader positioned at the start of the strings, or `None` if
/// there are no strings.
pub fn string_stream(data: &[u8], ver: Version, endbit: usize) -> Option<BitReader<'_>> {
    if endbit == 0 || endbit > data.len() * 8 {
        return None;
    }
    let mut r = BitReader::new(data, ver);
    let flag_pos = endbit - 1;
    r.set_pos(flag_pos);
    if !r.b().ok()? {
        return None;
    }
    let mut p = flag_pos.checked_sub(16)?;
    r.set_pos(p);
    let mut size = r.rs().ok()? as usize;
    if size & 0x8000 != 0 {
        p = p.checked_sub(16)?;
        r.set_pos(p);
        let hi = r.rs().ok()? as usize;
        size = (size & 0x7FFF) | (hi << 15);
    }
    let start = p.checked_sub(size)?;
    let mut s = BitReader::with_end(data, ver, p);
    s.set_pos(start);
    Some(s)
}
