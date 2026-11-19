#!/usr/bin/env python3
"""Writes the "LÖVE name lookup" page of the user guide.

Every Smitten item whose name differs from LÖVE's carries the LÖVE name as
a `#[doc(alias = "...")]`. This script collects those aliases from the
source, works out which item each one is attached to, and writes a table of
LÖVE names and the Smitten items that replace them.

Run it from anywhere; paths are relative to the repository. When the API
reference has been built with `cargo doc` first, each entry links to its
item's page. Otherwise entries link to an API search for the LÖVE name,
which rustdoc answers through the same aliases.

    python3 site/tools/love_names.py
"""

from __future__ import annotations

import html
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCES = [ROOT / "crates/smitten/src", ROOT / "crates/smitten-core/src"]
ALL_HTML = ROOT / "target/doc/smitten/all.html"
OUT = ROOT / "site/content/guide/love-names.md"

ALIAS = re.compile(r'#\[doc\(alias\s*=\s*"([^"]+)"\)\]')
FN = re.compile(r"^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:unsafe\s+)?fn\s+(\w+)")
TYPE = re.compile(r"^(\s*)pub\s+(struct|enum|trait|type|const)\s+(\w+)")
VARIANT = re.compile(r"^(\s+)([A-Z]\w*)\s*(?:[,({=]|$)")
IMPL = re.compile(r"^impl(?:<[^{]*?>)?\s+(?:[\w:<>', ]+?\s+for\s+)?(?:[\w:]+::)?(\w+)")
OWNER = re.compile(r"^pub\s+(?:trait|enum)\s+(\w+)")


def module_of(path: Path) -> str:
    """The public module a source file's free items appear in."""
    for source in SOURCES:
        if source in path.parents:
            parts = path.relative_to(source).with_suffix("").parts
            break
    if "smitten-core" in str(path):
        return ""
    first = parts[0]
    if len(parts) == 1 and first in {"lib", "error", "main_loop", "result", "context"}:
        return ""
    return first


def owner_at(lines: list[str], index: int) -> str | None:
    """The impl, trait or enum a line at `index` sits inside, if any."""
    for line in reversed(lines[:index]):
        if line.startswith(("}", "mod ", "#[cfg(test)]")):
            return None
        match = IMPL.match(line) or OWNER.match(line)
        if match:
            return match.group(1)
    return None


def find_item(lines: list[str], index: int):
    """The item declared after the attribute on line `index`."""
    for line in lines[index + 1 : index + 40]:
        stripped = line.strip()
        if not stripped or stripped.startswith(("#[", "///", "//")):
            continue
        if match := FN.match(line):
            return "fn", match.group(2), len(match.group(1)) > 0
        if match := TYPE.match(line):
            return match.group(2), match.group(3), len(match.group(1)) > 0
        if match := VARIANT.match(line):
            return "variant", match.group(2), True
        return None
    return None


def rustdoc_pages() -> dict[str, str]:
    """Maps each item's path, such as `graphics::Image`, to its page."""
    if not ALL_HTML.exists():
        return {}
    text = ALL_HTML.read_text(encoding="utf-8")
    return {
        html.unescape(name): href
        for href, name in re.findall(r'<a href="([^"]+)">([^<]+)</a>', text)
        if href.endswith(".html") and "/" not in name
    }


def type_page(pages: dict[str, str], name: str) -> tuple[str, str] | None:
    """The full path and page of the type called `name`."""
    for path, href in pages.items():
        if path == name or path.endswith("::" + name):
            if re.search(r"(struct|enum|trait|type)\.", href):
                return path, href
    return None


def main() -> int:
    pages = rustdoc_pages()
    entries: dict[str, set[tuple[str, str]]] = defaultdict(set)
    unresolved = []

    for source in SOURCES:
        for path in sorted(source.rglob("*.rs")):
            lines = path.read_text(encoding="utf-8").splitlines()
            for index, line in enumerate(lines):
                for alias in ALIAS.findall(line):
                    item = find_item(lines, index)
                    if item is None:
                        unresolved.append(f"{path.relative_to(ROOT)}:{index + 1} {alias}")
                        continue
                    kind, name, nested = item
                    module = module_of(path)
                    search = f"index.html?search={alias}"
                    if nested:
                        owner = owner_at(lines, index)
                        if owner is None:
                            unresolved.append(f"{path.relative_to(ROOT)}:{index + 1} {alias}")
                            continue
                        found = type_page(pages, owner)
                        anchor = "variant" if kind == "variant" else "method"
                        if found:
                            full, href = found
                            text, link = f"{full}::{name}", f"{href}#{anchor}.{name}"
                        else:
                            prefix = f"{module}::" if module else ""
                            text, link = f"{prefix}{owner}::{name}", search
                    else:
                        full = f"{module}::{name}" if module else name
                        text, link = full, pages.get(full, search)
                    entries[alias].add((text, link))

    for problem in unresolved:
        print(f"could not place alias: {problem}", file=sys.stderr)

    groups: dict[str, list[str]] = defaultdict(list)
    for alias in entries:
        if alias.startswith("love."):
            group = ".".join(alias.split(".")[:2])
        elif ":" in alias:
            group = alias.split(":")[0]
        else:
            group = "Other names"
        groups[group].append(alias)

    def group_order(name: str):
        return (not name.startswith("love."), name == "Other names", name.lower())

    out = [
        "+++",
        'title = "LÖVE name lookup"',
        'description = "Find the Smitten item that replaces a LÖVE function whose name changed."',
        "weight = 14",
        "[extra]",
        'nav = "LÖVE name lookup"',
        "no_toc = true",
        "+++",
        "",
        "<!-- Generated by site/tools/love_names.py from the doc aliases in the",
        "source. Edit the aliases, not this file. -->",
        "",
        "Most LÖVE functions keep their name in Smitten, in snake case and",
        "without `get`: `love.graphics.setColor` is `graphics::set_color` and",
        "`Source:getVolume` is `Source::volume`. [Coming from LÖVE](@/guide/coming-from-love.md)",
        "explains those rules.",
        "",
        "This page lists the LÖVE names that are search aliases in the",
        "[API reference](../../api/smitten/index.html), mostly ones whose",
        "Smitten item is named or shaped differently: constructors such as",
        "`newImage`, which are `load` or `new` on the type, getters that take",
        "a mode argument in LÖVE, and functions that moved to another module.",
        "Searching the API reference for any of them, such as `newImage`,",
        "finds the item.",
        "",
    ]
    for group in sorted(groups, key=group_order):
        anchor_title = group if group != "Other names" else "Other names"
        out.append(f"## {anchor_title}")
        out.append("")
        out.append("| LÖVE | Smitten |")
        out.append("|---|---|")
        for alias in sorted(groups[group], key=str.lower):
            items = ", ".join(
                f"[`{text}`](../../api/smitten/{link})"
                for text, link in sorted(entries[alias])
            )
            out.append(f"| `{alias}` | {items} |")
        out.append("")

    OUT.write_text("\n".join(out), encoding="utf-8")
    count = len(entries)
    print(f"wrote {OUT.relative_to(ROOT)} with {count} LÖVE names", file=sys.stderr)
    return 1 if unresolved else 0


if __name__ == "__main__":
    sys.exit(main())
