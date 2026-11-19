#!/usr/bin/env python3
"""Builds the documentation site: the Zola guide plus the rustdoc API reference.

    python3 site/tools/build.py                 # build into site/public
    python3 site/tools/build.py --serve         # build, then serve it locally
    python3 site/tools/build.py --base-url URL  # build for another address

Needs Zola (https://www.getzola.org) on the path, and everything `cargo doc`
needs, which `just setup` installs. The GitHub Pages workflow runs this too.
"""

from __future__ import annotations

import argparse
import functools
import http.server
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SITE = ROOT / "site"
PUBLIC = SITE / "public"
# The crates whose API games use. smitten-core's items are re-exported by
# smitten, but its pages are built too so that every link resolves.
CRATES = ["smitten", "smitten-core"]


def run(*command: str, cwd: Path = ROOT) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def broken_links(base_url: str | None) -> list[str]:
    """Links in the guide's pages that lead to no file or no anchor."""
    prefix = (base_url or "").rstrip("/")
    problems = []
    for page in PUBLIC.rglob("*.html"):
        if PUBLIC / "api" in page.parents:
            continue
        text = page.read_text(encoding="utf-8")
        for href in re.findall(r'(?:href|src|poster)="([^"]+)"', text):
            if prefix and href.startswith(prefix):
                target = href[len(prefix):]
            elif href.startswith(("http:", "https:", "mailto:", "#")):
                continue
            else:
                target = os.path.normpath(os.path.join("/", str(page.parent.relative_to(PUBLIC)), href))
            target, _, anchor = target.partition("#")
            path = PUBLIC / target.split("?")[0].lstrip("/")
            if path.is_dir():
                path = path / "index.html"
            if not path.exists():
                problems.append(f"{page.relative_to(PUBLIC)}: {href}")
            elif anchor and path.suffix == ".html" and f'id="{anchor}"' not in path.read_text(encoding="utf-8"):
                problems.append(f"{page.relative_to(PUBLIC)}: {href} (no such anchor)")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base-url", help="the address the site will be served from")
    parser.add_argument("--serve", action="store_true", help="serve the site on localhost")
    parser.add_argument("--port", type=int, default=1111)
    args = parser.parse_args()

    if shutil.which("zola") is None:
        print("Zola is not installed. See https://www.getzola.org/documentation/getting-started/installation/", file=sys.stderr)
        return 1

    base_url = args.base_url
    if args.serve and base_url is None:
        base_url = f"http://127.0.0.1:{args.port}"

    doc_args = ["cargo", "doc", "--no-deps"]
    for crate in CRATES:
        doc_args += ["-p", crate]
    run(*doc_args)

    zola = ["zola", "build", "--output-dir", str(PUBLIC), "--force"]
    if base_url:
        zola += ["--base-url", base_url]
    run(*zola, cwd=SITE)

    api = PUBLIC / "api"
    shutil.copytree(ROOT / "target" / "doc", api, dirs_exist_ok=True)
    (api / "index.html").write_text(
        '<!doctype html><meta charset="utf-8"><title>Smitten API</title>'
        '<meta http-equiv="refresh" content="0; url=smitten/index.html">'
        '<a href="smitten/index.html">The Smitten API reference</a>\n',
        encoding="utf-8",
    )
    configured = re.search(r'^base_url\s*=\s*"([^"]+)"', (SITE / "config.toml").read_text(encoding="utf-8"), re.M)
    problems = broken_links(base_url or configured.group(1))
    for problem in problems:
        print(f"broken link in {problem}", file=sys.stderr)
    if problems:
        return 1
    print(f"Built the site in {PUBLIC.relative_to(ROOT)}")

    if args.serve:
        handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(PUBLIC))
        with http.server.ThreadingHTTPServer(("127.0.0.1", args.port), handler) as server:
            print(f"Serving on {base_url}/ (Ctrl+C stops)")
            try:
                server.serve_forever()
            except KeyboardInterrupt:
                pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
