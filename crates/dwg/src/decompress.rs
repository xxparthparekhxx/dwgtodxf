//! R2004+ section page decompression (spec 4.7), an LZ77 variant.

use crate::error::{Error, Result};

struct Src<'a> {
    d: &'a [u8],
    i: usize,
}

impl Src<'_> {
    #[inline]
    fn byte(&mut self) -> Result<u8> {
        let b = *self.d.get(self.i).ok_or(Error::Eof)?;
        self.i += 1;
        Ok(b)
    }
    #[inline]
    fn peek(&self) -> Result<u8> {
        self.d.get(self.i).copied().ok_or(Error::Eof)
    }

    /// Literal length; returns 0 without consuming if the next byte is an opcode.
    fn literal_length(&mut self) -> Result<usize> {
        let b = self.peek()?;
        if b >= 0x10 {
            return Ok(0);
        }
        self.i += 1;
        if b != 0 {
            return Ok(b as usize + 3);
        }
        let mut total = 0x0F;
        loop {
            let b = self.byte()?;
            if b == 0 {
                total += 0xFF;
            } else {
                return Ok(total + b as usize + 3);
            }
        }
    }

    fn long_compression_offset(&mut self) -> Result<usize> {
        let b = self.byte()?;
        if b != 0 {
            return Ok(b as usize);
        }
        let mut total = 0xFF;
        loop {
            let b = self.byte()?;
            if b == 0 {
                total += 0xFF;
            } else {
                return Ok(total + b as usize);
            }
        }
    }

    /// Returns (offset, literal count 0-3).
    fn two_byte_offset(&mut self) -> Result<(usize, usize)> {
        let b1 = self.byte()? as usize;
        let b2 = self.byte()? as usize;
        Ok(((b1 >> 2) | (b2 << 6), b1 & 3))
    }

    fn copy_literal(&mut self, out: &mut Vec<u8>, n: usize) -> Result<()> {
        let s = self.d.get(self.i..self.i + n).ok_or(Error::Eof)?;
        out.extend_from_slice(s);
        self.i += n;
        Ok(())
    }
}

/// Decompress one R2004 page. `size_hint` is the expected decompressed size; output beyond it (plus a
/// little slack) is treated as corruption.
pub fn decompress_r18(src: &[u8], size_hint: usize) -> Result<Vec<u8>> {
    let limit = size_hint.max(0x7400) + 0x1000;
    let mut out = Vec::with_capacity(size_hint.min(1 << 24));
    let mut s = Src { d: src, i: 0 };

    let lit = s.literal_length()?;
    s.copy_literal(&mut out, lit)?;

    while s.i < src.len() {
        let op1 = s.byte()?;
        let (comp_bytes, comp_offset, mut lit_count) = match op1 {
            0x10 => {
                let n = s.long_compression_offset()? + 9;
                let (off, lc) = s.two_byte_offset()?;
                (n, off + 0x3FFF, lc)
            }
            0x11 => break,
            0x12..=0x1F => {
                let n = (op1 & 0x0F) as usize + 2;
                let (off, lc) = s.two_byte_offset()?;
                (n, off + 0x3FFF, lc)
            }
            0x20 => {
                let n = s.long_compression_offset()? + 0x21;
                let (off, lc) = s.two_byte_offset()?;
                (n, off, lc)
            }
            0x21..=0x3F => {
                let n = op1 as usize - 0x1E;
                let (off, lc) = s.two_byte_offset()?;
                (n, off, lc)
            }
            0x40..=0xFF => {
                let n = ((op1 & 0xF0) >> 4) as usize - 1;
                let op2 = s.byte()? as usize;
                let off = (op2 << 2) | ((op1 as usize & 0x0C) >> 2);
                (n, off, (op1 & 3) as usize)
            }
            _ => {
                return Err(Error::Corrupt(format!("bad compression opcode {op1:#x}")));
            }
        };

        if out.len() + comp_bytes + lit_count > limit {
            return Err(Error::Corrupt("decompressed page larger than declared".into()));
        }
        // back-reference; may overlap the bytes being written
        let start = out
            .len()
            .checked_sub(comp_offset + 1)
            .ok_or_else(|| Error::Corrupt("compression offset before start".into()))?;
        if comp_bytes <= comp_offset + 1 {
            out.extend_from_within(start..start + comp_bytes);
        } else {
            for k in 0..comp_bytes {
                let b = out[start + k];
                out.push(b);
            }
        }

        if lit_count == 0 {
            lit_count = s.literal_length()?;
        }
        s.copy_literal(&mut out, lit_count)?;
    }
    Ok(out)
}
