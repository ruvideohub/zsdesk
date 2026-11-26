//! Resolve the Dock theme without requiring Dock to be running or installed.
use std::{
    cell::RefCell,
    path::PathBuf,
    time::{Duration, Instant},
};

thread_local! { static CACHE: RefCell<Option<(Instant, Option<bool>)>> = const { RefCell::new(None) }; }

fn match_windows_setting() -> Option<bool> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((at, value)) = *cache {
            if at.elapsed() < Duration::from_secs(1) {
                return value;
            }
        }
        let value = std::env::var_os("APPDATA").and_then(|base| {
            let path = PathBuf::from(base)
                .join("AuraUI")
                .join("Dock")
                .join("config.json");
            // The Dock file is user-controlled; avoid reading unexpectedly huge files.
            if std::fs::metadata(&path).ok()?.len() > 2 * 1024 * 1024 {
                return None;
            }
            let raw = std::fs::read(path).ok()?;
            aura_core::desktop_style::dock_match_windows_setting(&raw)
        });
        *cache = Some((Instant::now(), value));
        value
    })
}

pub fn dock_dark() -> bool {
    let forced = std::env::var("LAMINAR_THEME").ok();
    aura_core::desktop_style::resolve_dock_dark(
        match_windows_setting(),
        aura_windows::theme::system_dark_mode(),
        forced.as_deref(),
    )
}
