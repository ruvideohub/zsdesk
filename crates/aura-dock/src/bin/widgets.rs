//! `widgets.exe` — the desktop-widget host (see `aura_dock::run_widgets`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> windows::core::Result<()> {
    aura_dock::run_widgets()
}
