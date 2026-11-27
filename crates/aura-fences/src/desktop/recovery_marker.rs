//! Separate, bounded intent marker for restoring Explorer icons after a crash.
//!
//! Callers hold the profile recovery lock. Arm only after verifying that the
//! icons are visible, and remove only after verifying that they are visible
//! again. The profile itself may be damaged or unavailable during recovery.
use std::{fs::{self, File}, io::{self, Read}, path::{Path, PathBuf}};

const MARKER: &[u8] = b"AuraFences restore previously visible desktop icons v1\n";

fn marker_path(config: &Path) -> PathBuf { config.with_extension("recovery") }

pub(super) fn is_armed(config: &Path) -> io::Result<bool> {
    let path = marker_path(config);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return match fs::symlink_metadata(&path) {
                Ok(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "recovery marker cannot be opened")),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e),
            };
        }
        Err(error) => return Err(error),
    };
    let mut data = Vec::new();
    file.take(1024).read_to_end(&mut data)?;
    if data != MARKER { return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid recovery marker")); }
    Ok(true)
}

pub(super) fn arm(config: &Path) -> io::Result<()> {
    if is_armed(config)? { return Err(io::Error::new(io::ErrorKind::AlreadyExists, "recovery marker is already armed")); }
    aura_core::atomic_file::write(&marker_path(config), MARKER)?;
    if !is_armed(config)? { return Err(io::Error::other("recovery marker did not persist")); }
    Ok(())
}

pub(super) fn clear(config: &Path) -> io::Result<()> {
    if !is_armed(config)? { return Ok(()); }
    match fs::remove_file(marker_path(config)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Recovery is intentionally separate from the profile JSON. The callbacks
/// let us exercise unavailable Shell, failed restore and already-visible icons
/// without changing a real user's desktop in unit tests.
pub(super) fn recover_with(
    config: &Path,
    mut visible: impl FnMut() -> Option<bool>,
    mut show: impl FnMut(),
) -> io::Result<bool> {
    if !is_armed(config)? { return Ok(false); }
    match visible() {
        Some(true) => {},
        Some(false) => show(),
        None => return Err(io::Error::other("desktop icon visibility is unavailable; recovery marker retained")),
    }
    if visible() != Some(true) {
        return Err(io::Error::other("desktop icons were not restored; recovery marker retained"));
    }
    clear(config)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!("aura-recovery-marker-{}-{}", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&root).unwrap(); root
    }
    #[test]
    fn marker_survives_broken_or_missing_profile_and_can_be_cleared() {
        let root = fixture(); let config = root.join("desktop-fences.json");
        assert!(!is_armed(&config).unwrap());
        arm(&config).unwrap();
        fs::write(&config, b"broken").unwrap();
        assert!(is_armed(&config).unwrap());
        fs::remove_file(&config).unwrap();
        assert!(is_armed(&config).unwrap());
        clear(&config).unwrap();
        assert!(!is_armed(&config).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn recovery_restores_only_with_valid_marker_and_confirmed_visibility() {
        use std::cell::Cell;
        let root = fixture(); let config = root.join("desktop-fences.json");
        let visible = Cell::new(false); let calls = Cell::new(0);
        let read = || Some(visible.get());
        let show = || { calls.set(calls.get() + 1); visible.set(true); };
        assert!(!recover_with(&config, read, show).unwrap());
        assert_eq!(calls.get(), 0, "without our marker do not change the user's icons");
        arm(&config).unwrap();
        assert!(recover_with(&config, read, show).unwrap());
        assert_eq!(calls.get(), 1); assert!(!is_armed(&config).unwrap());
        arm(&config).unwrap();
        assert!(recover_with(&config, read, show).unwrap());
        assert_eq!(calls.get(), 1, "already-visible icons must not be toggled");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_or_unknown_visibility_keeps_marker_for_retry() {
        let root = fixture(); let config = root.join("desktop-fences.json");
        arm(&config).unwrap();
        assert!(recover_with(&config, || None, || panic!("must not guess")).is_err());
        assert!(is_armed(&config).unwrap());
        assert!(recover_with(&config, || Some(false), || {}).is_err());
        assert!(is_armed(&config).unwrap());
        assert!(recover_with(&config, || Some(true), || panic!("already visible")).unwrap());
        assert!(!is_armed(&config).unwrap()); fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_marker_is_not_overwritten_or_removed() {
        let root = fixture(); let config = root.join("desktop-fences.json");
        let path = marker_path(&config); fs::write(&path, b"unrecognized").unwrap();
        assert!(is_armed(&config).is_err());
        assert!(arm(&config).is_err()); assert!(clear(&config).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"unrecognized");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn existing_valid_marker_is_not_overwritten() {
        let root = fixture(); let config = root.join("desktop-fences.json");
        arm(&config).unwrap(); assert_eq!(arm(&config).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert!(is_armed(&config).unwrap()); clear(&config).unwrap(); clear(&config).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
