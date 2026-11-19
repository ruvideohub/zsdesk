//! Read-only catalogue of installed applications, built from Start Menu
//! shortcuts. Plain file-system code, so it is testable without Windows.

use std::path::{Path, PathBuf};

/// One entry of the all-applications grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    /// Display name (shortcut file stem).
    pub name: String,
    /// Shortcut or executable passed to the Shell.
    pub path: String,
}

/// Where Windows keeps the Start Menu shortcuts.
pub fn start_menu_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        directories.push(PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Some(programdata) = std::env::var_os("PROGRAMDATA") {
        directories.push(PathBuf::from(programdata).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if directories.is_empty() {
        // Non-Windows hosts (tests, cross-compilation) still get a valid list.
        directories.push(PathBuf::from("/usr/share/applications"));
    }
    directories
}

/// Recursively collects `.lnk` and `.exe` entries, sorted by name and
/// de-duplicated case-insensitively.
pub fn scan(directories: &[PathBuf]) -> Vec<AppEntry> {
    let mut entries = Vec::new();
    for directory in directories { walk(directory, 0, &mut entries); }
    entries.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    let mut seen = std::collections::HashSet::new();
    entries.retain(|entry| seen.insert(entry.name.to_lowercase()));
    entries
}

fn walk(directory: &Path, depth: usize, entries: &mut Vec<AppEntry>) {
    if depth > 4 || entries.len() > 4000 { return; }
    let Ok(listing) = std::fs::read_dir(directory) else { return };
    for entry in listing.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.contains("uninstall") || name == "startup" { continue; }
            walk(&path, depth + 1, entries);
            continue;
        }
        if !kind.is_file() { continue; }
        let extension = path.extension().map(|value| value.to_string_lossy().to_lowercase()).unwrap_or_default();
        if extension != "lnk" && extension != "exe" { continue; }
        let Some(stem) = path.file_stem().map(|value| value.to_string_lossy().to_string()) else { continue };
        let name = stem.trim().to_string();
        if name.is_empty() || name.to_lowercase().starts_with("uninstall") { continue; }
        entries.push(AppEntry { name, path: path.to_string_lossy().to_string() });
    }
}

/// Filters a catalogue by a whitespace-separated, case-insensitive query.
pub fn filter(entries: &[AppEntry], query: &str) -> Vec<usize> {
    let terms: Vec<String> = query.split_whitespace().map(|term| term.to_lowercase()).collect();
    entries.iter().enumerate()
        .filter(|(_, entry)| {
            let name = entry.name.to_lowercase();
            terms.iter().all(|term| name.contains(term))
        })
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("zsdesk-dock-catalogue-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn scanning_walks_subfolders_sorts_and_deduplicates() {
        let root = temp_dir("scan");
        std::fs::create_dir_all(root.join("Accessories")).unwrap();
        std::fs::create_dir_all(root.join("Uninstall")).unwrap();
        std::fs::write(root.join("Zoom.lnk"), b"").unwrap();
        std::fs::write(root.join("Accessories/Notepad.lnk"), b"").unwrap();
        std::fs::write(root.join("Accessories/notepad.LNK"), b"").unwrap();
        std::fs::write(root.join("Uninstall/Uninstall Zoom.lnk"), b"").unwrap();
        std::fs::write(root.join("notes.txt"), b"").unwrap();
        let entries = scan(&[root.clone()]);
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        // Windows treats the two spellings as one file and keeps whichever case
        // the filesystem stored, so the assertion may not pin the casing: what
        // matters is that one entry survives, deduplicated and without the
        // uninstall shortcut.
        assert_eq!(names.len(), 2, "{names:?}");
        assert_eq!(names.iter().filter(|name| name.eq_ignore_ascii_case("notepad")).count(), 1, "{names:?}");
        assert!(names.iter().any(|name| *name == "Zoom"), "{names:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        assert!(scan(&[PathBuf::from(r"Z:\definitely\missing")]).is_empty());
    }

    #[test]
    fn deep_trees_are_bounded() {
        let root = temp_dir("deep");
        let mut current = root.clone();
        for level in 0..8 {
            current = current.join(format!("level{level}"));
            std::fs::create_dir_all(&current).unwrap();
        }
        std::fs::write(current.join("Too.deep.lnk"), b"").unwrap();
        std::fs::write(root.join("Top.lnk"), b"").unwrap();
        let entries = scan(&[root.clone()]);
        assert_eq!(entries.len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filtering_matches_every_term() {
        let entries = vec![
            AppEntry { name: "Visual Studio Code".into(), path: "a.lnk".into() },
            AppEntry { name: "Paint".into(), path: "b.lnk".into() },
        ];
        assert_eq!(filter(&entries, ""), vec![0, 1]);
        assert_eq!(filter(&entries, "code"), vec![0]);
        assert_eq!(filter(&entries, "VISUAL code"), vec![0]);
        assert!(filter(&entries, "code paint").is_empty());
    }
}
