/// One row this application adds to a Shell menu.
///
/// Deliberately not the UI toolkit's menu model: the caller already owns the list it
/// derived these from, and a label is only ever shown. Borrowed text lives as long as the
/// menu being built, which is the only thing that reads it.
pub enum MenuRow<'a> {
    Command { label: std::borrow::Cow<'a, str>, identity: std::borrow::Cow<'a, str>, checked: bool, enabled: bool },
    Submenu { label: std::borrow::Cow<'a, str>, items: Vec<MenuRow<'a>> },
    Separator,
}

/// Inserts application rows after a Shell menu's own rows.
///
/// The returned identities keep the order they were inserted in, so the caller decodes a
/// chosen command number by position. Numbers start at [`crate::shell::menu::APP_FIRST`],
/// the range the Shell never uses; a submenu's children take numbers from the same
/// counter in the order they are built.
pub fn insert_menu_items(menu: HMENU, rows: &[MenuRow<'_>]) -> Vec<String> {
    let mut identities = Vec::new();
    let count = unsafe { GetMenuItemCount(Some(menu)) }.max(0) as u32;
    let mut position = count;
    let mut command = crate::shell::menu::APP_FIRST;
    // The Shell's rows and ours are separated, but only when both sides have something
    // to show: a leading separator would look like a drawing bug.
    let separated = count > 0 && !rows.is_empty()
        && unsafe { InsertMenuW(menu, position, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null()) }.is_ok();
    if separated { position += 1; }
    insert_application_rows(menu, &mut position, rows, &mut identities, &mut command);
    if identities.is_empty() && separated {
        // Nothing was inserted after all, so the separator would separate nothing.
        let _ = unsafe { DeleteMenu(menu, count, MF_BYPOSITION) };
    }
    identities
}

fn insert_application_rows(menu: HMENU, position: &mut u32, rows: &[MenuRow<'_>], identities: &mut Vec<String>, command: &mut u32) {
    for row in rows {
        match row {
            MenuRow::Command { label, identity, checked, enabled } => {
                let value = *command;
                let flags = MF_BYPOSITION | MF_STRING | if *checked { MF_CHECKED } else { MF_UNCHECKED }
                    | if *enabled { MF_ENABLED } else { MF_GRAYED };
                let text = wide(label);
                if unsafe { InsertMenuW(menu, *position, flags, value as usize, PCWSTR(text.as_ptr())) }.is_err() {
                    // Stop here rather than leave a hole: the caller decodes a command number
                    // by position, so the inserted rows must stay a prefix of the intended
                    // order. The number this row would have taken is not spent either.
                    return;
                }
                *command = command.saturating_add(1);
                identities.push(identity.to_string());
                *position += 1;
            }
            MenuRow::Submenu { label, items } => {
                let Ok(submenu) = (unsafe { CreatePopupMenu() }) else { continue };
                // Children are numbered and recorded as they are built, so a submenu that
                // cannot be attached must take its children's numbers back with it.
                let mark = (identities.len(), *command);
                let mut child = 0u32;
                insert_application_rows(submenu, &mut child, items, identities, command);
                let text = wide(label);
                if unsafe { InsertMenuW(menu, *position, MF_BYPOSITION | MF_POPUP | MF_ENABLED, submenu.0 as usize, PCWSTR(text.as_ptr())) }.is_err() {
                    let _ = unsafe { DestroyMenu(submenu) };
                    identities.truncate(mark.0);
                    *command = mark.1;
                    // Same reason as above: what was inserted stays a prefix of the order.
                    return;
                }
                *position += 1;
            }
            MenuRow::Separator => {
                let _ = unsafe { InsertMenuW(menu, *position, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null()) };
                *position += 1;
            }
        }
    }
}

fn wide(text: &str) -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() }

