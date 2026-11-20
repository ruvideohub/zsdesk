//! Gives `auraservice.exe` an icon and a version block.
//!
//! The service is the one program of the suite a person sees without opening anything:
//! the settings window, the installer's shortcut and the uninstall list. Without a
//! resource it was a blank generic program. The icon comes from
//! `resources/auraservice.ico`, drawn by `scripts/make-icons.py` in the same gold as the
//! rest, as a gear. The windows are set up in `window.rs`, so no manifest is needed.

fn main() {
    println!("cargo:rerun-if-changed=resources/auraservice.rc");
    println!("cargo:rerun-if-changed=resources/auraservice.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    embed_resource::compile("resources/auraservice.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed the service icon and version block");
}
