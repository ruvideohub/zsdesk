//! Per-user startup of this Dock installation, without a launcher service.
use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};
use crate::startup_policy::{self, Change};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("AuraDock");

#[derive(Debug)]
pub enum Error {
    Conflict { current: String, replacement: String },
    Other(String),
}

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
    if size as usize > (value.len() - 1) * 2 || size % 2 != 0 { return Err("Повреждённая запись автозапуска".into()); }
    let data = &value[..size as usize / 2];
    let end = data.iter().position(|&ch| ch == 0).unwrap_or(data.len());
    if data[end..].iter().any(|&ch| ch != 0) { return Err("Запись автозапуска содержит скрытые данные и не изменена".into()); }
    String::from_utf16(&data[..end]).map(Some).map_err(|e| e.to_string())
}

fn write(command: &str) -> Result<(), String> {
    let data: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, RUN, NAME, REG_SZ.0, Some(data.as_ptr().cast()), (data.len() * 2) as u32) }
        .ok().map_err(|e| e.to_string())
}

pub fn current() -> Result<bool, String> {
    let expected = expected()?;
    let value = read()?;
    Ok(value.as_deref().is_some_and(|s| startup_policy::is_own(s, &expected)))
}

pub fn set(enabled: bool) -> Result<(), Error> {
    let replacement = expected().map_err(Error::Other)?;
    let old = read().map_err(Error::Other)?;
    let change = startup_policy::decide(old.as_deref(), &replacement, enabled);
    if change == Change::Conflict {
        return Err(Error::Conflict { current: old.unwrap_or_default(), replacement });
    }
    if change == Change::None { return Ok(()); }
    if read().map_err(Error::Other)? != old { return Err(Error::Other("Запись автозапуска изменилась; попробуйте ещё раз".into())); }
    match change {
        Change::Write => write(&replacement).map_err(Error::Other),
        Change::Delete => unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) }.ok().map_err(|e| Error::Other(e.to_string())),
        _ => Ok(()),
    }
}

/// Called only after the user sees and explicitly approves both complete commands.
pub fn replace_confirmed(approved_old: &str, approved_new: &str) -> Result<(), String> {
    let new = expected()?;
    let old = read()?;
    if !startup_policy::replacement_allowed(approved_old, approved_new, old.as_deref(), &new) {
        return Err("Запись автозапуска изменилась; попробуйте ещё раз".into());
    }
    write(&new)
}
