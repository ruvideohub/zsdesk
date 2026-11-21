//! `AuraExplorer.exe` — the file manager. Shared infrastructure lives in the common crates;
//! the standalone desktop-grid host is the separate `Fences.exe` application. This is only the entry point.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    aura_explorer::run()
}
