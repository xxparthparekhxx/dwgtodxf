//! dwg2dxf: convert DWG files to DXF.

use std::process::ExitCode;
use std::time::Instant;

use dwg::model::Kind;
use dwg::Version;

const USAGE: &str = "usage: dwg2dxf [--info] [--dxf-version R2000|R2004|R2007|R2010|R2013|R2018] INPUT.dwg [OUTPUT.dxf]";

fn parse_version(s: &str) -> Option<Version> {
    Some(match s.trim_start_matches(['R', 'r']) {
        "2000" => Version::R2000,
        "2004" => Version::R2004,
        "2007" => Version::R2007,
        "2010" => Version::R2010,
        "2013" => Version::R2013,
        "2018" => Version::R2018,
        _ => return None,
    })
}

fn main() -> ExitCode {
    let mut info = false;
    let mut version = None;
    let mut files = vec![];
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--info" => info = true,
            "--dxf-version" => match args.next().as_deref().and_then(parse_version) {
                Some(v) => version = Some(v),
                None => {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => files.push(a),
        }
    }
    let Some(input) = files.first().cloned() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let t0 = Instant::now();
    let d = match dwg::read_file(&input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if info {
        println!("version: {}  maint: {}  codepage: {}", d.version().name(), d.maint, d.codepage);
        println!("classes: {}  objects: {}  undecodable: {}", d.classes.len(), d.objects.len(), d.failed.len());
        let mut types: std::collections::BTreeMap<&str, (usize, usize)> = Default::default();
        for o in &d.objects {
            let e = types.entry(o.dxf_name.as_str()).or_default();
            e.0 += 1;
            if o.error.is_some() || matches!(o.kind, Kind::Unsupported) {
                e.1 += 1;
            }
        }
        for (k, (n, bad)) in types {
            let note = if bad > 0 { format!("  ({bad} not decoded)") } else { String::new() };
            println!("  {k:<32} {n:>6}{note}");
        }
        if std::env::var("DWG_HEADER").is_ok() {
            let mut v: Vec<_> = d.header.vars.iter().collect();
            v.sort_by_key(|(k, _)| *k);
            for (k, val) in v {
                println!("  ${k} = {val:?}");
            }
        }
        println!("read in {:?}", t0.elapsed());
        return ExitCode::SUCCESS;
    }
    let out = files.get(1).cloned().unwrap_or_else(|| {
        std::path::Path::new(&input).with_extension("dxf").to_string_lossy().into_owned()
    });
    let (text, report) = dwg::dxf::write_with_report(&d, &dwg::dxf::Options { version });
    if let Err(e) = std::fs::write(&out, text) {
        eprintln!("error: {out}: {e}");
        return ExitCode::FAILURE;
    }
    let skipped: usize = report.skipped.iter().map(|(_, n)| n).sum();
    eprintln!(
        "{input} -> {out}: {} entities, {} objects{} in {:.1?}",
        report.entities_written,
        report.objects_written,
        if skipped > 0 { format!(", {skipped} skipped") } else { String::new() },
        t0.elapsed()
    );
    ExitCode::SUCCESS
}
