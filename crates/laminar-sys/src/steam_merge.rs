//! Merge a discovered Steam shortcut with an existing AppsFolder entry without
//! changing its launch ID or rewriting any user preferences.
use crate::{apps::AppEntry, steam_discovery::GameShortcut};
use std::collections::HashMap;

fn name_key(name: &str) -> String { name.trim().to_lowercase() }

pub(crate) fn merge(apps: &mut Vec<AppEntry>, games: Vec<GameShortcut>, mut icon_missing: impl FnMut(&str) -> Option<bool>) {
    let original_count = apps.len();
    let mut name_counts = HashMap::<String, usize>::new();
    for game in &games { *name_counts.entry(name_key(&game.name)).or_default() += 1; }
    for game in games {
        let Some(path) = game.path.to_str().map(str::to_owned) else { continue };
        // A Steam URI is definitive, even if the display names are different.
        let exact: Vec<usize> = apps[..original_count].iter().enumerate().filter_map(|(i, app)| {
            (app.icon_source.is_none() && (app.id.eq_ignore_ascii_case(&path)
                || crate::steam_shortcut::game_id(&app.id) == Some(game.appid.as_str()))).then_some(i)
        }).collect();
        let matched = if exact.len() == 1 { Some(exact[0]) } else if exact.is_empty() && name_counts.get(&name_key(&game.name)) == Some(&1) {
            // AppsFolder sometimes supplies an opaque id. Only use a fallback
            // when the name is unique on both sides AND the existing entry's
            // icon really cannot be drawn. Distinct apps with working icons,
            // conflicting Steam IDs and ambiguous names must remain separate.
            let same_name: Vec<usize> = apps[..original_count].iter().enumerate()
                .filter_map(|(i, app)| (name_key(&app.name) == name_key(&game.name)).then_some(i)).collect();
            if same_name.len() == 1 {
                let i = same_name[0];
                let app = &apps[i];
                if app.icon_source.is_none() && crate::steam_shortcut::game_id(&app.id).is_none()
                    && icon_missing(&app.id) == Some(true) && icon_missing(&path) == Some(false) { Some(i) } else { None }
            } else { None }
        } else { None };
        if let Some(i) = matched {
            // Keep the original Shell ID: Launchpad pins, categories and the
            // launch target must not silently switch to a different shortcut.
            if !apps[i].id.eq_ignore_ascii_case(&path) { apps[i].icon_source = Some(path); }
        } else {
            apps.push(AppEntry { id: path, name: game.name, icon_source: None });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn app(id: &str, name: &str) -> AppEntry { AppEntry { id: id.into(), name: name.into(), icon_source: None } }
    fn game(id: &str, name: &str, path: &str) -> GameShortcut { GameShortcut { appid: id.into(), name: name.into(), path: PathBuf::from(path) } }
    #[test]
    fn steam_identity_combines_entries_without_changing_existing_id() {
        let mut apps = vec![app("steam://rungameid/400", "Portal (Steam)")];
        merge(&mut apps, vec![game("400", "Portal", "/game/Portal.url")], |_| panic!("identity requires no probe"));
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "steam://rungameid/400");
        assert_eq!(apps[0].icon_source.as_deref(), Some("/game/Portal.url"));
    }
    #[test]
    fn unique_unrenderable_shell_entry_reuses_the_game_icon() {
        let mut apps = vec![app("opaque-shell-id", "Portal")];
        merge(&mut apps, vec![game("400", "portal", "/game/Portal.url")], |id| Some(id == "opaque-shell-id"));
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "opaque-shell-id");
        assert_eq!(apps[0].icon_source.as_deref(), Some("/game/Portal.url"));
    }
    #[test]
    fn distinct_working_app_or_conflicting_steam_id_is_never_hidden() {
        let mut apps = vec![app("other-platform", "Portal")];
        merge(&mut apps, vec![game("400", "Portal", "/game/Portal.url")], |_| Some(false));
        assert_eq!(apps.len(), 2);
        let mut apps = vec![app("missing-icon", "Portal")];
        merge(&mut apps, vec![game("400", "Portal", "/game/Portal.url")], |_| Some(true));
        assert_eq!(apps.len(), 2, "do not discard either if both icons are unavailable");
        let mut apps = vec![app("steam://run/999", "Portal")];
        merge(&mut apps, vec![game("400", "Portal", "/game/Portal.url")], |_| Some(true));
        assert_eq!(apps.len(), 2);
    }
    #[test]
    fn ambiguous_names_are_never_merged() {
        let mut apps = vec![app("one", "Portal"), app("two", "Portal")];
        merge(&mut apps, vec![game("400", "Portal", "/game/Portal.url")], |_| panic!("ambiguous"));
        assert_eq!(apps.len(), 3);
        let mut apps = vec![app("one", "Portal")];
        merge(&mut apps, vec![game("400", "Portal", "/game/first.url"), game("401", "Portal", "/game/second.url")], |_| panic!("ambiguous"));
        assert_eq!(apps.len(), 3);
    }
    #[test]
    fn same_id_never_adds_a_duplicate() {
        let mut apps = vec![app("/game/Portal.url", "Portal")];
        merge(&mut apps, vec![game("400", "Portal", "/game/Portal.url")], |_| panic!("same path"));
        assert_eq!(apps.len(), 1);
        assert!(apps[0].icon_source.is_none());
    }
}
