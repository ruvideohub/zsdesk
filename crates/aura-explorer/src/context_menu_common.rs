//! Common actions use the host's validated target. Shell extension discovery
//! starts only when the full menu is requested.
//!
//! This page is the first thing a right click shows. It is built from AuraExplorer's
//! own command set and never instantiates `IContextMenu`, so no third-party
//! shell extension (7-Zip, WPS, Git, …) is loaded and nothing can block. The
//! full Shell menu stays one deliberate step away: "显示更多选项" here, or a
//! Shift right click, which skips this page entirely.
//!
//! Everything the page offers is done by AuraExplorer itself: shortcut creation,
//! ZIP compression, "Open with" handler enumeration and a terminal all call the
//! narrow Shell API they need instead of enumerating every registered verb.
use super::{Mode, Request};
use aura_ui::menus::{self, MenuItemSpec, MenuSpec, ZsAccelerator, ZsAcceleratorKey};
use aura_windows::shell::{file_transfer, menu::{ContextMenuOptions, MenuOutcome}};

pub(super) fn enabled(request: &Request) -> bool {
    request.options.quick_first && !request.extended && matches!(request.mode, Mode::More | Mode::Background)
}

/// What the page may offer for this request.
pub(super) struct Target {
    background: bool,
    count: usize,
    files: bool,
    open: bool,
    rename: bool,
    paste: bool,
    folder: bool,
    /// Exactly one `.zip` is selected, so «Распаковать здесь» has something to do.
    archive: bool,
}

impl Target {
    pub(super) fn of(request: &Request) -> Self {
        let background = request.mode == Mode::Background;
        let files = request.file_actions && (background || file_transfer::paths_from_pidls(&request.pidls).is_ok());
        // A selected folder is a paste target, exactly like the Shell menu treats it.
        let single_folder = !background && request.pidls.len() == 1 && is_folder(&request.pidls[0]);
        let archive = !background
            && request.pidls.len() == 1
            && file_transfer::paths_from_pidls(&request.pidls)
                .is_ok_and(|paths| crate::archive::archive_among(&paths).is_some());
        Self {
            background,
            count: request.pidls.len(),
            files,
            open: request.open && !background && request.pidls.len() == 1,
            rename: request.rename && !background && request.pidls.len() == 1,
            paste: file_transfer::has_paste_content() && (background || single_folder),
            folder: request.folder.is_some(),
            archive,
        }
    }
}

pub(super) fn specification(request: &Request, handlers: &[String]) -> MenuSpec {
    command_page(&request.options, &Target::of(request), handlers)
}

fn is_folder(pidl: &[u8]) -> bool {
    let Ok(item) = aura_windows::shell::pidl::item_from_pidl(pidl) else { return false; };
    unsafe { item.GetAttributes(windows::Win32::System::SystemServices::SFGAO_FOLDER) }
        .is_ok_and(|value| value.0 & windows::Win32::System::SystemServices::SFGAO_FOLDER.0 != 0)
}

fn command_page(options: &ContextMenuOptions, target: &Target, handlers: &[String]) -> MenuSpec {
    let mut rows: Vec<MenuItemSpec> = Vec::new();
    // A hidden command stays usable by its keyboard shortcut; only the row goes.
    let push = |rows: &mut Vec<MenuItemSpec>, available: bool, action: Action| {
        let label = aura_core::i18n::text(action.caption());
        if !available || !aura_windows::shell::menu::common_command_visible(options, action.id(), label) { return; }
        rows.push(match action.accelerator() {
            Some(accelerator) => menus::keyboard(label, action.id(), accelerator),
            None => menus::item(label, action.id()),
        });
    };
    let files = target.files && !target.background;
    push(&mut rows, target.open && target.count == 1, Action::Open);
    if files && target.count == 1 {
        // The applications the Shell would list, plus the picker. Without a
        // usable enumeration only the picker remains, so the row still works.
        let mut entries: Vec<MenuItemSpec> = handlers.iter().enumerate()
            .map(|(index, name)| menus::item(name, &format!("openwith:{index}"))).collect();
        if !entries.is_empty() { entries.push(MenuItemSpec::Separator); }
        entries.push(menus::item(aura_core::i18n::text("选择其他应用…"), "openwith"));
        rows.push(menus::submenu(aura_core::i18n::text("打开方式"), entries));
    }
    if !rows.is_empty() { rows.push(MenuItemSpec::Separator); }
    push(&mut rows, files, Action::Cut);
    push(&mut rows, files, Action::Copy);
    push(&mut rows, files, Action::CopyPath);
    push(&mut rows, target.paste, Action::Paste);
    push(&mut rows, target.background && target.files, Action::NewFolder);
    push(&mut rows, target.background, Action::Refresh);
    push(&mut rows, target.rename && target.count == 1, Action::Rename);
    push(&mut rows, files, Action::Delete);
    push(&mut rows, files, Action::Shortcut);
    push(&mut rows, files, Action::Compress);
    push(&mut rows, target.archive, Action::Extract);
    push(&mut rows, target.background && target.folder, Action::Terminal);
    push(&mut rows, true, Action::Properties);
    rows.push(MenuItemSpec::Separator);
    push(&mut rows, true, Action::More);
    menus::menu(rows)
}

/// One row of the page: its identifier, its catalogue caption and its shortcut.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Open, Cut, Copy, CopyPath, Paste, NewFolder, Refresh, Rename, Delete, Shortcut, Compress, Extract, Terminal, Properties, More,
}

impl Action {
    fn id(self) -> &'static str {
        match self {
            Self::Open => "open", Self::Cut => "cut", Self::Copy => "copy", Self::CopyPath => "copypath",
            Self::Paste => "paste", Self::NewFolder => "newfolder", Self::Refresh => "refresh",
            Self::Rename => "rename", Self::Delete => "delete", Self::Shortcut => "shortcut",
            Self::Compress => "compress", Self::Extract => "extract",
            Self::Terminal => "terminal", Self::Properties => "properties",
            Self::More => "more",
        }
    }
    fn caption(self) -> &'static str {
        match self {
            Self::Open => "打开", Self::Cut => "剪切", Self::Copy => "复制", Self::CopyPath => "复制路径",
            Self::Paste => "粘贴", Self::NewFolder => "新建文件夹", Self::Refresh => "刷新",
            Self::Rename => "重命名", Self::Delete => "删除", Self::Shortcut => "创建快捷方式",
            Self::Compress => "压缩为 ZIP 文件", Self::Extract => "解压到当前文件夹",
            Self::Terminal => "在此处打开终端",
            Self::Properties => "属性", Self::More => "显示更多选项",
        }
    }
    fn accelerator(self) -> Option<ZsAccelerator> {
        let enter = ZsAccelerator::new(ZsAcceleratorKey::Enter);
        Some(match self {
            Self::Open => enter,
            Self::Cut => ZsAccelerator::primary_character('X'),
            Self::Copy => ZsAccelerator::primary_character('C'),
            Self::CopyPath => ZsAccelerator::primary_character('C').shifted(),
            Self::Paste => ZsAccelerator::primary_character('V'),
            Self::NewFolder => ZsAccelerator::primary_character('N').shifted(),
            Self::Refresh => ZsAccelerator::new(ZsAcceleratorKey::Function(5)),
            Self::Rename => ZsAccelerator::new(ZsAcceleratorKey::Function(2)),
            Self::Delete => ZsAccelerator::new(ZsAcceleratorKey::Delete),
            Self::Properties => enter.with_alt(),
            Self::Shortcut | Self::Compress | Self::Extract | Self::Terminal | Self::More => return None,
        })
    }
}

pub(super) fn outcome(id: &str) -> Option<MenuOutcome> {
    Some(match id {
        "open" => MenuOutcome::Open,
        "cut" => MenuOutcome::Cut,
        "copy" => MenuOutcome::Copy,
        "copypath" => MenuOutcome::CopyPath,
        "paste" => MenuOutcome::Paste,
        "newfolder" => MenuOutcome::NewFolder,
        "refresh" => MenuOutcome::Refresh,
        "rename" => MenuOutcome::Rename,
        "delete" => MenuOutcome::Delete { permanent:false },
        "compress" => MenuOutcome::Compress,
        "extract" => MenuOutcome::Extract,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aura_ui::menus::UiCommand;

    fn ids(spec: MenuSpec) -> Vec<String> {
        fn collect(items: Vec<MenuItemSpec>, into: &mut Vec<String>) {
            for row in items {
                match row {
                    MenuItemSpec::Command { command: UiCommand::Custom { id,.. },.. } => into.push(id),
                    MenuItemSpec::Submenu { menu,.. } => collect(menu.items, into),
                    _ => {}
                }
            }
        }
        let mut result = Vec::new();
        collect(spec.items, &mut result);
        result
    }

    fn row<'a>(spec: &'a MenuSpec, id: &str) -> Option<&'a MenuItemSpec> {
        spec.items.iter().find(|row| matches!(row,
            MenuItemSpec::Command { command: UiCommand::Custom { id: current,.. },.. } if current == id))
    }

    fn target(background: bool, count: usize, files: bool, open: bool, rename: bool, paste: bool) -> Target {
        Target { background, count, files, open, rename, paste, folder: true, archive: false }
    }

    #[test]
    fn the_page_offers_every_common_command() {
        let spec = command_page(&ContextMenuOptions::default(), &target(false, 1, true, true, true, false), &[]);
        assert_eq!(ids(spec), [
            "open", "openwith", "cut", "copy", "copypath", "rename", "delete", "shortcut", "compress",
            "properties", "more",
        ]);
    }

    #[test]
    fn multiple_selection_drops_the_single_item_rows() {
        let spec = command_page(&ContextMenuOptions::default(), &target(false, 3, true, true, true, false), &[]);
        assert_eq!(ids(spec), ["cut", "copy", "copypath", "delete", "shortcut", "compress", "properties", "more"]);
    }

    #[test]
    fn empty_space_offers_folder_commands_only() {
        let spec = command_page(&ContextMenuOptions::default(), &target(true, 0, true, false, false, true), &[]);
        assert_eq!(ids(spec), ["paste", "newfolder", "refresh", "terminal", "properties", "more"]);
    }

    #[test]
    fn open_with_lists_enumerated_apps_and_the_picker() {
        let handlers = vec!["Блокнот".to_string(), "Visual Studio Code".to_string()];
        let spec = command_page(&ContextMenuOptions::default(), &target(false, 1, true, true, true, false), &handlers);
        assert_eq!(ids(spec), [
            "open", "openwith:0", "openwith:1", "openwith", "cut", "copy", "copypath", "rename", "delete",
            "shortcut", "compress", "properties", "more",
        ]);
        // Without enumerated handlers the picker is still reachable.
        let bare = command_page(&ContextMenuOptions::default(), &target(false, 1, true, true, true, false), &[]);
        assert_eq!(ids(bare).iter().filter(|id| id.starts_with("openwith")).cloned().collect::<Vec<_>>(), ["openwith"]);
    }

    #[test]
    fn hidden_commands_stay_hidden() {
        let mut options = ContextMenuOptions::default();
        options.hidden_verbs.push("copy".into());
        options.hidden_verbs.push("new".into());
        let spec = command_page(&options, &target(true, 0, true, false, false, true), &[]);
        assert_eq!(ids(spec), ["paste", "refresh", "terminal", "properties", "more"]);
    }

    #[test]
    fn rows_carry_their_keyboard_hints() {
        let spec = command_page(&ContextMenuOptions::default(), &target(false, 1, true, true, true, false), &[]);
        let accelerator = |id: &str| match row(&spec, id) {
            Some(MenuItemSpec::Command { accelerator,.. }) => *accelerator,
            _ => None,
        };
        assert_eq!(accelerator("copy"), Some(ZsAccelerator::primary_character('C')));
        assert_eq!(accelerator("copypath"), Some(ZsAccelerator::primary_character('C').shifted()));
        assert_eq!(accelerator("rename"), Some(ZsAccelerator::new(ZsAcceleratorKey::Function(2))));
        assert_eq!(accelerator("properties"), Some(ZsAccelerator::new(ZsAcceleratorKey::Enter).with_alt()));
        assert_eq!(accelerator("shortcut"), None);
    }

    #[test]
    fn outcomes_stay_inside_the_host_contract() {
        for id in ["open", "cut", "copy", "copypath", "paste", "newfolder", "refresh", "rename", "delete", "compress", "extract"] {
            assert!(outcome(id).is_some(), "{id}");
        }
        // These are executed inside the worker instead of returning to the host.
        for id in ["shortcut", "terminal", "properties", "more", "openwith"] {
            assert!(outcome(id).is_none(), "{id}");
        }
    }
}
