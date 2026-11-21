#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod system_shell;
#[cfg(windows)]
mod context_menu_process;
#[cfg(windows)]
mod archive;
#[cfg(windows)]
mod context_preferences;
#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod diagnostics;
#[cfg(windows)]
mod edition;
#[cfg(windows)]
mod path_identity;
#[cfg(windows)]
mod flyout_menu;
#[cfg(windows)]
mod language;
#[cfg(windows)]
mod quick_search;
#[cfg(windows)]
mod runtime;
#[cfg(windows)]
mod service_start;
#[cfg(windows)]
mod storage;

#[cfg(windows)]
use anyhow::{Context, Result, ensure};
#[cfg(windows)]
use std::{ffi::OsString, path::PathBuf};
#[cfg(windows)]
use windows::{core::{PCWSTR, w}, Win32::{Foundation::{CloseHandle, HANDLE}, System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK}}};
#[cfg(windows)]
use desktop::DesktopCommand;

#[cfg(windows)]
struct Options {
    profile_dir: Option<PathBuf>,
    command: Option<DesktopCommand>,
    guard: Option<(u32, Option<u32>)>,
    import_source: Option<PathBuf>,
    import_report: Option<PathBuf>,
    apply_import: bool,
    startup_action: Option<bool>,
    uninstall_startup: bool,
    help: bool,
}

#[cfg(windows)]
fn next_argument<I: Iterator<Item = OsString>>(args: &mut I, message: &'static str) -> Result<OsString> {
    args.next().with_context(|| message)
}

#[cfg(windows)]
fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Options> {
    let mut args = args.into_iter();
    let mut options = Options { profile_dir: None, command: None, guard: None, import_source: None,
        import_report: None, apply_import: false, startup_action: None, uninstall_startup: false, help: false };
    while let Some(value) = args.next() {
        let flag = value.to_string_lossy();
        match flag.as_ref() {
            "--profile-dir" => {
                ensure!(options.profile_dir.is_none(), "--profile-dir may only be supplied once");
                options.profile_dir = Some(PathBuf::from(next_argument(&mut args, "missing profile directory")?));
            }
            "--organize" => set_command(&mut options.command, DesktopCommand::Organize)?,
            "--restore" => set_command(&mut options.command, DesktopCommand::Restore)?,
            "--new-fence" => set_command(&mut options.command, DesktopCommand::NewFence)?,
            "--map" => set_command(&mut options.command, DesktopCommand::Map(PathBuf::from(next_argument(&mut args, "missing folder path")?)))?,
            "--tray" => set_command(&mut options.command, DesktopCommand::Tray)?,
            "--quit" => set_command(&mut options.command, DesktopCommand::Quit)?,
            "--diagnostics" => set_command(&mut options.command, DesktopCommand::Diagnostics(PathBuf::from(next_argument(&mut args, "missing diagnostics path")?)))?,
            "--fences-guard" => {
                ensure!(options.guard.is_none(), "--fences-guard may only be supplied once");
                let pid = next_argument(&mut args, "missing Fences process id")?.to_string_lossy().parse::<u32>().context("invalid Fences process id")?;
                options.guard = Some((pid, None));
            }
            "--guard-thread" => {
                let thread = next_argument(&mut args, "missing desktop thread id")?.to_string_lossy().parse::<u32>().context("invalid desktop thread id")?;
                let Some((_, current)) = options.guard.as_mut() else { anyhow::bail!("--guard-thread requires --fences-guard"); };
                ensure!(current.is_none(), "--guard-thread may only be supplied once");
                *current = Some(thread);
            }
            "--import-tencent" => {
                ensure!(options.import_source.is_none(), "--import-tencent may only be supplied once");
                options.import_source = Some(PathBuf::from(next_argument(&mut args, "missing import source")?));
            }
            "--import-report" => {
                ensure!(options.import_report.is_none(), "--import-report may only be supplied once");
                options.import_report = Some(PathBuf::from(next_argument(&mut args, "missing import report path")?));
            }
            "--apply-import" => options.apply_import = true,
            "--enable-startup" => {
                ensure!(options.startup_action.is_none(), "startup action may only be supplied once");
                options.startup_action = Some(true);
            }
            "--disable-startup" => {
                ensure!(options.startup_action.is_none(), "startup action may only be supplied once");
                options.startup_action = Some(false);
            }
            "--uninstall-startup" => options.uninstall_startup = true,
            "--build-info" => anyhow::bail!("--build-info is handled before normal argument parsing"),
            "--help" | "-h" => options.help = true,
            _ => anyhow::bail!("unknown Fences argument: {flag}"),
        }
    }
    ensure!(!(options.apply_import && options.import_source.is_none()), "--apply-import requires --import-tencent");
    ensure!(options.import_source.is_none() || options.import_report.is_some(), "--import-tencent requires --import-report");
    ensure!(options.import_source.is_none() || options.command.is_none(), "layout import cannot be combined with a host command");
    ensure!(options.guard.is_none() || (options.import_source.is_none() && options.command.is_none()), "guard cannot be combined with an app command");
    ensure!(options.startup_action.is_none() || (options.import_source.is_none() && options.command.is_none() && options.guard.is_none()
        && options.import_report.is_none() && !options.apply_import && !options.uninstall_startup), "startup configuration cannot be combined with another command");
    ensure!(!options.uninstall_startup || (options.import_source.is_none() && options.command.is_none() && options.guard.is_none()
        && options.startup_action.is_none() && !options.apply_import), "startup removal cannot be combined with another command");
    Ok(options)
}

#[cfg(windows)]
fn set_command(current: &mut Option<DesktopCommand>, next: DesktopCommand) -> Result<()> {
    ensure!(current.is_none(), "only one Fences command may be supplied");
    *current = Some(next);
    Ok(())
}

#[cfg(windows)]
fn run() -> Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "--build-info") {
        ensure!(args.len() == 1, "--build-info cannot be combined with other arguments");
        println!("{}", serde_json::json!({"product":"Fences", "version":env!("CARGO_PKG_VERSION"),
            "standalone":true, "platform":"windows-x64"}));
        return Ok(());
    }
    language::initialize_environment();
    if let Some(result) = context_menu_process::run_worker_if_requested() { return result; }
    let options = parse(args)?;
    if options.help { show_help(); return Ok(()); }

    let explicit_profile = options.profile_dir.is_some();
    let profile = match options.profile_dir {
        Some(path) => std::path::absolute(path)?,
        None => storage::profile_directory(),
    };
    std::fs::create_dir_all(&profile)?;
    let layout = storage::layout_path(&profile);
    let settings = storage::settings_path(&profile);
    let quitting = options.command == Some(DesktopCommand::Quit);
    if !explicit_profile && options.guard.is_none() && options.import_source.is_none() && !quitting
        && options.startup_action.is_none() && !options.uninstall_startup {
        if let Err(error) = storage::migrate_legacy_profile(&layout, &settings) {
            runtime::trace(&format!("legacy profile migration skipped: {error:#}"));
        }
    }

    if let Some((pid, thread)) = options.guard { return desktop::guard(layout, pid, thread); }
    if let Some(source) = options.import_source {
        return desktop::migration::run(&source, &layout,
            options.import_report.as_deref().expect("validated import report"), options.apply_import);
    }
    if options.uninstall_startup {
        let executable = std::env::current_exe()?;
        return desktop::uninstall_owned_startup(&layout, &executable);
    }
    if let Some(enabled) = options.startup_action {
        return desktop::set_startup_enabled(&layout, enabled);
    }

    let _lifetime = ApplicationLifetime::new()?;
    desktop::run(layout, settings, options.command.unwrap_or(DesktopCommand::Show))
}

#[cfg(windows)]
fn show_help() {
    let text = "Fences — desktop and folder grids\n\nUsage: Fences.exe [--profile-dir DIR] [--organize|--restore|--new-fence|--map DIR|--tray|--quit]\n       Fences.exe --import-tencent SOURCE --import-report FILE [--apply-import]\n       Fences.exe --enable-startup|--disable-startup|--uninstall-startup\n\nThis is a standalone application; it does not start AuraExplorer or AuraDock. Open actions use Windows Explorer.";
    unsafe { MessageBoxW(None, PCWSTR(wide(text).as_ptr()), w!("Fences"), MB_OK); }
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() }

#[cfg(windows)]
fn show_error(error: &anyhow::Error) {
    let message = wide(&format!("{error:#}"));
    unsafe { MessageBoxW(None, PCWSTR(message.as_ptr()), w!("Fences"), MB_OK | MB_ICONERROR); }
}

#[cfg(windows)]
pub(crate) struct ApplicationLifetime(Option<HANDLE>);
#[cfg(windows)]
impl ApplicationLifetime {
    fn new() -> Result<Self> {
        let name = wide(edition::application_mutex());
        Ok(Self(Some(unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr()))? })))
    }
}
#[cfg(windows)]
impl Drop for ApplicationLifetime {
    fn drop(&mut self) { if let Some(handle) = self.0.take() { let _ = unsafe { CloseHandle(handle) }; } }
}

#[cfg(windows)]
fn main() {
    if let Err(error) = run() { show_error(&error); std::process::exit(1); }
}
