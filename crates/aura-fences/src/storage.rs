//! Fences-owned profile and settings files.
//!
//! Desktop layout remains `desktop-fences.json`. Application preferences are
//! stored separately in `fences-settings.json`; the Explorer session is read
//! only once as an optional migration source.

use std::{
    fs,
    io,
    path::{Path, PathBuf},
    sync::{Mutex, atomic::{AtomicU64, Ordering}},
    time::SystemTime,
};

use anyhow::{Context, Result, ensure};
use aura_core::model::explorer::{ColorScheme, ContextMenuPreferences};
use serde::{Deserialize, Serialize};

const MAX_SETTINGS_BYTES: u64 = 8 * 1024 * 1024;
static SAVE_LOCK: Mutex<()> = Mutex::new(());
static SAVE_SERIAL: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FencesSession { pub ui: FencesUi }
impl Default for FencesSession { fn default() -> Self { Self { ui: FencesUi::default() } } }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FencesUi {
    pub context_menu: ContextMenuPreferences,
    pub appearance: FencesAppearance,
}
impl Default for FencesUi {
    fn default() -> Self { Self { context_menu: ContextMenuPreferences::default(), appearance: FencesAppearance::default() } }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FencesAppearance { pub color_scheme: ColorScheme }
impl Default for FencesAppearance { fn default() -> Self { Self { color_scheme: ColorScheme::System } } }

fn product_home() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("AuraFences")
}
fn portable_directory() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf))
        .filter(|dir| dir.join("portable.flag").is_file()).map(|dir| dir.join("data"))
}
fn selected_directory() -> Option<PathBuf> {
    let selector = product_home().join("profile-location.json");
    let metadata = fs::metadata(&selector).ok()?;
    if metadata.len() > 128 * 1024 { return None; }
    let directory: PathBuf = serde_json::from_slice(&fs::read(selector).ok()?).ok()?;
    (directory.is_absolute() && directory.is_dir()).then_some(directory)
}
pub fn profile_directory() -> PathBuf {
    portable_directory().or_else(selected_directory).unwrap_or_else(product_home)
}
pub fn layout_path(profile: &Path) -> PathBuf { profile.join("desktop-fences.json") }
pub fn settings_path(profile: &Path) -> PathBuf { profile.join("fences-settings.json") }

pub fn default_session() -> FencesSession { FencesSession::default() }

pub fn load(path: &Path) -> Result<FencesSession> {
    if !path.try_exists()? { return Ok(default_session()); }
    let metadata = fs::metadata(path).with_context(|| format!("cannot inspect {}", path.display()))?;
    ensure!(metadata.is_file() && metadata.len() <= MAX_SETTINGS_BYTES, "Fences settings file is invalid or too large");
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_slice(&bytes).context("invalid Fences settings")
}

pub fn save(path: &Path, session: &FencesSession) -> Result<()> {
    let _write = SAVE_LOCK.lock().map_err(|_| anyhow::anyhow!("Fences profile save lock is poisoned"))?;
    let bytes = serde_json::to_vec_pretty(session)?;
    write_atomic(path, &bytes)
}

pub fn choose_profile_directory(destination: Option<&Path>) -> Result<()> {
    ensure!(portable_directory().is_none(), "portable mode uses the data folder beside Fences.exe");
    let selector = product_home().join("profile-location.json");
    let Some(directory) = destination else {
        return match fs::remove_file(selector) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        };
    };
    ensure!(directory.is_absolute() && directory.is_dir(), "profile folder is not a valid absolute directory");
    fs::create_dir_all(product_home())?;
    write_atomic(&selector, &serde_json::to_vec(directory)?)
}

/// One-time compatibility migration. Existing Explorer/Fences files are copied
/// as data only; no AuraExplorer process, binary, or package is needed.
pub fn migrate_legacy_profile(layout: &Path, settings: &Path) -> Result<()> {
    let Some(base) = dirs::config_dir() else { return Ok(()); };
    let old_profiles = [
        "AuraExplorer", "AuraExplorer-DesktopBox", "AuraExplorer-Theme",
        "ZSDesk", "ZSDesk-DesktopBox", "ZSDesk-Theme",
    ].into_iter().map(|name| base.join(name)).collect::<Vec<_>>();

    if !layout.try_exists()? {
        let candidates = old_profiles.iter().map(|dir| dir.join("desktop-fences.json"));
        if let Some(source) = newest_file(candidates) {
            let bytes = read_limited(&source, 64 * 1024 * 1024)?;
            write_atomic(layout, &bytes)?;
        }
    }
    if !settings.try_exists()? {
        let candidates = old_profiles.iter().map(|dir| dir.join("explorer-session.json"));
        if let Some(source) = newest_file(candidates) {
            // Serde ignores unrelated Explorer fields; only Fences' own menu
            // policy and color scheme are imported into the new profile.
            let bytes = read_limited(&source, MAX_SETTINGS_BYTES)?;
            let migrated: FencesSession = serde_json::from_slice(&bytes).context("legacy settings could not be migrated")?;
            save(settings, &migrated)?;
        }
    }
    Ok(())
}

fn newest_file(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    paths.into_iter().filter_map(|path| {
        let metadata = fs::metadata(&path).ok()?;
        if !metadata.is_file() { return None; }
        Some((metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH), path))
    }).max_by_key(|(modified, _)| *modified).map(|(_, path)| path)
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    ensure!(metadata.is_file() && metadata.len() <= limit, "legacy profile file is too large");
    Ok(fs::read(path)?)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    let temporary = path.with_extension(format!("{}-{}.tmp", std::process::id(), SAVE_SERIAL.fetch_add(1, Ordering::Relaxed)));
    fs::write(&temporary, bytes)?;
    let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if let Err(error) = unsafe { MoveFileExW(windows::core::PCWSTR(from.as_ptr()), windows::core::PCWSTR(to.as_ptr()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}
