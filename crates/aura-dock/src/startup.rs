//! Per-user startup of this Dock installation, without a launcher service.
use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};
use crate::startup_policy::{self, Change};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("AuraDock");

fn expected() -> Result<String, String> {
    let path = std::env::current_exe().map_err(|e| e.to_string())?;
    let value = path.to_str().ok_or("Путь Dock нельзя представить в UTF-8")?;
    if value.contains('"') || !path.is_absolute() { return Err("Недопустимый путь Dock".into()); }
    Ok(format!("\"{value}\""))
}

fn read() -> Result<Option<String>, String> {
    let mut size = 0u32;
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, None, None, Some(&mut size)) };
    if status.0 == 2 { return Ok(None); }
    status.ok().map_err(|e| e.to_string())?;
    if !(2..=65_536).contains(&size) || size % 2 != 0 { return Err("Повреждённая запись автозапуска".into()); }
    let mut value = vec![0u16; size as usize / 2 + 1];
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, None, Some(value.as_mut_ptr().cast()), Some(&mut size)) }
        .ok().map_err(|e| e.to_string())?;
    let end = value.iter().position(|&ch| ch == 0).unwrap_or(value.len());
    String::from_utf16(&value[..end]).map(Some).map_err(|e| e.to_string())
}

pub fn current() -> Result<bool, String> {
    let expected = expected()?;
    let value = read()?;
    Ok(value.as_deref().is_some_and(|s| s.trim().eq_ignore_ascii_case(&expected)))
}

pub fn set(enabled: bool) -> Result<(), String> {
    let expected = expected()?;
    let old = read()?;
    let change = startup_policy::decide(old.as_deref(), &expected, enabled);
    if change == Change::Conflict { return Err("Запись AuraDock принадлежит другой установке и не изменена".into()); }
    if change == Change::None { return Ok(()); }
    if read()? != old { return Err("Запись автозапуска изменилась; попробуйте ещё раз".into()); }
    match change {
        Change::Write => {
            let data: Vec<u16> = expected.encode_utf16().chain(Some(0)).collect();
            unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, RUN, NAME, REG_SZ.0, Some(data.as_ptr().cast()), (data.len() * 2) as u32) }.ok().map_err(|e| e.to_string())
        }
        Change::Delete => unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) }.ok().map_err(|e| e.to_string()),
        _ => Ok(()),
    }
}
