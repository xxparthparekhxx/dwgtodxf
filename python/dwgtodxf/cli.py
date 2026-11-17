from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

from . import __version__
from .core import info, to_dxf, to_pdf
from .errors import DwgError

DXF_VERSIONS = ("R2000", "R2004", "R2007", "R2010", "R2013", "R2018")


def _page_size(value: str):
    if "x" in value.lower():
        w, h = value.lower().split("x", 1)
        return (float(w), float(h))
    return value


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="dwgtodxf", description="Convert DWG files to DXF or PDF.")
    p.add_argument("inputs", nargs="+", type=Path, help="input .dwg files")
    p.add_argument("-o", "--output", type=Path,
                   help="output file (single input) or directory (several inputs)")
    p.add_argument("-f", "--format", choices=["dxf", "pdf"],
                   help="output format (default: from --output suffix, else dxf)")
    p.add_argument("--dxf-version", choices=DXF_VERSIONS, help="DXF version (default: same as DWG)")
    p.add_argument("--info", action="store_true", help="print a summary of each file instead")
    p.add_argument("--layout", action="append", dest="layouts",
                   help="PDF: layout to render (repeatable, 'all' for every layout); default Model")
    p.add_argument("--page-size", type=_page_size, help="PDF: A4, A3, ..., or WxH in mm")
    p.add_argument("--portrait", action="store_true", help="PDF: portrait orientation")
    p.add_argument("--mono", action="store_true", help="PDF: black & white")
    p.add_argument("--lineweights", action="store_true", help="PDF: use real lineweights")
    p.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    args = p.parse_args(argv)

    fmt = args.format
    if fmt is None:
        fmt = "pdf" if args.output and args.output.suffix.lower() == ".pdf" else "dxf"
    multi = len(args.inputs) > 1
    if multi and args.output and args.output.suffix:
        p.error("with several inputs, --output must be a directory")
    layouts = args.layouts[0] if args.layouts and len(args.layouts) == 1 else args.layouts

    failed = 0
    for src in args.inputs:
        try:
            if args.info:
                i = info(src)
                print(f"{src}: {i['version']}, {i['objects']} objects, layers: {len(i['layers'])}")
                for name, n in i["entities"]:
                    print(f"  {name:<20} {n}")
                if i["unsupported"]:
                    print("  not converted: " + ", ".join(f"{k} x{v}" for k, v in i["unsupported"]))
                continue
            if args.output is None:
                dst = src.with_suffix("." + fmt)
            elif multi or args.output.is_dir():
                dst = args.output / (src.stem + "." + fmt)
            else:
                dst = args.output
            t0 = time.perf_counter()
            if fmt == "dxf":
                to_dxf(src, dst, version=args.dxf_version)
            else:
                to_pdf(src, dst, layouts=layouts, page_size=args.page_size,
                       landscape=not args.portrait, monochrome=args.mono,
                       lineweights=args.lineweights)
            print(f"{src} -> {dst} ({(time.perf_counter() - t0) * 1000:.0f} ms)")
        except (DwgError, OSError, ValueError, ImportError) as e:
            failed += 1
            print(f"error: {src}: {e}", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
