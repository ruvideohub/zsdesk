//! Shared desktop-surface design tokens, independent of any windowing backend.
//! Applications may override opacity, tint and corner radius per surface.
pub const SURFACE_DARK: [u8; 3] = [32, 32, 40];
pub const SURFACE_LIGHT: [u8; 3] = [246, 246, 250];
pub const CORNER_RADIUS_DIP: u8 = 24;
/// Corner radius of an interactive item or compact inset panel.
pub const ITEM_RADIUS_DIP: u8 = 8;
