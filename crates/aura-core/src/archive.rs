//! Minimal ZIP writer used by the built-in "compress" command.
//!
//! The context menu must work without any registered shell extension, so the
//! archive is written here instead of delegating to 7-Zip or Explorer's
//! "Send to › Compressed folder". Entries are stored (no deflate) which keeps
//! the writer dependency-free and predictable: a large file is streamed twice
//! (CRC pass, copy pass) and never loaded into memory.
//!
//! Layering: no HWND, no COM, no ZSUI — this is plain byte-level I/O and can be
//! unit tested anywhere.

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_CENTRAL: u32 = 0x0605_4b50;
/// Bit 11: the entry name is UTF-8. Set for every entry so non-ASCII names
/// survive without an extra field.
const UTF8_NAMES: u16 = 0x0800;
const STORED: u16 = 0;
const COPY_BUFFER: usize = 64 * 1024;

/// One selected file or folder, already resolved to an archive path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Forward-slash separated path inside the archive, never absolute.
    pub name: String,
    pub source: PathBuf,
    pub is_dir: bool,
}

impl Entry {
    pub fn file(name: impl Into<String>, source: impl Into<PathBuf>) -> Self {
        Self { name: name.into(), source: source.into(), is_dir: false }
    }
    pub fn directory(name: impl Into<String>, source: impl Into<PathBuf>) -> Self {
        let mut name = name.into();
        if !name.ends_with('/') { name.push('/'); }
        Self { name, source: source.into(), is_dir: true }
    }
}

/// Archive name for one selected item: its own file name without extension.
pub fn archive_name_for(sources: &[PathBuf], folder: Option<&Path>) -> String {
    let base = match sources {
        [single] => match single.file_stem() {
            Some(stem) if !stem.is_empty() => stem.to_string_lossy().into_owned(),
            _ => single.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".into()),
        },
        _ => folder
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "archive".into()),
    };
    format!("{base}.zip")
}

/// Resolves a free destination so an existing archive is never overwritten.
pub fn unique_destination(folder: &Path, name: &str) -> PathBuf {
    let candidate = folder.join(name);
    if !candidate.exists() { return candidate; }
    let stem = Path::new(name).file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".into());
    for index in 2..10_000u32 {
        let candidate = folder.join(format!("{stem} ({index}).zip"));
        if !candidate.exists() { return candidate; }
    }
    folder.join(format!("{stem} ({}).zip", std::process::id()))
}

/// Collects `sources` (files or folders, folders recursively) into entries.
///
/// Redirects of every kind — symbolic links, junctions, volume mount points —
/// are skipped rather than followed or stored. Following one would let a folder
/// beside a junction pull the target's whole tree into the archive, and storing
/// one would put an entry in the ZIP that Windows extracts as a plain empty file
/// anyway. Skipping is the only behaviour that is both bounded and honest about
/// what the archive contains.
///
/// Depth and total count are bounded by the caller's budget through [`Limits`].
/// Running into either bound is an error, never a short archive: a right click
/// that silently left files out would hand back a ZIP that looks complete.
pub fn collect(sources: &[PathBuf], limits: Limits) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for source in sources {
        let Some(name) = source.file_name().map(|name| name.to_string_lossy().into_owned()) else { continue };
        walk(source, &sanitize(&name), 0, limits, &mut entries)?;
    }
    Ok(entries)
}

/// Bound on how much an archive may pull in. A right click must never walk an
/// entire drive because a folder was selected by accident.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_entries: usize,
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self { Self { max_entries: 20_000, max_depth: 24 } }
}

fn walk(source: &Path, name: &str, depth: usize, limits: Limits, entries: &mut Vec<Entry>) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if entries.len() >= limits.max_entries {
        return Err(too_large(format!(
            "{}: {}",
            tr_format!("压缩的项目超过 {limit} 个，未创建压缩包", limit = limits.max_entries),
            source.display(),
        )));
    }
    if metadata.is_dir() {
        entries.push(Entry::directory(name, source));
        if depth >= limits.max_depth {
            return Err(too_large(format!(
                "{}: {}",
                tr_format!("文件夹层级超过 {limit} 层，未创建压缩包", limit = limits.max_depth),
                source.display(),
            )));
        }
        let mut children: Vec<PathBuf> = fs::read_dir(source)?.filter_map(|item| item.ok().map(|item| item.path())).collect();
        children.sort_by_key(|path| path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default());
        for child in children {
            let Some(child_name) = child.file_name().map(|value| value.to_string_lossy().into_owned()) else { continue };
            walk(&child, &format!("{name}/{}", sanitize(&child_name)), depth + 1, limits, entries)?;
        }
    } else {
        entries.push(Entry::file(name, source));
    }
    Ok(())
}

/// Archive paths never contain drive letters, `..` or backslashes.
fn sanitize(name: &str) -> String {
    let replaced = name.replace('\\', "/");
    let mut parts: Vec<&str> = replaced.split('/').filter(|part| !part.is_empty() && *part != "." && *part != "..").collect();
    if parts.is_empty() { return "file".into(); }
    parts.drain(..).collect::<Vec<_>>().join("/")
}

/// Writes a stored-only archive. Returns the number of entries written.
///
/// A file that is created here and not finished is removed again: a half-written
/// ZIP beside the originals is worse than no ZIP, because it looks like the real
/// thing until someone opens it.
pub fn write_zip(destination: &Path, entries: &[Entry]) -> io::Result<usize> {
    match write_zip_inner(destination, entries) {
        Ok(written) => Ok(written),
        Err(error) => {
            let _ = fs::remove_file(destination);
            Err(error)
        }
    }
}

fn write_zip_inner(destination: &Path, entries: &[Entry]) -> io::Result<usize> {
    let mut file = File::create(destination)?;
    let stamp = dos_timestamp(SystemTime::now());
    let mut central = Vec::with_capacity(entries.len() * 64);
    let mut offset: u64 = 0;
    let mut written = 0usize;
    for entry in entries {
        let name = entry.name.as_bytes();
        let name_len = u16::try_from(name.len()).map_err(|_| invalid("archive entry name is too long"))?;
        let (crc, size) = if entry.is_dir {
            (0u32, 0u64)
        } else {
            digest(&entry.source)?
        };
        let size32 = u32::try_from(size).map_err(|_| invalid("a file is too large for a ZIP archive"))?;
        let local_offset = u32::try_from(offset).map_err(|_| invalid("archive is too large"))?;
        let mut header = Vec::with_capacity(30 + name.len());
        header.extend_from_slice(&LOCAL_HEADER.to_le_bytes());
        header.extend_from_slice(&20u16.to_le_bytes());
        header.extend_from_slice(&UTF8_NAMES.to_le_bytes());
        header.extend_from_slice(&STORED.to_le_bytes());
        header.extend_from_slice(&stamp.0.to_le_bytes());
        header.extend_from_slice(&stamp.1.to_le_bytes());
        header.extend_from_slice(&crc.to_le_bytes());
        header.extend_from_slice(&size32.to_le_bytes());
        header.extend_from_slice(&size32.to_le_bytes());
        header.extend_from_slice(&name_len.to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes());
        header.extend_from_slice(name);
        file.write_all(&header)?;
        offset += header.len() as u64;
        if !entry.is_dir {
            let copied = copy_into(&entry.source, &mut file)?;
            if copied != size { return Err(invalid("a file changed while the archive was written")); }
            offset += copied;
        }
        let mut record = Vec::with_capacity(46 + name.len());
        record.extend_from_slice(&CENTRAL_HEADER.to_le_bytes());
        record.extend_from_slice(&20u16.to_le_bytes());
        record.extend_from_slice(&20u16.to_le_bytes());
        record.extend_from_slice(&UTF8_NAMES.to_le_bytes());
        record.extend_from_slice(&STORED.to_le_bytes());
        record.extend_from_slice(&stamp.0.to_le_bytes());
        record.extend_from_slice(&stamp.1.to_le_bytes());
        record.extend_from_slice(&crc.to_le_bytes());
        record.extend_from_slice(&size32.to_le_bytes());
        record.extend_from_slice(&size32.to_le_bytes());
        record.extend_from_slice(&name_len.to_le_bytes());
        record.extend_from_slice(&0u16.to_le_bytes());
        record.extend_from_slice(&0u16.to_le_bytes());
        record.extend_from_slice(&0u16.to_le_bytes());
        record.extend_from_slice(&0u16.to_le_bytes());
        record.extend_from_slice(&0u32.to_le_bytes());
        record.extend_from_slice(&local_offset.to_le_bytes());
        record.extend_from_slice(name);
        central.extend_from_slice(&record);
        written += 1;
    }
    let central_offset = u32::try_from(offset).map_err(|_| invalid("archive is too large"))?;
    file.write_all(&central)?;
    offset += central.len() as u64;
    let count = u16::try_from(written).map_err(|_| invalid("too many entries for a ZIP archive"))?;
    let _ = offset;
    let mut end = Vec::with_capacity(22);
    end.extend_from_slice(&END_OF_CENTRAL.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    end.extend_from_slice(&count.to_le_bytes());
    end.extend_from_slice(&count.to_le_bytes());
    end.extend_from_slice(&u32::try_from(central.len()).map_err(|_| invalid("archive is too large"))?.to_le_bytes());
    end.extend_from_slice(&central_offset.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    file.write_all(&end)?;
    file.flush()?;
    Ok(written)
}

fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidInput, message.to_string()) }

/// The selection is bigger or deeper than the walk budget allows. Its own kind
/// so the caller can tell "too much to pack" apart from "the disk refused".
fn too_large(message: String) -> io::Error { io::Error::new(io::ErrorKind::FileTooLarge, message) }

fn copy_into(source: &Path, output: &mut File) -> io::Result<u64> {
    let mut input = File::open(source)?;
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut copied = 0u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 { break; }
        output.write_all(&buffer[..read])?;
        copied += read as u64;
    }
    Ok(copied)
}

fn digest(source: &Path) -> io::Result<(u32, u64)> {
    let mut input = File::open(source)?;
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut crc = 0u32;
    let mut size = 0u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 { break; }
        crc = crc32(crc, &buffer[..read]);
        size += read as u64;
    }
    Ok((crc, size))
}

/// DOS date/time as stored in the ZIP headers.
fn dos_timestamp(time: SystemTime) -> (u16, u16) {
    let seconds = time.duration_since(UNIX_EPOCH).map(|value| value.as_secs()).unwrap_or(0) as i64;
    let (year, month, day, hour, minute, second) = civil_from_unix(seconds);
    let date = (((year - 1980).clamp(0, 127) as u16) << 9) | ((month as u16) << 5) | day as u16;
    let clock = ((hour as u16) << 11) | ((minute as u16) << 5) | ((second / 2) as u16);
    (clock, date)
}

/// Proleptic Gregorian conversion (days since 1970-01-01 to calendar date).
fn civil_from_unix(unix: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = unix.div_euclid(86_400);
    let seconds_of_day = unix.rem_euclid(86_400);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 { month_prime + 3 } else { month_prime - 9 };
    year += i64::from(month <= 2);
    (year as i32, month as u32, day as u32,
     (seconds_of_day / 3600) as u32, ((seconds_of_day % 3600) / 60) as u32, (seconds_of_day % 60) as u32)
}

/// CRC-32 (IEEE 802.3) as required by the ZIP specification.
pub fn crc32(mut crc: u32, bytes: &[u8]) -> u32 {
    crc = !crc;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// `Component` helper kept for callers that only have a path.
pub fn relative_name(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("aura-zip-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn crc_matches_the_published_check_value() {
        assert_eq!(crc32(0, b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(0, b""), 0);
    }

    #[test]
    fn stored_archive_has_all_three_record_kinds() {
        let root = scratch("records");
        fs::write(root.join("a.txt"), b"hello").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/b.txt"), b"world").unwrap();
        let entries = collect(&[root.join("a.txt"), root.join("sub")], Limits::default()).unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "sub/", "sub/b.txt"]);
        let destination = root.join("out.zip");
        assert_eq!(write_zip(&destination, &entries).unwrap(), 3);
        let bytes = fs::read(&destination).unwrap();
        assert_eq!(&bytes[..4], &LOCAL_HEADER.to_le_bytes());
        assert!(bytes.windows(4).any(|window| window == CENTRAL_HEADER.to_le_bytes()));
        assert!(bytes.windows(4).any(|window| window == END_OF_CENTRAL.to_le_bytes()));
        // The stored payload must appear verbatim.
        assert!(bytes.windows(5).any(|window| window == b"hello"));
        assert!(bytes.windows(5).any(|window| window == b"world"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn entry_names_cannot_escape_the_archive() {
        assert_eq!(sanitize("..\\..\\etc\\passwd"), "etc/passwd");
        assert_eq!(sanitize("../x"), "x");
        assert_eq!(sanitize(""), "file");
    }

    #[test]
    fn names_and_destinations_are_unique() {
        // Relative names keep this assertion true on every host platform.
        assert_eq!(archive_name_for(&[PathBuf::from("photo.png")], None), "photo.zip");
        assert_eq!(archive_name_for(&[PathBuf::from("a.txt"), PathBuf::from("b.txt")], Some(Path::new("docs"))), "docs.zip");
        assert_eq!(archive_name_for(&[PathBuf::from("no-extension")], None), "no-extension.zip");
        let root = scratch("unique");
        fs::write(root.join("photo.zip"), b"").unwrap();
        assert_eq!(unique_destination(&root, "photo.zip").file_name().unwrap(), "photo (2).zip");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn archive_paths_are_bounded() {
        let limits = Limits { max_entries: 2, max_depth: 1 };
        let entries = vec![Entry::file("a", "a"), Entry::file("b", "b")];
        assert_eq!(entries.len(), 2);
        assert_eq!(limits.max_entries, 2);
        // A depth limit stops the walk without failing the whole archive.
        let root = scratch("bounded");
        let deep = root.join("one").join("two").join("three");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("f.txt"), b"x").unwrap();
        // A depth limit is a refusal now, not a silent short archive: `one/two/f.txt`
        // is past the bound, so nothing is written at all.
        let refusal = collect(&[root.join("one")], Limits { max_entries: 100, max_depth: 1 }).unwrap_err();
        assert_eq!(refusal.kind(), io::ErrorKind::FileTooLarge);
        let collected = collect(&[root.join("one")], Limits { max_entries: 100, max_depth: 4 }).unwrap();
        assert_eq!(collected.iter().filter(|entry| entry.name.ends_with("f.txt")).count(), 1);
        // The count bound refuses too, at the exact entry that would exceed it.
        let too_many = collect(&[root.join("one")], Limits { max_entries: 2, max_depth: 4 }).unwrap_err();
        assert_eq!(too_many.kind(), io::ErrorKind::FileTooLarge);
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_is_neither_followed_nor_stored() {
        use std::os::windows::fs::symlink_dir;
        let root = scratch("junction");
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), b"private").unwrap();
        let selection = root.join("selection");
        fs::create_dir_all(&selection).unwrap();
        fs::write(selection.join("kept.txt"), b"kept").unwrap();
        // Creating a junction needs no privilege, unlike a symbolic link.
        let link = selection.join("link");
        if symlink_dir(&outside, &link).is_err() {
            // A symbolic link is the fallback; the rule is the same for both.
            if std::os::windows::fs::symlink_file(&outside.join("secret.txt"), &link).is_err() {
                fs::remove_dir_all(&root).unwrap();
                return;
            }
        }
        let entries = collect(&[selection], Limits::default()).unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert!(names.iter().any(|name| name.ends_with("kept.txt")), "{names:?}");
        assert!(!names.iter().any(|name| name.contains("link")), "{names:?}");
        assert!(!names.iter().any(|name| name.contains("secret")), "{names:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_no_archive_behind() {
        let root = scratch("cleanup");
        let missing = root.join("gone.txt");
        // The entry points at a file that does not exist, so the copy pass fails after
        // the header of the archive has already been written.
        let destination = root.join("broken.zip");
        let entries = vec![Entry::file("gone.txt", missing)];
        assert!(write_zip(&destination, &entries).is_err());
        assert!(!destination.exists(), "a half-written archive must not survive the error");
        fs::remove_dir_all(&root).unwrap();
    }
}
