//! The application group of the right-click menu: what the Shell does not offer.
//!
//! The first page of a right click is the file's own menu, built by the Shell — 7-Zip,
//! Git, "Send to", "Open with", "Give access to", everything the file type and the folder
//! register. This module only adds the rows the Shell has no verb for: a path on the
//! clipboard, a packer of our own, a terminal, and the applications behind "Open with".
//!
//! This page used to be made only of AuraExplorer's own rows. That is what read as an
//! empty, wrong menu: none of the commands the file actually has were on it, and reaching
//! them took a second click. The Shell's rows now come first and these follow them in the
//! same popup.
//!
//! Hidden-command preferences apply to both halves, and every row here keeps an ordinary
//! string identity, so a chosen row is decoded before any Shell command number is.
use super::{Mode, Request};
use aura_windows::{
    desktop::MenuRow,
    shell::{file_transfer, menu::MenuOutcome},
};

/// Whether this request shows the page at all.
///
/// Only the two pages that display a menu get the group: a request opened to run a single
/// verb, a properties request or the catalog probe never shows it, and a Shift right click
/// asks for the Shell's extended verbs only.
pub(super) fn enabled(request: &Request) -> bool {
    !request.extended && matches!(request.mode, Mode::More | Mode::Background)
}

/// What this request may offer.
pub(super) struct Target {
    background: bool,
    count: usize,
    /// The host reports whether the target takes file commands; behind a background the
    /// same flag means "this folder is a real file-system folder".
    files: bool,
    folder: bool,
    /// The single selection resolved to a path, which "Open with" and "Create shortcut"
    /// both need to name an item.
    addressed: bool,
}

impl Target {
    pub(super) fn of(request: &Request) -> Self {
        let background = request.mode == Mode::Background;
        let addressed = request.pidls.len() == 1
            && file_transfer::paths_from_pidls(&request.pidls).is_ok_and(|paths| !paths.is_empty());
        Self { background, count: request.pidls.len(), files: request.file_actions, folder: request.folder.is_some(), addressed }
    }
}

/// One row of the group: the identity a chosen row is decoded by, plus the caption the
/// text catalogue translates. The identity is what the host and the worker agree on; the
/// caption is only ever shown.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    CopyPath,
    Compress,
    Shortcut,
    Terminal,
}

impl Action {
    fn id(self) -> &'static str {
        match self {
            Self::CopyPath => "copypath",
            Self::Compress => "compress",
            Self::Shortcut => "shortcut",
            Self::Terminal => "terminal",
        }
    }
    fn caption(self) -> &'static str {
        match self {
            Self::CopyPath => "复制路径",
            Self::Compress => "压缩为 ZIP 文件",
            Self::Shortcut => "创建快捷方式",
            Self::Terminal => "在此处打开终端",
        }
    }
}

/// The group, ready to insert under the Shell's rows.
///
/// Rows and identities come back together and in the same order: the menu numbers its
/// commands by position, so a chosen number is decoded through exactly this list instead
/// of through a second traversal that could drift from the first. Labels borrow from
/// `handlers` — the applications the Shell named — and from the text catalogue.
pub(super) fn application_rows<'a>(request: &Request, handlers: &'a [String])
        -> (Vec<MenuRow<'a>>, Vec<String>) {
    rows_for(&request.options, &Target::of(request), handlers)
}

/// The rows themselves, decided only by the target and the preferences.
///
/// Kept apart from [`application_rows`] so the plan can be reasoned about — and tested —
/// without resolving a single item: nothing here asks the Shell anything.
fn rows_for<'a>(options: &aura_windows::shell::menu::ContextMenuOptions, target: &Target, handlers: &'a [String])
        -> (Vec<MenuRow<'a>>, Vec<String>) {
    let mut rows: Vec<MenuRow<'a>> = Vec::new();
    let mut identities: Vec<String> = Vec::new();
    let push = |rows: &mut Vec<MenuRow<'a>>, identities: &mut Vec<String>, available: bool, action: Action| {
        let label = aura_core::i18n::text(action.caption());
        // A hidden command stays usable by its keyboard shortcut; only the row goes.
        if !available || !aura_windows::shell::menu::common_command_visible(options, action.id(), label) {
            return;
        }
        identities.push(action.id().to_owned());
        rows.push(MenuRow::Command { label: label.into(), identity: action.id().into(), checked: false, enabled: true });
    };
    let files = target.files && !target.background;
    push(&mut rows, &mut identities, files, Action::CopyPath);
    // The picker needs the file's own path: a row whose target turned out to be
    // unreadable would open the chooser for nothing, so it is only shown when there is a
    // path to hand the Shell.
    let addressable = files && target.count == 1 && target.addressed;
    if addressable {
        // The applications the Shell would list for this type, plus the picker. A failed
        // enumeration still leaves the picker, so the row always does something.
        let mut entries: Vec<MenuRow<'a>> = Vec::new();
        for (index, name) in handlers.iter().enumerate() {
            let identity = format!("openwith:{index}");
            identities.push(identity.clone());
            entries.push(MenuRow::Command { label: name.as_str().into(), identity: identity.into(), checked: false, enabled: true });
        }
        if !entries.is_empty() { entries.push(MenuRow::Separator); }
        identities.push("openwith".into());
        entries.push(MenuRow::Command {
            label: aura_core::i18n::text("选择其他应用…").into(),
            identity: "openwith".into(),
            checked: false,
            enabled: true,
        });
        rows.push(MenuRow::Submenu { label: aura_core::i18n::text("打开方式").into(), items: entries });
    }
    push(&mut rows, &mut identities, files, Action::Compress);
    // The Shell builds shortcuts through "Send to" only; a direct row keeps the command
    // where Windows Explorer puts it.
    push(&mut rows, &mut identities, addressable, Action::Shortcut);
    push(&mut rows, &mut identities, target.background && target.folder, Action::Terminal);
    (rows, identities)
}

/// What the host does with a chosen row, if the row is the host's to do.
///
/// Everything the Shell owns is picked out of the native menu by the Shell itself, so only
/// these three ever travel back. The terminal, the shortcut and both halves of "Open with"
/// are executed here, in the worker, and return as `NativeHandled` or `ShellInvoked`.
pub(super) fn outcome(id: &str) -> Option<MenuOutcome> {
    Some(match id {
        "copypath" => MenuOutcome::CopyPath,
        "compress" => MenuOutcome::Compress,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The target is the whole input of the plan, so the tests state it directly instead
    /// of going through a Shell: nothing here needs an item to resolve.
    fn target(background: bool, count: usize, files: bool, addressed: bool, folder: bool) -> Target {
        Target { background, count, files, folder, addressed }
    }

    fn commands<'a>(rows: &'a [MenuRow<'a>]) -> Vec<&'a str> {
        rows.iter()
            .filter_map(|row| match row {
                MenuRow::Command { identity, .. } => Some(identity.as_ref()),
                _ => None,
            })
            .collect()
    }

    fn labels(rows: &[MenuRow<'_>]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                MenuRow::Command { label, .. } | MenuRow::Submenu { label, .. } => label.to_string(),
                MenuRow::Separator => "―".into(),
            })
            .collect()
    }

    fn plan(target: &Target, handlers: &[String]) -> (Vec<String>, Vec<String>) {
        let (rows, identities) = rows_for(&aura_windows::shell::menu::ContextMenuOptions::default(), target, handlers);
        (labels(&rows), identities)
    }

    #[test]
    fn the_group_offers_only_what_the_shell_does_not() {
        // Cut, copy, rename, delete, "Send to" and properties are the Shell's own rows on
        // this page; repeating them here would be two answers to one click.
        let (labels, identities) = plan(&target(false, 1, true, true, true), &[]);
        assert_eq!(identities, ["copypath", "openwith", "compress", "shortcut"]);
        assert_eq!(labels, ["复制路径", "打开方式", "压缩为 ZIP 文件", "创建快捷方式"]);
    }

    #[test]
    fn a_selection_without_file_commands_gets_no_file_rows() {
        let (labels, identities) = plan(&target(false, 3, false, false, true), &[]);
        assert!(labels.is_empty() && identities.is_empty());
    }

    #[test]
    fn open_with_lists_the_enumerated_apps_and_then_the_picker() {
        let handlers = vec!["Блокнот".to_string(), "Visual Studio Code".to_string()];
        let (rows, identities) = rows_for(
            &aura_windows::shell::menu::ContextMenuOptions::default(),
            &target(false, 1, true, true, true),
            &handlers,
        );
        assert_eq!(identities, ["copypath", "openwith:0", "openwith:1", "openwith", "compress", "shortcut"]);
        let MenuRow::Submenu { items, .. } = &rows[1] else { panic!("expected the Open with submenu") };
        assert_eq!(commands(items), ["openwith:0", "openwith:1", "openwith"]);
        assert!(matches!(items[2], MenuRow::Separator), "applications are separated from the picker");
    }

    #[test]
    fn an_empty_enumeration_leaves_the_picker_alone() {
        let (rows, identities) = rows_for(
            &aura_windows::shell::menu::ContextMenuOptions::default(),
            &target(false, 1, true, true, true),
            &[],
        );
        let MenuRow::Submenu { items, .. } = &rows[1] else { panic!("expected the Open with submenu") };
        assert!(items.iter().all(|row| !matches!(row, MenuRow::Separator)), "a lone separator separates nothing");
        assert_eq!(commands(items), ["openwith"]);
        assert!(identities.contains(&"openwith".to_string()));
    }

    #[test]
    fn an_item_that_names_no_path_loses_open_with_and_the_shortcut() {
        // Both rows name the item, so both go when the selection resolves to no path.
        let (labels, identities) = plan(&target(false, 1, true, false, true), &[]);
        assert_eq!(identities, ["copypath", "compress"]);
        assert_eq!(labels, ["复制路径", "压缩为 ZIP 文件"]);
    }

    #[test]
    fn a_folder_background_gets_a_terminal_and_no_file_commands() {
        let (labels, identities) = plan(&target(true, 0, true, false, true), &[]);
        assert_eq!(identities, ["terminal"]);
        assert_eq!(labels, ["在此处打开终端"], "the terminal runs inside the worker, it never returns to the host");
    }

    #[test]
    fn a_background_without_a_folder_gets_nothing() {
        let (labels, identities) = plan(&target(true, 0, true, false, false), &[]);
        assert!(labels.is_empty() && identities.is_empty());
    }

    #[test]
    fn a_shortcut_needs_exactly_one_item() {
        let (_, one) = plan(&target(false, 1, true, true, true), &[]);
        assert!(one.iter().any(|id| id == "shortcut"));
        let (_, several) = plan(&target(false, 3, true, true, true), &[]);
        assert!(!several.iter().any(|id| id == "shortcut"), "a shortcut names one item");
        // Open with is a single-item row for the same reason.
        assert!(!several.iter().any(|id| id == "openwith"));
    }

    #[test]
    fn hidden_commands_stay_hidden_but_keep_their_shortcuts() {
        let mut options = aura_windows::shell::menu::ContextMenuOptions::default();
        options.hidden_verbs.push("copypath".into());
        let (_, identities) = rows_for(&options, &target(false, 1, true, true, true), &[]);
        assert!(!identities.iter().any(|id| id == "copypath"), "the row goes, the shortcut does not");
        assert!(identities.iter().any(|id| id == "openwith"), "one hidden row does not hide the rest");
    }

    #[test]
    fn outcomes_stay_inside_the_host_contract() {
        for id in ["copypath", "compress"] {
            assert!(outcome(id).is_some(), "{id}");
        }
        // Rows the worker executes on its own: a shortcut, a terminal, and either half of
        // "Open with".
        for id in ["shortcut", "terminal", "openwith", "openwith:0"] {
            assert!(outcome(id).is_none(), "{id}");
        }
        // The Shell's rows keep their native identities and never come back here.
        for id in ["open", "cut", "copy", "paste", "rename", "delete", "properties", "more"] {
            assert!(outcome(id).is_none(), "{id}");
        }
    }
}
