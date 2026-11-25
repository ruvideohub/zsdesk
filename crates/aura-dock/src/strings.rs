//! The panel's own labels, in Russian and English.
//!
//! Upstream Laminar Dock is English-only. AuraExplorer ships the panel in Russian by default
//! (`language: "ru"` in the settings file; `"en"` for English, `"auto"` to follow the
//! Windows interface language).
//!
//! Translation happens at the few places that draw or build text — `menu::MenuItem`,
//! `launchpad::Launchpad::txt` — and matches whole strings. Anything not in the table
//! passes through untouched, so app names, paths, file names and the user's own text are
//! never rewritten by accident.

use std::sync::atomic::{AtomicU8, Ordering};

const EN: u8 = 0;
const RU: u8 = 1;

/// The panel speaks Russian unless asked otherwise.
static LANGUAGE: AtomicU8 = AtomicU8::new(RU);

/// `"ru"`, `"en"` or `"auto"` (Windows' own interface language).
pub fn set_language(code: &str) {
    let russian = match code.trim().to_ascii_lowercase().as_str() {
        "en" | "english" => false,
        "auto" => windows_is_russian(),
        // "ru" and anything unexpected: the panel ships in Russian.
        _ => true,
    };
    LANGUAGE.store(if russian { RU } else { EN }, Ordering::Relaxed);
}

pub fn russian() -> bool {
    LANGUAGE.load(Ordering::Relaxed) == RU
}

fn windows_is_russian() -> bool {
    const RUSSIAN: u16 = 0x0419;
    let language = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    language == RUSSIAN
}

/// Translate a label; text that has no translation is returned unchanged.
pub fn tr(text: &str) -> String {
    if russian() {
        if let Some(russian) = lookup(text) {
            return russian.to_string();
        }
        if let Some(russian) = patterned(text) {
            return russian;
        }
    }
    text.to_string()
}

/// Labels that carry a number: "Close All 3 Windows", "All Apps   12".
fn patterned(text: &str) -> Option<String> {
    if let Some(count) = text.strip_prefix("Background apps (").and_then(|s| s.strip_suffix(')')) {
        if !count.is_empty() && count.chars().all(|c| c.is_ascii_digit()) {
            return Some(format!("Фоновые приложения ({count})"));
        }
    }
    if let Some(count) = text.strip_prefix("Show hidden tray icons (").and_then(|s| s.strip_suffix(')')) {
        if !count.is_empty() && count.chars().all(|c| c.is_ascii_digit()) {
            return Some(format!("Показать скрытые значки области уведомлений ({count})"));
        }
    }
    // The control centre's network count: "4 available".
    if let Some(count) = text.strip_suffix(" available") {
        if !count.is_empty() && count.chars().all(|c| c.is_ascii_digit()) {
            return Some(format!("{count} доступно"));
        }
    }
    if let Some(rest) = text.strip_prefix("Close All ") {
        if let Some(count) = rest.strip_suffix(" Windows") {
            return Some(format!("Закрыть все окна ({count})"));
        }
    }
    if let Some(rest) = text.strip_prefix("All Apps") {
        return Some(format!("Все приложения{rest}"));
    }
    None
}

/// Same, for the menu builder, which already owns its strings.
pub fn tr_owned(text: String) -> String {
    if russian() {
        if let Some(russian) = lookup(&text) {
            return russian.to_string();
        }
    }
    text
}

fn lookup(text: &str) -> Option<&'static str> {
    Some(match text {
        "Top screen edge gap" => "Отступ от верхнего края экрана",
        "Distance to the top edge when the dock is at the top." => "Расстояние до края экрана, когда панель расположена сверху.",
        "Bottom screen edge gap" => "Отступ от нижнего края экрана",
        "Distance to the bottom edge when the dock is at the bottom." => "Расстояние до края экрана, когда панель расположена снизу.",
        "Gap to maximized windows" => "Зазор до развёрнутых окон",
        "Extra reserved space between windows and the dock; ignored when auto-hide is on." => "Дополнительное место между окнами и панелью; не применяется при автоскрытии.",
        "Right-click an app in All apps to pin it here" => "Чтобы закрепить приложение здесь, нажмите на него правой кнопкой в разделе «Все приложения»",
        // ------------------------------------------------- clock card (right-click it)
        "Desktop Clock" => "Часы на рабочем столе",
        "Corner: Top left" => "Угол: верхний левый",
        "Corner: Top right" => "Угол: верхний правый",
        "Corner: Bottom left" => "Угол: нижний левый",
        "Corner: Bottom right" => "Угол: нижний правый",
        "Size: Compact" => "Размер: компактный",
        "Size: Normal" => "Размер: обычный",
        "Size: Large" => "Размер: крупный",
        "24-Hour Time" => "24-часовой формат",
        "Show the Date" => "Показывать дату",
        "Show Seconds" => "Показывать секунды",
        "Hide the Clock" => "Убрать часы",

        // ---------------------------------------------------------------- widgets
        "Widgets on the Panel" => "Виджеты на панели",
        "More Widgets…" => "Другие виджеты…",
        "Clock" => "Часы",
        "Today's Forecast" => "Погода на сегодня",
        "Calendar" => "Календарь",
        "Resource Monitor" => "Монитор ресурсов",
        "Battery" => "Батарея",
        "Now Playing Compact" => "Сейчас играет",
        "Network" => "Сеть",
        "Quick Controls" => "Быстрые действия",

        // ------------------------------------------------- control centre
        "Control Center" => "Центр управления",
        "Airplane Mode" => "Режим полёта",
        "Night Light" => "Ночной свет",
        "Do Not Disturb" => "Не беспокоить",
        "Dark Mode" => "Тёмная тема",
        "Light Mode" => "Светлая тема",
        "Microphone" => "Микрофон",
        "Brightness" => "Яркость",
        "Muted" => "Без звука",
        "Unavailable" => "Недоступно",
        "Off" => "Выкл",
        "Connected" => "Подключено",
        "No networks" => "Нет сетей",
        "Nothing playing" => "Ничего не играет",
        "Plugged in" => "Питание от сети",
        "No networks in range" => "Нет доступных сетей",
        "Turn on Location access to see networks" => "Включите доступ к геопозиции, чтобы видеть сети",

        // ---------------------------------------------------------------- dock menu
        "Open" => "Открыть",
        "Open New Window" => "Открыть новое окно",
        "Keep in Dock" => "Оставить в панели",
        "Add Separator After" => "Добавить разделитель",
        "Remove Separator" => "Убрать разделитель",
        "New Stack" => "Новый стек",
        "Rename Stack…" => "Переименовать стек…",
        "Ungroup Stack" => "Разгруппировать стек",
        "Remove from Dock" => "Убрать из панели",
        "Connect…" => "Подключить…",
        "Reset" => "Сбросить",
        "Show in Launchpad" => "Показать в Launchpad",
        "Pin to Launchpad" => "Закрепить в Launchpad",
        "Unpin from Launchpad" => "Открепить от Launchpad",
        "Open Recycle Bin" => "Открыть корзину",
        "Task Manager" => "Диспетчер задач",
        "Settings" => "Параметры",
        "File Explorer" => "Проводник",
        "Terminal" => "Терминал",
        "Automatically Hide the Dock" => "Автоматически скрывать панель",
        "Magnification" => "Увеличение значков",
        "Hide the Windows Taskbar" => "Скрывать панель задач Windows",
        "Dock Settings…" => "Настройки панели…",
        "Check for Updates…" => "Проверить обновления…",
        "Quit AuraDock" => "Выйти из AuraDock",
        "Restart and Update" => "Перезапустить и обновить",
        "What's New" => "Что нового",
        "Remind Me Tomorrow" => "Напомнить завтра",
        "Remind Me Next Week" => "Напомнить через неделю",
        "Skip This Version" => "Пропустить эту версию",
        "Update AuraDock" => "Обновить AuraDock",
        "OK" => "ОК",
        "Close Window" => "Закрыть окно",
        "Run as Administrator" => "Запустить от имени администратора",
        "Open File Location" => "Открыть расположение файла",
        "Lock" => "Заблокировать",
        "Sign out" => "Выйти из системы",
        "Sleep" => "Спящий режим",
        "Shut down" => "Завершить работу",
        "Restart" => "Перезагрузка",
        "User" => "Пользователь",
        "Launchpad" => "Launchpad",
        "Background app" => "Фоновое приложение",
        // ---------------------------------------------------------------- updates
        "Developer build" => "Сборка разработчика",
        "Developer build: updates are off." => "Сборка разработчика: обновления выключены.",
        "Updates: not checked yet." => "Обновления: ещё не проверялись.",
        "Up to date." => "Обновлений нет.",
        "Checking for updates…" => "Проверка обновлений…",
        "Couldn't check for updates" => "Не удалось проверить обновления",
        "Couldn't start the update" => "Не удалось начать обновление",
        "This copy of AuraDock can't update itself" => "Эта копия AuraDock не обновляет себя",
        "Only copies installed with Setup.exe check for updates." => {
            "Обновления проверяют только копии, установленные установщиком."
        }
        // ---------------------------------------------------------------- widget menus
        "Add to Dock" => "Добавить в панель",
        "Change Location…" => "Изменить местоположение…",
        "Move Up" => "Выше",
        "Move Down" => "Ниже",
        "Move to Left Column" => "В левую колонку",
        "Move to Right Column" => "В правую колонку",
        "Remove Widget" => "Убрать виджет",
        "Turn Off in Settings…" => "Отключить в настройках…",
        // ---------------------------------------------------------------- launchpad
        "Top Hit" => "Лучший результат",
        "Commands" => "Команды",
        "Run" => "Выполнить",
        "Confirm" => "Подтвердить",
        "Lock Screen" => "Заблокировать экран",
        "Lock this PC" => "Заблокировать компьютер",
        "Take a Screenshot" => "Сделать снимок экрана",
        "Snipping Tool" => "Ножницы",
        "Sign Out" => "Выйти из системы",
        "Shut Down" => "Завершить работу",
        "Put this PC to sleep" => "Перевести компьютер в спящий режим",
        "Close your apps and sign out" => "Закрыть приложения и выйти из системы",
        "Close your apps and restart" => "Закрыть приложения и перезагрузить компьютер",
        "Close your apps and turn off" => "Закрыть приложения и выключить компьютер",
        "Press Enter again to sleep" => "Нажмите Enter ещё раз для перехода в спящий режим",
        "Press Enter again to sign out" => "Нажмите Enter ещё раз для выхода из системы",
        "Press Enter again to restart" => "Нажмите Enter ещё раз для перезагрузки",
        "Press Enter again to shut down" => "Нажмите Enter ещё раз для выключения",
        "Search apps, settings, files, or calculate" => "Поиск приложений, настроек, файлов или калькулятор",
        "Communication" => "Общение",
        "Games" => "Игры",
        "Productivity" => "Работа",
        "Media" => "Медиа",
        "System" => "Система",
        "Other" => "Другое",
        "Category" => "Категория",
        "Choose category" => "Выбрать категорию",
        "Automatic category" => "Автоматически",
        "No apps in this category" => "В этой категории пока нет приложений",
        "New category" => "Новая категория",
        "Category name" => "Название категории",
        "Enter a unique category name" => "Введите уникальное название категории",
        "Add apps" => "Добавить",
        "Choose apps" => "Выберите приложения",
        "Search apps" => "Поиск приложений",
        "Create" => "Создать",
        "Pinned" => "Закреплённые",
        "All apps" => "Все приложения",
        "Hide all apps" => "Свернуть приложения",
        "Laminar Settings" => "Настройки панели",
        "Changes here apply immediately and are saved to config.json." => {
            "Изменения применяются сразу и сохраняются в config.json."
        }
        "Open settings file" => "Открыть файл настроек",
        // Settings sections
        "Dock" => "Панель",
        "Glass" => "Стекло",
        "Behavior" => "Поведение",
        "Start Dock when I sign in" => "Запускать Dock при входе в Windows",
        "Dock starts the desktop widgets too." => "Dock также запускает виджеты рабочего стола.",
        "About" => "О программе",
        "Widgets" => "Виджеты",
        // Dock section
        "Icon size" => "Размер значков",
        "Icons grow as the pointer moves across the dock." => "Значки растут под указателем.",
        "Magnified size" => "Размер при увеличении",
        "Magnification spread" => "Ширина увеличения",
        "Icon spacing" => "Расстояние между значками",
        "Shelf padding" => "Отступы панели",
        "Corner radius" => "Радиус углов",
        // Glass section
        "Tint" => "Тонировка",
        "Lower is more transparent." => "Меньше — прозрачнее.",
        "Edge highlight" => "Подсветка кромки",
        "The bright rim around the glass." => "Светлая кайма по краю стекла.",
        "Match Windows light/dark mode" => "Как в Windows: светлая/тёмная тема",
        "Light glass in light mode. Off: always dark." => "Светлое стекло в светлой теме. Выкл.: всегда тёмное.",
        "Extra blur" => "Дополнительное размытие",
        "Added to Windows' own blur (it can't go lower than that)." => {
            "Добавляется к размытию Windows (меньше него не бывает)."
        }
        "Panel shadow" => "Тень панели",
        "Depth under Launchpad's three panels." => "Глубина под панелью Launchpad.",
        "Widget shadow" => "Тень виджетов",
        "Depth under widgets and tiles." => "Глубина под виджетами и плитками.",
        // Behavior section
        "Automatically hide the dock" => "Автоматически скрывать панель",
        "Push the pointer into the bottom edge to bring it back." => "Подведите указатель к нижнему краю, чтобы вернуть.",
        "Reveal zone height" => "Высота зоны появления",
        "Reveal delay" => "Задержка появления",
        "Hide delay" => "Задержка скрытия",
        "Click the focused app to minimize it" => "Клик по активному приложению сворачивает его",
        "Windows taskbar behaviour." => "Как в панели задач Windows.",
        "Windows key opens Launchpad" => "Клавиша Windows открывает Launchpad",
        "Win + other keys still work as usual." => "Win + другие клавиши работают как обычно.",
        "Hide the Windows taskbar" => "Скрывать панель задач Windows",
        "A watchdog brings it back if Laminar exits." => "Сторож вернёт её, если панель завершится.",
        "Show the dock on all displays" => "Показывать панель на всех экранах",
        "Otherwise only on the main display." => "Иначе только на основном экране.",
        "Each display shows only its own windows" => "Каждый экран показывает только свои окна",
        "Pinned apps always show." => "Закреплённые приложения видны всегда.",
        // Widgets section
        "Background Apps widget" => "Виджет фоновых приложений",
        "Windows Update widget" => "Виджет обновления Windows",
        "Windows Settings widget" => "Виджет параметров Windows",
        "Always in Launchpad unless turned off here." => "Всегда в Launchpad, если не выключить здесь.",
        // About section
        "Start Laminar when you sign in" => "Запускать панель при входе",
        "Off until you turn it on. Installed copies only." => "Выключено, пока не включите. Только установленные копии.",
        "Check for updates automatically" => "Проверять обновления автоматически",
        "New versions download quietly; an icon in the dock offers to install them." => {
            "Новые версии скачиваются тихо; значок в панели предложит установить."
        }
        "How often to check" => "Как часто проверять",
        "Daily by default. Check for updates (above) works any time." => {
            "По умолчанию раз в день. «Проверить обновления» работает в любой момент."
        }
        "At startup" => "При запуске",
        "Daily" => "Раз в день",
        "Weekly" => "Раз в неделю",
        // ---------------------------------------------------------------- Wi-Fi panel
        "Wi-Fi is off." => "Wi-Fi выключен.",
        "Looking for networks…" => "Поиск сетей…",
        "No networks found." => "Сети не найдены.",
        "Other Networks…" => "Другие сети…",
        "Wi-Fi Settings" => "Параметры Wi-Fi",
        "Options" => "Параметры",
        "Open Location Settings" => "Открыть параметры геолокации",
        "Windows needs Location access for desktop apps to list Wi-Fi networks." => {
            "Чтобы показывать сети Wi-Fi, Windows нужен доступ к геолокации для классических приложений."
        }
        "Checking…" => "Проверка…",
        "Check again" => "Проверить снова",
        "Done" => "Готово",
        // ------------------------------------------------- Windows pages in search
        "Bluetooth & devices" => "Bluetooth и устройства",
        "Display" => "Дисплей",
        "Sound" => "Звук",
        "Network & internet" => "Сеть и Интернет",
        "Windows Update" => "Центр обновления Windows",
        "Installed apps" => "Установленные приложения",
        "Default apps" => "Приложения по умолчанию",
        "Power & sleep" => "Питание и спящий режим",
        "Personalization" => "Персонализация",
        "Background (wallpaper)" => "Фон (обои)",
        "Colors & dark mode" => "Цвета и тёмная тема",
        "Notifications" => "Уведомления",
        "Accounts" => "Учётные записи",
        "Date & time" => "Дата и время",
        "Mouse" => "Мышь",
        "Printers & scanners" => "Принтеры и сканеры",
        "Storage" => "Память",
        "About this PC" => "О компьютере",
        // ------------------------------------------------- the panel's own settings
        "Laminar: Dock size & magnification" => "Панель: размер и увеличение",
        "Laminar: Glass & transparency" => "Панель: стекло и прозрачность",
        "Laminar: Auto-hide & behavior" => "Панель: автопрятание и поведение",
        "Laminar: Windows key & taskbar" => "Панель: клавиша Windows и панель задач",
        "Laminar: Widgets & connections" => "Панель: виджеты и подключения",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{set_language, tr, tr_owned, russian};

    #[test]
    fn russian_is_the_default_and_the_table_matches_whole_labels() {
        set_language("ru");
        assert!(russian());
        assert_eq!(tr("All apps"), "Все приложения");
        assert_eq!(tr("Pinned"), "Закреплённые");
        // Data — app names, paths, the user's own words — is never rewritten.
        assert_eq!(tr("Microsoft Edge"), "Microsoft Edge");
        assert_eq!(tr(r"C:\Windows\explorer.exe"), r"C:\Windows\explorer.exe");
        assert_eq!(tr(""), "");
    }

    #[test]
    fn power_confirmation_labels_are_translated() {
        for text in [
            "Press Enter again to sleep", "Press Enter again to sign out",
            "Press Enter again to restart", "Press Enter again to shut down",
            "Confirm", "Run", "Shut Down", "Sign Out", "Sleep", "Restart",
        ] {
            assert!(super::lookup(text).is_some(), "{text}");
        }
    }

    #[test]
    fn english_switches_the_panel_back() {
        set_language("en");
        assert!(!russian());
        assert_eq!(tr("All apps"), "All apps");
        assert_eq!(tr_owned("Open".to_string()), "Open");
        set_language("ru");
        assert_eq!(tr_owned("Open".to_string()), "Открыть");
    }

    #[test]
    fn an_unknown_language_code_still_gives_russian() {
        set_language("");
        assert!(russian());
        set_language("klingon");
        assert!(russian());
    }
}
