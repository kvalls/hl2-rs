//! Source-resource HUD behavior shared by hosts through explicit draw commands.
pub mod canvas;
pub mod console;
pub mod fades;
pub mod hud;
#[cfg(windows)]
mod native_font;
