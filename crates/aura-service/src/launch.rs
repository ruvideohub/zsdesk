//! What the components are, where their settings live, and what is running now.
//!
//! The settings are one small JSON file. Every component has exactly one switch — "run it" —
//! because that is what a launcher needs: the service keeps the switched-on components
//! running (and starts them at logon), and stops the ones that are switched off. A separate
//! "at logon" switch per component would only ever disagree with this one.

use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Foundation::WPARAM;

/// One component the service can run.
pub struct Component {
    /// Stable id, used in the settings file and on the command line.
    pub id: &'static str,
    /// What the settings window calls it.
    pub name: &'static str,
    /// What it does, in one line.
    pub hint: &'static str,
    /// The executable, next to `auraservice.exe`.
    pub exe: &'static str,
    /// Its arguments.
    pub args: &'static [&'static str],
    /// Whether it runs at logon unless the user says otherwise.
    pub on_by_default: bool,
    /// Does it take `--profile-dir`? The service passes one profile down to every component
    /// that understands it, so the components never disagree about where the settings are.
    pub takes_profile: bool,
}

/// The components, in the order the settings window lists them.
pub const COMPONENTS: &[Component] = &[
    Component {
        id: "fences",
        name: "Сетки рабочего стола",
        hint: "Группы значков на рабочем столе, значок в трее",
        exe: "fences.exe",
        args: &["--tray"],
        on_by_default: true,
        takes_profile: true,
    },
    Component {
        id: "dock",
        name: "Панель и Launchpad",
        hint: "Панель внизу экрана, Launchpad, контрол-центр",
        exe: "AuraDock.exe",
        args: &[],
        on_by_default: true,
        takes_profile: false,
    },
    Component {
        id: "widgets",
        name: "Виджеты рабочего стола",
        hint: "Часы в углу рабочего стола (их видят и настройки панели)",
        exe: "widgets.exe",
        args: &[],
        on_by_default: true,
        takes_profile: false,
    },
    Component {
        id: "files",
        name: "Файловый менеджер",
        hint: "Окно AuraExplorer — закрой его, и компонент останется выключенным",
        exe: "AuraExplorer.exe",
        args: &[],
        on_by_default: false,
        takes_profile: true,
    },
];

/// The settings file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Bumped when the shape of the file changes; an older file still loads.
    pub version: u32,
    /// Should the service start at logon (mirrors the logon entry).
    pub autostart: bool,
    /// Per component id: keep it running.
    pub run: std::collections::BTreeMap<String, bool>,
    /// The one profile folder handed to the components that take `--profile-dir`. Empty means
    /// "each component decides", which is what a default installation wants.
    pub profile: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let mut run = std::collections::BTreeMap::new();
        for component in COMPONENTS {
            run.insert(component.id.to_string(), component.on_by_default);
        }
        Self { version: 1, autostart: true, run, profile: None }
    }
}

impl Settings {
    /// Should this component be running?
    pub fn wants(&self, component: &Component) -> bool {
        *self.run.get(component.id).unwrap_or(&component.on_by_default)
    }

    pub fn set(&mut self, id: &str, on: bool) {
        self.run.insert(id.to_string(), on);
    }

    /// The profile folder to pass down, if one is set and it is a real folder.
    pub fn profile_dir(&self) -> Option<PathBuf> {
        let path = PathBuf::from(self.profile.as_deref()?);
        (path.is_absolute() && path.is_dir()).then_some(path)
    }
}

/// `%APPDATA%\AuraUI\Service` — settings and the log, next to the other components' files.
pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("AuraUI").join("Service")
}

pub fn settings_path() -> PathBuf {
    dir().join("launch.json")
}

pub fn log_path() -> PathBuf {
    dir().join("auraservice.log")
}

/// The folder `auraservice.exe` lives in: the components are its neighbours.
pub fn install_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)).unwrap_or_default()
}

/// Where a component's executable is, if it is installed.
pub fn exe_of(component: &Component) -> Option<PathBuf> {
    let path = install_dir().join(component.exe);
    path.is_file().then_some(path)
}

pub fn load() -> Settings {
    std::fs::read_to_string(settings_path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir())?;
    let text = serde_json::to_string_pretty(settings).unwrap_or_default();
    std::fs::write(settings_path(), text)
}

/// Append one line to the log, keeping it from growing without bound.
pub fn log(line: &str) {
    let path = log_path();
    if std::fs::metadata(&path).map(|m| m.len() > 256 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let _ = writeln!(file, "{}", line);
    }
}

/// The processes with this executable name. One lookup serves the status column, the
/// "already running elsewhere?" check, and stopping a component.
pub fn pids(exe: &str) -> Vec<u32> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    let wanted = exe.to_ascii_lowercase();
    let mut found = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let name = &entry.szExeFile;
            let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
            if String::from_utf16_lossy(&name[..len]).to_ascii_lowercase() == wanted {
                found.push(entry.th32ProcessID);
            }
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    found
}

/// Is a process with this executable name running? (The settings window's status column.)
pub fn running(exe: &str) -> bool {
    !pids(exe).is_empty()
}

/// Close a component the way the user would: a `WM_CLOSE` to each of its windows, and only if
/// it has none (a background helper) an end of the process.
pub fn close(exe: &str) -> bool {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE};

    struct Closing {
        pids: Vec<u32>,
        posted: usize,
    }

    unsafe extern "system" fn to_each_window(hwnd: windows::Win32::Foundation::HWND, param: LPARAM) -> BOOL {
        let closing = unsafe { &mut *(param.0 as *mut Closing) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if closing.pids.contains(&pid) {
            let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            closing.posted += 1;
        }
        BOOL(1)
    }

    let pids = pids(exe);
    if pids.is_empty() {
        return false;
    }
    let mut closing = Closing { pids, posted: 0 };
    let _ = unsafe { EnumWindows(Some(to_each_window), LPARAM(&mut closing as *mut Closing as isize)) };
    if closing.posted > 0 {
        return true;
    }
    let mut ended = false;
    for pid in &closing.pids {
        if let Ok(process) = unsafe { OpenProcess(PROCESS_TERMINATE, false, *pid) } {
            ended |= unsafe { TerminateProcess(process, 1) }.is_ok();
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(process) };
        }
    }
    ended
}

/// Start the supervisor (`auraservice.exe --run`) without a console window. It is safe to call
/// when one is already running: the second instance finds the mutex taken and exits.
pub fn start_service() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(exe) = std::env::current_exe() else { return false };
    std::process::Command::new(exe)
        .arg("--run")
        .current_dir(std::env::var_os("USERPROFILE").unwrap_or_default())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .is_ok()
}

/// One running component, and when it may be started again.
pub struct Running {
    pub id: &'static str,
    pub child: Child,
    pub retry_at: Instant,
}

impl Running {
    /// Has this component's process exited?
    pub fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }
}

/// The logon entry.
pub mod logon {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    };

    use windows::core::w;

    /// One entry for the whole suite: it used to be `AuraExplorer --desktop --tray`, now it is
    /// the service, which starts the components itself.
    pub const VALUE: &str = "AuraExplorer";
    const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    /// The command the entry holds for this installation.
    pub fn command() -> Option<String> {
        let exe = super::install_dir().join("auraservice.exe");
        let exe = if exe.is_file() { exe } else { std::env::current_exe().ok()? };
        Some(format!("\"{}\" --run", exe.display()))
    }

    pub fn current() -> Option<String> {
        let name = wide(VALUE);
        let mut bytes = 4096u32;
        let mut value = vec![0u16; bytes as usize / 2];
        unsafe {
            RegGetValueW(HKEY_CURRENT_USER, RUN, PCWSTR(name.as_ptr()), RRF_RT_REG_SZ, None, Some(value.as_mut_ptr().cast()), Some(&mut bytes)).ok()?;
        }
        let len = value.iter().position(|unit| *unit == 0).unwrap_or(value.len());
        Some(String::from_utf16_lossy(&value[..len]))
    }

    /// Writes the entry (on) or removes it.
    pub fn set(on: bool) -> bool {
        let name = wide(VALUE);
        let mut key = HKEY::default();
        unsafe {
            if RegCreateKeyExW(HKEY_CURRENT_USER, RUN, None, None, REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut key, None).is_err() {
                return false;
            }
        }
        let result = if on {
            match command() {
                Some(command) => {
                    let value = wide(&command);
                    let bytes = unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2) };
                    unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)) }.is_ok()
                }
                None => false,
            }
        } else {
            let status = unsafe { RegDeleteValueW(key, PCWSTR(name.as_ptr())) };
            // 0 = removed, 2 = the value was not there; both are what was asked for.
            status.0 == 0 || status.0 == 2
        };
        let _ = unsafe { RegCloseKey(key) };
        result
    }

    /// Is our own command the one in the logon entry?
    pub fn is_ours() -> bool {
        let Some(current) = current() else { return false };
        command().is_some_and(|ours| current.trim().eq_ignore_ascii_case(ours.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_component_has_one_switch_and_a_default() {
        let settings = Settings::default();
        for component in COMPONENTS {
            assert!(settings.run.contains_key(component.id), "{} is missing its switch", component.id);
            assert_eq!(settings.wants(component), component.on_by_default, "{} starts against its default", component.id);
            assert!(component.exe.ends_with(".exe"), "{} is not an executable name", component.id);
        }
        // The file manager is the one thing a logon should not open.
        assert!(!settings.wants(COMPONENTS.iter().find(|component| component.id == "files").unwrap()));
        assert!(settings.autostart);
    }

    #[test]
    fn a_switch_survives_the_settings_file() {
        let mut settings = Settings::default();
        settings.set("dock", false);
        let text = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.wants(COMPONENTS.iter().find(|component| component.id == "dock").unwrap()), false);
        assert_eq!(back.run, settings.run);
        assert_eq!(back.autostart, settings.autostart);
    }

    #[test]
    fn an_older_settings_file_still_loads() {
        // A file from before the profile field: everything else keeps working.
        let back: Settings = serde_json::from_str(r#"{"version":1,"autostart":false,"run":{"dock":true}}"#).unwrap();
        assert!(!back.autostart);
        assert!(back.wants(COMPONENTS.iter().find(|component| component.id == "dock").unwrap()));
        // A component the file does not mention keeps its default.
        assert!(back.wants(COMPONENTS.iter().find(|component| component.id == "fences").unwrap()));
        assert!(back.profile.is_none());
    }

    #[test]
    fn only_a_real_profile_folder_is_passed_down() {
        let mut settings = Settings::default();
        assert!(settings.profile_dir().is_none());
        settings.profile = Some("relative\\folder".into());
        assert!(settings.profile_dir().is_none(), "a relative folder would depend on the component's own working directory");
        settings.profile = Some(std::env::temp_dir().display().to_string());
        assert!(settings.profile_dir().is_some());
        // Only the components that understand the argument receive it.
        assert!(COMPONENTS.iter().find(|component| component.id == "fences").unwrap().takes_profile);
        assert!(!COMPONENTS.iter().find(|component| component.id == "widgets").unwrap().takes_profile);
    }
}
