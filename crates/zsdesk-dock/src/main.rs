//! ZSDesk dock — a macOS-style application strip that replaces the Start menu
//! workflow on the Windows desktop.
//!
//! The binary runs as its own single-instance process (like the desktop
//! fences component), reads its pins from `dock.json` inside the ZSDesk
//! profile and talks to the Shell for icons, launching and window control.
//!
//! Landing pages: [`crate::dock`] for the window, [`crate::layout`] for the
//! geometry, [`crate::model`] for the running-application grouping.

#![cfg_attr(all(windows, not(debug_assertions), not(test)), windows_subsystem = "windows")]
// The dock window itself is Windows-only; the portable modules stay covered by
// their unit tests on other hosts, where nothing in them is reachable from main.
#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

mod catalogue;
mod config;
mod layout;
mod magnify;
mod model;
mod panel;
mod paths;
mod status;
mod tray;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod apps;
#[cfg(windows)]
mod dock;
#[cfg(windows)]
mod icons;
#[cfg(windows)]
mod language;
#[cfg(windows)]
mod launcher;
#[cfg(windows)]
mod panel_window;
#[cfg(windows)]
mod winlist;

#[cfg(windows)]
fn main() {
    if let Err(error) = app::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("zsdesk-dock is a Windows component; this build only contains the portable logic.");
}
