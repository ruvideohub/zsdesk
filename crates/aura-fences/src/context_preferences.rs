//! Fences-owned context-menu policy, shared with the isolated Shell-menu worker.
use std::path::Path;
use aura_core::model::explorer::{ContextMenuMode,ContextMenuPreferences};
use aura_windows::shell::menu::{ContextMenuOptions,set_context_menu_options};
static SCHEME:std::sync::atomic::AtomicU8=std::sync::atomic::AtomicU8::new(0);
static SHARED:std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<std::path::PathBuf,ContextMenuPreferences>>>=std::sync::OnceLock::new();
pub fn register(profile:&Path,options:&ContextMenuPreferences){SHARED.get_or_init(Default::default).lock().unwrap_or_else(|e|e.into_inner()).entry(profile.into()).or_insert_with(||options.clone());}
pub fn publish(profile:&Path,options:&ContextMenuPreferences){SHARED.get_or_init(Default::default).lock().unwrap_or_else(|e|e.into_inner()).insert(profile.into(),options.clone());}
pub fn current(profile:&Path)->Option<ContextMenuPreferences>{SHARED.get()?.lock().unwrap_or_else(|e|e.into_inner()).get(profile).cloned()}
/// One place that turns the profile into what the menu code needs.
///
/// There is no backend picker any more: "our own" is the drawn menu, "classic" is the Shell's
/// own TrackPopupMenu. The hidden-command list is the catalog's, nothing else — the per-item
/// checkboxes that used to hide single commands are gone, because the catalog already hides
/// the commands that actually exist.
pub fn native(options:&ContextMenuPreferences)->ContextMenuOptions {
    let own = options.mode == ContextMenuMode::Own;
    // There is no choice left about which page comes first: every right click shows the
    // file's own menu with this application's group under it. Only the style remains.
    ContextMenuOptions{modern:own,drawn:own,
        hidden_verbs:options.hidden_commands.clone(),shown_verbs:options.shown_commands.clone(),
        scheme:scheme()}
}
/// "Modern" still means the Windows 11 shaped page rather than the classic one; it is our own
/// drawing now, so the two styles come down to one flag: ours or the Shell's.
pub fn modern(options:&ContextMenuPreferences)->bool{options.mode==ContextMenuMode::Own}
/// Colour scheme the menus are drawn in; set whenever the window applies its appearance.
pub fn set_scheme(value:aura_core::model::explorer::ColorScheme){SCHEME.store(match value{
    aura_core::model::explorer::ColorScheme::System=>0,aura_core::model::explorer::ColorScheme::Light=>1,aura_core::model::explorer::ColorScheme::Dark=>2},std::sync::atomic::Ordering::Relaxed);}
fn scheme()->aura_core::model::explorer::ColorScheme{match SCHEME.load(std::sync::atomic::Ordering::Relaxed){
    1=>aura_core::model::explorer::ColorScheme::Light,2=>aura_core::model::explorer::ColorScheme::Dark,_=>aura_core::model::explorer::ColorScheme::System}}
pub fn apply_from_profile(profile:&Path){if let Ok(session)=crate::storage::load(profile){set_context_menu_options(native(&session.ui.context_menu));}}
