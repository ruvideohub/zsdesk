//! Import a user-selected UTF-8 M3U station catalogue off the UI thread.
//! Import never modifies the source file and only replaces the saved copy after validation.
use std::io::Read;
use std::path::Path;
use crate::radio_playlist::{self, Station};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OPENFILENAMEW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR};

pub enum Choice { Cancelled, Failed, Loaded(Vec<Station>) }

pub fn path() -> std::path::PathBuf { crate::config::dir().join("radio-playlist.m3u") }

fn read_bounded(path: &Path) -> Option<(Vec<u8>, Vec<Station>)> {
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take((radio_playlist::MAX_PLAYLIST_BYTES + 1) as u64).read_to_end(&mut bytes).ok()?;
    let stations = radio_playlist::parse(&bytes).ok()?;
    if stations.is_empty() { return None; }
    Some((bytes, stations))
}

pub fn load_saved() -> Option<Vec<Station>> { read_bounded(&path()).map(|(_, stations)| stations) }

/// A common file dialog on a worker's STA avoids nesting the Dock's RefCell message loop.
pub fn choose() -> Choice {
    let filter: Vec<u16> = "M3U playlists\0*.m3u;*.m3u8\0All files\0*.*\0\0".encode_utf16().collect();
    let title: Vec<u16> = "Select station playlist (UTF-8 M3U)\0".encode_utf16().collect();
    let mut filename = vec![0u16; 32768];
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrFile: PWSTR(filename.as_mut_ptr()),
        nMaxFile: filename.len() as u32,
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if !unsafe { GetOpenFileNameW(&mut dialog) }.as_bool() { return Choice::Cancelled; }
    let end = filename.iter().position(|&c| c == 0).unwrap_or(filename.len());
    use std::os::windows::ffi::OsStringExt;
    let selected = std::path::PathBuf::from(std::ffi::OsString::from_wide(&filename[..end]));
    let Some((bytes, stations)) = read_bounded(&selected) else { return Choice::Failed };
    let target = path();
    let Some(dir) = target.parent() else { return Choice::Failed };
    if std::fs::create_dir_all(dir).is_err() { return Choice::Failed; }
    // Write a new file before touching the old import. Windows ReplaceFile is not
    // needed: a backup allows restoration if the final rename fails.
    let temp = target.with_extension("m3u.new");
    let backup = target.with_extension("m3u.bak");
    if std::fs::write(&temp, bytes).is_err() { return Choice::Failed; }
    let had_old = target.exists();
    if had_old {
        let _ = std::fs::remove_file(&backup);
        if std::fs::rename(&target, &backup).is_err() { let _ = std::fs::remove_file(&temp); return Choice::Failed; }
    }
    if std::fs::rename(&temp, &target).is_err() {
        if had_old { let _ = std::fs::rename(&backup, &target); }
        let _ = std::fs::remove_file(&temp);
        return Choice::Failed;
    }
    if had_old { let _ = std::fs::remove_file(backup); }
    Choice::Loaded(stations)
}
