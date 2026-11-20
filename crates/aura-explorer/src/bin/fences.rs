//! `fences.exe` — the desktop fence host (see `aura_explorer::run_fences`).
//!
//! The file window starts this program for the desktop. It is the same desktop mode
//! `AuraExplorer.exe --desktop` runs, in a program of its own.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    aura_explorer::run_fences()
}
