//! The dock's settings file: `%APPDATA%\AuraExplorer\Dock\config.json`.
//!
//! This is the same idea as the prototype's settings object. Until the native
//! Settings window exists, edit the file (right-click the dock, choose
//! "Edit settings file…"); the dock notices the save and applies it live.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Marks a separator in `pins`.
pub const SEPARATOR: &str = "|";

/// One entry in the dock: an app id, a separator ("|"), or a stack of apps.
/// In config.json: `"MSEdge"`, `"|"`, or `{ "stack": "Creative", "apps": ["...", "..."] }`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Pin {
    Id(String),
    Stack { stack: String, apps: Vec<String> },
    /// A widget in the dock: `{ "widget": "clock" }` or `{ "widget": "clock", "size": "small" }`
    /// (ids in widgets/catalog.rs).
    Widget {
        widget: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<crate::widgets::catalog::Size>,
    },
}

/// A widget in a Launchpad column: `"clock"` or `{ "id": "clock", "size": "small" }`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum WRef {
    Id(String),
    Sized { id: String, size: crate::widgets::catalog::Size },
}

impl WRef {
    pub fn new(id: &str, size: crate::widgets::catalog::Size) -> Self {
        // Keep the short form when the size is the widget's own default.
        match crate::widgets::catalog::get(id) {
            Some(d) if d.size == size => WRef::Id(id.to_string()),
            _ => WRef::Sized { id: id.to_string(), size },
        }
    }
    pub fn id(&self) -> &str {
        match self {
            WRef::Id(s) | WRef::Sized { id: s, .. } => s,
        }
    }
    pub fn size(&self) -> crate::widgets::catalog::Size {
        match self {
            WRef::Sized { size, .. } => *size,
            WRef::Id(id) => crate::widgets::catalog::get(id).map(|d| d.size).unwrap_or_default(),
        }
    }
}

/// Which corner of the monitor the desktop clock card sits in.
///
/// In config.json: `"top-right"`, `"bottom-left"` and so on.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ClockCorner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ClockCorner {
    /// The order the card's menu lists the corners in.
    pub const ALL: [ClockCorner; 4] =
        [ClockCorner::TopLeft, ClockCorner::TopRight, ClockCorner::BottomLeft, ClockCorner::BottomRight];

    pub fn label(self) -> &'static str {
        match self {
            ClockCorner::TopLeft => "Top left",
            ClockCorner::TopRight => "Top right",
            ClockCorner::BottomLeft => "Bottom left",
            ClockCorner::BottomRight => "Bottom right",
        }
    }

    /// A number the window procedure can put in a message, and back.
    pub fn code(self) -> usize {
        match self {
            ClockCorner::TopLeft => 0,
            ClockCorner::TopRight => 1,
            ClockCorner::BottomLeft => 2,
            ClockCorner::BottomRight => 3,
        }
    }

    pub fn from_code(code: usize) -> Self {
        *Self::ALL.get(code).unwrap_or(&ClockCorner::TopRight)
    }
}

impl Pin {
    pub fn sep() -> Self {
        Pin::Id(SEPARATOR.to_string())
    }
    pub fn is_sep(&self) -> bool {
        matches!(self, Pin::Id(s) if s == SEPARATOR)
    }
    /// The app id, for plain app pins.
    pub fn app(&self) -> Option<&str> {
        match self {
            Pin::Id(s) if s != SEPARATOR => Some(s),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    // Dock appearance (device-independent pixels)
    pub icon_size: f32,
    pub magnify: bool,
    pub magnified_size: f32,
    /// How many icons either side grow, roughly.
    pub magnify_range: f32,
    pub icon_spacing: f32,
    pub padding: f32,
    pub corner_radius: f32,
    /// 0–0.8 opacity of the dark tint over the blurred desktop.
    pub glass_tint: f32,
    /// 0–1 strength of the specular rim.
    pub edge_highlight: f32,
    /// Light glass when Windows' apps use light mode, dark glass when they use dark mode.
    /// Off: always dark.
    pub match_windows_theme: bool,
    /// Extra blur (pixels) on top of Windows' own background blur. 0: Windows' blur only.
    pub extra_blur: f32,

    // Install and updates (installed copies only; see updater.rs)
    /// Start Laminar when the user signs in to Windows.
    pub start_at_login: bool,
    /// Check GitHub for new versions and download them quietly.
    pub check_updates: bool,
    /// How often the automatic check runs, in hours: 0 = only when Laminar starts,
    /// 24 = once a day (default), 168 = once a week.
    pub update_check_hours: u32,
    /// Unix time (seconds) of the last automatic check, so restarts don't check again.
    pub last_update_check: u64,
    /// Unix time (seconds) before which the update icon stays hidden ("Remind me later").
    pub update_snooze_until: u64,
    /// A version the user chose to skip.
    pub update_skip: String,

    // Behaviour
    /// Slide the dock off-screen; bring it back by pushing the pointer into the bottom edge.
    pub auto_hide: bool,
    /// Height in pixels of the bottom-edge band that reveals a hidden dock.
    pub reveal_zone: f32,
    pub reveal_delay_ms: u32,
    pub hide_delay_ms: u32,
    /// Hide the Windows taskbar while Laminar runs (a watchdog restores it on exit or crash).
    pub hide_windows_taskbar: bool,
    /// Hide the panel while a full-screen app (a game, a video) is in front.
    pub hide_fullscreen: bool,
    /// Clicking the app that is already in front minimizes it (Windows behaviour).
    pub click_to_minimize: bool,
    /// Pressing the Windows key on its own opens Launchpad instead of Start.
    pub win_key_launchpad: bool,
    /// Show a dock on every display (otherwise only the main display).
    pub show_on_all_displays: bool,
    /// Each display's dock lists only the apps with windows on that display.
    pub per_display_windows: bool,
    /// Apps shown under "Pinned" in Launchpad (empty: use the dock's pinned apps).
    pub launchpad_pins: Vec<String>,
    /// Weather location (default Toronto).
    pub weather_city: String,
    pub weather_lat: f32,
    pub weather_lon: f32,
    /// Widgets in Launchpad's left column, top to bottom (ids in widgets/catalog.rs).
    pub launchpad_widgets: Vec<WRef>,
    /// Widgets in Launchpad's right column (Control Center), top to bottom.
    pub right_widgets: Vec<WRef>,
    /// Always-on widgets (turn off here to remove them).
    pub show_background_apps: bool,
    pub show_windows_update: bool,
    pub show_windows_settings: bool,
    /// 0–1 strength of the shadow under Launchpad's panels.
    pub panel_shadow: f32,
    /// 0–1 strength of the shadow under widgets and tiles.
    pub card_shadow: f32,
    /// Widget and connector settings that aren't secret, by connector id
    /// (e.g. "github": {"username": "…"}). API keys live in Windows Credential Manager.
    pub widget_settings: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,

    /// A clock card in the corner of the desktop, macOS style.
    pub desktop_clock: bool,

    /// Which corner the clock card sits in (right-click the card to change it).
    pub clock_corner: ClockCorner,
    /// Size multiplier for the clock card: 0.85 compact, 1.0 normal, 1.2 large.
    pub clock_scale: f32,
    /// 12-hour time with AM/PM instead of 24-hour.
    pub clock_12h: bool,
    /// The weekday and date under the time.
    pub clock_date: bool,
    /// Seconds in the time; the card then repaints every second.
    pub clock_seconds: bool,

    /// The panel's own language: "ru" (default), "en", or "auto" for the Windows
    /// interface language. See strings.rs.
    pub language: String,

    /// Pinned apps, left to right. Each entry is an AppUserModelID (as used by
    /// `shell:AppsFolder`) or a full .exe path; `"|"` is a separator; an object is a stack.
    pub pins: Vec<Pin>,

    /// Where the desktop clock card was dragged to (physical px on its monitor). `None`
    /// means "a corner", which is what `clock_corner` says.
    #[serde(default)]
    pub clock_pos: Option<[i32; 2]>,

    /// The desktop clock stays where it is: dragging does nothing.
    #[serde(default)]
    pub clock_locked: bool,

    /// The control centre button was put on the panel once. After that it's the user's
    /// call: drop it from the widget menu and it stays dropped.
    #[serde(default)]
    pub cc_pin_added: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            icon_size: 54.0,
            magnify: true,
            magnified_size: 88.0,
            magnify_range: 2.6,
            icon_spacing: 8.0,
            padding: 8.0,
            corner_radius: 24.0,
            glass_tint: 0.24,
            edge_highlight: 0.55,
            match_windows_theme: true,
            extra_blur: 0.0,
            // Off until the user turns it on (see updater::sync_start_at_login).
            start_at_login: false,
            check_updates: false,
            update_check_hours: 24,
            last_update_check: 0,
            update_snooze_until: 0,
            update_skip: String::new(),
            auto_hide: false,
            reveal_zone: 4.0,
            reveal_delay_ms: 120,
            hide_delay_ms: 450,
            hide_windows_taskbar: true,
            hide_fullscreen: true,
            click_to_minimize: true,
            win_key_launchpad: true,
            show_on_all_displays: true,
            per_display_windows: false,
            launchpad_pins: Vec::new(),
            weather_city: "Toronto".into(),
            weather_lat: 43.65,
            weather_lon: -79.38,
            launchpad_widgets: crate::widgets::catalog::default_launchpad(),
            right_widgets: crate::widgets::catalog::default_right(),
            show_background_apps: true,
            show_windows_update: true,
            show_windows_settings: true,
            panel_shadow: 0.42,
            card_shadow: 0.35,
            widget_settings: Default::default(),
            language: "ru".into(),
            desktop_clock: true,
            clock_corner: ClockCorner::TopRight,
            clock_scale: 1.0,
            clock_12h: false,
            clock_date: true,
            clock_seconds: false,
            clock_pos: None,
            clock_locked: false,
            cc_pin_added: false,
            pins: Vec::new(),
        }
    }
}

pub fn dir() -> PathBuf {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(migrate_legacy_dir);
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("AuraUI").join("Dock")
}

/// The panel used to keep its settings in `%APPDATA%\ZSDesk\Dock`. Copy them over once:
/// a rename shouldn't look like a fresh install (pins, widget placement, brightness).
fn migrate_legacy_dir() {
    let Some(base) = std::env::var_os("APPDATA").map(PathBuf::from) else { return };
    let new = base.join("AuraUI").join("Dock");
    let old = base.join("ZSDesk").join("Dock");
    if new.exists() || !old.is_dir() {
        return;
    }
    let _ = std::fs::create_dir_all(&new);
    let Ok(entries) = std::fs::read_dir(&old) else { return };
    for entry in entries.flatten() {
        if entry.path().is_file() {
            let _ = std::fs::copy(entry.path(), new.join(entry.file_name()));
        }
    }
}

pub fn path() -> PathBuf {
    dir().join("config.json")
}

impl Config {
    /// Load the settings file. Returns None if it doesn't exist or can't be parsed;
    /// a running dock then keeps its current settings until the file is fixed.
    /// (Never write anything here: the dock watches this folder, so a write would retrigger a reload.)
    pub fn load() -> Option<Config> {
        let text = std::fs::read_to_string(path()).ok()?;
        // Some editors (and PowerShell) save UTF-8 with a byte-order mark.
        let text = text.trim_start_matches('\u{feff}');
        serde_json::from_str::<Config>(text).ok().map(Config::sanitized)
    }

    /// Panels saved before the control centre had a button of its own get one, once.
    /// Returns true when the file should be written back.
    pub fn ensure_control_center_pin(&mut self) -> bool {
        if self.cc_pin_added {
            return false;
        }
        self.cc_pin_added = true;
        if !self.pins.iter().any(|p| matches!(p, Pin::Widget { widget, .. } if widget == "control-center")) {
            // In front of the other widgets, behind the apps: a system button's place.
            let at = self.pins.iter().position(|p| matches!(p, Pin::Widget { .. })).unwrap_or(self.pins.len());
            self.pins.insert(at, Pin::Widget { widget: "control-center".into(), size: None });
        }
        true
    }

    /// True if a settings file exists (even if it currently fails to parse).
    pub fn exists() -> bool {
        path().exists()
    }

    /// Clamp values so a typo in the file can't break the dock.
    pub fn sanitized(mut self) -> Self {
        self.icon_size = self.icon_size.clamp(24.0, 128.0);
        self.magnified_size = self.magnified_size.clamp(self.icon_size, 192.0);
        self.magnify_range = self.magnify_range.clamp(1.0, 8.0);
        self.icon_spacing = self.icon_spacing.clamp(0.0, 32.0);
        self.padding = self.padding.clamp(2.0, 24.0);
        self.corner_radius = self.corner_radius.clamp(0.0, 64.0);
        self.glass_tint = self.glass_tint.clamp(0.0, 0.9);
        self.edge_highlight = self.edge_highlight.clamp(0.0, 1.0);
        self.reveal_zone = self.reveal_zone.clamp(1.0, 40.0);
        self.extra_blur = self.extra_blur.clamp(0.0, 40.0);
        // Forget widgets this version doesn't know (e.g. after a downgrade).
        self.launchpad_widgets.retain(|w| crate::widgets::catalog::get(w.id()).is_some());
        self.right_widgets.retain(|w| crate::widgets::catalog::get(w.id()).is_some());
        self.pins.retain(|p| !matches!(p, Pin::Widget { widget, .. } if crate::widgets::catalog::get(widget).is_none()));
        // Always-on widgets: present (in either column) when switched on, gone when off.
        for (key, id) in crate::widgets::catalog::PINNED {
            let on = match *key {
                "show_background_apps" => self.show_background_apps,
                "show_windows_update" => self.show_windows_update,
                _ => self.show_windows_settings,
            };
            let present = self.launchpad_widgets.iter().chain(self.right_widgets.iter()).any(|w| w.id() == *id);
            if on && !present {
                self.right_widgets.push(WRef::Id(id.to_string()));
            } else if !on {
                self.launchpad_widgets.retain(|w| w.id() != *id);
                self.right_widgets.retain(|w| w.id() != *id);
            }
        }
        self.clock_scale = self.clock_scale.clamp(0.75, 1.5);
        self.panel_shadow = self.panel_shadow.clamp(0.0, 1.0);
        self.card_shadow = self.card_shadow.clamp(0.0, 1.0);
        // Pins saved before version-folder handling point into a folder an update deletes.
        // (In memory only: the next save writes the stable path.)
        let fix = |s: &mut String| {
            if crate::apps::is_path(s) {
                *s = crate::apps::stable_path(s).to_lowercase();
            }
        };
        for p in &mut self.pins {
            match p {
                Pin::Id(s) => fix(s),
                Pin::Stack { apps, .. } => apps.iter_mut().for_each(fix),
                Pin::Widget { .. } => {}
            }
        }
        self
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(dir());
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path(), text);
        }
    }

    /// True if anything that changes the dock's layout or look differs.
    pub fn visuals_differ(&self, o: &Config) -> bool {
        self.icon_size != o.icon_size
            || self.magnify != o.magnify
            || self.magnified_size != o.magnified_size
            || self.magnify_range != o.magnify_range
            || self.icon_spacing != o.icon_spacing
            || self.padding != o.padding
            || self.corner_radius != o.corner_radius
            || self.glass_tint != o.glass_tint
            || self.edge_highlight != o.edge_highlight
            || self.match_windows_theme != o.match_windows_theme
            || self.extra_blur != o.extra_blur
            || self.pins != o.pins
            || self.auto_hide != o.auto_hide
            || self.per_display_windows != o.per_display_windows
            || self.card_shadow != o.card_shadow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::catalog::Size;

    #[test]
    fn widget_entries_old_and_new() {
        let c: Config = serde_json::from_str(r#"{"launchpad_widgets": ["clock", {"id": "analog", "size": "large"}], "pins": ["|", {"widget": "clock"}, {"widget": "weather", "size": "small"}]}"#).unwrap();
        assert_eq!(c.launchpad_widgets[0].size(), Size::Medium);
        assert_eq!(c.launchpad_widgets[1].size(), Size::Large);
        assert!(matches!(&c.pins[1], Pin::Widget { widget, size: None } if widget == "clock"));
        assert!(matches!(&c.pins[2], Pin::Widget { size: Some(Size::Small), .. }));
    }

    #[test]
    fn always_on_widgets_follow_their_switches() {
        let mut c = Config { right_widgets: Vec::new(), ..Config::default() };
        c = c.sanitized();
        assert!(c.right_widgets.iter().any(|w| w.id() == "background-apps"));
        c.show_windows_update = false;
        c = c.sanitized();
        assert!(!c.right_widgets.iter().any(|w| w.id() == "windows-update"));
        // A widget id this version doesn't know is dropped instead of breaking the column.
        c.launchpad_widgets.push(WRef::Id("no-such-widget".into()));
        c = c.sanitized();
        assert!(!c.launchpad_widgets.iter().any(|w| w.id() == "no-such-widget"));
    }
}
