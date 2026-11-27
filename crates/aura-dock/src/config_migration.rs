//! Crash-safe, non-destructive migration of the old Dock directory.
use std::{fs, io, path::{Path, PathBuf}, sync::atomic::{AtomicU64, Ordering}};
static NEXT: AtomicU64 = AtomicU64::new(1);

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

/// Publish without replacing any existing destination, including an empty directory.
#[cfg(windows)]
fn publish(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" { fn MoveFileW(from: *const u16, to: *const u16) -> i32; }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileW(from.as_ptr(), to.as_ptr()) } == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

// Used by the portable unit tests only; the deployed implementation is MoveFileW above.
#[cfg(not(windows))]
fn publish(from: &Path, to: &Path) -> io::Result<()> {
    if to.try_exists()? { return Err(io::Error::new(io::ErrorKind::AlreadyExists, "profile already exists")); }
    fs::rename(from, to)
}

pub fn migrate(old: &Path, new: &Path) -> io::Result<()> {
    migrate_with(old, new, |source, target| {
        let mut input = fs::File::open(source)?;
        let permissions = input.metadata()?.permissions();
        let mut output = fs::OpenOptions::new().write(true).create_new(true).open(target)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        output.set_permissions(permissions)
    })
}

fn migrate_with(old: &Path, new: &Path, mut copy: impl FnMut(&Path, &Path) -> io::Result<()>) -> io::Result<()> {
    if new.try_exists()? { return Ok(()); }
    let old_meta = match fs::metadata(old) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !old_meta.is_dir() { return Err(io::Error::new(io::ErrorKind::InvalidData, "legacy Dock profile is not a directory")); }
    let parent = new.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "profile has no parent"))?;
    fs::create_dir_all(parent)?;
    let stage = (0..100).find_map(|_| {
        let path = parent.join(format!(".dock-migration-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match fs::create_dir(&path) {
            Ok(()) => Some(Ok(Staging(path))),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => None,
            Err(e) => Some(Err(e)),
        }
    }).unwrap_or_else(|| Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not create migration staging directory")))?;
    for entry in fs::read_dir(old)? {
        let entry = entry?;
        // Preserve the former flat-directory behavior without deleting anything in the old profile.
        if entry.metadata()?.is_file() { copy(&entry.path(), &stage.0.join(entry.file_name()))?; }
    }
    // Atomic same-volume move. Failure leaves new untouched; the caller must not start with defaults.
    publish(&stage.0, new)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!("dock-migration-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&root).unwrap(); root
    }
    #[test]
    fn copies_flat_profile_and_keeps_source() {
        let root = fixture(); let old = root.join("old"); let new = root.join("profile/Dock");
        fs::create_dir(&old).unwrap(); fs::write(old.join("config.json"), b"pins").unwrap();
        fs::write(old.join("notes.txt"), b"user data").unwrap();
        migrate(&old, &new).unwrap();
        assert_eq!(fs::read(new.join("config.json")).unwrap(), b"pins");
        assert_eq!(fs::read(old.join("notes.txt")).unwrap(), b"user data");
        assert_eq!(fs::read(new.join("notes.txt")).unwrap(), b"user data");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn partial_copy_fails_without_publishing_and_retry_succeeds() {
        let root = fixture(); let old = root.join("old"); let new = root.join("profile/Dock");
        fs::create_dir(&old).unwrap(); fs::write(old.join("config.json"), b"pins").unwrap();
        let result = migrate_with(&old, &new, |_, target| { fs::write(target, b"partial")?; Err(io::Error::other("injected copy failure")) });
        assert!(result.is_err()); assert!(!new.exists());
        assert_eq!(fs::read(old.join("config.json")).unwrap(), b"pins");
        assert_eq!(fs::read_dir(new.parent().unwrap()).unwrap().count(), 0);
        migrate(&old, &new).unwrap(); assert_eq!(fs::read(new.join("config.json")).unwrap(), b"pins");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn preexisting_destination_is_never_touched() {
        let root = fixture(); let old = root.join("old"); let new = root.join("new");
        fs::create_dir(&old).unwrap(); fs::write(old.join("config.json"), b"old").unwrap();
        fs::create_dir(&new).unwrap(); fs::write(new.join("config.json"), b"new").unwrap();
        migrate(&old, &new).unwrap(); assert_eq!(fs::read(new.join("config.json")).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn publish_refuses_destination_created_mid_migration() {
        let root = fixture(); let old = root.join("old"); let new = root.join("new");
        fs::create_dir(&old).unwrap(); fs::write(old.join("config.json"), b"old").unwrap();
        let result = migrate_with(&old, &new, |source, target| {
            fs::copy(source, target)?; fs::create_dir(&new)?; fs::write(new.join("config.json"), b"other")?; Ok(())
        });
        assert!(result.is_err()); assert_eq!(fs::read(new.join("config.json")).unwrap(), b"other");
        assert_eq!(fs::read(old.join("config.json")).unwrap(), b"old");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_source_is_a_noop_and_blocked_parent_is_an_error() {
        let root = fixture(); let old = root.join("old"); let new = root.join("profile/Dock");
        migrate(&old, &new).unwrap(); assert!(!new.exists());
        fs::create_dir(&old).unwrap(); fs::write(root.join("profile"), b"blocked").unwrap();
        assert!(migrate(&old, &new).is_err()); assert!(old.is_dir());
        fs::remove_dir_all(root).unwrap();
    }
}
