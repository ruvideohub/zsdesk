//! Bounded local discovery of Steam Internet shortcuts for Launchpad.
//! Ordinary web shortcuts are not applications and are never included.
use std::{collections::{HashSet, VecDeque}, path::{Path, PathBuf}};

const MAX_ENTRIES_PER_ROOT: usize = 10_000;
const MAX_SHORTCUTS: usize = 2_000;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GameShortcut { pub path: PathBuf, pub name: String, pub appid: String }

/// Roots and their maximum directory depth; roots are supplied by Known Folder
/// APIs on Windows. Never follow symlinks/junctions while walking.
pub(crate) fn scan(roots: &[(PathBuf, usize)], pinned: &[PathBuf]) -> Vec<GameShortcut> {
    let mut games = Vec::new();
    let mut appids = HashSet::new();
    // Explicit local pins are allowed outside the standard folders. Check only
    // a bounded number and never open network paths or non-shortcut files.
    for path in pinned.iter().take(512) {
        let text = path.to_string_lossy();
        if path.is_absolute() && !text.starts_with("\\\\") && !text.starts_with("//") {
            consider(path, &mut games, &mut appids);
        }
    }
    for (root, depth) in roots {
        if games.len() >= MAX_SHORTCUTS { break; }
        let mut queue = VecDeque::from([(root.clone(), 0usize)]);
        let mut remaining = MAX_ENTRIES_PER_ROOT;
        while let Some((directory, level)) = queue.pop_front() {
            if remaining == 0 || games.len() >= MAX_SHORTCUTS { break; }
            if !ordinary_directory(&directory) { continue; }
            let Ok(entries) = std::fs::read_dir(&directory) else { continue };
            // Bound both traversal and temporary allocation, including on huge dirs.
            let mut entries: Vec<_> = entries.take(remaining).filter_map(Result::ok).collect();
            remaining = remaining.saturating_sub(entries.len());
            entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
            for entry in entries {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_dir() && level < *depth && ordinary_directory(&path) {
                    queue.push_back((path, level + 1));
                } else if kind.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("url")) {
                    consider(&path, &mut games, &mut appids);
                    if games.len() >= MAX_SHORTCUTS { break; }
                }
            }
        }
    }
    games
}

fn consider(path: &Path, games: &mut Vec<GameShortcut>, appids: &mut HashSet<String>) {
    let Some(shortcut) = crate::steam_shortcut::read(path) else { return };
    let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(str::trim).filter(|s| !s.is_empty()) else { return };
    if appids.insert(shortcut.appid.clone()) {
        games.push(GameShortcut { path: path.to_path_buf(), name: name.to_owned(), appid: shortcut.appid });
    }
}

fn ordinary_directory(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else { return false };
    if !metadata.is_dir() || metadata.file_type().is_symlink() { return false; }
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        // Includes junctions and other reparse points, not just symbolic links.
        if metadata.file_attributes() & 0x400 != 0 { return false; }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SERIAL: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("aura-steam-discovery-{}-{}", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&p).unwrap(); Self(p)
        }
        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path); std::fs::create_dir_all(path.parent().unwrap()).unwrap(); std::fs::write(path, text).unwrap();
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    fn steam(id: &str) -> String { format!("[InternetShortcut]\nURL=steam://rungameid/{id}\n") }

    #[test]
    fn only_real_steam_games_are_included_and_duplicate_appids_coalesce() {
        let f = Fixture::new();
        f.write("desktop/Portal.url", &steam("400"));
        f.write("desktop/Web.url", "[InternetShortcut]\nURL=https://example.com\n");
        f.write("desktop/Invalid.url", &steam("../bad"));
        f.write("programs/Steam/Portal again.url", &steam("400"));
        f.write("programs/Steam/Игра.url", &steam("570"));
        f.write("programs/Steam/not-a-shortcut.txt", &steam("123"));
        let games = scan(&[(f.0.join("desktop"), 0), (f.0.join("programs"), 6)], &[]);
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].appid, "400");
        assert_eq!(games[0].name, "Portal");
        assert_eq!(games[1].name, "Игра");
        assert_eq!(games[1].appid, "570");
    }

    #[test]
    fn local_icon_is_selected_only_for_a_safe_existing_ico() {
        let f = Fixture::new();
        f.write("game.ico", "icon fixture");
        let mut shortcut = crate::steam_shortcut::parse(&format!("[InternetShortcut]\nURL=steam://run/400\nIconFile={}\nIconIndex=0\n", f.0.join("game.ico").display())).unwrap();
        assert_eq!(crate::steam_shortcut::local_icon(&shortcut), Some(f.0.join("game.ico").as_path()));
        shortcut.icon_index = 1;
        assert_eq!(crate::steam_shortcut::local_icon(&shortcut), None);
        shortcut.icon_index = 0;
        shortcut.icon_file = Some(f.0.join("missing.ico"));
        assert_eq!(crate::steam_shortcut::local_icon(&shortcut), None);
    }

    #[test]
    fn explicit_local_pin_wins_over_a_duplicate_in_start_menu() {
        let f = Fixture::new();
        f.write("custom/My game.url", &steam("400"));
        f.write("programs/Game.url", &steam("400"));
        f.write("custom/Web.url", "[InternetShortcut]\nURL=https://example.com\n");
        let games = scan(&[(f.0.join("programs"), 2)], &[f.0.join("custom/My game.url"), f.0.join("custom/Web.url"), PathBuf::from("relative.url")]);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].path, f.0.join("custom/My game.url"));
    }

    #[test]
    fn depth_and_invalid_roots_are_respected() {
        let f = Fixture::new();
        f.write("root/Top.url", &steam("1"));
        f.write("root/nested/Nested.url", &steam("2"));
        let games = scan(&[(f.0.join("missing"), 5), (f.0.join("root"), 0)], &[]);
        assert_eq!(games.iter().map(|g| g.appid.as_str()).collect::<Vec<_>>(), vec!["1"]);
        let games = scan(&[(f.0.join("root"), 1)], &[]);
        assert_eq!(games.len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn directory_and_shortcut_symlinks_are_not_followed() {
        let f = Fixture::new();
        f.write("outside/Game.url", &steam("1"));
        std::fs::create_dir_all(f.0.join("root")).unwrap();
        std::os::unix::fs::symlink(f.0.join("outside"), f.0.join("root/linked-dir")).unwrap();
        std::os::unix::fs::symlink(f.0.join("outside/Game.url"), f.0.join("root/Game.url")).unwrap();
        assert!(scan(&[(f.0.join("root"), 6)], &[]).is_empty());
    }
}
