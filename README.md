# dwgtodxf

[![CI](https://github.com/xxparthparekhxx/dwgtodxf/actions/workflows/ci.yml/badge.svg)](https://github.com/xxparthparekhxx/dwgtodxf/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/dwgtodxf?label=pypi)](https://pypi.org/project/dwgtodxf/)

Fast DWG → DXF (and PDF) conversion for Python, built on its own DWG decoder
written in Rust. No ODA File Converter, no LibreDWG, no external programs.

```python
import dwgtodxf

dwgtodxf.to_dxf("plan.dwg")                     # -> plan.dxf
dwgtodxf.to_dxf("plan.dwg", "out.dxf", version="R2000")
dwgtodxf.to_pdf("plan.dwg")                     # -> plan.pdf   (needs the [pdf] extra)
dwgtodxf.to_pdf("plan.dwg", layouts="all")      # one page per paper-space layout
dwgtodxf.convert("plan.dwg", "out/plan.pdf")    # format from the suffix

data = dwgtodxf.dwg_to_dxf_bytes(open("plan.dwg", "rb").read())   # in memory
doc = dwgtodxf.read("plan.dwg")                 # ezdxf Drawing (needs ezdxf)
dwgtodxf.info("plan.dwg")                       # version, entity counts, layers, gaps
```

```sh
dwgtodxf plan.dwg                        # -> plan.dxf
dwgtodxf *.dwg -o out/                   # batch
dwgtodxf plan.dwg -o plan.pdf --mono --page-size A3
dwgtodxf plan.dwg --info                 # what's inside, what isn't converted
```

There is also a standalone Rust binary: `cargo run --release --bin dwg2dxf -- plan.dwg`.

## Install

```sh
pip install dwgtodxf            # DWG -> DXF, no other dependencies
pip install "dwgtodxf[pdf]"     # + PDF output and read() into ezdxf
```

Prebuilt wheels cover Linux (x86-64, ARM64, musl), macOS (Intel, Apple
silicon) and Windows, for Python 3.10+. Elsewhere pip builds from source,
which needs a Rust toolchain. For development:

```sh
pip install maturin && maturin develop --release
```

## What it reads

| DWG version          | AutoCAD   | status |
|----------------------|-----------|--------|
| AC1012 / AC1014      | R13 / R14 | ✅ |
| AC1015               | 2000–2002 | ✅ |
| AC1018               | 2004–2006 | ✅ |
| AC1021               | 2007–2009 | ✅ |
| AC1024               | 2010–2012 | ✅ |
| AC1027               | 2013–2017 | ✅ |
| AC1032               | 2018–2026 | ✅ |
| pre-R13 (R12 and older) | –      | ❌ different, undocumented format |
| password-protected   | –         | ❌ |

**Decoded natively** (geometry, layers, colours incl. true colour, linetypes,
lineweights, transparency, XDATA, extension dictionaries):
LINE, CIRCLE, ARC, ELLIPSE, POINT, LWPOLYLINE, POLYLINE (2D/3D/mesh/pface) +
VERTEX, SPLINE, TEXT, MTEXT, ATTRIB, ATTDEF, INSERT/MINSERT, BLOCKs, all
DIMENSION types, LEADER, TOLERANCE, MLINE, HATCH (incl. gradients), SOLID,
TRACE, 3DFACE, RAY, XLINE, IMAGE, WIPEOUT, VIEWPORT, 3DSOLID/REGION/BODY
(ACIS, written as SAT or as SAB in DXF R2013+); tables LAYER, LTYPE, STYLE,
APPID, BLOCK_RECORD (DIMSTYLE/VPORT/VIEW/UCS by name); objects DICTIONARY,
XRECORD, LAYOUT, PLOTSETTINGS, MLINESTYLE, IMAGEDEF, RASTERVARIABLES,
DICTIONARYVAR, placeholders; ~240 header variables ($INSUNITS, $EXTMIN,
$LTSCALE, dimension defaults, ...). Splines drawn through fit points get the
same control points AutoCAD computes.

**Everything else with proxy graphics** (MULTILEADER, ACAD_TABLE,
ARC_DIMENSION, vertical-product objects such as AEC/Civil/MEP entities, ...)
is converted from the pre-rendered graphics AutoCAD stores in the file into
plain lines, arcs, polylines, text, hatches and faces, so it still shows up.

Not converted: entities that have neither a native decoder nor proxy
graphics, and non-graphical add-on objects (constraint networks, visual
styles, AutoCAD Mechanical standards, ...). Dimension styles are written by
name only (dimensions still display, via their stored geometry blocks).
`info()` and `dwg2dxf --info` list what a given file contains that is not
converted.

Drawings with binary ACIS solids (R2004+) are written as DXF R2018 by default,
since older DXF versions cannot hold that data; asking for an older version
writes the solids' wireframe instead.

Malformed or hostile files are rejected with an error, never a crash: the
decoder caps allocations by file size and is fuzzed with tens of thousands of
corrupted inputs.

The DXF output keeps the original handles and ownership, and passes ezdxf's
strict reader and auditor on the test corpus.

## Speed

Objects are decoded in parallel. On a 2.2 MB DWG with 48k objects, DWG → 8.5 MB
DXF takes about 80 ms (LibreDWG's `dwg2dxf`: 180 ms). Typical drawings convert in
a few milliseconds. A real 1.9 MB R2018 single-line diagram (28k entities)
converts in about 0.1 s; rendering that to PDF (pure Python, ezdxf) takes about 25 s.

## Layout

```
crates/dwg/      pure-Rust DWG reader + DXF writer (+ dwg2dxf binary)
  src/bits.rs        bit-level codes (BS, BL, BD, MC, handles, ...)
  src/file.rs        R13-R2000 and R2004+ containers, section maps
  src/decompress.rs  R2004 LZ77 variant
  src/r2007.rs       R2007 container: Reed-Solomon pages, R21 compression
  src/classes.rs     classes and object map
  src/objects.rs     object framing, common entity data, string streams
  src/entities.rs    entity decoders
  src/tables.rs      tables, dictionaries, other objects
  src/header.rs      header variables
  src/datastorage.rs R2013+ data storage section (ACIS of solids)
  src/spline.rs      control points for fit-point splines
  src/proxy.rs       proxy graphics -> primitives
  src/dxf.rs         DXF writer
crates/py/       PyO3 bindings (dwgtodxf._native)
python/dwgtodxf/ Python API, CLI, PDF rendering (ezdxf + PyMuPDF)
```

## Tests

```sh
cargo test -p dwg                               # unit tests
DWG_TEST_DATA=/path/to/dwgs cargo test -p dwg --release      # + fuzzing
DWGTODXF_TEST_DATA=/path/to/dwgs pytest
```

Sample DWGs are not shipped. CI downloads LibreDWG's `test/test-data` folder
(140 DWGs, R13-R2018) and uses it only as test input.

## How it was built

The decoder is a clean-room implementation based on the Open Design Alliance's
public *Open Design Specification for .dwg files*, checked against reference
DXF files. Several places where real files differ from the spec are documented
in the code (object-map delta reset per block, R2010+ object framing, R2007+
table record flags, R2010 graphics size, R2007 section name lengths, ...).

## License

MIT. DWG is a trademark of Autodesk, Inc.; this project is not affiliated with
Autodesk or the Open Design Alliance.
