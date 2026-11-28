//! Stable category ids and conservative local app classification.
//! The user override wins; unknown apps remain in Other (and in All apps).

pub const CATEGORIES: [(&str, &str); 6] = [
    ("communication", "Communication"), ("games", "Games"), ("productivity", "Productivity"),
    ("media", "Media"), ("system", "System"), ("other", "Other"),
];
pub const VIEWS: [&str; 8] = ["Pinned", "All apps", "Communication", "Games", "Productivity", "Media", "System", "Other"];

pub fn category<'a>(name: &str, id: &str, assigned: Option<&'a str>) -> &'a str {
    if let Some(value) = assigned.filter(|value| CATEGORIES.iter().any(|(id, _)| id == value)) {
        return value;
    }
    let name = name.to_lowercase();
    let id = id.to_lowercase();
    // The installed-app collector admits .url entries only after verifying
    // that they are Steam game shortcuts; ordinary web links are filtered.
    if id.ends_with(".url") { return "games"; }
    let text = format!("{name} {id}");
    let any = |terms: &[&str]| terms.iter().any(|term| text.contains(term));
    if any(&["telegram", "discord", "lolka", "whatsapp", "signal", "skype", "teams", "zoom", "slack"]) { "communication" }
    else if any(&["steam", "epic games", "battle.net", "minecraft", "riot client", "ubisoft connect", "gog galaxy"]) { "games" }
    else if any(&["visual studio", "vscode", "code.exe", "notepad", "блокнот", "office", "word", "excel", "powerpoint", "libreoffice", "obsidian"]) { "productivity" }
    else if any(&["spotify", "vlc", "youtube", "obs studio", "photoshop", "фотографии", "music", "музыка", "video", "видео"]) { "media" }
    else if any(&["windows security", "безопасность windows", "calculator", "калькулятор", "task manager", "диспетчер задач", "terminal", "powershell", "проводник", "file explorer", "settings", "параметры"]) { "system" }
    else { "other" }
}

/// Existing manual single-category assignments override automatic classification.
/// Extra memberships allow an app in multiple categories; exclusions can remove
/// an automatic match without disturbing the user's other categories.
pub fn contains(selected: &str, name: &str, id: &str, assigned: Option<&str>, added: &[String], excluded: &[String]) -> bool {
    if excluded.iter().any(|value| value == id) { return false; }
    added.iter().any(|value| value == id) || category(name, id, assigned) == selected
}

pub fn set_membership(id: &str, member: bool, added: &mut Vec<String>, excluded: &mut Vec<String>) {
    added.retain(|v| v != id);
    excluded.retain(|v| v != id);
    if member { added.push(id.to_string()); } else { excluded.push(id.to_string()); }
}

pub fn valid_name(name: &str, existing: &[String]) -> bool {
    let name = name.trim();
    !name.is_empty() && name.chars().count() <= 48 && !name.chars().any(char::is_control)
        && !existing.iter().any(|old| old.trim().eq_ignore_ascii_case(name) || old.trim().to_lowercase() == name.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn memberships_add_and_exclude_without_affecting_other_categories() {
        let mut added = Vec::new();
        let mut excluded = Vec::new();
        assert!(contains("communication", "Telegram", "tg", None, &added, &excluded));
        set_membership("tg", false, &mut added, &mut excluded);
        assert!(!contains("communication", "Telegram", "tg", None, &added, &excluded));
        set_membership("tg", true, &mut added, &mut excluded);
        assert!(contains("communication", "Telegram", "tg", None, &added, &excluded));
        assert!(contains("custom:1", "Telegram", "tg", None, &added, &excluded));
    }
    #[test]
    fn names_reject_empty_duplicates_and_controls() {
        assert!(!valid_name("   ", &[]));
        assert!(!valid_name(" ИГРЫ ", &["игры".into()]));
        assert!(!valid_name("bad\nname", &[]));
        assert!(valid_name("Избранное", &["игры".into()]));
    }
    #[test]
    fn automatic_and_manual_classification() {
        assert_eq!(category("Telegram", "telegram.exe", None), "communication");
        assert_eq!(category("Steam", "steam.exe", None), "games");
        assert_eq!(category("Unknown", "foo", None), "other");
        assert_eq!(category("Steam", "steam.exe", Some("media")), "media");
        assert_eq!(category("Steam", "steam.exe", Some("invalid")), "games");
        assert_eq!(category("Portal", r"C:\Users\User\Desktop\Portal.URL", None), "games");
        assert_eq!(category("Portal", r"C:\Users\User\Desktop\Portal.url", Some("other")), "other");
    }
}
