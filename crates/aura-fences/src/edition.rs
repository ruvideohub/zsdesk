//! Product identity and profile namespace for the standalone Fences app.

pub const DESKTOP: bool = true;
pub const THEMED: bool = false;

pub const fn profile_name() -> &'static str { "AuraFences" }
pub const fn desktop_startup_value() -> &'static str { "AuraFences.Desktop" }
pub const fn application_mutex() -> &'static str { "Local\\AuraFences.Application" }
