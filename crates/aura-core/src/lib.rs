//! Shared and Explorer domain models, localization, and common utilities.
//!
//! The Fences desktop-grid model lives in `aura-fences-model`, keeping that
//! product's domain types out of Explorer's package. Domain code has no HWND,
//! COM pointer, or ZSUI control dependencies.

#[macro_use]
pub mod i18n;
pub mod archive;
pub mod atomic_file;
pub mod error;
pub mod model;
pub mod path;
