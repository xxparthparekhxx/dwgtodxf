//! Pure-Rust DWG reader and DXF writer.
//!
//! ```no_run
//! let bytes = std::fs::read("plan.dwg").unwrap();
//! let drawing = dwg::read(&bytes).unwrap();
//! let dxf = dwg::dxf::write(&drawing, &Default::default());
//! std::fs::write("plan.dxf", dxf).unwrap();
//! ```

pub mod bits;
pub mod classes;
mod datastorage;
mod decompress;
pub mod dxf;
mod entities;
pub mod error;
pub mod file;
pub mod header;
pub mod model;
pub mod objects;
pub mod proxy;
mod r2007;
pub mod spline;
mod strings;
mod tables;
pub mod version;

use rayon::prelude::*;
use std::collections::HashMap;

pub use error::{Error, Result};
pub use model::{Drawing, Object};
pub use version::Version;

/// Decode a DWG file held in memory.
///
/// Never panics: an internal error on a malformed file is reported as
/// [`Error::Corrupt`].
pub fn read(data: &[u8]) -> Result<Drawing> {
    match std::panic::catch_unwind(|| read_inner(data)) {
        Ok(r) => r,
        Err(p) => {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(Error::Corrupt(format!("internal error while decoding: {msg}")))
        }
    }
}

fn read_inner(data: &[u8]) -> Result<Drawing> {
    let sec = file::read_sections(data)?;
    let ver = sec.version;
    let classes = classes::read_classes(&sec.classes, ver, sec.maint)?;
    let header = header::read_header(&sec.header, ver, sec.maint).unwrap_or_default();
    let map = classes::read_object_map(&sec.handles, ver)?;

    // Later entries for the same handle win (spec 20.4: duplicates).
    let mut uniq: HashMap<u64, usize> = HashMap::with_capacity(map.len());
    for (h, off) in map {
        uniq.insert(h, off);
    }
    let mut entries: Vec<(u64, usize)> = uniq.into_iter().collect();
    entries.sort_unstable();

    let results: Vec<std::result::Result<Object, (u64, String)>> = entries
        .par_iter()
        .map(|&(h, off)| {
            objects::read_object(&sec.objects, off, ver, &classes).map_err(|e| (h, e.to_string()))
        })
        .collect();

    let mut d = Drawing {
        version: Some(ver),
        maint: sec.maint,
        codepage: sec.codepage,
        classes,
        header,
        ..Default::default()
    };
    d.objects.reserve(results.len());
    for r in results {
        match r {
            Ok(o) => d.objects.push(o),
            Err((h, e)) => d.failed.push((h, 0, e)),
        }
    }
    // R2013+: binary ACIS of solids lives in the data storage section
    if !sec.datastorage.is_empty() {
        let mut sab = datastorage::sab_records(&sec.datastorage);
        for o in d.objects.iter_mut() {
            if o.sab.is_none() && matches!(o.dxf_name.as_str(), "3DSOLID" | "REGION" | "BODY") {
                if let Some(b) = sab.remove(&o.handle) {
                    o.sab = Some(b);
                    if o.error.is_none() {
                        o.kind = model::Kind::Plain;
                    }
                }
            }
        }
    }
    d.index = d.objects.iter().enumerate().map(|(i, o)| (o.handle, i)).collect();
    Ok(d)
}

/// Read a DWG file from disk.
pub fn read_file(path: impl AsRef<std::path::Path>) -> Result<Drawing> {
    let data = std::fs::read(path)?;
    read(&data)
}

/// Convert DWG bytes to DXF text.
pub fn dwg_to_dxf(data: &[u8], opts: &dxf::Options) -> Result<String> {
    let d = read(data)?;
    Ok(dxf::write(&d, opts))
}
