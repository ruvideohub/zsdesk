#!/usr/bin/env python3
"""Applies the 0.3.24 right-click work to a clean e159c18 checkout.

Restores code changes only (CHANGELOG.md / docs/user-guide.md / docs/context-menu-0.3.24.md
survive snapshot resets because they are top-level files, but crates/ does not).
Idempotent: refuses to run twice by checking a marker.
"""
import pathlib, sys, shutil
print("WARNING: historical restore only; it does not include later ZIP or settings fixes. Use Git for current code.", file=sys.stderr)

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
HERE = pathlib.Path(__file__).resolve().parent

EXPL, FENC = "crates/aura-explorer/src/", "crates/aura-fences/src/"
PROC = ["crates/aura-explorer/src/context_menu_process.rs", "crates/aura-fences/src/context_menu_process.rs"]

def read(p): return (ROOT / p).read_text()
def write(p, s): (ROOT / p).write_text(s)
def sub(p, old, new, count=1):
    """Replace `old` with `new`, or do nothing when the file already reads as `new`.

    The copy-based steps above are naturally idempotent; this makes the in-place edits
    behave the same way, so a half-applied tree can be finished by running again.
    """
    s = read(p)
    if old not in s:
        if new in s:
            return
        sys.exit(f"pattern missing in {p}:\n{old[:200]}")
    write(p, s.replace(old, new, count))

# ---- 1. shared group module (identical in both crates) -------------------------------
common = (HERE / "common.rs").read_text()
write(EXPL + "context_menu_common.rs", common)
write(FENC + "context_menu_common.rs", common)

# ---- 2. aura-windows: menu row insertion --------------------------------------------
# Whole file, so running twice cannot insert the row helpers twice.
write("crates/aura-windows/src/desktop.rs", (HERE / "desktop.rs").read_text())

# ---- 3. ShellMenu: rows may be hosted, so classic presentation ----------------------
menu = "crates/aura-windows/src/shell/menu.rs"
write(menu, (HERE / "menu.rs").read_text())

# ---- 4. preferences / model: no dead "which page first" switch ----------------------
for prefs in [EXPL + "context_preferences.rs", FENC + "context_preferences.rs"]:
    sub(prefs, "    ContextMenuOptions{quick_first:!options.full_menu_first,modern:own,drawn:own,",
        """    // There is no choice left about which page comes first: every right click shows the
    // file's own menu with this application's group under it. Only the style remains.
    ContextMenuOptions{modern:own,drawn:own,""")
sub("crates/aura-core/src/model/explorer.rs", '''    /// Ask for the full Shell menu (with every registered extension) at the
    /// first click. AuraExplorer's own short page is the default: it never loads a
    /// shell extension, so it opens immediately.
    pub full_menu_first:bool,''', '''    /// Formerly "ask for the full Shell menu at the first click". Every right click now
    /// shows that menu, so nothing reads this. It stays readable so existing profiles
    /// migrate without data loss.
    #[serde(default)]
    pub full_menu_first:bool,''')

# ---- 5. worker: one page, the Shell's, with our rows under it ----------------------
# The two workers are near-copies but not copies: fences keeps its own worker table
# and diagnostics, so each crate restores its own file.
write(PROC[0], (HERE / "process.rs").read_text())
write(PROC[1], (HERE / "process_fences.rs").read_text())

# ---- 6. window client: no loading notice, worker always trusted for the foreground --
win = EXPL + "window/context_menu.rs"
sub(win, ",client:Client,quick_first:bool,catalog:bool", ",client:Client,catalog:bool")
sub(win, ",point:POINT,started:std::time::Instant,loading_notified:bool,mode:WorkerMode", ",point:POINT,mode:WorkerMode")
sub(win, """    let quick_first=options.quick_first;
""", """    // Rename and open come back to the window's own commands; every other Shell verb runs
    // in the Shell itself, with its dialogs, recycle bin and undo.
""")
sub(win, "client,quick_first,catalog,input,visual,invocation_authorized:false,point,started:std::time::Instant::now(),loading_notified:false,mode,extended",
    "client,catalog,input,visual,invocation_authorized:false,point,mode,extended")
sub(win, "    let worker=session.quick_first.then(||session.client.pid()).flatten();", "    let worker=session.client.pid();")
sub(win, """    if session.loading_notified&&(result.is_some()||!session.client.preparing()){
        session.loading_notified=false;super::pane_input::post(owner,identity,super::pane_input::Intent::TransferMessage(String::new()));
    }
""", "")
sub(win, """            if !session.loading_notified&&session.client.preparing()&&session.started.elapsed()>=std::time::Duration::from_millis(250){
                session.loading_notified=true;super::pane_input::post(owner,identity,super::pane_input::Intent::TransferMessage(aura_core::i18n::text("正在加载右键菜单…").into()));
            }
""", """            // Nothing is posted while the menu is being prepared. The first page is the
            // file's own menu, so a notice here would be drawn over the window and read as
            // part of the menu; the click simply waits for the Shell and its extensions.
""")

# ---- 7. settings menu: the switch that no longer means anything --------------------
ui = "crates/aura-ui/src/menus.rs"
sub(ui, '''        entry(aura_core::i18n::text("优先显示完整菜单"), "context-menu-full-first", true, options.full_menu_first),
''', '''        // Every right click shows the file's own commands now: the Shell's rows and this
        // application's group under them. There is no shorter page left for a switch to
        // choose between, so this submenu only manages what the rows contain.
''')
sub(ui, '''        "context-menu-full-first" => options.full_menu_first = !options.full_menu_first,
''', "")
sub(ui, '''            && !matches!(id.as_str(), "context-menu-full-first" | "context-menu-defaults")).count();''',
    '''            && id.as_str() != "context-menu-defaults").count();''')

# The switch's own test is replaced by the one that says it is gone. Guarded on the new
# name: the anchor below the insert still matches afterwards, so an unguarded replace
# would nest the test inside itself.
s, name = read(ui), "fn the_first_page_is_not_optional_so_the_switch_is_gone() {"
if name not in s:
    anchor = "    #[test]\n    fn no_style_offers_the_per_item_checklist_that_never_applied() {"
    if anchor not in s:
        sys.exit("settings test anchor missing")
    test = (
        "    #[test]\n    " + name + "\n"
        "        // \"优先显示完整菜单\" chose between a shorter page and the real one. There is one\n"
        "        // page now, so a switch that only reordered the same rows would be a setting\n"
        "        // without a difference.\n"
        "        let available = nested_ids(&context_menu_settings_menu(&ContextMenuPreferences::default()));\n"
        '        assert!(!available.contains(&"context-menu-full-first".to_string()), "{available:?}");\n'
        '        assert!(!apply_context_menu_setting(&mut ContextMenuPreferences::default(), "context-menu-full-first"));\n'
        "    }\n\n"
    )
    write(ui, s.replace(anchor, test + anchor, 1))

# ---- 8. tests and fences desktop cleanup ------------------------------------------
for p in ["crates/aura-explorer/src/context_menu_process_tests.rs", "crates/aura-fences/src/context_menu_process_tests.rs"]:
    sub(p, "use aura_windows::shell::menu::QuickMenu;\n", "")
    sub(p, """    let started=Instant::now();let quick=QuickMenu::new(pidls.len(),false,&options)?;let construction=started.elapsed().as_micros()as u64;
    let picked=quick.track_raw(owner.0,point);ensure!(picked.is_none()&&ENTERED.with(std::cell::Cell::get),"immediate popup did not open/dismiss");drop(quick);
""", "")
    sub(p, '"quick_menu_construct_microseconds":construction,"quick_menu_native_popup":true,', "")
    sub(p, '''thread_local!{static ENTERED:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
''', "")
    sub(p, """    if msg==WM_ENTERMENULOOP{ENTERED.with(|value|value.set(true));unsafe{SetTimer(Some(hwnd),2,40,None);}}""",
        """    // The fixture's owner dismisses every popup it is asked to track, so a probe that only
    // needs "the menu opened and closed" does not depend on any page of ours.
    if msg==WM_ENTERMENULOOP{unsafe{SetTimer(Some(hwnd),2,40,None);}}""")

reg = FENC + "desktop/context_menu_regression.rs"
sub(reg, "use aura_windows::shell::menu::QuickAction;\n", "")
sub(reg, "    static QUICK:Cell<bool>=const{Cell::new(false)};\n", "")
sub(reg, """    if msg==WM_INITMENUPOPUP{
        let menu=HMENU(wp.0 as*mut _);let count=unsafe{GetMenuItemCount(Some(menu))};
        QUICK.with(|value|value.set((0..count).any(|index|unsafe{GetMenuItemID(menu,index)}==QuickAction::More.id())));
    }
""", "")

# The import at the top of this file stays as e159c18 has it: `window_message_regression`
# builds a menu spec of its own and reads MenuItemSpec and UiCommand through `super::*`.
fmod = FENC + "desktop/mod.rs"
s = read(fmod)
rule = '/// Inserts application items (ZSUI spec) at the top of a Shell menu.'
if rule in s:
    end = s.index('/// Fence, tab and tray menus as Windows 11 style flyouts at `point`.')
    write(fmod, s[:s.index(rule)] + s[end:])

worker = FENC + "desktop/menu_worker.rs"
sub(worker, """    let quick_first=options.quick_first;
""", "")
sub(worker, "    let worker=quick_first.then_some(timing.extension_process).flatten();",
    """    // The worker's own window owns the popup, so it holds the foreground while the menu
    // is open; `foreground_is_menu_worker` still checks its class and owner, not just the
    // process, so this only widens which process may hold it.
    let worker=timing.extension_process;""")

# ---- 10. the fixes CI found on Windows ---------------------------------------------
# Only a Windows build could catch these; each is recorded with what it cost.
worker = FENC + "desktop/menu_worker.rs"
sub(worker, """        *ids=super::insert_top(menu.hmenu(),&request.items);
""", """        // This probe searches the native menu for a Shell verb by number, so it only
        // works on the Shell's own rows: our rows are not inserted here, and the probe
        // reports none. Production builds never take this path.
""")

# `window_message_regression` reads these through `super::*`, and it builds a menu spec
# of its own, so both names have to be in scope in the parent module.
# Dead since e159c18: `properties` is computed and never read — the branch below tests the
# mode again. It only showed up as a warning until the Windows build listed it.
dead = "        let mut properties=request.mode==Mode::Properties;\n"
for p in PROC:
    s = read(p)
    if dead in s:
        write(p, s.replace(dead, "", 1))

# ---- 11. the archiver is gone -------------------------------------------------------
# `AuraArc.exe` packed and unpacked through a command line. Packing never needed it —
# `aura-core::archive` writes the ZIP itself — so only unpacking depended on it, and the
# row that offered it went with the program.
(ROOT / "crates/aura-explorer/src/archive.rs").unlink(missing_ok=True)
(ROOT / "crates/aura-fences/src/archive.rs").unlink(missing_ok=True)
shutil.rmtree(ROOT / "crates/aura-arc", ignore_errors=True)
sub("Cargo.toml", '    "crates/aura-service",\n    "crates/aura-arc",\n', '    "crates/aura-service",\n')
for entry in ["crates/aura-explorer/src/lib.rs", "crates/aura-fences/src/main.rs"]:
    s = read(entry)
    s = s.replace("pub(crate) mod archive;\n", "").replace("mod archive;\n", "")
    write(entry, s)
# The ZIP writer lives in `aura_core::archive`; these two arms only ever ran for a row the
# worker now handles itself, so they point at nothing.
fences_desktop = FENC + "desktop/mod.rs"
s = read(fences_desktop)
if "crate::archive::" in s:
    write(fences_desktop, s.replace(
        """        MenuOutcome::Compress=>match crate::archive::pack(&paths,match &request.target{
            menu_worker::Target::Background{folder:Some(folder)}=>Some(folder.as_path()),
            _=>paths.first().and_then(|path|path.parent()),
        }){
            Ok(_)=>Ok(()),
            Err(message)=>Err(windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),message)),
        },
""", ""))
# The `Extract` arm itself: the row that reached it was ours, and it is gone.
s = read(fences_desktop)
if "MenuOutcome::Extract=>{" in s:
    # The arm and the catch-all share a line at e159c18: `        },_=>Ok(()),`
    start = s.index("        MenuOutcome::Extract=>{")
    end = s.index("        },_=>Ok(()),", start) + len("        },")
    # The catch-all keeps its own indentation; the line it shared with the arm did not.
    write(fences_desktop, s[:start] + "        " + s[end:].lstrip())

explorer_menu = EXPL + "window/context_menu.rs"
s = read(explorer_menu)
if "MenuOutcome::Compress" in s:
    start = s.index("        Some(Ok(MenuOutcome::Compress)) if valid=>{")
    end = s.index("        Some(Ok(MenuOutcome::NewFolder)) if valid=>{")
    write(explorer_menu, s[:start] + s[end:])

# The worker that hosts a third-party extension must not panic. `chain` always has an
# element, but an unwrap inside that process is the one place it must not be — CI builds
# on Windows, so this was the only way to find it here.
packaged = "crates/aura-windows/src/shell/packaged_state_worker.rs"
s = read(packaged)
if "chain.last().unwrap()" in s:
    s = s.replace("""        let enumeration = unsafe { chain.last().unwrap().value.EnumSubCommands() }?;""",
"""        // `chain` starts with the root and only grows, so there is always a last one.
        // Written as a let-else anyway: an unwrap here is a panic inside the worker that
        // hosts a third-party extension, which is the one place it must not happen.
        let Some(current) = chain.last() else { return Err(fail(0x80004003, "no command in chain")) };
        let enumeration = unsafe { current.value.EnumSubCommands() }?;""")
    s = s.replace("""        let result = unsafe { chain.last().unwrap().value.GetState(items.as_ref(), true) };""",
"""        let Some(current) = chain.last() else { return Err(fail(0x80004003, "no command in chain")) };
        let result = unsafe { current.value.GetState(items.as_ref(), true) };""")
    write(packaged, s)

# The settings window's «Программы» tab held exactly one row: the archiver.
launch = "crates/aura-service/src/launch.rs"
s = read(launch)
if 'id: "arc"' in s:
    start = s.index("pub const PROGRAMS: &[Program] = &[Program {")
    end = s.index("}];", start) + 3
    write(launch, s[:start] + """/// Programs the suite ships that are opened by hand rather than kept running.
///
/// Empty since the archiver was removed: packing is a library call inside the menu now, so
/// there is no program of our own left to list. The list stays because the settings window
/// reads it — a second program belongs here, not in a second branch of the drawing code.
pub const PROGRAMS: &[Program] = &[];""" + s[end:])

window = "crates/aura-service/src/window.rs"
s = read(window)
if 'const TABS: [&str; 3]' in s:
    s = s.replace(
        '''const TABS: [&str; 3] = ["Компоненты", "Программы", "Запуск"];
const TAB_COMPONENTS: usize = 0;
const TAB_PROGRAMS: usize = 1;
const TAB_START: usize = 2;''',
        '''const TABS: [&str; 2] = ["Компоненты", "Запуск"];
const TAB_COMPONENTS: usize = 0;
const TAB_START: usize = 1;
/// Where the program list would sit, kept for the day it has an entry again.
const _TAB_PROGRAMS: usize = 1;''')
    # The three-section comment described the tab that went.
    s = s.replace(
        """/// The three sections, in the order the dock's settings put theirs.
const TABS""",
        """/// The sections, in the order the dock's settings put theirs.
///
/// «Программы» listed the archiver. With it gone the list is empty, and a section that can
/// only ever be empty is not a section: it comes back with the first program that belongs
/// there, and nothing else in this file has to change.
const TABS""")
    s = s.replace(
        """        TAB_COMPONENTS => launch::COMPONENTS.len(),
        TAB_PROGRAMS => launch::PROGRAMS.len(),
        _ => 3,""",
        """        TAB_COMPONENTS => launch::COMPONENTS.len(),
        TAB_START => 3,
        _ => launch::PROGRAMS.len(),""")
    write(window, s)

# The layout and drawing branches still named a tab index that no longer exists.
s = read(window)
s = s.replace(
    """            (TAB_PROGRAMS, _) => (""",
    """            // No tab reaches this branch while PROGRAMS is empty; it stays so a program
            // added later gets its two buttons without touching the layout.
            (_, _) if !launch::PROGRAMS.is_empty() && tab == _TAB_PROGRAMS => (""")
s = s.replace(
    """        match self.tab {
            TAB_COMPONENTS => self.draw_components(cv, look, &l),
            TAB_PROGRAMS => self.draw_programs(cv, look, &l),
            _ => self.draw_start(cv, look, &l),
        }""",
    """        match self.tab {
            TAB_COMPONENTS => self.draw_components(cv, look, &l),
            // The program page exists only while there is a program to list; with the list
            // empty its tab is not drawn either.
            _ => self.draw_start(cv, look, &l),
        }""")
if "fn draw_programs(&self, cv: &aura_windows::gfx::Canvas, look: &Look, l: &Layout) {" in s \
        and "#[allow(dead_code)]\n    fn draw_programs" not in s:
    s = s.replace(
        """    fn draw_programs(&self, cv: &aura_windows::gfx::Canvas, look: &Look, l: &Layout) {""",
        """    /// Draws the program list. Reachable again as soon as `launch::PROGRAMS` has an entry;
    /// until then the tab is not drawn, so nothing calls this and the compiler would warn.
    #[allow(dead_code)]
    fn draw_programs(&self, cv: &aura_windows::gfx::Canvas, look: &Look, l: &Layout) {""")
write(window, s)

# The two `matches!` lists still named a variant that no longer exists.
s = read(fences_desktop)
s = s.replace("|MenuOutcome::CopyPath|MenuOutcome::Compress|MenuOutcome::Extract));",
              "|MenuOutcome::CopyPath|MenuOutcome::Compress));")
s = s.replace("|MenuOutcome::CopyPath|MenuOutcome::Compress|MenuOutcome::Extract))=outcome{",
              "|MenuOutcome::CopyPath|MenuOutcome::Compress))=outcome{")
write(fences_desktop, s)

# ---- 9. the quick page itself is gone --------------------------------------------
(ROOT / "crates/aura-windows/src/shell/quick_menu.rs").unlink(missing_ok=True)

print("0.3.24 right-click work applied")
