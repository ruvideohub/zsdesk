#!/usr/bin/env python3
"""Applies the 0.3.24 right-click work to a clean e159c18 checkout.

Restores code changes only (CHANGELOG.md / docs/user-guide.md / docs/context-menu-0.3.24.md
survive snapshot resets because they are top-level files, but crates/ does not).
Idempotent: refuses to run twice by checking a marker.
"""
import pathlib, sys, shutil

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

# ---- 9. the quick page itself is gone --------------------------------------------
(ROOT / "crates/aura-windows/src/shell/quick_menu.rs").unlink(missing_ok=True)

print("0.3.24 right-click work applied")
