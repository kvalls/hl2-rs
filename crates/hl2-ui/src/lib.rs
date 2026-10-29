//! Half-Life 2's own UI over the source-ui toolkit: GameUI (menus, Options pages, loading,
//! backgrounds, chapters), the HUD, captions, fades, the settings and the console commands.
//! Hosts drive it through explicit draw commands.
pub mod closecaption;
pub mod config;
pub mod console;
pub mod fades;
pub mod flexible_frame;
pub mod gameui;
pub mod hdr;
pub mod hud;
pub mod input;
pub mod proportional;
pub mod tuning;
