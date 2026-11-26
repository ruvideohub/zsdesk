//! Non-destructive, bounded access to the same notes file used by the widget gallery.
use std::{fs::{self, OpenOptions}, io::{self, Read}, path::Path};

const MAX_PREVIEW_BYTES: u64 = 64 * 1024;

/// Missing is an empty note; other I/O failures remain distinct from an empty note.
pub fn read(path: &Path) -> io::Result<String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(MAX_PREVIEW_BYTES).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Only called after an explicit edit command. Never truncate an existing file.
pub fn ensure_file(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    OpenOptions::new().write(true).create(true).truncate(false).open(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_empty_and_existing_notes_are_safe() {
        let dir = std::env::temp_dir().join(format!("aura-desktop-notes-{}", std::process::id()));
        let path = dir.join("nested").join("notes.txt");
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(read(&path).unwrap(), "");
        assert!(!path.exists());
        ensure_file(&path).unwrap();
        fs::write(&path, "Привет\nWorld").unwrap();
        ensure_file(&path).unwrap();
        assert_eq!(read(&path).unwrap(), "Привет\nWorld");
        assert_eq!(fs::read_to_string(&path).unwrap(), "Привет\nWorld");
        fs::write(&path, vec![b'x'; MAX_PREVIEW_BYTES as usize + 100]).unwrap();
        assert_eq!(read(&path).unwrap().len(), MAX_PREVIEW_BYTES as usize);
        fs::remove_dir_all(&dir).unwrap();
    }
}
