"""Tests for the Python API.

DWG samples are not shipped with the repository. Point DWGTODXF_TEST_DATA at
a folder of .dwg files (searched recursively) to run the conversion tests,
for example LibreDWG's test/test-data directory.
"""

import os
from pathlib import Path

import pytest

import dwgtodxf
from dwgtodxf.cli import main

DATA = os.environ.get("DWGTODXF_TEST_DATA")
DWGS = []
if DATA and Path(DATA).is_dir():
    DWGS = sorted(
        p for p in Path(DATA).rglob("*.dwg")
        if p.read_bytes()[:6] in (b"AC1012", b"AC1014", b"AC1015", b"AC1018", b"AC1021", b"AC1024", b"AC1027", b"AC1032")
    )

needs_data = pytest.mark.skipif(not DWGS, reason="set DWGTODXF_TEST_DATA to a folder of .dwg files")
SAMPLE = DWGS[:12] if DWGS else []


def test_rejects_garbage(tmp_path):
    with pytest.raises(dwgtodxf.DwgError):
        dwgtodxf.dwg_to_dxf_bytes(b"not a dwg file at all")
    p = tmp_path / "bad.dwg"
    p.write_bytes(b"AC1015" + b"\0" * 64)
    with pytest.raises(dwgtodxf.DwgError):
        dwgtodxf.to_dxf(p)


def test_pre_r13_is_unsupported():
    with pytest.raises(dwgtodxf.UnsupportedVersionError):
        dwgtodxf.dwg_to_dxf_bytes(b"AC1009" + b"\0" * 64)


def test_missing_file():
    with pytest.raises(FileNotFoundError):
        dwgtodxf.to_dxf("does-not-exist.dwg")


def test_bad_dxf_version(tmp_path):
    with pytest.raises(ValueError):
        dwgtodxf.dwg_to_dxf_bytes(b"AC1015", version="R12")


def test_convert_bad_suffix(tmp_path):
    with pytest.raises(ValueError):
        dwgtodxf.convert("x.dwg", tmp_path / "x.svg")


@needs_data
@pytest.mark.parametrize("dwg", SAMPLE, ids=lambda p: f"{p.parent.name}/{p.name}")
def test_to_dxf_is_valid(dwg, tmp_path):
    ezdxf = pytest.importorskip("ezdxf")
    out = dwgtodxf.to_dxf(dwg, tmp_path / "out.dxf")
    doc = ezdxf.readfile(out)  # strict reader
    auditor = doc.audit()
    assert not auditor.errors


@needs_data
def test_bytes_api_and_versions():
    data = SAMPLE[0].read_bytes()
    assert dwgtodxf.is_dwg(data)
    for v in ("R2000", "R2018"):
        dxf = dwgtodxf.dwg_to_dxf_bytes(data, version=v)
        assert dxf.startswith(b"  0\nSECTION")
        acad = {"R2000": b"AC1015", "R2018": b"AC1032"}[v]
        assert acad in dxf[:200]


@needs_data
def test_info():
    i = dwgtodxf.info(SAMPLE[0])
    assert i["version"].startswith("R")
    assert i["objects"] > 0
    assert "0" in i["layers"]


@needs_data
def test_read_into_ezdxf():
    pytest.importorskip("ezdxf")
    doc = dwgtodxf.read(SAMPLE[0])
    assert doc.modelspace() is not None


@needs_data
def test_to_pdf(tmp_path):
    pymupdf = pytest.importorskip("pymupdf")
    pytest.importorskip("ezdxf")
    out = dwgtodxf.to_pdf(SAMPLE[0], tmp_path / "x.pdf", page_size="A4", monochrome=True)
    with pymupdf.open(out) as pdf:
        assert pdf.page_count == 1


@needs_data
def test_cli(tmp_path):
    assert main([str(SAMPLE[0]), "-o", str(tmp_path / "a.dxf"), "--dxf-version", "R2000"]) == 0
    assert (tmp_path / "a.dxf").read_bytes()[:20].startswith(b"  0\nSECTION")
    assert main([str(p) for p in SAMPLE[:3]] + ["-o", str(tmp_path)]) == 0
    assert main([str(SAMPLE[0]), "--info"]) == 0
