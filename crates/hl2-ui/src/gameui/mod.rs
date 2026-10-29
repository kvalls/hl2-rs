//! Resource-driven GameUI: main menu, quit confirmation and the options dialog.
//!
//! Models in this module are plain data with explicit inputs and typed effects, so they are unit
//! tested without a window. Text, menu structure, control layout and key actions come from the
//! installed game (see `docs/gameui.md`); hosts only translate input and apply effects.
pub mod chapters;
pub mod display;
pub mod loading;
pub mod localize;
pub mod menu;
pub mod new_game;
pub mod options;
pub mod recommendation;
pub mod resources;
pub mod settings;
pub mod video;
pub mod video_confirm;
