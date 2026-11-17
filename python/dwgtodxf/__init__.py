"""Fast DWG to DXF/PDF conversion with a native (Rust) DWG decoder.

    import dwgtodxf

    dwgtodxf.to_dxf("plan.dwg")                 # -> plan.dxf
    dwgtodxf.to_pdf("plan.dwg")                 # -> plan.pdf   (needs the [pdf] extra)
    dwgtodxf.convert("plan.dwg", "out/a.pdf")   # format from suffix
    doc = dwgtodxf.read("plan.dwg")             # ezdxf Drawing (needs ezdxf)
    dwgtodxf.info("plan.dwg")                   # version, counts, layers
"""

from .core import DXF_VERSIONS, convert, dwg_to_dxf_bytes, info, is_dwg, read, to_dxf, to_pdf
from .errors import DwgError, UnsupportedVersionError

__version__ = "0.1.0"

__all__ = [
    "DXF_VERSIONS",
    "DwgError",
    "UnsupportedVersionError",
    "convert",
    "dwg_to_dxf_bytes",
    "info",
    "is_dwg",
    "read",
    "to_dxf",
    "to_pdf",
]
