#!/usr/bin/env python3
"""Check that every user-visible string has a translation.

The product's source strings are Chinese and `aura_core::i18n::text` looks them up in
`crates/aura-core/resources/strings.en.json`. A missing entry does not fail anything at
compile time — it just shows Chinese to a user who reads Russian. That is how a Chinese
row appeared in the fence settings and in the file manager while every build was green.

So: every literal handed to `text(`, `tr!` or `tr_format!` must have an entry. Run it from
the repository root:

    python3 scripts/check-translations.py
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CATALOGUE = ROOT / "crates" / "aura-core" / "resources" / "strings.en.json"
CJK = re.compile(r"[\u3400-\u9fff\uff00-\uffef]")
LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"')
CALL = re.compile(r"(?:i18n::text|\btext|tr_format!|\btr!)\s*\(")
SKIP_FILES = ("_tests.rs", "_regression.rs")


def unescape(literal: str) -> str:
    """The characters a Rust string literal really holds."""
    out: list[str] = []
    index = 0
    simple = {"n": "\n", "r": "\r", "t": "\t", "0": "\0", "\\": "\\", '"': '"', "'": "'"}
    while index < len(literal):
        character = literal[index]
        if character != "\\":
            out.append(character)
            index += 1
            continue
        index += 1
        if index >= len(literal):
            break
        escape = literal[index]
        if escape in simple:
            out.append(simple[escape])
            index += 1
        elif escape == "u" and literal[index + 1] == "{":
            end = literal.index("}", index)
            out.append(chr(int(literal[index + 2:end], 16)))
            index = end + 1
        else:
            out.append(escape)
            index += 1
    return "".join(out)


def without_tests(source: str) -> str:
    marker = re.search(r"#\[cfg\(test\)\]", source)
    return source[: marker.start()] if marker else source


def main() -> int:
    catalogue = json.loads(CATALOGUE.read_text(encoding="utf-8"))
    missing: dict[str, list[str]] = {}
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        if any(path.name.endswith(suffix) for suffix in SKIP_FILES) or "examples" in path.parts:
            continue
        code = without_tests(path.read_text(encoding="utf-8", errors="replace"))
        for number, line in enumerate(code.split("\n"), 1):
            stripped = line.lstrip()
            if stripped.startswith(("//", "*", "/*")) or not CALL.search(line):
                continue
            for literal in LITERAL.findall(line):
                if not CJK.search(literal):
                    continue
                wanted = unescape(literal)
                if wanted not in catalogue:
                    where = f"{path.relative_to(ROOT)}:{number}"
                    missing.setdefault(wanted, []).append(where)
    if missing:
        print(f"нет перевода: {len(missing)} строк")
        for wanted, places in sorted(missing.items()):
            print(f"  {places[0]}  {wanted!r}")
        return 1
    print(f"перевод есть у каждой строки, которую показывает интерфейс ({len(catalogue)} записей)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
