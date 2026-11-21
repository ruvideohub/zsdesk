//! AuraExplorer release edition identity, selected with Cargo features.
//!
//! The `desktop` feature means that release packages include the separate Fences.exe
//! companion. The Fences host and model are not part of AuraExplorer.

pub const NETWORK: bool = cfg!(feature = "network");
pub const DESKTOP: bool = cfg!(feature = "desktop");
pub const THEMED: bool = cfg!(feature = "themed");

pub const fn profile_name() -> &'static str {
    if THEMED { "AuraExplorer-Theme" } else { "AuraExplorer" }
}

pub const fn application_id() -> &'static str {
    if THEMED { "AuraExplorer.Theme" } else { "AuraExplorer.Native" }
}

pub const fn application_mutex() -> &'static str {
    if THEMED { "Local\\AuraExplorer.Theme.Application" } else { "Local\\AuraExplorer.Native.Application" }
}

pub const fn key() -> &'static str {
    if THEMED { return "themed"; }
    match (NETWORK, DESKTOP) {
        (true, true) => "full",
        (true, false) => "network",
        (false, true) => "desktop",
        (false, false) => "standard",
    }
}

pub fn name() -> &'static str {
    aura_core::i18n::text(if THEMED { "主题版" } else { match (NETWORK, DESKTOP) {
        (true, true) => "全量版",
        (true, false) => "网络版",
        (false, true) => "桌面版",
        (false, false) => "标准版",
    }})
}
