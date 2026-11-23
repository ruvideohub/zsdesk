//! Shell work behind AuraExplorer's own context page.
//!
//! Everything here runs inside the isolated menu worker (an STA child process
//! with a job memory limit) and deliberately avoids `IContextMenu`: only the
//! narrow APIs the page actually needs are called, so a slow or broken third
//! party extension can never hold the first click.
use std::{
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, IDataObject, IPersistFile},
        UI::Shell::{
            IAssocHandler, IShellLinkW, SHAssocEnumHandlers, ShellLink, ASSOC_FILTER_NONE,
        },
    },
};

/// More than this and the submenu stops being a menu.
const MAX_HANDLERS: usize = 12;

fn wide(value: &OsStr) -> Vec<u16> { value.encode_wide().chain(Some(0)).collect() }

/// Applications the Shell itself would offer for this file type, in its order.
///
/// Enumeration failure is not fatal: the page keeps its "choose another app"
/// row, which is the only guaranteed entry point.
pub fn open_with_handlers(path: &Path) -> Vec<(String, IAssocHandler)> {
    let Some(extension) = path.extension().filter(|extension| !extension.is_empty()) else { return Vec::new(); };
    let extension = wide(OsStr::new(&format!(".{}", extension.to_string_lossy())));
    let enumerated = (|| -> windows::core::Result<Vec<(String, IAssocHandler)>> {
        let enumerator = unsafe { SHAssocEnumHandlers(PCWSTR(extension.as_ptr()), ASSOC_FILTER_NONE) }?;
        let mut handlers = Vec::new();
        while handlers.len() < MAX_HANDLERS {
            let mut batch = [None, None, None, None];
            let mut fetched = 0u32;
            unsafe { enumerator.Next(&mut batch, Some(&mut fetched)) }?;
            if fetched == 0 { break; }
            for handler in batch.into_iter().flatten() {
                let name = unsafe { handler.GetUIName() }.ok()
                    .filter(|value| !value.0.is_null())
                    .and_then(|value| {
                        // The name is Shell-allocated text: copy it, then release it.
                        let text = unsafe { PCWSTR(value.0).to_string() }.ok();
                        unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(value.0.cast())) };
                        text
                    })
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                if let Some(name) = name { handlers.push((name, handler)); }
            }
        }
        Ok(handlers)
    })();
    enumerated.unwrap_or_default()
}

/// Runs one enumerated handler against the selection's data object.
pub fn invoke_handler(handler: &IAssocHandler, data: &IDataObject) -> Result<()> {
    unsafe { handler.Invoke(data) }?;
    Ok(())
}

/// Creates a shortcut next to the item. An existing file is never replaced.
pub fn create_shortcut(item: &Path) -> Result<PathBuf> {
    let folder = item.parent().context("shortcut target has no folder")?;
    let stem = item.file_stem().filter(|stem| !stem.is_empty())
        .or_else(|| item.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "shortcut".into());
    let destination = free_name(folder, &stem, "lnk");
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }?;
    let target = wide(item.as_os_str());
    unsafe { link.SetPath(PCWSTR(target.as_ptr())) }?;
    let working = wide(folder.as_os_str());
    unsafe { link.SetWorkingDirectory(PCWSTR(working.as_ptr())) }?;
    let persist: IPersistFile = link.cast()?;
    let saved = wide(destination.as_os_str());
    unsafe { persist.Save(PCWSTR(saved.as_ptr()), true) }?;
    Ok(destination)
}

/// Writes a ZIP archive of the selection into `folder` and returns its path.
pub fn compress(sources: &[PathBuf], folder: &Path) -> Result<PathBuf> {
    anyhow::ensure!(!sources.is_empty(), "{}", aura_core::i18n::text("没有可压缩的项目"));
    let entries = aura_core::archive::collect(sources, aura_core::archive::Limits::default())?;
    anyhow::ensure!(!entries.is_empty(), "{}", aura_core::i18n::text("没有可压缩的项目"));
    let name = aura_core::archive::archive_name_for(sources, Some(folder));
    Ok(aura_core::archive::write_unique_zip(folder, &name, &entries)?)
}

/// Opens a console in `folder`: Windows Terminal when installed, otherwise the
/// bundled PowerShell, which starts in its working directory.
pub fn open_terminal(folder: &Path) -> Result<()> {
    use std::{os::windows::process::CommandExt, process::Command};
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    if Command::new("wt.exe").arg("-d").arg(folder).creation_flags(CREATE_NEW_CONSOLE).current_dir(folder).spawn().is_ok() {
        return Ok(());
    }
    Command::new("powershell.exe").arg("-NoExit").creation_flags(CREATE_NEW_CONSOLE).current_dir(folder).spawn()?;
    Ok(())
}

fn free_name(folder: &Path, stem: &str, extension: &str) -> PathBuf {
    let stem = stem.trim_end_matches(&format!(".{extension}"));
    let candidate = folder.join(format!("{stem}.{extension}"));
    if !candidate.exists() { return candidate; }
    for index in 2..10_000u32 {
        let candidate = folder.join(format!("{stem} ({index}).{extension}"));
        if !candidate.exists() { return candidate; }
    }
    folder.join(format!("{stem} ({}).{extension}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_names_never_overwrite() {
        let root = std::env::temp_dir().join(format!("aura-shortcut-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(free_name(&root, "report", "lnk").file_name().unwrap(), "report.lnk");
        std::fs::write(root.join("report.lnk"), b"").unwrap();
        assert_eq!(free_name(&root, "report", "lnk").file_name().unwrap(), "report (2).lnk");
        // A source that already ends in .lnk does not become report.lnk.lnk.
        assert_eq!(free_name(&root, "report.lnk", "lnk").file_name().unwrap(), "report (2).lnk");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn handlers_are_only_looked_up_for_a_file_type() {
        assert!(open_with_handlers(Path::new(r"C:\no-extension")).is_empty());
    }
}
