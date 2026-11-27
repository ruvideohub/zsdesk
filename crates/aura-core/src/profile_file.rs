//! Fallible JSON profile loading and protected atomic replacement.
//!
//! Only a missing file is treated as a fresh profile. An existing unreadable,
//! oversized or invalid profile must remain untouched, even on shutdown.
use serde::{de::DeserializeOwned, Serialize};
use std::{fs::File, io::{self, Read}, path::Path};

const MAX_PROFILE_BYTES: u64 = 16 * 1024 * 1024;

fn read_existing<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A dangling link is an existing user object, not a fresh profile.
            return match std::fs::symlink_metadata(path) {
                Ok(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "profile path exists but cannot be opened")),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            };
        }
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(MAX_PROFILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "profile exceeds 16 MiB"));
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    serde_json::from_slice(bytes).map(Some).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Return None only when no file exists; an unreadable existing file is an error.
pub fn load_existing<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    read_existing(path)
}

pub fn load_or_default<T: DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    Ok(read_existing(path)?.unwrap_or_default())
}

/// Refuse to replace a profile that became unreadable or invalid since loading.
/// Callers must serialize all legitimate writers of the same profile themselves.
pub fn save<T: DeserializeOwned + Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let _ = read_existing::<T>(path)?;
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "serialized profile exceeds 16 MiB"));
    }
    crate::atomic_file::write(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::{fs, sync::atomic::{AtomicU64, Ordering}};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Profile { value: u32 }
    fn fixture() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aura-profile-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&dir).unwrap();
        dir
    }
    #[test]
    fn missing_profile_defaults_and_valid_profile_roundtrips() {
        let dir = fixture(); let path = dir.join("profile.json");
        assert!(load_existing::<Profile>(&path).unwrap().is_none());
        assert_eq!(load_or_default::<Profile>(&path).unwrap(), Profile::default());
        save(&path, &Profile { value: 42 }).unwrap();
        assert_eq!(load_or_default::<Profile>(&path).unwrap(), Profile { value: 42 });
        save(&path, &Profile { value: 7 }).unwrap();
        assert_eq!(load_or_default::<Profile>(&path).unwrap().value, 7);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn invalid_profile_is_not_replaced() {
        let dir = fixture(); let path = dir.join("profile.json");
        let damaged = b"{\"value\":broken"; fs::write(&path, damaged).unwrap();
        assert_eq!(load_or_default::<Profile>(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(save(&path, &Profile::default()).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&path).unwrap(), damaged);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn read_failure_does_not_replace_directory() {
        let dir = fixture(); let path = dir.join("profile.json");
        fs::create_dir(&path).unwrap(); fs::write(path.join("keep"), b"keep").unwrap();
        assert!(load_or_default::<Profile>(&path).is_err());
        assert!(save(&path, &Profile::default()).is_err());
        assert_eq!(fs::read(path.join("keep")).unwrap(), b"keep");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn oversized_profile_is_preserved() {
        let dir = fixture(); let path = dir.join("profile.json");
        let file = File::create(&path).unwrap(); file.set_len(MAX_PROFILE_BYTES + 1).unwrap();
        assert_eq!(load_or_default::<Profile>(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(save(&path, &Profile::default()).is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_PROFILE_BYTES + 1);
        fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn dangling_profile_link_is_not_replaced() {
        let dir = fixture(); let path = dir.join("profile.json");
        std::os::unix::fs::symlink(dir.join("missing"), &path).unwrap();
        assert!(load_or_default::<Profile>(&path).is_err());
        assert!(save(&path, &Profile::default()).is_err());
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn oversized_serialized_profile_does_not_replace_existing() {
        let dir = fixture(); let path = dir.join("profile.json");
        save(&path, &String::from("old")).unwrap();
        assert!(save(&path, &"x".repeat(MAX_PROFILE_BYTES as usize)).is_err());
        assert_eq!(load_or_default::<String>(&path).unwrap(), "old");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn bom_is_accepted() {
        let dir = fixture(); let path = dir.join("profile.json");
        fs::write(&path, b"\xef\xbb\xbf{\"value\":4}").unwrap();
        assert_eq!(load_or_default::<Profile>(&path).unwrap().value, 4);
        fs::remove_dir_all(dir).unwrap();
    }
}
