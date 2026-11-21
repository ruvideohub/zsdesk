# Fences boundary and packaging audit

Status snapshot: 2026-11-19, workspace `/home/user/zsdesk-github-checkout`. Scope is the standalone Fences extraction and its handoff/package paths. This does not claim that widgets, AuraService, or AuraArc were split.

## Acceptance target

- `Fences.exe` is a standalone application, not an AuraExplorer desktop mode or process worker.
- Fences owns its profile and can best-effort migrate a legacy desktop layout/settings.
- Opening a file or folder from Fences delegates to the system `explorer.exe`.
- The complete AuraUI bundle contains `Fences.exe`, and AuraService may supervise it as an independent component.
- A standalone Fences ZIP/optional installer contains no AuraExplorer, AuraDock, AuraService, or widgets executable.

## Current structure

| Crate/component | Role and boundary |
|---|---|
| `crates/aura-fences-model` | Neutral domain/model crate for desktop layout and fence rules; consumed by Fences, not Explorer. |
| `crates/aura-fences` | Binary package with target `Fences`; owns the desktop host, Fences resources, startup commands and lifecycle. Direct internal dependencies are `aura-core`, `aura-fences-model`, `aura-ui`, and `aura-windows`; no app-crate dependency. |
| `crates/aura-explorer` | File manager. The former `src/desktop` host and direct model dependency are removed. Legacy desktop-host CLI arguments are rejected; menu actions locate adjacent `Fences.exe`. The `desktop` feature is packaging metadata only. |
| `crates/aura-dock` | Separate panel companion. `widgets.exe` remains a child process of AuraDock; Fences does not launch Explorer or Dock. |
| `crates/aura-service` | Integrated suite launcher. It supervises Fences, AuraDock, and optionally AuraExplorer; it does not include their application code. |

Fences stores its profile under `%APPDATA%\\AuraFences`, has portable mode and a one-time best-effort migration from older AuraExplorer/ZSDesk profile layouts. `crates/aura-fences/src/system_shell.rs` passes open targets to the system `explorer.exe`.

## Packaging and installer paths

- `packaging/bundle.json` is the source for the complete AuraUI bundle and includes `Fences.exe` as the `aura-fences` / `Fences` Cargo target.
- `scripts/package.ps1` builds/assembles that complete bundle from `bundle.json`; it does not build edition-specific AuraExplorer variants. `scripts/verify-bundle.ps1` verifies the staged files and manifest.
- `.github/workflows/dock-build.yml` builds `aura-fences --bin Fences` separately, then stages and checks the complete suite with `scripts/package.ps1`.
- `scripts/package-fences.ps1` produces a standalone Fences ZIP and an optional Inno Setup installer. It includes the Fences executable, runtime/support files and licenses, and rejects other Aura application executables in the ZIP.
- `packaging/windows/aura.iss` and `packaging/windows/install.ps1` install the full suite. AuraService owns the integrated startup entry; the suite removes the startup entry for its exact Fences copy.
- `packaging/windows/fences.iss` and `install-fences.ps1` provide a separate user-level Fences install with an optional Fences-owned startup entry. It does not require AuraExplorer, AuraDock, or AuraService.
- `scripts/check_fences_boundary.py` checks crate boundaries, the Explorer-to-Fences handoff, and static packaging/installer assumptions.

## Static checks performed

- `python3 scripts/check_fences_boundary.py` passes. It is a static source/packaging check, not a compiler test.
- `python3 scripts/check-resources.py` passes for six programs at version 0.3.23.
- `python3 scripts/check-translations.py` passes for all 1,149 user-visible string entries.
- Python `tomllib` parses the workspace manifests and `Cargo.lock`; JSON parsing also passes.
- The `aura-explorer` lockfile entry no longer lists `aura-fences-model`; the new `aura-fences` and `aura-fences-model` package entries are present.

## Not verified in this environment

`cargo`, `rustc`, `rustfmt`, `pwsh`/`powershell`, and Inno Setup are unavailable here. Therefore no Rust build/test, PowerShell parser run, installer compile, Windows runtime smoke test, or actual ZIP/installer generation has completed. `Cargo.lock` was edited manually and still must be checked/regenerated with Cargo on the target toolchain.

## Required Windows validation

1. Run `cargo test --workspace --locked`; build `aura-explorer`, `aura-fences --bin Fences`, and the full suite with `--locked`. Resolve build/test errors and regenerate `Cargo.lock` with Cargo if needed.
2. Parse/run the PowerShell packaging and install helpers; compile standalone `fences.iss` and the complete `aura.iss` installer.
3. Smoke-test install, upgrade and uninstall. Verify the complete suite starts Fences without `AuraExplorer.exe --desktop`, standalone Fences works with no AuraExplorer/AuraDock present, legacy settings migrate, startup ownership is correct, and Open actions invoke Windows Explorer.
4. Inspect generated ZIP/installer contents and manifest hashes on Windows before release.
