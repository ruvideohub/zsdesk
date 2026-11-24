//! Language-independent aliases for explicit Launchpad actions.
//!
//! Do not add a bare noun for a destructive action ("корзина" should not mean
//! emptying it), or a directional phrase for a toggle that could do the opposite.

pub fn russian(key: &str) -> &'static [&'static str] {
    match key {
        "lock" => &[
            "заблокировать",
            "блокировка экрана",
            "заблокировать компьютер",
        ],
        "snip" => &["скриншот", "снимок экрана", "сделать скриншот"],
        "sleep" => &["спящий режим", "сон компьютера"],
        "signout" => &["выйти из системы", "выход из системы"],
        "restart" => &["перезагрузка", "перезагрузить", "перезапуск компьютера"],
        "shutdown" => &[
            "завершение работы",
            "завершить работу",
            "выключение компьютера",
            "выключить компьютер",
        ],
        _ => &[],
    }
}

/// Prefix matching only; generic later words must not suggest an action.
pub fn matching<'a>(key: &str, query: &'a str) -> impl Iterator<Item = &'static str> + 'a {
    russian(key)
        .iter()
        .copied()
        .filter(move |alias| !query.is_empty() && alias.starts_with(query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_explicit_and_do_not_route_to_toggles_or_empty_bin() {
        for key in ["lock", "snip", "sleep", "signout", "restart", "shutdown"] {
            assert!(!russian(key).is_empty(), "{key}");
            assert!(russian(key)
                .iter()
                .all(|s| !s.is_empty() && *s == s.to_lowercase()));
        }
        assert!(matching("shutdown", "выключить").any(|s| s == "выключить компьютер"));
        assert_eq!(matching("shutdown", "компьютер").count(), 0);
        assert_eq!(matching("shutdown", "").count(), 0);
        for key in [
            "emptybin",
            "wifi",
            "mute",
            "bluetooth",
            "dark",
            "not-a-command",
        ] {
            assert!(russian(key).is_empty(), "{key}");
        }
    }
}
