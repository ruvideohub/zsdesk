//! Profile location, resolved exactly like the file manager does it:
//!
//! 1. `--profile-dir` on the command line (used when the dock is started for a
//!    specific profile, e.g. from the ZSDesk menu),
//! 2. `portable.flag` next to `ZSDesk.exe` → `data/`,
//! 3. `profile-location.json` in the profile home → the user-selected folder,
//! 4. the profile home itself (`%APPDATA%\<profile>`).
//!
//! The resolution itself is pure so the precedence rules are unit-tested.

use std::path::{Path, PathBuf};

/// Name of the dock's settings file inside the profile folder.
pub const CONFIG_FILE: &str = "dock.json";

/// Folder that holds the profile when the user did not move it.
pub fn profile_home() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ZSDesk")
}

/// `data/` next to the executable, when `portable.flag` is present.
pub fn portable_directory() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    directory.join("portable.flag").is_file().then(|| directory.join("data"))
}

/// Applies the precedence rules above to already-read inputs.
pub fn resolve(home: &Path, portable: Option<PathBuf>, selected: Option<PathBuf>) -> PathBuf {
    portable.or(selected).unwrap_or_else(|| home.to_path_buf())
}

/// Reads `profile-location.json`; a relative or unparsable value is ignored.
pub fn selected_directory(home: &Path) -> Option<PathBuf> {
    let path = home.join("profile-location.json");
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.len() > 128 * 1024 { return None; }
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice::<PathBuf>(&bytes).ok().filter(|directory| directory.is_absolute())
}

/// Full path of `dock.json` for the current process.
pub fn config_path(override_directory: Option<PathBuf>) -> PathBuf {
    if let Some(directory) = override_directory.filter(|directory| directory.is_absolute()) {
        return directory.join(CONFIG_FILE);
    }
    let home = profile_home();
    let selected = selected_directory(&home);
    resolve(&home, portable_directory(), selected).join(CONFIG_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_and_selected_directories_win_over_the_home() {
        let home = Path::new(r"C:\Users\me\AppData\Roaming\ZSDesk");
        assert_eq!(resolve(home, None, None), home.to_path_buf());
        assert_eq!(resolve(home, None, Some(r"D:\Profile".into())), PathBuf::from(r"D:\Profile"));
        assert_eq!(resolve(home, Some(r"E:\portable\data".into()), Some(r"D:\Profile".into())),
            PathBuf::from(r"E:\portable\data"));
    }

    #[test]
    fn the_config_file_keeps_its_name() {
        let config = Path::new(r"D:\Profile").join(CONFIG_FILE);
        assert_eq!(config.file_name().unwrap(), "dock.json");
    }
}
