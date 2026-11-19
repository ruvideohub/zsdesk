//! `dock.json`: pins and appearance, stored next to the other ZSDesk profile
//! files so a portable copy keeps its dock.
//!
//! Reading never fails: a missing, unreadable or corrupt file falls back to
//! the defaults, because a dock that refuses to start is worse than a dock
//! with factory settings. Writing is atomic (temporary file + rename) so a
//! power loss cannot leave half a JSON document behind.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::normalize_exe;

/// Bumped only when an older reader could misinterpret a newer file.
pub const SCHEMA: u32 = 1;

/// One pinned icon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    /// What the dock launches: an `.exe`, a `.lnk` or a `shell:`/`ms-settings:` URI.
    pub path: String,
    /// Executable the `.lnk` points at; empty when `path` already is the executable.
    #[serde(default)]
    pub target: String,
    /// Title override; `None` uses the file name.
    #[serde(default)]
    pub label: Option<String>,
}

impl Pin {
    pub fn new(path: impl Into<String>, target: impl Into<String>, label: Option<String>) -> Self {
        Self { path: path.into(), target: target.into(), label }
    }

    /// Executable used to match running windows and to load the icon.
    pub fn executable(&self) -> &str {
        if self.target.trim().is_empty() { &self.path } else { &self.target }
    }

    pub fn matches_exe(&self, executable: &str) -> bool {
        let wanted = normalize_exe(self.executable());
        wanted == normalize_exe(executable) || normalize_exe(&self.path) == normalize_exe(executable)
    }
}

/// Appearance and behaviour of the dock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    pub schema: u32,
    pub pins: Vec<Pin>,
    /// macOS-style hover magnification.
    pub magnification: bool,
    /// Largest icon scale under the pointer (1.0 = off).
    pub magnify_scale: f32,
    /// Influence radius of the magnification, in icon pitches.
    pub magnify_influence: f32,
    /// Icon edge length in DIPs at 100 % DPI.
    pub icon_size: f32,
    /// Slide the dock off screen when the pointer leaves it.
    pub autohide: bool,
    pub show_launcher: bool,
    pub show_trash: bool,
    pub show_clock: bool,
    /// Network, volume and battery indicators next to the clock.
    pub show_status: bool,
    /// 24-hour clock; `false` appends AM/PM.
    pub clock_24h: bool,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            pins: default_pins(),
            magnification: true,
            magnify_scale: 1.85,
            magnify_influence: 2.5,
            icon_size: 48.0,
            autohide: false,
            show_launcher: true,
            show_trash: true,
            show_clock: true,
            show_status: true,
            clock_24h: true,
        }
    }
}

impl DockConfig {
    /// Keeps a hand-edited file from producing an unusable dock.
    pub fn sanitized(mut self) -> Self {
        self.schema = SCHEMA;
        self.magnify_scale = if self.magnify_scale.is_finite() { self.magnify_scale.clamp(1.0, 3.0) } else { 1.85 };
        self.magnify_influence = if self.magnify_influence.is_finite() { self.magnify_influence.clamp(0.5, 8.0) } else { 2.5 };
        self.icon_size = if self.icon_size.is_finite() { self.icon_size.clamp(24.0, 96.0) } else { 48.0 };
        self.pins.retain(|pin| !pin.path.trim().is_empty());
        let mut seen: Vec<String> = Vec::with_capacity(self.pins.len());
        self.pins.retain(|pin| {
            let key = normalize_exe(pin.executable());
            if seen.contains(&key) { false } else { seen.push(key); true }
        });
        self
    }

    /// Loads the profile; anything unreadable yields the defaults. On a fresh
    /// profile the seeded pins are pruned so only installed tools remain.
    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else { return Self::fresh() };
        match serde_json::from_str::<Self>(&text) {
            Ok(config) => config.sanitized(),
            Err(_) => Self::fresh(),
        }
    }

    /// Defaults with the seeded pins reduced to the tools that exist here.
    pub fn fresh() -> Self {
        let mut config = Self::default();
        config.pins = existing_pins(config.pins);
        config.sanitized()
    }

    /// Writes the profile beside `path`, creating the directory when needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(directory) = path.parent() { std::fs::create_dir_all(directory)?; }
        let bytes = serde_json::to_vec_pretty(&self.clone().sanitized())
            .map_err(std::io::Error::other)?;
        let temporary = path.with_extension(format!("{}-{}.tmp", std::process::id(), unique()));
        std::fs::write(&temporary, bytes)?;
        // `rename` replaces the target atomically on NTFS and on Unix.
        match std::fs::rename(&temporary, path) {
            Ok(()) => Ok(()),
            Err(error) => { let _ = std::fs::remove_file(&temporary); Err(error) }
        }
    }

    pub fn is_pinned(&self, executable: &str) -> bool {
        self.pins.iter().any(|pin| pin.matches_exe(executable))
    }

    /// Adds a pin unless the executable already has one. Returns `true` when
    /// the list changed.
    pub fn pin(&mut self, pin: Pin) -> bool {
        if self.is_pinned(pin.executable()) { return false; }
        self.pins.push(pin);
        true
    }

    /// Removes the pin of `executable`, returning the removed entry.
    pub fn unpin(&mut self, executable: &str) -> Option<Pin> {
        let index = self.pins.iter().position(|pin| pin.matches_exe(executable))?;
        Some(self.pins.remove(index))
    }

}

fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CLOCK: AtomicU64 = AtomicU64::new(1);
    let tick = CLOCK.fetch_add(1, Ordering::Relaxed);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos() as u64 + tick)
        .unwrap_or(tick)
}

/// What a first start shows: common tools that exist on every Windows 10/11
/// installation. Missing entries are pruned by `existing_pins`.
pub fn default_pins() -> Vec<Pin> {
    let windows = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let system = PathBuf::from(&windows).join("System32");
    let mut pins = Vec::new();
    let mut add = |path: PathBuf, label: Option<&str>| {
        let path = path.to_string_lossy().to_string();
        pins.push(Pin::new(path.clone(), path, label.map(str::to_string)));
    };
    add(PathBuf::from(&windows).join("explorer.exe"), None);
    add(system.join("notepad.exe"), None);
    add(system.join("calc.exe"), None);
    add(system.join("mspaint.exe"), None);
    add(system.join("WindowsPowerShell/v1.0/powershell.exe"), None);
    pins.push(Pin::new("ms-settings:", "SystemSettings.exe", None));
    pins
}

/// Drops pins whose executable does not exist, keeping `shell:` style entries.
pub fn existing_pins(pins: Vec<Pin>) -> Vec<Pin> {
    // A pin is usable when either what it launches or the executable behind it
    // exists; `ms-settings:` and `shell:` URIs have no file on disk.
    let usable = |value: &str| {
        let value = value.trim();
        !value.is_empty() && (is_shell_uri(value) || Path::new(value).is_file())
    };
    pins.into_iter()
        .filter(|pin| usable(&pin.path) || usable(pin.executable()))
        .collect()
}

fn is_shell_uri(value: &str) -> bool {
    value.contains(':') && !value.contains('\\') && !value.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("zsdesk-dock-{tag}-{}", unique()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn defaults_round_trip_through_json() {
        let config = DockConfig::default();
        let text = serde_json::to_string(&config).unwrap();
        let parsed: DockConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.sanitized(), config.sanitized());
        assert!(DockConfig::default().magnification);
    }

    #[test]
    fn partial_files_keep_the_remaining_defaults() {
        let parsed: DockConfig = serde_json::from_str(r#"{"magnification":false}"#).unwrap();
        assert!(!parsed.magnification);
        assert_eq!(parsed.icon_size, DockConfig::default().icon_size);
        assert!(parsed.show_clock);
    }

    #[test]
    fn hand_edited_values_are_clamped() {
        let parsed = serde_json::from_str::<DockConfig>(
            r#"{"magnify_scale":99.0,"icon_size":-4.0,"magnify_influence":0.0}"#).unwrap().sanitized();
        assert_eq!(parsed.magnify_scale, 3.0);
        assert_eq!(parsed.icon_size, 24.0);
        assert_eq!(parsed.magnify_influence, 0.5);
    }

    #[test]
    fn a_corrupt_or_missing_file_falls_back_to_defaults() {
        let directory = temp_dir("corrupt");
        let path = directory.join("dock.json");
        assert_eq!(DockConfig::load(&path), DockConfig::fresh());
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(DockConfig::load(&path), DockConfig::fresh());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saving_is_atomic_and_reloads_identically() {
        let directory = temp_dir("save");
        let path = directory.join("profile/dock.json");
        let mut config = DockConfig::default();
        config.magnification = false;
        config.pins = vec![Pin::new(r"C:\Tools\App.exe", "", Some("Приложение".into()))];
        config.save(&path).unwrap();
        assert_eq!(DockConfig::load(&path), config.clone().sanitized());
        // No temporary files are left behind.
        let leftovers = std::fs::read_dir(path.parent().unwrap()).unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn pins_are_unique_and_can_be_removed() {
        let mut config = DockConfig { pins: Vec::new(), ..DockConfig::default() };
        assert!(config.pin(Pin::new(r"C:\Tools\App.exe", "", None)));
        assert!(!config.pin(Pin::new(r"c:\tools\APP.exe", "", None)));
        assert!(config.is_pinned(r"C:\TOOLS\app.exe"));
        assert!(config.pin(Pin::new(r"C:\Start\Editor.lnk", r"C:\Tools\Editor.exe", None)));
        assert!(config.is_pinned(r"C:\Tools\Editor.exe"));
        assert!(config.unpin(r"C:\tools\app.exe").is_some());
        assert!(!config.is_pinned(r"C:\Tools\App.exe"));
        assert!(config.unpin(r"C:\Tools\App.exe").is_none());
    }

    #[test]
    fn shell_uris_survive_pruning_but_missing_files_do_not() {
        let pins = vec![
            Pin::new("ms-settings:", "SystemSettings.exe", None),
            Pin::new(r"C:\does\not\exist.exe", "", None),
        ];
        let kept = existing_pins(pins);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].path, "ms-settings:");
    }
}
