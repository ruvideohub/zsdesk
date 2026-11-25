//! Resolve the target of a Shell shortcut without launching it or searching for a moved file.
use std::{os::windows::ffi::{OsStrExt, OsStringExt}, path::{Path, PathBuf}};
use windows::{core::{Interface, PCWSTR, Result, Error, HRESULT}, Win32::{
    System::Com::{CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ},
    UI::Shell::{IShellLinkW, ShellLink, SHCreateItemFromIDList, SHOpenFolderAndSelectItems, IShellItem, SIGDN_FILESYSPATH},
}};

fn invalid() -> Error { Error::from_hresult(HRESULT(0x80070002u32 as i32)) }

/// Do not claim similarly named verbs for non-shortcut Shell items.
pub fn is_shortcut_pidl(pidl: &[u8]) -> bool {
    let Ok(item) = aura_windows::shell::pidl::item_from_pidl(pidl) else { return false; };
    let Ok(name) = (unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }) else { return false; };
    let value = unsafe { name.to_hstring() };
    unsafe { CoTaskMemFree(Some(name.0.cast())) };
    PathBuf::from(std::ffi::OsString::from_wide(&value)).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
}

/// COM must already be initialized on the caller's thread. Return the target's PIDL,
/// not the shortcut's PIDL; preserves virtual Shell targets and Unicode names.
fn shortcut_target(path: &Path) -> Result<Vec<u8>> {
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lnk")) { return Err(invalid()); }
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }?;
    let file: IPersistFile = link.cast()?;
    unsafe { file.Load(PCWSTR(name.as_ptr()), STGM_READ) }?;
    let mut target_path = vec![0u16; 32768];
    if unsafe { link.GetPath(&mut target_path, std::ptr::null_mut(), 0) }.is_ok() {
        let len = target_path.iter().position(|&unit| unit == 0).unwrap_or(target_path.len());
        if len > 0 && !PathBuf::from(std::ffi::OsString::from_wide(&target_path[..len])).try_exists().unwrap_or(false) {
            return Err(invalid());
        }
    }
    let pidl = unsafe { link.GetIDList() }?;
    if pidl.is_null() { return Err(invalid()); }
    struct Pidl(*mut windows::Win32::UI::Shell::Common::ITEMIDLIST);
    impl Drop for Pidl { fn drop(&mut self) { unsafe { CoTaskMemFree(Some(self.0.cast())) } } }
    let pidl = Pidl(pidl);
    let item: IShellItem = unsafe { SHCreateItemFromIDList(pidl.0) }?;
    // A virtual target has no filesystem name. For filesystem targets fail rather
    // than opening a misleading folder when the target has disappeared.
    if let Ok(name) = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) } {
        let name_value = unsafe { name.to_hstring() };
        let path = PathBuf::from(std::ffi::OsString::from_wide(&name_value));
        unsafe { CoTaskMemFree(Some(name.0.cast())) };
        if !path.try_exists().unwrap_or(false) { return Err(invalid()); }
    }
    let bytes = aura_windows::shell::pidl::pidl_bytes(pidl.0);
    if bytes.len() <= 2 || !aura_windows::shell::pidl::valid(&bytes) { return Err(invalid()); }
    Ok(bytes)
}

pub fn open_shortcut_location(path: &Path) -> Result<()> {
    let target = shortcut_target(path)?;
    // cidl=0 selects this fully-qualified item in its parent, including Shell namespaces.
    unsafe { SHOpenFolderAndSelectItems(target.as_ptr().cast(), None, 0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::{Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED}};
    #[test]
    fn shortcut_target_resolves_unicode_target_and_rejects_missing_or_non_links() -> Result<()> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?; }
        struct Com; impl Drop for Com { fn drop(&mut self) { unsafe { CoUninitialize(); } } } let _com=Com;
        let root=std::env::temp_dir().join(format!("Fences shortcut тест {}",std::process::id()));
        std::fs::create_dir_all(root.join("назначение")).unwrap(); let target=root.join("назначение").join("цель с пробелами.txt");let shortcut=root.join("ярлык.lnk");
        std::fs::write(&target,b"target").unwrap();
        let wide=|p:&Path|p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
        let link:IShellLinkW=unsafe{CoCreateInstance(&ShellLink,None,CLSCTX_INPROC_SERVER)}?;
        unsafe{link.SetPath(PCWSTR(wide(&target).as_ptr()))?;}
        let file:IPersistFile=link.cast()?;unsafe{file.Save(PCWSTR(wide(&shortcut).as_ptr()),true)?;}
        let bytes=shortcut_target(&shortcut)?;let item=aura_windows::shell::pidl::item_from_pidl(&bytes)?;
        let value=unsafe{item.GetDisplayName(SIGDN_FILESYSPATH)}?;
        let actual=unsafe{value.to_hstring()};unsafe{CoTaskMemFree(Some(value.0.cast()))};
        assert_eq!(PathBuf::from(std::ffi::OsString::from_wide(&actual)),target);
        assert!(shortcut_target(&target).is_err());std::fs::remove_file(&target).unwrap();assert!(shortcut_target(&shortcut).is_err());
        std::fs::remove_file(&shortcut).unwrap();std::fs::remove_dir(root.join("назначение")).unwrap();std::fs::remove_dir(&root).unwrap();Ok(())
    }
}
