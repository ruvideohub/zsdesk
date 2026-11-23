#!/usr/bin/env python3
"""Static guard for the standalone Fences boundary and the integrated AuraUI package."""
from pathlib import Path
import json
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]
FENCES = ROOT / "crates" / "aura-fences"
MODEL = ROOT / "crates" / "aura-fences-model"
EXPLORER = ROOT / "crates" / "aura-explorer"


def fail(message: str) -> None:
    raise SystemExit(f"Fences boundary check failed: {message}")


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8-sig", errors="replace")


workspace = tomllib.loads(read(ROOT / "Cargo.toml"))
members = set(workspace["workspace"]["members"])
for member in ("crates/aura-fences", "crates/aura-fences-model"):
    if member not in members:
        fail(f"{member} is not a workspace member")

manifest = tomllib.loads(read(FENCES / "Cargo.toml"))
if manifest["package"]["name"] != "aura-fences":
    fail("standalone package name changed unexpectedly")
if not any(binary.get("name") == "Fences" for binary in manifest.get("bin", [])):
    fail("standalone binary target Fences is missing")

app_crates = {"aura-explorer", "aura-dock", "aura-service"}
allowed_internal = {"aura-core", "aura-fences-model", "aura-ui", "aura-windows"}
direct = set()
internal_paths = set()
for section in ("dependencies", "dev-dependencies", "build-dependencies"):
    dependencies = manifest.get(section, {})
    direct |= set(dependencies)
    internal_paths |= {name for name, value in dependencies.items()
                       if isinstance(value, dict) and "path" in value}
for target_tables in manifest.get("target", {}).values():
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        dependencies = target_tables.get(section, {})
        direct |= set(dependencies)
        internal_paths |= {name for name, value in dependencies.items()
                           if isinstance(value, dict) and "path" in value}
if direct & app_crates:
    fail(f"Fences depends on an app crate: {sorted(direct & app_crates)}")
if not internal_paths <= allowed_internal:
    fail(f"non-neutral internal path dependencies: {sorted(internal_paths - allowed_internal)}")

source_files = list(FENCES.rglob("*.rs"))
source = "\n".join(read(path) for path in source_files)
entry = read(FENCES / "src" / "main.rs")
declared_modules = set(re.findall(r"(?m)^mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;", entry))
declared_roots = declared_modules | {"ApplicationLifetime"}
for path in source_files:
    roots = set(re.findall(r"\bcrate::([A-Za-z_][A-Za-z0-9_]*)", read(path)))
    missing = roots - declared_roots
    if missing:
        fail(f"undeclared crate roots in {path.relative_to(ROOT)}: {sorted(missing)}")

for pattern, label in (
    (r"\baura_explorer::", "AuraExplorer Rust-crate reference"),
    (r"\baura_dock::", "AuraDock Rust-crate reference"),
    (r"AuraExplorer\.exe", "AuraExplorer executable dependency"),
    (r"AuraDock\.exe", "AuraDock executable dependency"),
    (r"--desktop(?:\s|\")", "legacy AuraExplorer --desktop mode"),
):
    if re.search(pattern, source):
        fail(label)
if not (MODEL / "src" / "desktop.rs").is_file():
    fail("Fences model source is missing")
if (ROOT / "crates" / "aura-core" / "src" / "model" / "desktop.rs").exists():
    fail("desktop model is still in aura-core")

old_refs = []
for path in (ROOT / "crates").rglob("*.rs"):
    if "aura-fences" in path.parts:
        continue
    text = read(path)
    if "aura_core::model::desktop" in text or "crate::model::desktop" in text:
        old_refs.append(str(path.relative_to(ROOT)))
if old_refs:
    fail("old model path remains: " + ", ".join(old_refs))

# AuraExplorer may launch a separate Fences binary, but no longer owns its host or model.
if (EXPLORER / "src" / "desktop").exists():
    fail("legacy Fences host directory remains inside AuraExplorer")
explorer_manifest = tomllib.loads(read(EXPLORER / "Cargo.toml"))
explorer_dependencies = set(explorer_manifest.get("dependencies", {}))
if "aura-fences-model" in explorer_dependencies:
    fail("AuraExplorer still depends on aura-fences-model")
if "desktop-box" in explorer_manifest.get("features", {}):
    fail("desktop-box remains an AuraExplorer feature")
explorer_source = "\n".join(read(path) for path in EXPLORER.rglob("*.rs"))
if re.search(r"\bDESKTOP_ONLY\b|\bdesktop_only\b", explorer_source):
    fail("AuraExplorer exposes the obsolete desktop-only build identity")
app_source = read(EXPLORER / "src" / "app.rs")
if 'Some("--desktop"' not in app_source or "legacy_desktop_host_arguments_are_rejected" not in app_source:
    fail("AuraExplorer does not explicitly reject its legacy --desktop mode")

lock = tomllib.loads(read(ROOT / "Cargo.lock"))
packages = {(package["name"], package["version"]): package for package in lock["package"]}
if not any(name == "aura-fences" for name, _ in packages):
    fail("Cargo.lock has no aura-fences package entry")
if not any(name == "aura-fences-model" for name, _ in packages):
    fail("Cargo.lock has no aura-fences-model package entry")
explorer_lock = next((package for package in lock["package"] if package["name"] == "aura-explorer"), None)
if explorer_lock is None or any(dep.split(" ")[0] == "aura-fences-model" for dep in explorer_lock["dependencies"]):
    fail("Cargo.lock still connects AuraExplorer to aura-fences-model")

shell_handoff = read(FENCES / "src" / "system_shell.rs")
if 'Command::new("explorer.exe")' not in shell_handoff:
    fail("Fences does not hand file/folder opens to Windows Explorer")
fences_host = read(FENCES / "src" / "desktop" / "host.rs")
for misleading in ("在 AuraFences 中打开", "打开 AuraFences 文件管理器"):
    if misleading in fences_host:
        fail(f"Fences menu promises an in-app target that is delegated elsewhere: {misleading}")

# The integrated bundle builds Fences from its own Cargo package and the service starts that exact file.
bundle = json.loads(read(ROOT / "packaging" / "bundle.json"))
fences_entry = next((entry for entry in bundle["entries"] if entry["name"] == "Fences.exe"), None)
if not fences_entry or not fences_entry.get("required"):
    fail("Fences.exe is not a required integrated-bundle entry")
if fences_entry.get("source") != {"kind": "cargo", "package": "aura-fences", "bin": "Fences"}:
    fail("bundle.json does not build Fences.exe from aura-fences --bin Fences")
package_script = read(ROOT / "scripts" / "package.ps1")
if "source.kind -eq 'cargo'" not in package_script or "($bin + '.exe')" not in package_script:
    fail("the generic bundle packager does not build/copy Cargo binaries by bundle.json")
launch = read(ROOT / "crates" / "aura-service" / "src" / "launch.rs")
if not re.search(r'id:\s*"fences"[\s\S]*?exe:\s*"Fences\.exe"[\s\S]*?args:\s*&\["--tray"\]', launch):
    fail("AuraService does not launch Fences.exe --tray")

install_helper = read(ROOT / "packaging" / "windows" / "install.ps1")
if "'Fences.exe'" not in install_helper or "FencesLink" not in install_helper:
    fail("suite install helper does not require and expose Fences.exe")
if "Set-Autostart 'AuraExplorer'" not in install_helper or "Set-Autostart 'Fences'" in install_helper:
    fail("suite autostart is not owned exclusively by AuraService")
if re.search(r"Start-Process[^\n]*--desktop", install_helper):
    fail("suite installer still starts the removed AuraExplorer desktop mode")

aura_installer = read(ROOT / "packaging" / "windows" / "aura.iss")
if 'Source: "{#SourceDir}\\Fences.exe"' not in aura_installer:
    fail("AuraUI Inno installer does not include Fences.exe")
if 'Parameters: "--run"' not in aura_installer or 'Parameters: "--enable-startup"' in aura_installer:
    fail("AuraUI Inno installer does not preserve service-owned startup")
if 'Fences.exe"; Parameters: "--uninstall-startup"' not in aura_installer:
    fail("AuraUI Inno installer does not clean this copy's former standalone startup")

# Standalone Fences retains its own optional sign-in setting, without requiring Explorer or the service.
fences_package = read(ROOT / "scripts" / "package-fences.ps1")
fences_installer = read(ROOT / "packaging" / "windows" / "fences.iss")
if "Fences.exe" not in fences_package or "manifest.json" not in fences_package:
    fail("standalone Fences package misses its executable or manifest")
if 'CloseApplicationsFilter=Fences.exe' not in fences_installer or 'CloseApplicationsFilter=Fences.exe;AuraExplorer.exe' in fences_installer:
    fail("standalone Fences installer closes or depends on AuraExplorer")
if '--enable-startup' not in fences_installer:
    fail("standalone Fences installer does not configure its own optional startup")
for license_name in ("MPL-2.0.txt", "MinHook-LICENSE.txt", "ExplorerColumnView-MIT.txt"):
    if license_name not in fences_package or not (ROOT / "packaging" / "licenses" / license_name).is_file():
        fail(f"standalone Fences package misses its applicable license text: {license_name}")

# Both app menus open the same launch-settings window when AuraService is installed.
explorer_settings = read(EXPLORER / "src" / "window" / "startup_settings.rs")
fences_bridge = read(FENCES / "src" / "service_start.rs")
fences_menu = read(FENCES / "src" / "desktop" / "host.rs")
if '"launch-settings" => crate::service_start::open_settings()' not in explorer_settings:
    fail("AuraExplorer startup settings do not open AuraService settings")
if 'command.arg("--settings")' not in fences_bridge or 'crate::service_start::available()' not in fences_menu:
    fail("Fences does not bridge to the shared AuraService settings window")

print("Fences boundary checks passed (static source/packaging checks only; no Cargo, Windows, or Inno build implied).")
