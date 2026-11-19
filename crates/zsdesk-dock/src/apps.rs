//! Shell actions behind the icons: launching, revealing, shortcuts and the
//! recycle bin. The application catalogue itself lives in `catalogue.rs`.

use windows::{
    core::{w, Interface, PCWSTR},
    Win32::{
        System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ},
        UI::Shell::{IShellLinkW, ShellExecuteW, ShellLink, SLGP_RAWPATH},
        UI::WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

pub use crate::catalogue::{scan, start_menu_directories, AppEntry};

/// Target executable of a `.lnk`; other paths are returned unchanged.
pub fn resolve_shortcut(path: &str) -> Option<String> {
    if !path.to_lowercase().ends_with(".lnk") { return Some(path.to_string()); }
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let file: IPersistFile = link.cast().ok()?;
    unsafe { file.Load(PCWSTR(wide.as_ptr()), STGM_READ) }.ok()?;
    let mut target = vec![0u16; 32768];
    unsafe { link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32) }.ok()?;
    let length = target.iter().position(|unit| *unit == 0).unwrap_or(target.len());
    (length > 0).then(|| String::from_utf16_lossy(&target[..length]))
}

/// Shell-executes `target` (`exe`, `.lnk`, `shell:` or `ms-settings:` URI).
pub fn launch(target: &str) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty() { return Err(zsdesk_core::i18n::text("找不到该应用").to_string()); }
    let wide: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let result = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(),
        PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(zsdesk_core::tr_format!("无法启动应用：{error}", error = format!("{}", result.0 as usize)))
    }
}

/// Opens Explorer with the item selected.
pub fn reveal(path: &str) {
    let quoted = format!("\"{path}\"");
    let arguments = if std::path::Path::new(path).is_dir() { quoted } else { format!("/select,{quoted}") };
    let _ = launch(&format!("explorer.exe {arguments}"));
}

/// Opens the recycle bin in a new Explorer window.
pub fn open_recycle_bin() { let _ = launch("shell:RecycleBinFolder"); }

/// Empties the recycle bin without the confirmation and progress dialogs.
pub fn empty_recycle_bin() {
    use windows::Win32::UI::Shell::{SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND};
    let _ = unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(),
        SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_shortcut_paths_resolve_to_themselves() {
        assert_eq!(resolve_shortcut(r"C:\Tools\App.exe").as_deref(), Some(r"C:\Tools\App.exe"));
        assert_eq!(resolve_shortcut("ms-settings:").as_deref(), Some("ms-settings:"));
    }

    #[test]
    fn launching_an_empty_target_reports_a_failure() {
        assert!(launch("   ").is_err());
    }
}
