//! Gives `AuraDock.exe` an icon and a version block.
//!
//! Without this the panel's executable shows up in Explorer, in the Start menu and in
//! the uninstall list as a blank generic program. The icon comes from
//! `resources/aura-dock.ico`, drawn by `scripts/make-icons.py` in the same gold as the
//! file manager's folder, and the resource file carries the version numbers so the
//! panel is recognisable next to `AuraExplorer.exe`.
//!
//! The panel needs no manifest (it is a normal per-monitor-DPI-aware program set up in
//! `main`), so the manifest is optional here — unlike `aura-explorer`, whose manifest is
//! required for its window hook and shell integration.

fn main() {
    println!("cargo:rerun-if-changed=resources/aura-dock.rc");
    println!("cargo:rerun-if-changed=resources/aura-dock.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    embed_resource::compile("resources/aura-dock.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed the panel icon and version block");
}
