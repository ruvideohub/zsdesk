//! Atomic replacement of a small settings file using a unique sibling temporary file.
use std::{fs::{self, OpenOptions}, io::{self, Write}, path::Path, sync::atomic::{AtomicU64, Ordering}};
static SERIAL: AtomicU64 = AtomicU64::new(1);

pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent"))?;
    fs::create_dir_all(parent)?;
    let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "settings path has no filename"))?;
    for _ in 0..100 {
        let mut temp_name = name.to_os_string();
        temp_name.push(format!(".{}.{}.tmp", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
        let temp = parent.join(temp_name);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let result = file.write_all(bytes).and_then(|_| file.sync_all());
        drop(file);
        let result = result.and_then(|_| replace(&temp, path));
        if result.is_err() { let _ = fs::remove_file(&temp); }
        return result;
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not reserve a temporary settings file"))
}

#[cfg(not(windows))]
fn replace(from: &Path, to: &Path) -> io::Result<()> { fs::rename(from, to) }

#[cfg(windows)]
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" { fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32; }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH, on the same volume.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } == 0 {
        Err(io::Error::last_os_error())
    } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replaces_existing_and_leaves_no_temp_files() {
        let dir = std::env::temp_dir().join(format!("aura-atomic-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        write(&path, b"old").unwrap();
        write(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_replace_preserves_existing_directory() {
        let dir = std::env::temp_dir().join(format!("aura-atomic-failure-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("keep"), b"important").unwrap();
        assert!(write(&path, b"new").is_err());
        assert_eq!(fs::read(path.join("keep")).unwrap(), b"important");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

}
