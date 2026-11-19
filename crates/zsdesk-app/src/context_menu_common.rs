//! Common actions use the host's validated target. Shell extension discovery
//! starts only when the full menu is requested.
//!
//! This page is the first thing a right click shows. It is built from ZSDesk's
//! own command set and never instantiates `IContextMenu`, so no third-party
//! shell extension (7-Zip, WPS, Git, …) is loaded and nothing can block. The
//! full Shell menu stays one deliberate step away: "显示更多选项" here, or
//! a Shift right click, which skips this page entirely.
use super::{Mode, Request};
use zsdesk_ui::menus::{self, MenuItemSpec, MenuSpec};
use zsdesk_windows::shell::{file_transfer, menu::{ContextMenuOptions, MenuOutcome}};

pub(super) fn enabled(request: &Request) -> bool {
    request.options.quick_first && !request.extended && matches!(request.mode, Mode::More | Mode::Background)
}

pub(super) fn specification(request: &Request) -> MenuSpec {
    let background = request.mode == Mode::Background;
    let files = request.file_actions && (background || file_transfer::paths_from_pidls(&request.pidls).is_ok());
    // A selected folder is a paste target, exactly like the Shell menu treats it.
    let single_folder = !background && request.pidls.len() == 1 && is_folder(&request.pidls[0]);
    let paste = file_transfer::has_paste_content() && (background || single_folder);
    command_menu(&request.options, background, request.pidls.len(), files, request.open, request.rename, paste)
}

fn is_folder(pidl: &[u8]) -> bool {
    let Ok(item) = zsdesk_windows::shell::pidl::item_from_pidl(pidl) else { return false; };
    unsafe { item.GetAttributes(windows::Win32::System::SystemServices::SFGAO_FOLDER) }
        .is_ok_and(|value| value.0 & windows::Win32::System::SystemServices::SFGAO_FOLDER.0 != 0)
}

fn command_menu(options: &ContextMenuOptions, background: bool, count: usize, files: bool, open: bool, rename: bool, paste: bool) -> MenuSpec {
    let mut commands: Vec<MenuItemSpec> = Vec::new();
    // A hidden command stays usable by its keyboard shortcut; only the row goes.
    let row = |commands: &mut Vec<MenuItemSpec>, available: bool, verb: &str, caption: &str, hint: &str| {
        let label = zsdesk_core::i18n::text(caption);
        if !available || !zsdesk_windows::shell::menu::common_command_visible(options, verb, label) { return; }
        let label = if hint.is_empty() { label.to_string() } else { format!("{label}\t{hint}") };
        commands.push(menus::item(&label, verb));
    };
    row(&mut commands, !background && count == 1 && open, "open", "打开", "Enter");
    row(&mut commands, !background && count == 1 && files, "openwith", "打开方式", "");
    if !commands.is_empty() { commands.push(MenuItemSpec::Separator); }
    row(&mut commands, !background && files, "cut", "剪切", "Ctrl+X");
    row(&mut commands, !background && files, "copy", "复制", "Ctrl+C");
    row(&mut commands, !background && files, "copypath", "复制路径", "Ctrl+Shift+C");
    row(&mut commands, paste, "paste", "粘贴", "Ctrl+V");
    row(&mut commands, background && files, "newfolder", "新建文件夹", "Ctrl+Shift+N");
    row(&mut commands, background, "refresh", "刷新", "F5");
    row(&mut commands, !background && count == 1 && files && rename, "rename", "重命名", "F2");
    row(&mut commands, !background && files, "delete", "删除", "Del");
    row(&mut commands, true, "properties", "属性", "Alt+Enter");
    commands.push(MenuItemSpec::Separator);
    row(&mut commands, true, "more", "显示更多选项", "");
    menus::menu(commands)
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
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zsdesk_ui::menus::UiCommand;

    fn ids(spec: MenuSpec) -> Vec<String> {
        spec.items.into_iter().filter_map(|row| match row {
            MenuItemSpec::Command { command: UiCommand::Custom { id,.. },.. } => Some(id),
            _ => None,
        }).collect()
    }

    fn label(spec: &MenuSpec, id: &str) -> String {
        spec.items.iter().find_map(|row| match row {
            MenuItemSpec::Command { label, command: UiCommand::Custom { id: current,.. },.. } if current == id => Some(label.clone()),
            _ => None,
        }).unwrap_or_default()
    }

    #[test]
    fn common_page_respects_targets_and_visibility() {
        let mut options=ContextMenuOptions::default();
        options.hidden_verbs.push("copy".into());
        let multi=ids(command_menu(&options,false,2,true,true,true,false));
        assert_eq!(multi,["cut","copypath","delete","properties","more"]);
        let folder=ids(command_menu(&options,true,1,true,false,false,true));
        assert_eq!(folder,["paste","newfolder","refresh","properties","more"]);
        let virtual_item=ids(command_menu(&options,false,1,false,false,false,false));
        assert_eq!(virtual_item,["properties","more"]);
        assert!(outcome("more").is_none() && outcome("properties").is_none() && outcome("openwith").is_none());
    }

    #[test]
    fn the_short_page_offers_no_extension_work() {
        // Every row is one of the host's own commands: the page must stay
        // meaningful when no shell extension is loaded at all.
        for id in ids(command_menu(&ContextMenuOptions::default(),false,1,true,true,true,false)) {
            assert!(matches!(id.as_str(),"open"|"openwith"|"cut"|"copy"|"copypath"|"rename"|"delete"|"properties"|"more"),"{id}");
        }
        assert!(outcome("copypath")==Some(MenuOutcome::CopyPath));
        assert!(outcome("newfolder")==Some(MenuOutcome::NewFolder));
        assert!(outcome("refresh")==Some(MenuOutcome::Refresh));
    }

    #[test]
    fn rows_carry_their_keyboard_hints() {
        let spec=command_menu(&ContextMenuOptions::default(),false,1,true,true,true,false);
        assert!(label(&spec,"copy").ends_with("\tCtrl+C"),"{}",label(&spec,"copy"));
        assert!(label(&spec,"rename").ends_with("\tF2"),"{}",label(&spec,"rename"));
        assert!(label(&spec,"copypath").ends_with("\tCtrl+Shift+C"),"{}",label(&spec,"copypath"));
        // A caption always survives translation: the label is never empty.
        assert!(!label(&spec,"copy").trim().is_empty());
    }
}
