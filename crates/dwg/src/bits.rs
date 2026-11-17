//! Bit-level reader for the DWG "bit codes" (spec chapter 2).
//!
//! DWG data is a big-endian *bit* stream (MSB of each byte first) carrying
//! little-endian multi-byte values.

use crate::error::{Error, Result};
use crate::version::Version;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Point3 { x, y, z }
    }
    pub const Z: Point3 = Point3::new(0.0, 0.0, 1.0);
}

/// A raw handle reference: |CODE(4)|COUNTER(4)|bytes...|
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HandleRef {
    pub code: u8,
    pub value: u64,
}

impl HandleRef {
    /// Resolve to an absolute handle. Offset codes (6, 8, A, C) are relative
    /// to the handle of the object being read.
    pub fn absolute(self, reference: u64) -> u64 {
        match self.code {
            0x6 => reference.wrapping_add(1),
            0x8 => reference.wrapping_sub(1),
            0xA => reference.wrapping_add(self.value),
            0xC => reference.wrapping_sub(self.value),
            _ => self.value,
        }
    }
}

/// Colour as stored in DWG (CMC / ENC).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Color {
    /// ACI index; 256 = BYLAYER, 0 = BYBLOCK.
    pub index: i16,
    /// 24-bit true colour, if set.
    pub rgb: Option<u32>,
    /// Raw transparency value (DXF 440), if set.
    pub transparency: Option<u32>,
    /// Colour book name ("BOOK$NAME"), if set.
    pub name: Option<String>,
    /// R2004+ entity colour references a DBCOLOR object (handle in handle stream).
    pub has_book_ref: bool,
}

impl Color {
    pub fn bylayer() -> Self {
        Color { index: 256, ..Default::default() }
    }
}

#[derive(Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    /// current bit position
    pos: usize,
    /// bit limit (exclusive)
    end: usize,
    pub ver: Version,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8], ver: Version) -> Self {
        BitReader { data, pos: 0, end: data.len() * 8, ver }
    }

    /// Reader over `data` limited to the first `end_bit` bits.
    pub fn with_end(data: &'a [u8], ver: Version, end_bit: usize) -> Self {
        BitReader { data, pos: 0, end: end_bit.min(data.len() * 8), ver }
    }

    #[inline]
    pub fn pos(&self) -> usize {
        self.pos
    }
    #[inline]
    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }
    #[inline]
    pub fn end(&self) -> usize {
        self.end
    }
    pub fn set_end(&mut self, end: usize) {
        self.end = end.min(self.data.len() * 8);
    }
    #[inline]
    pub fn remaining(&self) -> usize {
        self.end.saturating_sub(self.pos)
    }
    pub fn data(&self) -> &'a [u8] {
        self.data
    }
    pub fn align_byte(&mut self) {
        self.pos = (self.pos + 7) & !7;
    }

    #[inline]
    fn need(&self, bits: usize) -> Result<()> {
        // overflow-safe: sizes come from untrusted files
        if bits > self.end.saturating_sub(self.pos) {
            Err(Error::Eof)
        } else {
            Ok(())
        }
    }

    // ---- raw bits -------------------------------------------------------

    #[inline]
    pub fn b(&mut self) -> Result<bool> {
        self.need(1)?;
        let v = (self.data[self.pos >> 3] >> (7 - (self.pos & 7))) & 1;
        self.pos += 1;
        Ok(v == 1)
    }

    /// Read up to 8 bits as an unsigned value.
    #[inline]
    fn bits8(&mut self, n: usize) -> Result<u8> {
        self.need(n)?;
        let byte = self.pos >> 3;
        let off = self.pos & 7;
        let hi = self.data[byte] as u16;
        let lo = if off + n > 8 { self.data[byte + 1] as u16 } else { 0 };
        let word = (hi << 8) | lo;
        let v = (word >> (16 - off - n)) & ((1u16 << n) - 1);
        self.pos += n;
        Ok(v as u8)
    }

    #[inline]
    pub fn bb(&mut self) -> Result<u8> {
        self.bits8(2)
    }

    /// 3B: 1-3 bits, stop at the first 0 bit.
    pub fn b3(&mut self) -> Result<u8> {
        let mut v = 0u8;
        for _ in 0..3 {
            let bit = self.b()?;
            v = (v << 1) | bit as u8;
            if !bit {
                break;
            }
        }
        Ok(v)
    }

    #[inline]
    pub fn rc(&mut self) -> Result<u8> {
        self.need(8)?;
        let byte = self.pos >> 3;
        let off = self.pos & 7;
        let v = if off == 0 {
            self.data[byte]
        } else {
            (self.data[byte] << off) | (self.data[byte + 1] >> (8 - off))
        };
        self.pos += 8;
        Ok(v)
    }

    pub fn bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        self.need(n.checked_mul(8).ok_or(Error::Eof)?)?;
        if self.pos & 7 == 0 {
            let s = self.pos >> 3;
            self.pos += n * 8;
            return Ok(self.data[s..s + n].to_vec());
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.rc()?);
        }
        Ok(v)
    }

    pub fn skip_bits(&mut self, n: usize) -> Result<()> {
        self.need(n)?;
        self.pos += n;
        Ok(())
    }

    #[inline]
    fn le<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.need(N * 8)?;
        let mut a = [0u8; N];
        if self.pos & 7 == 0 {
            let s = self.pos >> 3;
            a.copy_from_slice(&self.data[s..s + N]);
            self.pos += N * 8;
        } else {
            for x in a.iter_mut() {
                *x = self.rc()?;
            }
        }
        Ok(a)
    }

    #[inline]
    pub fn rs(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.le::<2>()?))
    }
    #[inline]
    pub fn rl(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.le::<4>()?))
    }
    #[inline]
    pub fn rd(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.le::<8>()?))
    }
    pub fn rll(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.le::<8>()?))
    }

    // ---- compressed numbers --------------------------------------------

    #[inline]
    pub fn bs(&mut self) -> Result<u16> {
        Ok(match self.bb()? {
            0 => self.rs()?,
            1 => self.rc()? as u16,
            2 => 0,
            _ => 256,
        })
    }

    #[inline]
    pub fn bs_i(&mut self) -> Result<i16> {
        Ok(self.bs()? as i16)
    }

    #[inline]
    pub fn bl(&mut self) -> Result<u32> {
        Ok(match self.bb()? {
            0 => self.rl()?,
            1 => self.rc()? as u32,
            2 => 0,
            _ => return Err(Error::Corrupt("invalid BL code 3".into())),
        })
    }

    #[inline]
    pub fn bl_i(&mut self) -> Result<i32> {
        Ok(self.bl()? as i32)
    }

    /// BLL: 3 bits giving the byte count, then that many bytes (LSB first).
    pub fn bll(&mut self) -> Result<u64> {
        let n = self.bits8(3)? as usize;
        let mut v = 0u64;
        for i in 0..n {
            v |= (self.rc()? as u64) << (8 * i);
        }
        Ok(v)
    }

    #[inline]
    pub fn bd(&mut self) -> Result<f64> {
        Ok(match self.bb()? {
            0 => self.rd()?,
            1 => 1.0,
            2 => 0.0,
            _ => return Err(Error::Corrupt("invalid BD code 3".into())),
        })
    }

    /// DD: bit double with default.
    pub fn dd(&mut self, default: f64) -> Result<f64> {
        match self.bb()? {
            0 => Ok(default),
            1 => {
                let mut b = default.to_le_bytes();
                for x in b.iter_mut().take(4) {
                    *x = self.rc()?;
                }
                Ok(f64::from_le_bytes(b))
            }
            2 => {
                let mut b = default.to_le_bytes();
                b[4] = self.rc()?;
                b[5] = self.rc()?;
                for x in b.iter_mut().take(4) {
                    *x = self.rc()?;
                }
                Ok(f64::from_le_bytes(b))
            }
            _ => self.rd(),
        }
    }

    /// Modular char, signed (0x40 of the last byte is the sign).
    pub fn mc(&mut self) -> Result<i64> {
        let mut v: i64 = 0;
        let mut shift = 0;
        for _ in 0..9 {
            let b = self.rc()?;
            if b & 0x80 == 0 {
                let neg = b & 0x40 != 0;
                v |= ((b & 0x3F) as i64) << shift;
                return Ok(if neg { -v } else { v });
            }
            v |= ((b & 0x7F) as i64) << shift;
            shift += 7;
        }
        Err(Error::Corrupt("modular char too long".into()))
    }

    /// Modular char, unsigned.
    pub fn umc(&mut self) -> Result<u64> {
        let mut v: u64 = 0;
        let mut shift = 0;
        for _ in 0..10 {
            let b = self.rc()?;
            v |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
        }
        Err(Error::Corrupt("modular char too long".into()))
    }

    /// Modular short (object sizes).
    pub fn ms(&mut self) -> Result<u32> {
        let mut v: u32 = 0;
        let mut shift = 0;
        for _ in 0..3 {
            let w = self.rs()?;
            v |= ((w & 0x7FFF) as u32) << shift;
            if w & 0x8000 == 0 {
                return Ok(v);
            }
            shift += 15;
        }
        Err(Error::Corrupt("modular short too long".into()))
    }

    // ---- composite types ------------------------------------------------

    pub fn h(&mut self) -> Result<HandleRef> {
        let b = self.rc()?;
        let code = b >> 4;
        let n = (b & 0x0F) as usize;
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 8) | self.rc()? as u64;
        }
        Ok(HandleRef { code, value: v })
    }

    /// Read a handle and resolve it relative to `reference`.
    #[inline]
    pub fn ha(&mut self, reference: u64) -> Result<u64> {
        Ok(self.h()?.absolute(reference))
    }

    pub fn rd2(&mut self) -> Result<Point3> {
        Ok(Point3::new(self.rd()?, self.rd()?, 0.0))
    }
    pub fn rd3(&mut self) -> Result<Point3> {
        Ok(Point3::new(self.rd()?, self.rd()?, self.rd()?))
    }
    pub fn bd2(&mut self) -> Result<Point3> {
        Ok(Point3::new(self.bd()?, self.bd()?, 0.0))
    }
    pub fn bd3(&mut self) -> Result<Point3> {
        Ok(Point3::new(self.bd()?, self.bd()?, self.bd()?))
    }
    pub fn dd2(&mut self, d: Point3) -> Result<Point3> {
        Ok(Point3::new(self.dd(d.x)?, self.dd(d.y)?, 0.0))
    }
    pub fn dd3(&mut self, d: Point3) -> Result<Point3> {
        Ok(Point3::new(self.dd(d.x)?, self.dd(d.y)?, self.dd(d.z)?))
    }

    /// BE: bit extrusion.
    pub fn be(&mut self) -> Result<Point3> {
        if self.ver >= Version::R2000 {
            if self.b()? {
                return Ok(Point3::Z);
            }
        }
        let p = self.bd3()?;
        // AutoCAD writes (0,0,0) for "default" in some old files.
        if p == Point3::default() {
            return Ok(Point3::Z);
        }
        Ok(p)
    }

    /// BT: bit thickness.
    pub fn bt(&mut self) -> Result<f64> {
        if self.ver >= Version::R2000 && self.b()? {
            return Ok(0.0);
        }
        self.bd()
    }

    /// OT: object type.
    pub fn ot(&mut self) -> Result<u16> {
        if self.ver >= Version::R2010 {
            Ok(match self.bb()? {
                0 => self.rc()? as u16,
                1 => self.rc()? as u16 + 0x1F0,
                _ => self.rs()?,
            })
        } else {
            self.bs()
        }
    }

    /// T: 8-bit string (BS length + bytes).
    pub fn t8(&mut self) -> Result<String> {
        let n = self.bs()? as usize;
        let raw = self.bytes(n)?;
        Ok(decode_8bit(&raw))
    }

    /// TU: UTF-16 string (BS length in chars + UTF-16LE).
    pub fn tu(&mut self) -> Result<String> {
        let n = self.bs()? as usize;
        let mut units = Vec::with_capacity(n);
        for _ in 0..n {
            units.push(self.rs()?);
        }
        while units.last() == Some(&0) {
            units.pop();
        }
        Ok(String::from_utf16_lossy(&units))
    }

    /// Check 16-byte sentinel at current (byte-aligned) position.
    pub fn sentinel(&mut self, s: &[u8; 16]) -> Result<bool> {
        let b = self.bytes(16)?;
        Ok(b == s)
    }
}

/// Decode an 8-bit DWG string. Windows-1252 covers the vast majority of files;
/// non-ASCII text in DWG is usually stored as `\U+XXXX` escapes anyway.
pub fn decode_8bit(raw: &[u8]) -> String {
    let raw = match raw.iter().position(|&b| b == 0) {
        Some(i) => &raw[..i],
        None => raw,
    };
    if raw.is_ascii() {
        // SAFETY: ASCII is valid UTF-8
        return unsafe { std::str::from_utf8_unchecked(raw) }.to_owned();
    }
    const CP1252: [u16; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013,
        0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    raw.iter()
        .map(|&b| match b {
            0x80..=0x9F => char::from_u32(CP1252[(b - 0x80) as usize] as u32).unwrap_or('?'),
            _ => b as char,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| *c == '0' || *c == '1').collect();
        let mut out = vec![0u8; s.len().div_ceil(8)];
        for (i, c) in s.chars().enumerate() {
            if c == '1' {
                out[i / 8] |= 0x80 >> (i % 8);
            }
        }
        out
    }

    #[test]
    fn bitshort_spec_example() {
        let d = bits("0000000001000000011011010000111110");
        let mut r = BitReader::new(&d, Version::R2000);
        assert_eq!(r.bs().unwrap(), 257);
        assert_eq!(r.bs().unwrap(), 0);
        assert_eq!(r.bs().unwrap(), 256);
        assert_eq!(r.bs().unwrap(), 15);
        assert_eq!(r.bs().unwrap(), 0);
    }

    #[test]
    fn bitlong_spec_example() {
        let d = bits("000000000100000001000000000000000010010000111110");
        let mut r = BitReader::new(&d, Version::R2000);
        assert_eq!(r.bl().unwrap(), 257);
        assert_eq!(r.bl().unwrap(), 0);
        assert_eq!(r.bl().unwrap(), 15);
        assert_eq!(r.bl().unwrap(), 0);
    }

    #[test]
    fn modular_char() {
        let d = [0b1000_0010u8, 0b0010_0100];
        let mut r = BitReader::new(&d, Version::R2000);
        assert_eq!(r.mc().unwrap(), 4610);
        let d = [0x85u8, 0x4B];
        let mut r = BitReader::new(&d, Version::R2000);
        assert_eq!(r.mc().unwrap(), -(0x0B << 7 | 5));
    }

    #[test]
    fn handle() {
        let d = bits("0101 0010 00000101 11100111");
        let mut r = BitReader::new(&d, Version::R2000);
        let h = r.h().unwrap();
        assert_eq!((h.code, h.value), (5, 0x5E7));
    }
}
