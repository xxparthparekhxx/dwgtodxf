//! Integration tests over a directory of sample DWG files.
//!
//! Set DWG_TEST_DATA to a folder containing .dwg files (searched
//! recursively); the test is skipped when it is not set.

use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dwg")) {
            out.push(p);
        }
    }
}

#[test]
fn all_samples_convert() {
    let Ok(dir) = std::env::var("DWG_TEST_DATA") else {
        eprintln!("DWG_TEST_DATA not set, skipping");
        return;
    };
    let mut files = vec![];
    collect(Path::new(&dir), &mut files);
    files.sort();
    let mut checked = 0;
    for f in files {
        let data = std::fs::read(&f).unwrap();
        let d = match dwg::read(&data) {
            Ok(d) => d,
            Err(dwg::Error::Unsupported(m)) if m.contains("pre-R13") => continue,
            Err(e) => panic!("{}: {e}", f.display()),
        };
        assert!(d.failed.is_empty(), "{}: {} objects failed", f.display(), d.failed.len());
        let errors: Vec<_> = d.objects.iter().filter(|o| o.error.is_some()).map(|o| (o.handle, &o.dxf_name, o.error.clone())).collect();
        assert!(errors.is_empty(), "{}: decode errors {errors:?}", f.display());
        let dxf = dwg::dxf::write(&d, &Default::default());
        assert!(dxf.starts_with("  0\nSECTION\n") && dxf.ends_with("  0\nEOF\n"));
        checked += 1;
    }
    assert!(checked > 0, "no R13+ DWG files found in {dir}");
}

#[test]
fn rejects_non_dwg() {
    assert!(matches!(dwg::read(b"hello world"), Err(dwg::Error::NotDwg)));
    assert!(matches!(dwg::read(b"AC1009\0\0\0\0"), Err(dwg::Error::Unsupported(_))));
    assert!(dwg::read(b"AC1015").is_err());
    assert!(dwg::read(b"AC1018").is_err());
    assert!(dwg::read(b"AC1021").is_err());
}

#[test]
fn truncated_files_do_not_panic() {
    let Ok(dir) = std::env::var("DWG_TEST_DATA") else { return };
    let mut files = vec![];
    collect(Path::new(&dir), &mut files);
    files.sort();
    for f in files.iter().take(40) {
        let data = std::fs::read(f).unwrap();
        for cut in [data.len() / 3, data.len() / 2, data.len() * 9 / 10] {
            let _ = dwg::read(&data[..cut]);
        }
        // flipped bytes in the middle
        let mut bad = data.clone();
        for i in (bad.len() / 2..bad.len()).step_by(97) {
            bad[i] ^= 0x5A;
        }
        let _ = dwg::read(&bad).map(|d| dwg::dxf::write(&d, &Default::default()));
    }
}

/// Random byte mutations of every sample: decoding must return Ok or Err,
/// never panic, abort or exhaust memory. DWG_FUZZ_ITERS sets the rounds.
#[test]
fn fuzz_mutations() {
    let Ok(dir) = std::env::var("DWG_TEST_DATA") else { return };
    let iters: usize = std::env::var("DWG_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let mut files = vec![];
    collect(Path::new(&dir), &mut files);
    files.sort();
    let mut seed: u64 = std::env::var("DWG_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for f in &files {
        let data = std::fs::read(f).unwrap();
        if dwg::read(&data).is_err() {
            continue; // unsupported version
        }
        for _ in 0..iters {
            let mut m = data.clone();
            let n = 1 + (next() % 16) as usize;
            for _ in 0..n {
                // bias towards the headers and maps at the start and end
                let pos = match next() % 3 {
                    0 => (next() % 0x500.min(m.len() as u64)) as usize,
                    1 => m.len() - 1 - (next() % 0x2000.min(m.len() as u64)) as usize,
                    _ => (next() % m.len() as u64) as usize,
                };
                m[pos] = next() as u8;
            }
            if next() % 8 == 0 {
                let cut = (next() % m.len() as u64) as usize;
                m.truncate(cut.max(1));
            }
            // DWG_FUZZ_TRACE=<file>: keep the current input, to reproduce a crash
            if let Ok(t) = std::env::var("DWG_FUZZ_TRACE") {
                std::fs::write(&t, &m).unwrap();
            }
            if let Ok(d) = dwg::read(&m) {
                let _ = dwg::dxf::write(&d, &Default::default());
            }
        }
    }
}
