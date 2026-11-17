//! AcDb:Classes (spec chapter 10) and AcDb:Handles / object map (chapter 23).

use crate::bits::BitReader;
use crate::error::{corrupt, Result};
use crate::strings::string_stream;
use crate::version::Version;

const CLASSES_SN: [u8; 16] = [
    0x8D, 0xA1, 0xC4, 0xB8, 0xC4, 0xA9, 0xF8, 0xC5, 0xC0, 0xDC, 0xF4, 0x5F, 0xE7, 0xCF, 0xB6, 0x8A,
];

#[derive(Debug, Clone, Default)]
pub struct Class {
    pub number: u16,
    pub proxy_flags: u16,
    pub app_name: String,
    pub cpp_name: String,
    pub dxf_name: String,
    pub was_zombie: bool,
    /// 0x1F2 entity, 0x1F3 object
    pub item_class_id: u16,
    pub instance_count: u32,
}

impl Class {
    pub fn is_entity(&self) -> bool {
        self.item_class_id == 0x1F2
    }
}

pub fn read_classes(data: &[u8], ver: Version, maint: u8) -> Result<Vec<Class>> {
    let mut r = BitReader::new(data, ver);
    if !r.sentinel(&CLASSES_SN)? {
        return corrupt("classes sentinel not found");
    }
    let size = r.rl()? as usize;
    if (ver >= Version::R2010 && maint > 3) || ver >= Version::R2018 {
        let _hi = r.rl()?;
    }
    let data_start = r.pos();
    let data_end = data_start + size * 8;
    let mut strings = None;
    if ver >= Version::R2007 {
        let bitsize = r.rl()? as usize;
        // string stream is located from the end of the bit data
        strings = string_stream(data, ver, data_start + bitsize);
    }
    if ver >= Version::R2004 {
        let _max = r.bs()?;
        let _ = r.rc()?;
        let _ = r.rc()?;
        let _ = r.b()?;
    }
    let mut s = strings;
    let mut out = Vec::new();
    while r.pos() + 8 < data_end {
        let mut c = Class { number: r.bs()?, proxy_flags: r.bs()?, ..Default::default() };
        let mut tv = |r: &mut BitReader| -> Result<String> {
            match s.as_mut() {
                Some(sr) => sr.tu(),
                None => r.t8(),
            }
        };
        c.app_name = tv(&mut r)?;
        c.cpp_name = tv(&mut r)?;
        c.dxf_name = tv(&mut r)?;
        c.was_zombie = r.b()?;
        c.item_class_id = r.bs()?;
        if ver >= Version::R2004 {
            c.instance_count = r.bl()?;
            let _dwg_ver = r.bl()?;
            let _maint = r.bl()?;
            let _ = r.bl()?;
            let _ = r.bl()?;
        }
        if c.number < 500 || c.dxf_name.is_empty() {
            break;
        }
        out.push(c);
        if ver >= Version::R2007 {
            if let Some(sr) = s.as_ref() {
                if sr.remaining() < 16 {
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// Parse the object map into (handle, offset) pairs, in file order.
pub fn read_object_map(data: &[u8], ver: Version) -> Result<Vec<(u64, usize)>> {
    let mut out = Vec::with_capacity(data.len() / 3);
    let mut pos = 0usize;
    while pos + 2 <= data.len() {
        // deltas restart at every block
        let mut last_h: i64 = 0;
        let mut last_loc: i64 = 0;
        let size = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        if size <= 2 {
            break;
        }
        let end = (pos + size).min(data.len());
        let mut r = BitReader::with_end(data, ver, end * 8);
        r.set_pos((pos + 2) * 8);
        while r.pos() < end * 8 {
            let dh = r.umc()? as i64;
            let dl = r.mc()?;
            last_h += dh;
            last_loc += dl;
            if last_loc >= 0 {
                out.push((last_h as u64, last_loc as usize));
            }
        }
        pos = end + 2; // CRC
    }
    Ok(out)
}
