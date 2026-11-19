//! Pure dock content model: which windows belong to which icon.
//!
//! `winlist.rs` collects the facts about every top-level window, then this
//! module groups them into application entries and merges them with the pinned
//! list. Kept free of Win32 types (`HWND` travels as `isize`) so the grouping
//! and filtering rules are unit-testable.

use crate::config::Pin;

/// A top-level application window as the dock sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WinInfo {
    pub hwnd: isize,
    /// Full path of the owning executable.
    pub exe: String,
    pub title: String,
    pub minimized: bool,
    pub foreground: bool,
}

/// Windows of one executable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub exe: String,
    pub label: String,
    pub windows: Vec<WinInfo>,
}

/// What an icon stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A pinned or running application.
    App,
    /// The all-applications grid.
    Launcher,
    /// The recycle bin.
    Trash,
}

/// One dock icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub kind: Kind,
    /// Stable icon-cache key (normalized executable path or a shell id).
    pub key: String,
    pub label: String,
    /// Executable path, `.lnk` path or `shell:`/`ms-settings:` URI.
    pub target: String,
    pub pinned: bool,
    pub windows: Vec<WinInfo>,
}

impl Slot {
    pub fn is_running(&self) -> bool { !self.windows.is_empty() }

}

/// Comparison key for executable paths: case-insensitive, without the extended
/// prefix, with forward slashes folded.
pub fn normalize_exe(path: &str) -> String {
    let trimmed = path.trim().trim_start_matches(r"\\?\");
    trimmed.replace('/', "\\").to_lowercase()
}

/// Display name derived from a path: file stem without the extension.
pub fn label_from_path(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['\\', '/']);
    let file = trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed);
    let stem = match file.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => stem,
        _ => file,
    };
    let stem = stem.replace('_', " ");
    let cleaned = stem.trim();
    if cleaned.is_empty() { path.to_string() } else { cleaned.to_string() }
}

/// Windows that should have their own dock icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowFacts {
    pub own_process: bool,
    pub visible: bool,
    pub tool_window: bool,
    pub cloaked: bool,
    pub app_window: bool,
    pub has_title: bool,
    pub has_owner: bool,
}

/// Shell surfaces that are never application windows.
pub fn is_shell_class(class: &str) -> bool {
    matches!(class,
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "#32768"
        | "Windows.UI.Core.CoreWindow" | "ApplicationManager_DesktopShellWindow"
        | "MultitaskingViewFrame" | "ForegroundStaging" | "XamlExplorerHostIslandWindow"
        | "SysShadow" | "TaskListThumbnailWnd")
}

/// Whether a window becomes a dock icon.
pub fn is_application_window(facts: WindowFacts, class: &str) -> bool {
    if facts.own_process || !facts.visible || facts.cloaked { return false; }
    if is_shell_class(class) { return false; }
    if facts.tool_window && !facts.app_window { return false; }
    if !facts.has_title { return false; }
    // Owned windows are dialogs of another application icon.
    if facts.has_owner && !facts.app_window { return false; }
    true
}

/// Groups windows by executable and orders them for display.
pub fn group_by_exe(windows: Vec<WinInfo>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for window in windows {
        let key = normalize_exe(&window.exe);
        match groups.iter_mut().find(|group| normalize_exe(&group.exe) == key) {
            Some(group) => group.windows.push(window),
            None => groups.push(Group {
                exe: window.exe.clone(),
                label: label_from_path(&window.exe),
                windows: vec![window],
            }),
        }
    }
    for group in &mut groups {
        // The foreground window is the one a click should raise first.
        group.windows.sort_by(|left, right| {
            right.foreground.cmp(&left.foreground)
                .then(left.minimized.cmp(&right.minimized))
                .then(left.title.to_lowercase().cmp(&right.title.to_lowercase()))
        });
    }
    groups.sort_by(|left, right| left.label.to_lowercase().cmp(&right.label.to_lowercase()));
    groups
}

/// Builds the icon row: pins in their configured order, then applications that
/// are running without a pin, then the fixed trailing icons.
pub fn build_slots(pins: &[Pin], mut groups: Vec<Group>, show_launcher: bool, show_trash: bool) -> Vec<Slot> {
    let mut slots: Vec<Slot> = Vec::with_capacity(pins.len() + groups.len() + 2);
    for pin in pins {
        let key = normalize_exe(&pin.target);
        let index = groups.iter().position(|group| normalize_exe(&group.exe) == key
            || normalize_exe(&group.exe) == normalize_exe(&pin.path));
        let windows = match index {
            Some(index) => groups.remove(index).windows,
            None => Vec::new(),
        };
        let label = match pin.label.as_deref().map(str::trim) {
            Some(label) if !label.is_empty() => label.to_string(),
            _ => label_from_path(&pin.path),
        };
        slots.push(Slot { kind: Kind::App, key, label, target: pin.path.clone(), pinned: true, windows });
    }
    for group in groups {
        slots.push(Slot {
            kind: Kind::App,
            key: normalize_exe(&group.exe),
            label: group.label,
            target: group.exe,
            pinned: false,
            windows: group.windows,
        });
    }
    if show_launcher {
        slots.push(Slot { kind: Kind::Launcher, key: "dock:launcher".into(), label: String::new(),
            target: String::new(), pinned: false, windows: Vec::new() });
    }
    if show_trash {
        slots.push(Slot { kind: Kind::Trash, key: "dock:trash".into(), label: String::new(),
            target: String::new(), pinned: false, windows: Vec::new() });
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(exe: &str, title: &str, foreground: bool, minimized: bool) -> WinInfo {
        WinInfo { hwnd: 1, exe: exe.into(), title: title.into(), minimized, foreground }
    }

    fn pin(path: &str, label: Option<&str>) -> Pin {
        Pin { path: path.into(), target: path.into(), label: label.map(str::to_string) }
    }

    #[test]
    fn paths_are_compared_without_case_or_extended_prefixes() {
        assert_eq!(normalize_exe(r"\\?\C:\Windows\NOTEPAD.EXE"), normalize_exe(r"c:\windows\notepad.exe"));
        assert_eq!(normalize_exe("C:/Tools/App.exe"), normalize_exe(r"c:\tools\app.exe"));
        assert_eq!(label_from_path(r"C:\Program Files\My_App\my-app.exe"), "my-app");
        assert_eq!(label_from_path(r"C:\Program Files\My_App"), "My App");
        assert_eq!(label_from_path("ms-settings:"), "ms-settings:");
    }

    #[test]
    fn window_filter_skips_shell_surfaces_dialogs_and_tool_windows() {
        let base = WindowFacts { own_process: false, visible: true, tool_window: false, cloaked: false,
            app_window: false, has_title: true, has_owner: false };
        assert!(is_application_window(base, "Notepad"));
        assert!(!is_application_window(WindowFacts { own_process: true, ..base }, "Notepad"));
        assert!(!is_application_window(WindowFacts { visible: false, ..base }, "Notepad"));
        assert!(!is_application_window(WindowFacts { cloaked: true, ..base }, "Notepad"));
        assert!(!is_application_window(WindowFacts { tool_window: true, ..base }, "Notepad"));
        assert!(is_application_window(WindowFacts { tool_window: true, app_window: true, ..base }, "Notepad"));
        assert!(!is_application_window(WindowFacts { has_title: false, ..base }, "Notepad"));
        assert!(!is_application_window(WindowFacts { has_owner: true, ..base }, "Notepad"));
        assert!(is_application_window(WindowFacts { has_owner: true, app_window: true, ..base }, "Notepad"));
        for class in ["Progman", "WorkerW", "Shell_TrayWnd", "#32768"] {
            assert!(!is_application_window(base, class), "{class}");
        }
    }

    #[test]
    fn windows_are_grouped_by_executable_and_foreground_first() {
        let groups = group_by_exe(vec![
            window(r"C:\a\Editor.exe", "second.txt", false, false),
            window(r"C:\b\Browser.exe", "Site", true, false),
            window(r"c:\A\editor.EXE", "first.txt", false, true),
        ]);
        assert_eq!(groups.len(), 2);
        let editor = groups.iter().find(|group| group.label == "Editor").unwrap();
        assert_eq!(editor.windows.len(), 2);
        assert_eq!(editor.windows[0].title, "second.txt");
        assert_eq!(groups[0].label, "Browser");
    }

    #[test]
    fn pins_keep_their_order_and_absorb_running_windows() {
        let pins = vec![pin(r"C:\Windows\notepad.exe", None), pin(r"C:\Tools\Editor.exe", Some("Редактор"))];
        let groups = group_by_exe(vec![
            window(r"C:\Windows\NOTEPAD.exe", "Безымянный", true, false),
            window(r"C:\Other\Mail.exe", "Входящие", false, false),
        ]);
        let slots = build_slots(&pins, groups, true, true);
        assert_eq!(slots.len(), 5);
        assert_eq!(slots[0].label, "notepad");
        assert!(slots[0].pinned);
        assert_eq!(slots[0].windows.len(), 1);
        assert_eq!(slots[1].label, "Редактор");
        assert!(slots[1].windows.is_empty());
        assert_eq!(slots[2].label, "Mail");
        assert!(!slots[2].pinned);
        assert_eq!(slots[3].kind, Kind::Launcher);
        assert_eq!(slots[4].kind, Kind::Trash);
    }

    #[test]
    fn trailing_icons_are_optional() {
        let slots = build_slots(&[pin(r"C:\a.exe", None)], Vec::new(), false, false);
        assert_eq!(slots.len(), 1);
        assert!(slots.iter().all(|slot| slot.kind == Kind::App));
    }
}
