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
    pub bytes: u64,
    pub packed: u64,
    pub failed: usize,
    /// Files that were already in the archive and were therefore left alone.
    pub skipped: usize,
}

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_DIRECTORY: u32 = 0x0605_4b50;
const DESCRIPTOR: u32 = 0x0807_4b50;
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
}

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
    let base = length - window;
    let directory_size = u32(&tail, at + 12) as u64;
    let directory_offset = u32(&tail, at + 16) as u64;
    let count = u16(&tail, at + 10) as usize;
    if directory_offset == u32::MAX as u64 || directory_size == u32::MAX as u64 || count == u16::MAX as usize {
        return fail(format!("{} — архив ZIP64; такие пока не поддерживаются", archive.display()));
    }
    let _ = base;
    // Defensive: a truncated or hand-edited file must not make us allocate wildly.
    if directory_size > length {
        return fail(format!("{} повреждён: оглавление больше самого файла", archive.display()));
    }
    file.seek(SeekFrom::Start(directory_offset))?;
    let mut directory = vec![0u8; directory_size as usize];
    file.read_exact(&mut directory)?;
    parse_directory(&directory).map(|entries| {
        let _ = count;
        entries
    })
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
        entries.push(Entry { name, method, flags, crc, size, packed, offset, time, date, extra });
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
    let entries = entries(archive)?;
    let mut file = BufReader::new(File::open(archive)?);
    let mut summary = Summary::default();
    // Nothing may be written outside `dest`, however the archive was made.
    let root = absolute(dest)?;
    for entry in entries {
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
    let mut copy = |chunk: &[u8], crc: &mut crc32fast::Hasher, out: &mut BufWriter<File>, written: &mut u64| -> Result<()> {
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
            let mut decoder = DeflateDecoder::new(file.take(entry.packed));
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
    if crc.finalize() != entry.crc {
        let _ = fs::remove_file(target);
        return fail("контрольная сумма не совпала — файл повреждён".to_string());
    }
    // The directory's name may end a folder that the entry itself did not create.
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// Check every entry without writing anything.
pub fn test(archive: &Path, mut on_entry: impl FnMut(&Entry, bool)) -> Result<Summary> {
    let entries = entries(archive)?;
    let mut file = BufReader::new(File::open(archive)?);
    let mut summary = Summary::default();
    for entry in entries {
        if entry.is_folder() {
            summary.files += 1;
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
                    let mut decoder = DeflateDecoder::new(file.take(entry.packed));
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
pub fn add(archive: &Path, inputs: &[PathBuf], store: bool, mut on_entry: impl FnMut(&Entry, bool)) -> Result<Summary> {
    let mut collected: Vec<PathBuf> = Vec::new();
    for input in inputs {
        if !input.exists() {
            return fail(format!("нет такого файла или папки: {}", input.display()));
        }
        walk(input, &mut collected)?;
    }
    if collected.is_empty() {
        return fail("нечего упаковывать: папки пусты");
    }
    let temporary = archive.with_extension("auraarc-tmp");
    let old = if archive.exists() { entries(archive)? } else { Vec::new() };
    let mut known: BTreeSet<String> = old.iter().map(|entry| entry.name.clone()).collect();
    let mut out = BufWriter::new(File::create(&temporary)?);
    let mut summary = Summary::default();
    let mut written_entries: Vec<Entry> = Vec::with_capacity(old.len() + collected.len());

    // 1. The entries already in the archive, copied as they are.
    if !old.is_empty() {
        let mut source = BufReader::new(File::open(archive)?);
        for entry in &old {
            let offset = out.stream_position()?;
            let mut head = [0u8; 30];
            source.seek(SeekFrom::Start(entry.offset))?;
            source.read_exact(&mut head)?;
            if u32(&head, 0) != LOCAL_HEADER {
                return fail(format!("повреждён архив: запись «{}» не на месте", entry.name));
            }
            let name_length = u16(&head, 26) as usize;
            let extra_length = u16(&head, 28) as usize;
            out.write_all(&head)?;
            let mut skip = vec![0u8; name_length + extra_length];
            source.read_exact(&mut skip)?;
            out.write_all(&skip)?;
            let mut data = (&mut source).take(entry.packed);
            io::copy(&mut data, &mut out)?;
            let mut moved = entry.clone();
            moved.offset = offset;
            written_entries.push(moved);
        }
    }

    // 2. The new files, in the order they were named.
    for path in &collected {
        let name = entry_name(path, inputs)?;
        if known.contains(&name) {
            // Already inside: an archive does not silently hold one file twice.
            summary.skipped += 1;
            eprintln!("= {name}: уже в архиве, оставлен как есть");
            continue;
        }
        let entry = match add_one(&mut out, path, &name, store) {
            Ok(entry) => entry,
            Err(error) => {
                let broken = Entry { name: name.clone(), method: STORED, flags: 0, crc: 0, size: 0, packed: 0, offset: 0, time: 0, date: 0, extra: Vec::new() };
                eprintln!("! {name}: {error}");
                summary.failed += 1;
                on_entry(&broken, false);
                continue;
            }
        };
        known.insert(name);
        summary.files += 1;
        summary.bytes += entry.size;
        summary.packed += entry.packed;
        on_entry(&entry, true);
        written_entries.push(entry);
    }
    // 3. The directory, then the record that points at it.
    let directory_offset = out.stream_position()?;
    for entry in &written_entries {
        write_central(&mut out, entry)?;
    }
    let directory_size = out.stream_position()? - directory_offset;
    if written_entries.len() > u16::MAX as usize {
        return fail("в архиве больше 65 535 файлов: ZIP64 пока не поддерживается");
    }
    out.write_all(&END_OF_DIRECTORY.to_le_bytes())?;
    out.write_all(&0u16.to_le_bytes())?; // this disk
    out.write_all(&0u16.to_le_bytes())?; // where the directory starts
    out.write_all(&(written_entries.len() as u16).to_le_bytes())?;
    out.write_all(&(written_entries.len() as u16).to_le_bytes())?;
    out.write_all(&(directory_size.min(u32::MAX as u64) as u32).to_le_bytes())?;
    out.write_all(&(directory_offset.min(u32::MAX as u64) as u32).to_le_bytes())?;
    out.write_all(&0u16.to_le_bytes())?; // no archive comment
    out.flush()?;
    drop(out);
    fs::rename(&temporary, archive).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        Error(format!("не удалось записать {}: {error}", archive.display()))
    })?;
    Ok(summary)
}

/// Everything under `path`, folders included, in a stable order.
fn walk(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if path.is_dir() {
        out.push(path.to_path_buf());
        let mut children: Vec<PathBuf> = fs::read_dir(path)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect();
        children.sort();
        for child in children {
            walk(&child, out)?;
        }
    } else {
        out.push(path.to_path_buf());
    }
    Ok(())
}

/// The name an input gets inside the archive: the file's own name, or a folder's whole tree
/// starting at the folder itself — `Документы/вложено/б.txt`, never a path from the disk.
fn entry_name(path: &Path, inputs: &[PathBuf]) -> Result<String> {
    let parts: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        return fail(format!("не удалось назвать {}", path.display()));
    }
    // The input's own name marks where the archive's copy of the tree begins.
    let starts: Vec<String> = inputs
        .iter()
        .filter_map(|input| input.file_name().map(|name| name.to_string_lossy().into_owned()))
        .filter(|name| !name.is_empty())
        .collect();
    let chosen = match parts.iter().rposition(|part| starts.iter().any(|start| start == part)) {
        Some(at) => &parts[at..],
        None => &parts[parts.len() - 1..],
    };
    Ok(chosen.join("/"))
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
        };
        write_local(out, &entry)?;
        return Ok(entry);
    }
    if metadata.len() > u32::MAX as u64 {
        return fail("файл больше 4 ГБ: ZIP64 пока не поддерживается");
    }
    let offset = out.stream_position()?;
    let flags = UTF8_NAME | SIZES_AFTER;
    let method = if store { STORED } else { DEFLATED };
    // The header goes out first with zeros: the checksum is only known after the data.
    let placeholder = Entry {
        name: name.to_string(),
        method,
        flags,
        crc: 0,
        size: 0,
        packed: 0,
        offset,
        time,
        date,
        extra: Vec::new(),
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
    let packed = out.stream_position()? - start;
    // Over 4 GB of output is possible even from a small file of zeros; refuse rather than
    // write an archive that lies about its sizes.
    if packed > u32::MAX as u64 {
        return fail("сжатые данные больше 4 ГБ: ZIP64 пока не поддерживается");
    }
    let crc = crc.finalize();
    out.write_all(&DESCRIPTOR.to_le_bytes())?;
    out.write_all(&crc.to_le_bytes())?;
    out.write_all(&(packed as u32).to_le_bytes())?;
    out.write_all(&(size as u32).to_le_bytes())?;
    Ok(Entry { crc, size, packed, ..placeholder })
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
    out.write_all(&20u16.to_le_bytes())?; // made by
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
    out.write_all(&0u16.to_le_bytes())?; // internal attributes
    out.write_all(&0u32.to_le_bytes())?; // external attributes
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

/// A time the format can hold: two-second steps, from 1980.
fn dos_time(modified: Option<SystemTime>) -> (u16, u16) {
    let seconds = modified
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let (year, month, day, hour, minute, second) = civil(seconds);
    let year = year.clamp(1980, 2107) as u16;
    let date = ((year - 1980) << 9) | ((month as u16) << 5) | day as u16;
    let time = ((hour as u16) << 11) | ((minute as u16) << 5) | (second as u16 / 2);
    (time, date)
}

/// Seconds since the epoch as a calendar date and time, in UTC.
fn civil(seconds: u64) -> (i64, u32, u32, u32, u32, u32) {
    // Days from 1970-01-01, in the proleptic Gregorian calendar.
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (hour, minute, second) = ((rest / 3600) as u32, ((rest % 3600) / 60) as u32, (rest % 60) as u32);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month + 2) / 5 + 1;
    let month = month + if month < 10 { 3 } else { -9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32, hour, minute, second)
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
    fn a_calendar_date_comes_out_of_the_count() {
        // 2026-11-19 14:03:04 UTC
        let (year, month, day, hour, minute, second) = civil(1_795_096_984);
        assert_eq!((year, month, day), (2026, 11, 19));
        assert_eq!((hour, minute, second), (14, 3, 4));
        let (time, date) = dos_time(Some(UNIX_EPOCH + std::time::Duration::from_secs(1_795_096_984)));
        let year = 1980 + (date >> 9);
        assert_eq!(year, 2026);
        assert_eq!(date & 0x1F, 19);
        assert_eq!(time >> 11, 14);
    }
}
