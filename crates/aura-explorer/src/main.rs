//! `AuraExplorer.exe` — the file manager. Everything it shares with `fences.exe` lives in
//! the library (`src/lib.rs`); this file is only the entry point.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    aura_explorer::run()
}
