//! Python bindings: `dwgtodxf._native`.

use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

use dwg::model::Kind;
use dwg::Version;

fn parse_version(v: Option<&str>) -> PyResult<Option<Version>> {
    let Some(v) = v else { return Ok(None) };
    let v = v.trim().to_ascii_uppercase();
    let v = v.strip_prefix('R').unwrap_or(&v).to_string();
    Ok(Some(match v.as_str() {
        "2000" | "AC1015" => Version::R2000,
        "2004" | "AC1018" => Version::R2004,
        "2007" | "AC1021" => Version::R2007,
        "2010" | "AC1024" => Version::R2010,
        "2013" | "AC1027" => Version::R2013,
        "2018" | "AC1032" => Version::R2018,
        _ => {
            return Err(PyValueError::new_err(format!(
                "unsupported DXF version {v:?}; use R2000, R2004, R2007, R2010, R2013 or R2018"
            )))
        }
    }))
}

fn err(e: dwg::Error) -> PyErr {
    match e {
        dwg::Error::Io(m) => PyOSError::new_err(m),
        other => PyValueError::new_err(other.to_string()),
    }
}

/// Convert DWG bytes to DXF bytes.
#[pyfunction]
#[pyo3(signature = (data, version=None))]
fn dwg_to_dxf(py: Python<'_>, data: &[u8], version: Option<&str>) -> PyResult<Py<PyBytes>> {
    let opts = dwg::dxf::Options { version: parse_version(version)? };
    let text = py.detach(|| dwg::dwg_to_dxf(data, &opts)).map_err(err)?;
    Ok(PyBytes::new(py, text.as_bytes()).unbind())
}

/// Convert a DWG file to a DXF file. Returns a report dict.
#[pyfunction]
#[pyo3(signature = (src, dst, version=None))]
fn convert_file(py: Python<'_>, src: &str, dst: &str, version: Option<&str>) -> PyResult<Py<PyDict>> {
    let opts = dwg::dxf::Options { version: parse_version(version)? };
    let (report, ver) = py
        .detach(|| -> dwg::Result<_> {
            let data = std::fs::read(src)?;
            let d = dwg::read(&data)?;
            let (text, report) = dwg::dxf::write_with_report(&d, &opts);
            std::fs::write(dst, text)?;
            Ok((report, d.version()))
        })
        .map_err(err)?;
    let r = PyDict::new(py);
    r.set_item("dwg_version", ver.name())?;
    r.set_item("entities", report.entities_written)?;
    r.set_item("objects", report.objects_written)?;
    r.set_item("skipped", report.skipped.into_iter().collect::<Vec<_>>())?;
    Ok(r.unbind())
}

/// Summary of a DWG file: version, classes, entity and object counts.
#[pyfunction]
fn info(py: Python<'_>, data: &[u8]) -> PyResult<Py<PyDict>> {
    let d = py.detach(|| dwg::read(data)).map_err(err)?;
    let r = PyDict::new(py);
    r.set_item("version", d.version().name())?;
    r.set_item("acadver", d.version().acadver())?;
    r.set_item("maintenance", d.maint)?;
    r.set_item("codepage", d.codepage)?;
    r.set_item("objects", d.objects.len())?;
    r.set_item("entities", d.entity_counts())?;
    let mut unsupported: std::collections::BTreeMap<String, usize> = Default::default();
    for o in &d.objects {
        if matches!(o.kind, Kind::Unsupported) || o.error.is_some() {
            *unsupported.entry(o.dxf_name.clone()).or_default() += 1;
        }
    }
    r.set_item("unsupported", unsupported.into_iter().collect::<Vec<_>>())?;
    r.set_item("failed", d.failed.len())?;
    let layers: Vec<String> = d
        .objects
        .iter()
        .filter_map(|o| match &o.kind {
            Kind::TableEntry { kind: dwg::model::TableKind::Layer, name } => Some(name.clone()),
            _ => None,
        })
        .collect();
    r.set_item("layers", layers)?;
    Ok(r.unbind())
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(dwg_to_dxf, m)?)?;
    m.add_function(wrap_pyfunction!(convert_file, m)?)?;
    m.add_function(wrap_pyfunction!(info, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
