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
    let text = format!("{name} {id}");
    let any = |terms: &[&str]| terms.iter().any(|term| text.contains(term));
    if any(&["telegram", "discord", "lolka", "whatsapp", "signal", "skype", "teams", "zoom", "slack"]) { "communication" }
    else if any(&["steam", "epic games", "battle.net", "minecraft", "riot client", "ubisoft connect", "gog galaxy"]) { "games" }
    else if any(&["visual studio", "vscode", "code.exe", "notepad", "блокнот", "office", "word", "excel", "powerpoint", "libreoffice", "obsidian"]) { "productivity" }
    else if any(&["spotify", "vlc", "youtube", "obs studio", "photoshop", "фотографии", "music", "музыка", "video", "видео"]) { "media" }
    else if any(&["windows security", "безопасность windows", "calculator", "калькулятор", "task manager", "диспетчер задач", "terminal", "powershell", "проводник", "file explorer", "settings", "параметры"]) { "system" }
    else { "other" }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_and_manual_classification() {
        assert_eq!(category("Telegram", "telegram.exe", None), "communication");
        assert_eq!(category("Steam", "steam.exe", None), "games");
        assert_eq!(category("Unknown", "foo", None), "other");
        assert_eq!(category("Steam", "steam.exe", Some("media")), "media");
        assert_eq!(category("Steam", "steam.exe", Some("invalid")), "games");
    }
}
