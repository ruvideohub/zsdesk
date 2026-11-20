//! Gives `AuraArc.exe` an icon and a version block.
//!
//! The archiver had neither, so it showed up in Explorer, in the Start menu, in the
//! uninstall list and on the taskbar as a blank generic program next to three icons of
//! the same family. The icon comes from `resources/aura-arc.ico`, drawn by
//! `scripts/make-icons.py` in the same gold as the folder and the panel: a box with a
//! zipper. The window is a plain per-monitor-DPI-aware window set up in `window.rs`, so
//! no manifest is needed here.

fn main() {
    println!("cargo:rerun-if-changed=resources/aura-arc.rc");
    println!("cargo:rerun-if-changed=resources/aura-arc.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    embed_resource::compile("resources/aura-arc.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed the archiver icon and version block");
}
