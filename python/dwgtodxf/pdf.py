"""Render an ezdxf drawing to a vector PDF: ezdxf draws to SVG, PyMuPDF converts."""

from __future__ import annotations

import re
from collections.abc import Sequence

try:
    from ezdxf.addons.drawing import Frontend, RenderContext, config, layout, svg
    from ezdxf.document import Drawing
except ImportError as e:  # pragma: no cover
    raise ImportError("PDF output needs the 'pdf' extra: pip install 'dwgtodxf[pdf]'") from e

# (width, height) in mm, portrait
PAPER_SIZES: dict[str, tuple[float, float]] = {
    "A0": (841, 1189),
    "A1": (594, 841),
    "A2": (420, 594),
    "A3": (297, 420),
    "A4": (210, 297),
    "LETTER": (215.9, 279.4),
    "LEGAL": (215.9, 355.6),
    "TABLOID": (279.4, 431.8),
}

PageSize = str | tuple[float, float] | None


def _resolve_page(page_size: PageSize, landscape: bool, margin_mm: float) -> layout.Page:
    if isinstance(page_size, str):
        try:
            w, h = PAPER_SIZES[page_size.upper()]
        except KeyError:
            raise ValueError(f"unknown page size {page_size!r}, choose from {list(PAPER_SIZES)} "
                             "or pass (width_mm, height_mm)") from None
    else:
        w, h = page_size  # type: ignore[misc]
    if landscape and h > w:
        w, h = h, w
    return layout.Page(w, h, layout.Units.mm, margins=layout.Margins.all(margin_mm))


def _layout_page(dxf_layout, page_size: PageSize, landscape: bool, margin_mm: float) -> layout.Page:
    """Explicit page size wins; otherwise paperspace uses its own plot settings,
    modelspace (which rarely has usable ones) falls back to A3 landscape."""
    if page_size is not None:
        return _resolve_page(page_size, landscape, margin_mm)
    if not dxf_layout.is_modelspace:
        page = layout.Page.from_dxf_layout(dxf_layout)
        if page.width > 0 and page.height > 0:
            return page
    return _resolve_page("A3", True, margin_mm)


def _has_content(dxf_layout) -> bool:
    # A paperspace layout always contains its main viewport; require more than that.
    n = sum(1 for e in dxf_layout if e.dxftype() != "VIEWPORT")
    return n > 0 or (not dxf_layout.is_modelspace and sum(1 for _ in dxf_layout) > 1)


def select_layouts(doc: Drawing, layouts: str | Sequence[str] | None) -> list:
    """``None``/``"Model"`` -> modelspace, ``"all"`` -> every layout with content,
    a name or list of names -> those layouts."""
    if layouts is None:
        return [doc.modelspace()]
    if isinstance(layouts, str):
        if layouts.lower() == "all":
            chosen = [doc.layouts.get(name) for name in doc.layouts.names_in_taborder()]
            chosen = [lay for lay in chosen if _has_content(lay)]
            return chosen or [doc.modelspace()]
        layouts = [layouts]
    result = []
    for name in layouts:
        try:
            lay = doc.layouts.get(name)  # case-insensitive, 'Model' is modelspace
        except KeyError:
            lay = None
        if lay is None:
            raise ValueError(f"layout {name!r} not found, available: {doc.layouts.names_in_taborder()}")
        result.append(lay)
    return result


_STYLE_RE = re.compile(r"\.(C\d+)\s*\{([^}]*)\}")
_CLASS_RE = re.compile(r'class="(C\d+)"')


def _inline_styles(svg_text: str) -> str:
    """MuPDF's SVG importer ignores <style> classes; ezdxf's SVG backend uses
    them for stroke/fill. Turn every class into presentation attributes."""
    styles: dict[str, str] = {}
    for name, body in _STYLE_RE.findall(svg_text):
        attrs = []
        for decl in body.split(";"):
            if ":" not in decl:
                continue
            k, v = (x.strip() for x in decl.split(":", 1))
            if v and v != "none" or k in ("fill", "stroke"):
                attrs.append(f'{k}="{v}"')
        styles[name] = " ".join(attrs)
    return _CLASS_RE.sub(lambda m: styles.get(m.group(1), ""), svg_text)


def render_pdf_bytes(
    doc: Drawing,
    *,
    layouts: str | Sequence[str] | None = None,
    page_size: PageSize = None,
    landscape: bool = True,
    margin_mm: float = 10,
    monochrome: bool = False,
    lineweights: bool = False,
) -> bytes:
    import pymupdf

    cfg = config.Configuration(
        background_policy=config.BackgroundPolicy.WHITE,
        color_policy=config.ColorPolicy.BLACK if monochrome else config.ColorPolicy.COLOR,
        # like AutoCAD with LWDISPLAY off: thin lines unless asked for
        lineweight_scaling=1.0 if lineweights else 0.0,
    )
    out = pymupdf.open()
    for dxf_layout in select_layouts(doc, layouts):
        # The SVG backend scales linearly with drawing size, unlike ezdxf's
        # PyMuPDF backend, which becomes very slow on large drawings.
        backend = svg.SVGBackend()
        Frontend(RenderContext(doc), backend, config=cfg).draw_layout(dxf_layout, finalize=True)
        page = _layout_page(dxf_layout, page_size, landscape, margin_mm)
        try:
            text = _inline_styles(backend.get_string(page, settings=layout.Settings(fit_page=True)))
        except ValueError:  # nothing drawn on this layout: blank page
            w, h = page.width_in_mm, page.height_in_mm
            out.new_page(width=w / 25.4 * 72, height=h / 25.4 * 72)
            continue
        with pymupdf.open(stream=text.encode("utf-8"), filetype="svg") as svgdoc:
            pdf = svgdoc.convert_to_pdf()
        with pymupdf.open(stream=pdf, filetype="pdf") as one:
            out.insert_pdf(one)
    try:
        return out.tobytes(garbage=3, deflate=True)
    finally:
        out.close()
