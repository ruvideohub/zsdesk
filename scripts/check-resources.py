#!/usr/bin/env python3
"""Check that every program of the suite carries an icon and a version block.

A missing `.rc` is invisible until somebody sees a blank generic program in Explorer,
in the Start menu or in the uninstall list — which is exactly how `AuraArc.exe` and
`auraservice.exe` shipped without one. And a version block that nobody refreshes says
0.3.18 for two releases, so the file properties lie about the build.

Run from the repository root:

    python3 scripts/check-resources.py
"""
from __future__ import annotations

import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def workspace_version() -> str:
    with (ROOT / "Cargo.toml").open("rb") as handle:
        return tomllib.load(handle)["workspace"]["package"]["version"]


def programs() -> list[tuple[str, str]]:
    """Every executable the suite ships, with the package directory that builds it."""
    found: list[tuple[str, str]] = []
    for manifest in sorted(ROOT.glob("crates/*/Cargo.toml")):
        with manifest.open("rb") as handle:
            parsed = tomllib.load(handle)
        package = manifest.parent
        for binary in parsed.get("bin", []):
            name = binary.get("name") or package.name
            found.append((name, str(package.relative_to(ROOT))))
        # A package may ship both a library and a program: `aura-explorer` builds
        # `AuraExplorer.exe` from src/main.rs and lists only `fences` in `[[bin]]`, and
        # the unlisted one is the program a person sees most often.
        listed = {binary.get("path") for binary in parsed.get("bin", [])}
        if (package / "src" / "main.rs").exists() and "src/main.rs" not in listed:
            found.append((parsed["package"]["name"], str(package.relative_to(ROOT))))
    return found


def main() -> int:
    version = workspace_version()
    wanted = [f"{version}"]
    numeric = ",".join(version.split(".") + ["0"])
    problems: list[str] = []
    for name, directory in programs():
        package = ROOT / directory
        resources = package / "resources"
        rc_files = sorted(resources.glob("*.rc")) if resources.is_dir() else []
        if not rc_files:
            problems.append(f"{name}: нет .rc в {directory}/resources — файл будет без значка и без версии")
            continue
        text = "".join(rc.read_text(encoding="utf-8-sig", errors="replace") for rc in rc_files)
        for icon in re.findall(r'ICON\s+"([^"]+)"', text):
            icon_path = package / icon.replace("\\\\", "/")
            if not icon_path.is_file():
                problems.append(f"{name}: {rc_files[0].name} ссылается на {icon}, а файла нет")
        if f"FILEVERSION {numeric}" not in text:
            problems.append(f"{name}: FILEVERSION не {version} — свойства файла будут врать о сборке")
        if (package / "build.rs").is_file() and "embed_resource::compile" not in (package / "build.rs").read_text(encoding="utf-8", errors="replace"):
            problems.append(f"{name}: build.rs есть, но ресурсы не встраиваются")
        if not (package / "build.rs").is_file():
            problems.append(f"{name}: нет build.rs — .rc никто не встроит")
    if problems:
        for problem in problems:
            print(f"ошибка: {problem}")
        return 1
    print(f"значки и версии на месте у {len(programs())} программ ({version})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
