//! Packing and unpacking through `AuraArc.exe`.
//!
//! The archiver is a program of its own, so AuraExplorer does what a person would do: it finds
//! `AuraArc.exe` beside itself and starts it with a command. Nothing is linked into the file
//! manager, which is exactly why the same archiver also works with the rest of the suite
//! uninstalled — one copy, two ways to use it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A console of its own, so the archiver can show what it is doing while it works.
#[cfg(windows)]
const NEW_CONSOLE: u32 = 0x0000_0010;

/// Beside the running program first — that is where the bundle and the installer put it — then
/// wherever `PATH` looks, for a copy somebody placed there themselves.
fn find() -> Option<PathBuf> {
    if let Some(folder) = std::env::current_exe().ok().and_then(|current| current.parent().map(Path::to_path_buf)) {
        let candidate = folder.join("AuraArc.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|folder| folder.join("AuraArc.exe"))
        .find(|candidate| candidate.is_file())
}

fn start(kind: &str, archive: &Path, rest: &[PathBuf]) -> Result<(), String> {
    let exe = find().ok_or_else(|| aura_core::i18n::text("未找到 AuraArc.exe").to_string())?;
    let mut command = Command::new(exe);
    command.arg(kind).arg(archive);
    for path in rest {
        command.arg(path);
    }
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(NEW_CONSOLE);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| aura_core::tr_format!("无法启动压缩：{}", error))
}

/// A name nobody is using yet: `фото.zip`, then `фото (2).zip`, and so on.
fn free_name(folder: &Path, stem: &str) -> PathBuf {
    let mut candidate = folder.join(format!("{stem}.zip"));
    let mut number = 2;
    while candidate.exists() {
        candidate = folder.join(format!("{stem} ({number}).zip"));
        number += 1;
    }
    candidate
}

/// The name an archive of these items should carry: one item keeps its own name, several are
/// named after the folder they lie in.
fn name_for(paths: &[PathBuf], folder: &Path) -> String {
    let stem = match paths {
        [one] => one.file_stem().map(|stem| stem.to_string_lossy().into_owned()),
        _ => folder.file_name().map(|name| name.to_string_lossy().into_owned()),
    };
    stem.filter(|stem| !stem.is_empty()).unwrap_or_else(|| String::from("архив"))
}

/// «Сжать в ZIP»: an archive beside the items. Returns the archive that is being written.
pub(crate) fn pack(paths: &[PathBuf], folder: Option<&Path>) -> Result<PathBuf, String> {
    let folder = folder
        .map(Path::to_path_buf)
        .or_else(|| paths.first().and_then(|path| path.parent().map(Path::to_path_buf)))
        .ok_or_else(|| aura_core::i18n::text("无法读取所选项目的路径").to_string())?;
    let archive = free_name(&folder, &name_for(paths, &folder));
    start("a", &archive, paths)?;
    Ok(archive)
}

/// The folder an archive unpacks into when nobody says otherwise: beside it, named after it.
fn beside(archive: &Path) -> PathBuf {
    let stem = archive
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("архив"));
    match archive.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(stem),
        _ => PathBuf::from(stem),
    }
}

/// «Распаковать здесь»: the archive opens into the folder beside it. Returns that folder.
pub(crate) fn unpack(archive: &Path) -> Result<PathBuf, String> {
    start("x", archive, &[])?;
    Ok(beside(archive))
}

/// The first `.zip` among the selected items, if there is one.
pub(crate) fn archive_among(paths: &[PathBuf]) -> Option<PathBuf> {
    paths
        .iter()
        .find(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("zip")))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_item_names_the_archive_after_itself() {
        let folder = Path::new("C:\\работа");
        assert_eq!(name_for(&[folder.join("фото.png")], folder), "фото");
        assert_eq!(name_for(&[folder.join("отпуск")], folder), "отпуск");
    }

    #[test]
    fn several_items_name_it_after_the_folder() {
        let folder = Path::new("C:\\работа");
        assert_eq!(name_for(&[folder.join("а.txt"), folder.join("б.txt")], folder), "работа");
    }

    #[test]
    fn an_archive_opens_beside_itself() {
        assert_eq!(beside(Path::new("C:\\работа\\отпуск.zip")), Path::new("C:\\работа\\отпуск"));
        assert_eq!(beside(Path::new("отпуск.zip")), Path::new("отпуск"));
    }

    #[test]
    fn only_a_zip_offers_unpacking() {
        assert!(archive_among(&[PathBuf::from("C:\\фото.png")]).is_none());
        assert_eq!(
            archive_among(&[PathBuf::from("C:\\фото.ZIP")]),
            Some(PathBuf::from("C:\\фото.ZIP"))
        );
    }
}
