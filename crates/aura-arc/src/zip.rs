//! The `.zip` format, as much of it as a person needs day to day: pack, unpack, list, check.
//!
//! Written out by hand instead of pulled in with an archive crate so the archiver keeps the
//! promise every AuraUI component makes — it is one exe on its own, with two ordinary
//! compression libraries and nothing else.
//!
//! What is supported: stored and deflated entries, UTF-8 names, folders, appending to an
//! existing archive (its entries are copied over byte for byte, so nothing is re-compressed),
//! and the checks each entry carries. What is refused, with a reason instead of a corrupt file:
//! encrypted entries and ZIP64 (an archive over 4 GB or one with more than 65 535 entries).

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;

/// Something the person in front of the program can read and act on.
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self(error.to_string())
    }
}

type Result<T> = std::result::Result<T, Error>;

fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error(message.into()))
}

/// What one command did, for the line printed at the end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub files: usize,
    /// Folders are counted apart from files: «12 файлов и 3 папки», not «15 файлов».
    pub folders: usize,
    pub bytes: u64,
    pub packed: u64,
    pub failed: usize,
    /// Files that were already in the archive and were therefore left alone.
    pub skipped: usize,
}

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_DIRECTORY: u32 = 0x0605_4b50;
/// Entry flag: the name is UTF-8.
const UTF8_NAME: u16 = 1 << 11;
/// Entry flag: sizes and the checksum follow the data (the writer did not know them up front).
const SIZES_AFTER: u16 = 1 << 3;
/// Entry flag: encrypted — refused.
const ENCRYPTED: u16 = 1;

pub const STORED: u16 = 0;
const DEFLATED: u16 = 8;

/// One entry as the archive's own directory describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub flags: u16,
    pub crc: u32,
    pub size: u64,
    pub packed: u64,
    /// Where the entry's local header starts, in the archive.
    pub offset: u64,
    pub time: u16,
    pub date: u16,
    /// The local header's extra field, kept as it is when an archive is rebuilt.
    pub extra: Vec<u8>,
    /// Version and host that made the entry — kept, so a rewrite says the same thing.
    pub made_by: u16,
    /// Internal attributes (0 for a file, 1 for text).
    pub internal: u16,
    /// External attributes: on a FAT host the low byte is the DOS attribute set, which is how
    /// other unpackers tell a folder from a file.
    pub external: u32,
}

/// Version 2.0, by an MS-DOS/Windows host: the pairing every unpacker reads the same way.
const MADE_BY: u16 = 20;
/// DOS attribute: ordinary file.
const ATTR_FILE: u32 = 0x20;
/// DOS attribute: folder. Without it some tools unpack folders as empty files.
const ATTR_FOLDER: u32 = 0x10;

impl Entry {
    pub fn is_folder(&self) -> bool {
        self.name.ends_with('/')
    }
}

/// The archive's directory, read from the end of the file.
pub fn entries(archive: &Path) -> Result<Vec<Entry>> {
    let mut file = File::open(archive).map_err(|error| Error(format!("не удалось открыть {}: {error}", archive.display())))?;
    let length = file.seek(SeekFrom::End(0))?;
    if length < 22 {
        return fail(format!("{} не похож на архив: он слишком мал", archive.display()));
    }
    // The end record sits in the last 64 KiB (its own comment may be that long).
    let window = length.min(66_000);
    file.seek(SeekFrom::Start(length - window))?;
    let mut tail = vec![0u8; window as usize];
    file.read_exact(&mut tail)?;
    let Some(at) = find_end(&tail) else {
        return fail(format!("{} не похож на архив .zip: нет оглавления", archive.display()));
    };
    let directory_size = u32(&tail, at + 12) as u64;
    let directory_offset = u32(&tail, at + 16) as u64;
    let count = u16(&tail, at + 10) as usize;
    if directory_offset == u32::MAX as u64 || directory_size == u32::MAX as u64 || count == u16::MAX as usize {
        return fail(format!("{} — архив ZIP64; такие пока не поддерживаются", archive.display()));
    }
    // Defensive: a truncated or hand-edited file must not make us allocate wildly.
    if directory_size > length {
        return fail(format!("{} повреждён: оглавление больше самого файла", archive.display()));
    }
    file.seek(SeekFrom::Start(directory_offset))?;
    let mut directory = vec![0u8; directory_size as usize];
    file.read_exact(&mut directory)?;
    let entries = parse_directory(&directory)?;
    // The end record says how many entries the directory holds. When the two disagree the file
    // was cut short or patched by hand, and unpacking "as much as possible" would hide that.
    if count != 0 && count != entries.len() {
        return fail(format!(
            "{} повреждён: в оглавлении записано {count}, а найдено {}",
            archive.display(),
            entries.len()
        ));
    }
    Ok(entries)
}

/// Find the end-of-directory record, scanning backwards (its comment follows it).
fn find_end(tail: &[u8]) -> Option<usize> {
    if tail.len() < 22 {
        return None;
    }
    for at in (0..=tail.len() - 22).rev() {
        if u32(tail, at) == END_OF_DIRECTORY {
            return Some(at);
        }
    }
    None
}

fn parse_directory(directory: &[u8]) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut at = 0usize;
    while at + 46 <= directory.len() {
        if u32(directory, at) != CENTRAL_HEADER {
            return fail("повреждено оглавление архива");
        }
        let flags = u16(directory, at + 8);
        let method = u16(directory, at + 10);
        let time = u16(directory, at + 12);
        let date = u16(directory, at + 14);
        let crc = u32(directory, at + 16);
        let packed = u32(directory, at + 20) as u64;
        let size = u32(directory, at + 24) as u64;
        let name_length = u16(directory, at + 28) as usize;
        let extra_length = u16(directory, at + 30) as usize;
        let comment_length = u16(directory, at + 32) as usize;
        let offset = u32(directory, at + 42) as u64;
        let name_at = at + 46;
        if name_at + name_length + extra_length + comment_length > directory.len() {
            return fail("повреждено оглавление архива: запись выходит за его край");
        }
        let raw_name = &directory[name_at..name_at + name_length];
        let name = decode_name(raw_name, flags);
        let extra = directory[name_at + name_length..name_at + name_length + extra_length].to_vec();
        if flags & ENCRYPTED != 0 {
            return fail(format!("«{name}» зашифрован; такие архивы пока не поддерживаются"));
        }
        if packed == u32::MAX as u64 || size == u32::MAX as u64 || offset == u32::MAX as u64 {
            return fail(format!("«{name}» в формате ZIP64; такие архивы пока не поддерживаются"));
        }
        let made_by = u16(directory, at + 4);
        let internal = u16(directory, at + 36);
        let external = u32(directory, at + 38);
        entries.push(Entry { name, method, flags, crc, size, packed, offset, time, date, extra, made_by, internal, external });
        at = name_at + name_length + extra_length + comment_length;
    }
    Ok(entries)
}

fn decode_name(raw: &[u8], flags: u16) -> String {
    // Bit 11 promises UTF-8. Without it the name is in the writer's code page; UTF-8 first,
    // then the lossy fallback, beats refusing to open the archive at all.
    if flags & UTF8_NAME != 0 {
        return String::from_utf8_lossy(raw).into_owned();
    }
    match std::str::from_utf8(raw) {
        Ok(name) => name.to_string(),
        Err(_) => String::from_utf8_lossy(raw).into_owned(),
    }
}

/// List what an archive holds; the console prints it.
pub fn list(archive: &Path) -> Result<Vec<Entry>> {
    entries(archive)
}

/// Unpack everything into `dest`. A file that fails does not stop the rest: each is reported
/// and counted, and the caller decides what exit code that deserves.
pub fn extract(archive: &Path, dest: &Path, mut on_entry: impl FnMut(&Entry, bool)) -> Result<Summary> {
    unpack(archive, dest, None, &mut on_entry)
}

/// Unpack the named entries only — what «Копировать» does with a row of the table.
pub fn extract_only(archive: &Path, dest: &Path, names: &[String], mut on_entry: impl FnMut(&Entry, bool)) -> Result<Summary> {
    unpack(archive, dest, Some(names), &mut on_entry)
}

/// `wanted` is `None` for the whole archive, or the names picked in the table.
fn unpack(archive: &Path, dest: &Path, wanted: Option<&[String]>, on_entry: &mut impl FnMut(&Entry, bool)) -> Result<Summary> {
    let entries = entries(archive)?;
    let mut file = BufReader::new(File::open(archive)?);
    let mut summary = Summary::default();
    // Nothing may be written outside `dest`, however the archive was made.
    let root = absolute(dest)?;
    for entry in entries {
        if let Some(names) = wanted {
            if !picked(&entry, names) {
                continue;
            }
        }
        let Some(relative) = safe_relative(&entry.name) else {
            summary.failed += 1;
            on_entry(&entry, false);
            continue;
        };
        let target = root.join(&relative);
        let result = if entry.is_folder() {
            fs::create_dir_all(&target).map_err(Error::from)
        } else {
            extract_one(&mut file, &entry, &target)
        };
        match result {
            Ok(()) => {
                if entry.is_folder() {
                    summary.folders += 1;
                } else {
                    summary.files += 1;
                    summary.bytes += entry.size;
                }
                on_entry(&entry, true);
            }
            Err(error) => {
                summary.failed += 1;
                eprintln!("! {}: {error}", entry.name);
                on_entry(&entry, false);
            }
        }
    }
    Ok(summary)
}

/// Is this entry one of the names picked in the table? Picking a folder picks what is inside.
fn picked(entry: &Entry, names: &[String]) -> bool {
    names.iter().any(|name| {
        entry.name == *name || (name.ends_with('/') && entry.name.starts_with(name.as_str()))
    })
}

fn extract_one(file: &mut BufReader<File>, entry: &Entry, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    // The data starts after the local header, whose extra field may differ from the
    // directory's — so read the local header, not the numbers from the directory.
    file.seek(SeekFrom::Start(entry.offset))?;
    let mut header = [0u8; 30];
    file.read_exact(&mut header)?;
    if u32(&header, 0) != LOCAL_HEADER {
        return fail("повреждён заголовок записи");
    }
    let name_length = u16(&header, 26) as usize;
    let extra_length = u16(&header, 28) as usize;
    file.seek(SeekFrom::Current((name_length + extra_length) as i64))?;

    let mut out = BufWriter::new(File::create(target)?);
    let mut crc = crc32fast::Hasher::new();
    let mut written = 0u64;
    let copy = |chunk: &[u8], crc: &mut crc32fast::Hasher, out: &mut BufWriter<File>, written: &mut u64| -> Result<()> {
        crc.update(chunk);
        out.write_all(chunk)?;
        *written += chunk.len() as u64;
        Ok(())
    };
    match entry.method {
        STORED => {
            let mut left = entry.packed;
            let mut buffer = vec![0u8; 64 * 1024];
            while left > 0 {
                let want = left.min(buffer.len() as u64) as usize;
                file.read_exact(&mut buffer[..want])?;
                copy(&buffer[..want], &mut crc, &mut out, &mut written)?;
                left -= want as u64;
            }
        }
        DEFLATED => {
            let mut decoder = DeflateDecoder::new((&mut *file).take(entry.packed));
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let read = decoder.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                copy(&buffer[..read], &mut crc, &mut out, &mut written)?;
            }
        }
        other => return fail(format!("неизвестный способ сжатия {other}")),
    }
    out.flush()?;
    // The handle has to be closed before a damaged file can be deleted: Windows will not remove
    // a file that is still open.
    let file = out.into_inner().map_err(|error| Error(error.to_string()))?;
    if crc.finalize() != entry.crc {
        drop(file);
        let _ = fs::remove_file(target);
        return fail("контрольная сумма не совпала — файл повреждён".to_string());
    }
    stamp(&file, entry);
    drop(file);
    Ok(())
}

/// Check every entry without writing anything.
pub fn test(archive: &Path, mut on_entry: impl FnMut(&Entry, bool)) -> Result<Summary> {
    let entries = entries(archive)?;
    let mut file = BufReader::new(File::open(archive)?);
    let mut summary = Summary::default();
    for entry in entries {
        if entry.is_folder() {
            summary.folders += 1;
            on_entry(&entry, true);
            continue;
        }
        let checked = (|| -> Result<()> {
            file.seek(SeekFrom::Start(entry.offset))?;
            let mut header = [0u8; 30];
            file.read_exact(&mut header)?;
            if u32(&header, 0) != LOCAL_HEADER {
                return fail("повреждён заголовок записи");
            }
            let name_length = u16(&header, 26) as usize;
            let extra_length = u16(&header, 28) as usize;
            file.seek(SeekFrom::Current((name_length + extra_length) as i64))?;
            let mut crc = crc32fast::Hasher::new();
            let mut buffer = vec![0u8; 64 * 1024];
            match entry.method {
                STORED => {
                    let mut left = entry.packed;
                    while left > 0 {
                        let want = left.min(buffer.len() as u64) as usize;
                        file.read_exact(&mut buffer[..want])?;
                        crc.update(&buffer[..want]);
                        left -= want as u64;
                    }
                }
                DEFLATED => {
                    let mut decoder = DeflateDecoder::new((&mut file).take(entry.packed));
                    loop {
                        let read = decoder.read(&mut buffer)?;
                        if read == 0 {
                            break;
                        }
                        crc.update(&buffer[..read]);
                    }
                }
                other => return fail(format!("неизвестный способ сжатия {other}")),
            }
            if crc.finalize() != entry.crc {
                return fail("контрольная сумма не совпала — файл повреждён".to_string());
            }
            Ok(())
        })();
        match checked {
            Ok(()) => {
                summary.files += 1;
                summary.bytes += entry.size;
                on_entry(&entry, true);
            }
            Err(error) => {
                summary.failed += 1;
                eprintln!("! {}: {error}", entry.name);
                on_entry(&entry, false);
            }
        }
    }
    Ok(summary)
}

/// Add files and folders to an archive, creating it when it is not there yet.
///
/// An existing archive is rebuilt entry by entry: the old entries are copied byte for byte, so
/// appending a file never re-compresses the ones already inside.
pub fn add(archive: &Path, inputs: &[PathBuf], store: bool, on_entry: impl FnMut(&Entry, bool)) -> Result<Summary> {
    let target = absolute(archive)?;
    // The half-written archive gets its own name, so a crash leaves the old archive whole.
    let temporary = target.with_extension("auraarc-tmp");
    let mut collected: Vec<(PathBuf, PathBuf)> = Vec::new();
    for input in inputs {
        if !input.exists() {
            return fail(format!("нет такого файла или папки: {}", input.display()));
        }
        walk(input, input, &mut collected, 0)?;
    }
    // An archive cannot hold itself, nor the copy of itself being written next to it.
    collected.retain(|(_, path)| match absolute(path) {
        Ok(path) => path.as_path() != target && path.as_path() != temporary,
        Err(_) => true,
    });
    if collected.is_empty() {
        return fail("нечего упаковывать: папки пусты");
    }
    let outcome = write_archive(&target, &temporary, &collected, store, on_entry);
    if outcome.is_err() {
        // A failed run must not leave a broken half-archive lying about.
        let _ = fs::remove_file(&temporary);
    }
    outcome
}

fn write_archive(target: &Path, temporary: &Path, collected: &[(PathBuf, PathBuf)], store: bool, mut on_entry: impl FnMut(&Entry, bool)) -> Result<Summary> {
    let old = if target.exists() { entries(target)? } else { Vec::new() };
    if old.len() + collected.len() > u16::MAX as usize {
        return fail("в архиве было бы больше 65 535 записей: ZIP64 пока не поддерживается");
    }
    let mut known: BTreeSet<String> = old.iter().map(|entry| entry.name.clone()).collect();
    let mut out = BufWriter::new(File::create(temporary)?);
    let mut summary = Summary::default();

    // 1. What was already inside, byte for byte — nothing is re-compressed.
    let mut written_entries = copy_entries(&mut out, target, &old)?;

    // 2. The new files, in the order they were named.
    for (root, path) in collected {
        let name = entry_name(root, path)?;
        if known.contains(&name) {
            // Already inside: an archive does not silently hold one file twice.
            summary.skipped += 1;
            if !name.is_empty() {
                eprintln!("= {name}: уже в архиве, оставлен как есть");
            }
            continue;
        }
        let entry = match add_one(&mut out, path, &name, store) {
            Ok(entry) => entry,
            Err(error) => {
                let broken = Entry { name: name.clone(), method: STORED, flags: 0, crc: 0, size: 0, packed: 0, offset: 0, time: 0, date: 0, extra: Vec::new(), made_by: MADE_BY, internal: 0, external: 0 };
                eprintln!("! {name}: {error}");
                summary.failed += 1;
                on_entry(&broken, false);
                continue;
            }
        };
        known.insert(name);
        if entry.is_folder() {
            summary.folders += 1;
        } else {
            summary.files += 1;
            summary.bytes += entry.size;
            summary.packed += entry.packed;
        }
        on_entry(&entry, true);
        written_entries.push(entry);
    }

    // 3. The directory, then the pointer to it.
    finish(&mut out, &written_entries)?;
    drop(out);
    // Windows lets a rename replace the old archive, but not while another program holds it.
    commit(temporary, target)?;
    Ok(summary)
}

/// Copy the entries an archive already holds into the archive being written, byte for byte.
///
/// The local header is copied from the source too — its own name and its own extra field, which
/// may differ from the ones in the directory — and only four things in it are patched: the flags
/// (bit 3 «sizes come after the data» is cleared, because the sizes are written in the header
/// here and no descriptor follows), and the checksum and the two sizes, which are taken from the
/// directory. Everything after the header is copied as it lies: name, extra, then exactly
/// `packed` bytes of data. Writing the header *and* the source name would append the name twice
/// and shift every byte after it, which unpacks as a corrupt stream — that is what this function
/// did before, and the test `a_second_run_adds_instead_of_replacing` is what caught it.
fn copy_entries(out: &mut BufWriter<File>, source_path: &Path, old: &[Entry]) -> Result<Vec<Entry>> {
    let mut written = Vec::with_capacity(old.len());
    if old.is_empty() {
        return Ok(written);
    }
    let mut source = BufReader::new(File::open(source_path)?);
    for entry in old {
        let offset = out.stream_position()?;
        let mut head = [0u8; 30];
        source.seek(SeekFrom::Start(entry.offset))?;
        source.read_exact(&mut head)?;
        if u32(&head, 0) != LOCAL_HEADER {
            return fail(format!("повреждён архив: запись «{}» не на месте", entry.name));
        }
        let name_length = u16(&head, 26) as usize;
        let extra_length = u16(&head, 28) as usize;
        let mut moved = entry.clone();
        moved.offset = offset;
        // The directory is the authority on the checksum and the sizes; the name and the extra
        // field stay the ones the source actually wrote in front of the data.
        moved.flags = u16(&head, 6) & !SIZES_AFTER;
        head[6..8].copy_from_slice(&moved.flags.to_le_bytes());
        head[14..18].copy_from_slice(&(entry.crc as u32).to_le_bytes());
        head[18..22].copy_from_slice(&(entry.packed as u32).to_le_bytes());
        head[22..26].copy_from_slice(&(entry.size as u32).to_le_bytes());
        out.write_all(&head)?;
        // Name, extra and data — exactly as many bytes as the header just promised.
        let mut rest = (&mut source).take((name_length + extra_length) as u64 + entry.packed);
        let copied = io::copy(&mut rest, out)?;
        if copied != (name_length + extra_length) as u64 + entry.packed {
            return fail(format!("повреждён архив: запись «{}» обрывается", entry.name));
        }
        written.push(moved);
    }
    Ok(written)
}

/// The central directory, then the record that points at it.
fn finish(out: &mut BufWriter<File>, written: &[Entry]) -> Result<()> {
    let directory_offset = out.stream_position()?;
    for entry in written {
        write_central(out, entry)?;
    }
    let directory_size = out.stream_position()? - directory_offset;
    if written.len() > u16::MAX as usize || directory_size > u32::MAX as u64 || directory_offset > u32::MAX as u64 {
        return fail("архив вышел больше 4 ГБ или длиннее 65 535 записей: ZIP64 пока не поддерживается");
    }
    out.write_all(&END_OF_DIRECTORY.to_le_bytes())?;
    out.write_all(&0u16.to_le_bytes())?; // this disk
    out.write_all(&0u16.to_le_bytes())?; // where the directory starts
    out.write_all(&(written.len() as u16).to_le_bytes())?;
    out.write_all(&(written.len() as u16).to_le_bytes())?;
    out.write_all(&(directory_size as u32).to_le_bytes())?;
    out.write_all(&(directory_offset as u32).to_le_bytes())?;
    out.write_all(&0u16.to_le_bytes())?; // no archive comment
    out.flush()?;
    Ok(())
}

fn commit(temporary: &Path, target: &Path) -> Result<()> {
    fs::rename(temporary, target)
        .map_err(|error| Error(format!("не удалось записать {}: {error}", target.display())))
}

/// Take entries out of an archive: the rest is copied byte for byte, so nothing inside is
/// re-compressed and every other entry keeps its stamp and attributes.
pub fn remove(archive: &Path, names: &[String]) -> Result<Summary> {
    let target = absolute(archive)?;
    let old = entries(&target)?;
    let mut kept = Vec::with_capacity(old.len());
    let mut summary = Summary::default();
    for entry in old {
        if picked(&entry, names) {
            if entry.is_folder() {
                summary.folders += 1;
            } else {
                summary.files += 1;
                summary.bytes += entry.size;
                summary.packed += entry.packed;
            }
        } else {
            kept.push(entry);
        }
    }
    if summary.files + summary.folders == 0 {
        return fail(format!("в «{}» нет записей с такими именами", target.display()));
    }
    let temporary = target.with_extension("auraarc-tmp");
    let outcome = (|| -> Result<()> {
        let mut out = BufWriter::new(File::create(&temporary)?);
        let written = copy_entries(&mut out, &target, &kept)?;
        finish(&mut out, &written)?;
        drop(out);
        commit(&temporary, &target)
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    outcome?;
    Ok(summary)
}

/// How deep a folder tree is followed: a junction pointing back up would otherwise loop for
/// ever, and no honest archive is nested this deep.
const MAX_DEPTH: usize = 64;

/// Everything under `path`, folders included, in a stable order, each file paired with the
/// input it came from — the pair is what names it inside the archive, so two folders called
/// `docs` at different depths cannot be confused with one another.
fn walk(root: &Path, path: &Path, out: &mut Vec<(PathBuf, PathBuf)>, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return fail(format!("слишком глубокая папка: {}", path.display()));
    }
    let metadata = fs::symlink_metadata(path)?;
    // A junction or a symbolic link is copied as the link target's contents only when it is an
    // ordinary file; a linked *folder* is skipped, because following it can walk in circles.
    if metadata.file_type().is_symlink() && path.is_dir() {
        eprintln!("~ {}: ссылка на папку пропущена", path.display());
        return Ok(());
    }
    if metadata.is_dir() {
        out.push((root.to_path_buf(), path.to_path_buf()));
        let mut children: Vec<PathBuf> = fs::read_dir(path)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect();
        children.sort();
        for child in children {
            walk(root, &child, out, depth + 1)?;
        }
    } else {
        out.push((root.to_path_buf(), path.to_path_buf()));
    }
    Ok(())
}

/// The name an input gets inside the archive: the file's own name, or a folder's whole tree
/// starting at the folder itself — `Документы/вложено/б.txt`, never a path from the disk.
///
/// The name is taken from the input the file was actually walked from, so a folder called
/// `docs` that contains another `docs` keeps its depth instead of collapsing onto the first
/// match.
fn entry_name(root: &Path, path: &Path) -> Result<String> {
    let root = absolute(root)?;
    let path = absolute(path)?;
    let mut parts = vec![name_of(&root)?];
    if let Ok(rest) = path.strip_prefix(&root) {
        for component in rest.components() {
            if let Component::Normal(part) = component {
                parts.push(part.to_string_lossy().into_owned());
            }
        }
    }
    Ok(parts.join("/"))
}

/// A single name, the last piece of a path.
fn name_of(path: &Path) -> Result<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| Error(format!("не удалось назвать {}", path.display())))
}

fn add_one(out: &mut BufWriter<File>, path: &Path, name: &str, store: bool) -> Result<Entry> {
    let metadata = fs::metadata(path)?;
    let (time, date) = dos_time(metadata.modified().ok());
    if metadata.is_dir() {
        let entry = Entry {
            name: format!("{}/", name.trim_end_matches('/')),
            method: STORED,
            flags: UTF8_NAME,
            crc: 0,
            size: 0,
            packed: 0,
            offset: out.stream_position()?,
            time,
            date,
            extra: Vec::new(),
            made_by: MADE_BY,
            internal: 0,
            external: ATTR_FOLDER,
        };
        write_local(out, &entry)?;
        return Ok(entry);
    }
    if metadata.len() > u32::MAX as u64 {
        return fail("файл больше 4 ГБ: ZIP64 пока не поддерживается");
    }
    let offset = out.stream_position()?;
    let method = if store { STORED } else { DEFLATED };
    // The header goes out first with zeros — the checksum is only known after the data — and is
    // patched in place afterwards. A real header beats a data descriptor: every unpacker, old
    // and new, reads it without streaming tricks.
    let placeholder = Entry {
        name: name.to_string(),
        method,
        flags: UTF8_NAME,
        crc: 0,
        size: 0,
        packed: 0,
        offset,
        time,
        date,
        extra: Vec::new(),
        made_by: MADE_BY,
        internal: 0,
        external: ATTR_FILE,
    };
    write_local(out, &placeholder)?;

    let mut source = BufReader::new(File::open(path)?);
    let mut crc = crc32fast::Hasher::new();
    let mut buffer = vec![0u8; 256 * 1024];
    let start = out.stream_position()?;
    let mut size = 0u64;
    if store {
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            crc.update(&buffer[..read]);
            out.write_all(&buffer[..read])?;
            size += read as u64;
        }
    } else {
        let mut encoder = DeflateEncoder::new(&mut *out, Compression::new(6));
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            crc.update(&buffer[..read]);
            encoder.write_all(&buffer[..read])?;
            size += read as u64;
        }
        encoder.finish()?;
    }
    let end = out.stream_position()?;
    let packed = end - start;
    // Over 4 GB of output is possible even from a small file of zeros; refuse rather than
    // write an archive that lies about its sizes.
    if packed > u32::MAX as u64 {
        return fail("сжатые данные больше 4 ГБ: ZIP64 пока не поддерживается");
    }
    if size > u32::MAX as u64 {
        return fail("файл больше 4 ГБ: ZIP64 пока не поддерживается");
    }
    let filled = Entry { crc: crc.finalize(), size, packed, ..placeholder };
    out.seek(SeekFrom::Start(offset))?;
    write_local(out, &filled)?;
    out.seek(SeekFrom::Start(end))?;
    Ok(filled)
}

fn write_local(out: &mut BufWriter<File>, entry: &Entry) -> Result<()> {
    let name = entry.name.as_bytes();
    out.write_all(&LOCAL_HEADER.to_le_bytes())?;
    out.write_all(&20u16.to_le_bytes())?; // version needed to unpack
    out.write_all(&entry.flags.to_le_bytes())?;
    out.write_all(&entry.method.to_le_bytes())?;
    out.write_all(&entry.time.to_le_bytes())?;
    out.write_all(&entry.date.to_le_bytes())?;
    out.write_all(&entry.crc.to_le_bytes())?;
    out.write_all(&(entry.packed as u32).to_le_bytes())?;
    out.write_all(&(entry.size as u32).to_le_bytes())?;
    out.write_all(&(name.len() as u16).to_le_bytes())?;
    out.write_all(&(entry.extra.len() as u16).to_le_bytes())?;
    out.write_all(name)?;
    out.write_all(&entry.extra)?;
    Ok(())
}

fn write_central(out: &mut BufWriter<File>, entry: &Entry) -> Result<()> {
    let name = entry.name.as_bytes();
    out.write_all(&CENTRAL_HEADER.to_le_bytes())?;
    out.write_all(&entry.made_by.to_le_bytes())?; // made by
    out.write_all(&20u16.to_le_bytes())?; // needed to unpack
    out.write_all(&entry.flags.to_le_bytes())?;
    out.write_all(&entry.method.to_le_bytes())?;
    out.write_all(&entry.time.to_le_bytes())?;
    out.write_all(&entry.date.to_le_bytes())?;
    out.write_all(&entry.crc.to_le_bytes())?;
    out.write_all(&(entry.packed as u32).to_le_bytes())?;
    out.write_all(&(entry.size as u32).to_le_bytes())?;
    out.write_all(&(name.len() as u16).to_le_bytes())?;
    out.write_all(&(entry.extra.len() as u16).to_le_bytes())?;
    out.write_all(&0u16.to_le_bytes())?; // no comment
    out.write_all(&0u16.to_le_bytes())?; // first disk
    out.write_all(&entry.internal.to_le_bytes())?; // internal attributes
    out.write_all(&entry.external.to_le_bytes())?; // external attributes
    out.write_all(&(entry.offset as u32).to_le_bytes())?;
    out.write_all(name)?;
    out.write_all(&entry.extra)?;
    Ok(())
}

/// A path from the archive as a path under the destination, or `None` when it would escape.
pub fn safe_relative(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    let mut relative = PathBuf::new();
    for part in name.split('/') {
        match part {
            "" | "." => continue,
            // `..` anywhere, and anything that looks like a drive, would leave the folder.
            ".." => return None,
            part if part.contains(':') => return None,
            part => relative.push(part),
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

/// The destination itself, absolute, so the check above is about the truth.
fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(path))
}

/// A time the format can hold: two-second steps from 1980, in **local** time — the way Explorer,
/// 7-Zip and every other zip tool write it. Stored as UTC, the same archive would show every
/// file shifted by the time-zone offset on the other machine.
fn dos_time(modified: Option<SystemTime>) -> (u16, u16) {
    let (year, month, day, hour, minute, second) =
        modified.and_then(local_parts).unwrap_or((1980, 1, 1, 0, 0, 0));
    let year = year.clamp(1980, 2107) as u16;
    let date = ((year - 1980) << 9) | ((month as u16) << 5) | day as u16;
    let time = ((hour as u16) << 11) | ((minute as u16) << 5) | (second as u16 / 2);
    (time, date)
}

/// A moment as the calendar here says it, daylight saving included.
fn local_parts(time: SystemTime) -> Option<(u32, u32, u32, u32, u32, u32)> {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let since = time.duration_since(UNIX_EPOCH).ok()?;
    // A FILETIME counts 100 ns steps from 1601-01-01; the Unix epoch is 11 644 473 600 s later.
    let ticks = since.as_secs().checked_add(11_644_473_600)?.checked_mul(10_000_000)?
        + u64::from(since.subsec_nanos() / 100);
    let file = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    unsafe { FileTimeToSystemTime(&file, &mut utc).ok()? };
    let mut local = SYSTEMTIME::default();
    unsafe { SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()? };
    Some((
        local.wYear as u32,
        local.wMonth as u32,
        local.wDay as u32,
        local.wHour as u32,
        local.wMinute as u32,
        local.wSecond as u32,
    ))
}

/// Put the archived stamp back on the unpacked file: a restored folder should not look as if
/// everything in it was created this very minute.
fn stamp(file: &File, entry: &Entry) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{FILETIME, HANDLE, SYSTEMTIME};
    use windows::Win32::Storage::FileSystem::SetFileTime;
    use windows::Win32::System::Time::{SystemTimeToFileTime, TzSpecificLocalTimeToSystemTime};
    let year = 1980 + (entry.date >> 9) as u16;
    let month = (entry.date >> 5) & 0x0f;
    let day = entry.date & 0x1f;
    if month == 0 || day == 0 {
        return; // no time was stored with this entry
    }
    let local = SYSTEMTIME {
        wYear: year,
        wMonth: month,
        wDay: day,
        wHour: entry.time >> 11,
        wMinute: (entry.time >> 5) & 0x3f,
        wSecond: (entry.time & 0x1f) * 2,
        ..Default::default()
    };
    let mut utc = SYSTEMTIME::default();
    let mut written = FILETIME::default();
    unsafe {
        if TzSpecificLocalTimeToSystemTime(None, &local, &mut utc).is_err() {
            return;
        }
        if SystemTimeToFileTime(&utc, &mut written).is_err() {
            return;
        }
        let _ = SetFileTime(HANDLE(file.as_raw_handle() as _), None, None, Some(&written));
    }
}

fn u16(buffer: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buffer[at], buffer[at + 1]])
}

fn u32(buffer: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buffer[at], buffer[at + 1], buffer[at + 2], buffer[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("auraarc-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_file_survives_a_round_trip() {
        let temp = Temp::new("round-trip");
        let source = temp.0.join("письмо.txt");
        let text = "Здравствуйте!\n".repeat(500);
        fs::write(&source, &text).unwrap();
        let archive = temp.0.join("архив.zip");
        let summary = add(&archive, &[source.clone()], false, |_, _| {}).unwrap();
        assert_eq!(summary.files, 1);
        assert_eq!(summary.failed, 0);
        let listed = list(&archive).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "письмо.txt");
        assert_eq!(listed[0].size, text.len() as u64);
        // Repetitive text must actually compress.
        assert!(listed[0].packed < listed[0].size / 2, "packed {} vs {}", listed[0].packed, listed[0].size);

        let out = temp.0.join("распаковано");
        let extracted = extract(&archive, &out, |_, _| {}).unwrap();
        assert_eq!(extracted.files, 1);
        assert_eq!(fs::read_to_string(out.join("письмо.txt")).unwrap(), text);
    }

    #[test]
    fn a_folder_keeps_its_shape() {
        let temp = Temp::new("folder");
        let folder = temp.0.join("документы");
        fs::create_dir_all(folder.join("вложено")).unwrap();
        fs::write(folder.join("а.txt"), "а").unwrap();
        fs::write(folder.join("вложено").join("б.txt"), "б").unwrap();
        let archive = temp.0.join("папка.zip");
        add(&archive, &[folder], false, |_, _| {}).unwrap();
        let names: Vec<String> = list(&archive).unwrap().into_iter().map(|entry| entry.name).collect();
        assert!(names.contains(&"документы/".to_string()), "{names:?}");
        assert!(names.contains(&"документы/а.txt".to_string()), "{names:?}");
        assert!(names.contains(&"документы/вложено/б.txt".to_string()), "{names:?}");
        let out = temp.0.join("куда");
        extract(&archive, &out, |_, _| {}).unwrap();
        assert_eq!(fs::read_to_string(out.join("документы").join("вложено").join("б.txt")).unwrap(), "б");
    }

    #[test]
    fn a_second_run_adds_instead_of_replacing() {
        let temp = Temp::new("append");
        let first = temp.0.join("первый.txt");
        let second = temp.0.join("второй.txt");
        fs::write(&first, "раз".repeat(100)).unwrap();
        fs::write(&second, "два".repeat(100)).unwrap();
        let archive = temp.0.join("оба.zip");
        add(&archive, &[first], false, |_, _| {}).unwrap();
        let before = list(&archive).unwrap()[0].clone();
        add(&archive, &[second], false, |_, _| {}).unwrap();
        let listed = list(&archive).unwrap();
        assert_eq!(listed.len(), 2);
        // The entry already inside came through untouched (same checksum and packed size).
        let kept = listed.iter().find(|entry| entry.name == "первый.txt").unwrap();
        assert_eq!(kept.crc, before.crc);
        assert_eq!(kept.packed, before.packed);
        // The copied entry has to be readable straight away, not only at the end: a local header
        // that disagrees with the bytes behind it is exactly what unpacks as "corrupt deflate
        // stream", and it is the mistake this test was written for.
        let after_second = temp.0.join("после-второго");
        extract(&archive, &after_second, |_, _| {}).unwrap();
        assert_eq!(fs::read_to_string(after_second.join("первый.txt")).unwrap(), "раз".repeat(100));
        assert_eq!(fs::read_to_string(after_second.join("второй.txt")).unwrap(), "два".repeat(100));
        // And adding the same file twice does not make the archive hold it twice.
        add(&archive, &[temp.0.join("первый.txt")], false, |_, _| {}).unwrap();
        assert_eq!(list(&archive).unwrap().len(), 2);
        let out = temp.0.join("проверка");
        extract(&archive, &out, |_, _| {}).unwrap();
        assert_eq!(fs::read_to_string(out.join("второй.txt")).unwrap(), "два".repeat(100));
    }

    #[test]
    fn a_stored_archive_is_readable_too() {
        let temp = Temp::new("stored");
        let source = temp.0.join("файл.bin");
        fs::write(&source, vec![7u8; 4096]).unwrap();
        let archive = temp.0.join("без-сжатия.zip");
        add(&archive, &[source], true, |_, _| {}).unwrap();
        let entry = list(&archive).unwrap().remove(0);
        assert_eq!(entry.method, STORED);
        assert_eq!(entry.size, 4096);
        assert_eq!(entry.packed, 4096);
        let out = temp.0.join("out");
        extract(&archive, &out, |_, _| {}).unwrap();
        assert_eq!(fs::read(out.join("файл.bin")).unwrap(), vec![7u8; 4096]);
    }

    #[test]
    fn a_damaged_entry_is_caught() {
        let temp = Temp::new("damage");
        let source = temp.0.join("текст.txt");
        fs::write(&source, "данные".repeat(200)).unwrap();
        let archive = temp.0.join("битый.zip");
        add(&archive, &[source], false, |_, _| {}).unwrap();
        // Flip a byte in the compressed data, after the local header.
        let mut bytes = fs::read(&archive).unwrap();
        let at = 30 + "текст.txt".len() + 5;
        bytes[at] ^= 0xFF;
        fs::write(&archive, &bytes).unwrap();
        let summary = test(&archive, |_, _| {}).unwrap();
        assert_eq!(summary.failed, 1, "a broken entry must not pass the check");
        let out = temp.0.join("no");
        let extracted = extract(&archive, &out, |_, _| {}).unwrap();
        assert_eq!(extracted.failed, 1);
    }

    #[test]
    fn names_may_not_leave_the_destination() {
        assert_eq!(safe_relative("папка/файл.txt"), Some(PathBuf::from("папка").join("файл.txt")));
        assert_eq!(safe_relative("папка\\файл.txt"), Some(PathBuf::from("папка").join("файл.txt")));
        assert_eq!(safe_relative("../../windows/system32/evil.dll"), None);
        assert_eq!(safe_relative("/etc/passwd"), Some(PathBuf::from("etc").join("passwd")));
        assert_eq!(safe_relative("C:/windows/evil.dll"), None);
        assert_eq!(safe_relative(""), None);
    }

    #[test]
    fn a_stamp_is_written_in_local_time() {
        // 2026-11-19 14:03:04 UTC: whatever the time zone here, the archive must hold the
        // calendar the person sitting at the machine would read, not the UTC one.
        let moment = UNIX_EPOCH + std::time::Duration::from_secs(1_795_096_984);
        let (year, month, day, hour, _, _) = local_parts(moment).unwrap();
        let (time, date) = dos_time(Some(moment));
        assert_eq!(1980 + (date >> 9), year as u16);
        assert_eq!((date >> 5) & 0x0f, month as u16);
        assert_eq!(date & 0x1f, day as u16);
        assert_eq!(time >> 11, hour as u16);
    }

    #[test]
    fn an_unknown_time_still_makes_a_valid_date() {
        let (time, date) = dos_time(None);
        assert_eq!(time, 0);
        assert_eq!(1980 + (date >> 9), 1980);
        assert_eq!((date >> 5) & 0x0f, 1);
        assert_eq!(date & 0x1f, 1);
    }

    #[test]
    fn folders_are_counted_apart_from_files() {
        let temp = Temp::new("folders-counted");
        let folder = temp.0.join("проект");
        fs::create_dir_all(folder.join("исходники")).unwrap();
        fs::write(folder.join("читай.txt"), "текст").unwrap();
        fs::write(folder.join("исходники").join("код.rs"), "fn main() {}").unwrap();
        let archive = temp.0.join("проект.zip");
        let packed = add(&archive, &[folder], false, |_, _| {}).unwrap();
        // Two folders (проект, исходники) and two files — never «4 файла».
        assert_eq!(packed.folders, 2, "folders {}", packed.folders);
        assert_eq!(packed.files, 2, "files {}", packed.files);

        let out = temp.0.join("распаковано");
        let unpacked = extract(&archive, &out, |_, _| {}).unwrap();
        assert_eq!(unpacked.folders, 2);
        assert_eq!(unpacked.files, 2);

        let checked = test(&archive, |_, _| {}).unwrap();
        assert_eq!(checked.folders, 2);
        assert_eq!(checked.files, 2);
        assert_eq!(checked.failed, 0);
    }

    #[test]
    fn a_folder_entry_says_it_is_a_folder() {
        let temp = Temp::new("folder-attributes");
        let folder = temp.0.join("папка");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("файл.txt"), "x").unwrap();
        let archive = temp.0.join("attrs.zip");
        add(&archive, &[folder], false, |_, _| {}).unwrap();
        let listed = list(&archive).unwrap();
        let folder_entry = listed.iter().find(|entry| entry.is_folder()).unwrap();
        let file_entry = listed.iter().find(|entry| !entry.is_folder()).unwrap();
        // The DOS attribute byte is what other unpackers read to tell a folder from a file.
        assert_eq!(folder_entry.external & 0xff, ATTR_FOLDER);
        assert_eq!(file_entry.external & 0xff, ATTR_FILE);
        assert_eq!(folder_entry.made_by, MADE_BY);
    }

    #[test]
    fn removing_a_row_leaves_the_rest_whole() {
        let temp = Temp::new("remove");
        let first = temp.0.join("остаётся.txt");
        let second = temp.0.join("уходит.txt");
        fs::write(&first, "один".repeat(200)).unwrap();
        fs::write(&second, "два".repeat(200)).unwrap();
        let archive = temp.0.join("оба.zip");
        add(&archive, &[first.clone(), second], false, |_, _| {}).unwrap();
        let before = list(&archive).unwrap();
        let leaving = before.iter().find(|entry| entry.name == "уходит.txt").unwrap().clone();

        let summary = remove(&archive, &["уходит.txt".to_string()]).unwrap();
        assert_eq!(summary.files, 1);
        let after = list(&archive).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name, "остаётся.txt");
        // The entry that stayed came through byte for byte.
        let stayed = before.iter().find(|entry| entry.name == "остаётся.txt").unwrap();
        assert_eq!(after[0].crc, stayed.crc);
        assert_eq!(after[0].packed, stayed.packed);
        assert_ne!(after[0].offset, leaving.offset);
        // And the archive still unpacks.
        let out = temp.0.join("проверка");
        extract(&archive, &out, |_, _| {}).unwrap();
        assert!(out.join("остаётся.txt").exists());
        assert!(!out.join("уходит.txt").exists());
    }

    #[test]
    fn removing_a_folder_takes_what_is_inside_it() {
        let temp = Temp::new("remove-folder");
        let folder = temp.0.join("мусор");
        fs::create_dir_all(folder.join("глубже")).unwrap();
        fs::write(folder.join("глубже").join("файл.txt"), "x").unwrap();
        let keeps = temp.0.join("нужно.txt");
        fs::write(&keeps, "y").unwrap();
        let archive = temp.0.join("всё.zip");
        add(&archive, &[folder, keeps], false, |_, _| {}).unwrap();
        let summary = remove(&archive, &["мусор/".to_string()]).unwrap();
        assert_eq!(summary.folders, 2);
        assert_eq!(summary.files, 1);
        let names: Vec<String> = list(&archive).unwrap().into_iter().map(|entry| entry.name).collect();
        assert_eq!(names, vec!["нужно.txt".to_string()]);
    }

    #[test]
    fn copying_one_row_unpacks_only_that_row() {
        let temp = Temp::new("extract-only");
        let first = temp.0.join("первый.txt");
        let second = temp.0.join("второй.txt");
        fs::write(&first, "раз").unwrap();
        fs::write(&second, "два").unwrap();
        let archive = temp.0.join("два.zip");
        add(&archive, &[first, second], false, |_, _| {}).unwrap();
        let out = temp.0.join("копия");
        let summary = extract_only(&archive, &out, &["второй.txt".to_string()], |_, _| {}).unwrap();
        assert_eq!(summary.files, 1);
        assert!(out.join("второй.txt").exists());
        assert!(!out.join("первый.txt").exists());
    }

    #[test]
    fn extracting_a_selected_folder_keeps_its_tree_and_ignores_siblings() {
        let temp = Temp::new("extract-folder-only");
        let chosen = temp.0.join("chosen");
        let other = temp.0.join("other");
        fs::create_dir_all(chosen.join("nested")).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(chosen.join("nested").join("inside.txt"), "inside").unwrap();
        fs::write(other.join("outside.txt"), "outside").unwrap();
        let archive = temp.0.join("folders.zip");
        add(&archive, &[chosen, other], false, |_, _| {}).unwrap();

        let out = temp.0.join("selected");
        let summary = extract_only(&archive, &out, &["chosen/".to_string()], |_, _| {}).unwrap();
        assert_eq!(summary.files, 1);
        assert_eq!(summary.folders, 2);
        assert_eq!(fs::read_to_string(out.join("chosen/nested/inside.txt")).unwrap(), "inside");
        assert!(!out.join("other/outside.txt").exists());
    }

    #[test]
    fn a_folder_inside_a_folder_of_the_same_name_keeps_its_depth() {
        let folder = Path::new(r"C:\работа\docs");
        let nested = Path::new(r"C:\работа\docs\docs");
        assert_eq!(entry_name(folder, folder).unwrap(), "docs");
        assert_eq!(entry_name(folder, &nested.join("б.txt")).unwrap(), "docs/docs/б.txt");
    }
}
