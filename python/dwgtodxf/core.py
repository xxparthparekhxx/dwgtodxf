from __future__ import annotations

import io
import os
from collections.abc import Sequence
from pathlib import Path
from typing import TYPE_CHECKING, Any

from . import _native
from .errors import DwgError, UnsupportedVersionError

if TYPE_CHECKING:
    from ezdxf.document import Drawing

PathLike = str | os.PathLike

DXF_VERSIONS = ("R2000", "R2004", "R2007", "R2010", "R2013", "R2018")


def _check_version(version: str | None) -> str | None:
    if version is None:
        return None
    v = version.upper()
    if not v.startswith("R"):
        v = "R" + v
    if v not in DXF_VERSIONS:
        raise ValueError(f"unsupported DXF version {version!r}, choose from {DXF_VERSIONS}")
    return v


def _wrap(e: ValueError) -> DwgError:
    msg = str(e)
    if msg.startswith("unsupported"):
        return UnsupportedVersionError(msg)
    return DwgError(msg)


def _read_bytes(src: PathLike) -> bytes:
    p = Path(src)
    if not p.is_file():
        raise FileNotFoundError(p)
    return p.read_bytes()


def is_dwg(data: bytes) -> bool:
    return len(data) >= 6 and data[:2] == b"AC" and (data[2:6].isdigit() or data[2:6] in (b"1.40", b"1.50", b"2.10"))


def dwg_to_dxf_bytes(data: bytes, version: str | None = None) -> bytes:
    """Convert DWG file contents to DXF file contents (in memory)."""
    version = _check_version(version)
    try:
        return _native.dwg_to_dxf(data, version)
    except ValueError as e:
        raise _wrap(e) from None


def info(src: PathLike) -> dict[str, Any]:
    """Version, entity counts, layers and unsupported object types of a DWG."""
    try:
        return _native.info(_read_bytes(src))
    except ValueError as e:
        raise _wrap(e) from None


def to_dxf(src: PathLike, dst: PathLike | None = None, *, version: str | None = None) -> Path:
    """Convert a DWG file to DXF and return the output path.

    Args:
        src: input .dwg file.
        dst: output .dxf path (default: next to ``src`` with a .dxf suffix).
        version: DXF version to write ("R2000" ... "R2018"); default: same as the DWG.
    """
    version = _check_version(version)
    src = Path(src)
    if not src.is_file():
        raise FileNotFoundError(src)
    dst = Path(dst) if dst is not None else src.with_suffix(".dxf")
    dst.parent.mkdir(parents=True, exist_ok=True)
    try:
        _native.convert_file(str(src), str(dst), version)
    except ValueError as e:
        raise _wrap(e) from None
    return dst


def read(src: PathLike) -> "Drawing":
    """Read a DWG (or DXF) file into an ezdxf ``Drawing`` (needs ``ezdxf``)."""
    import ezdxf
    from ezdxf import recover

    data = _read_bytes(src)
    if is_dwg(data):
        data = dwg_to_dxf_bytes(data)
    try:
        return ezdxf.read(io.StringIO(data.decode("utf-8", errors="surrogateescape")))
    except ezdxf.DXFStructureError:
        doc, _ = recover.read(io.BytesIO(data))
        return doc


def to_pdf(
    src: PathLike | "Drawing",
    dst: PathLike | None = None,
    *,
    layouts: str | Sequence[str] | None = None,
    page_size: str | tuple[float, float] | None = None,
    landscape: bool = True,
    margin_mm: float = 10,
    monochrome: bool = False,
    lineweights: bool = False,
) -> Path:
    """Render a DWG/DXF file (or an ezdxf ``Drawing``) to a vector PDF.

    Needs the ``pdf`` extra: ``pip install dwgtodxf[pdf]``.

    Args:
        layouts: ``None`` = model space; ``"all"`` = one page per layout with
            content; a layout name or list of names.
        page_size: ``"A4"``, ``"A3"``, ... or ``(width_mm, height_mm)``.
            Default: the layout's plot settings, A3 for model space.
        monochrome: draw everything black.
        lineweights: draw lines with their real lineweights (default: thin lines).
    """
    from .pdf import render_pdf_bytes

    if isinstance(src, (str, os.PathLike)):
        doc = read(src)
        if dst is None:
            dst = Path(src).with_suffix(".pdf")
    else:
        doc = src
        if dst is None:
            if not doc.filename:
                raise ValueError("dst is required when passing an in-memory Drawing")
            dst = Path(doc.filename).with_suffix(".pdf")
    dst = Path(dst)
    data = render_pdf_bytes(doc, layouts=layouts, page_size=page_size, landscape=landscape,
                            margin_mm=margin_mm, monochrome=monochrome,
                            lineweights=lineweights)
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_bytes(data)
    return dst


def convert(src: PathLike, dst: PathLike, **kwargs) -> Path:
    """Convert ``src`` to ``dst``; the format comes from ``dst``'s suffix (.dxf or .pdf)."""
    suffix = Path(dst).suffix.lower()
    if suffix == ".dxf":
        return to_dxf(src, dst, **kwargs)
    if suffix == ".pdf":
        return to_pdf(src, dst, **kwargs)
    raise ValueError(f"unsupported output format {suffix!r}, use .dxf or .pdf")
