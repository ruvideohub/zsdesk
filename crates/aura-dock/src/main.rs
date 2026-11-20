//! `AuraDock.exe` — the panel. Everything it shares with `widgets.exe` lives in the
//! library (`src/lib.rs`); this file is only the entry point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> windows::core::Result<()> {
    aura_dock::run()
}
