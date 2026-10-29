//! A game-agnostic, Source-aware UI toolkit, as the Source SDK's `vgui_controls` is to every
//! Source game: canvas drawing, fonts, input frames, widgets and their scheme looks, dialog
//! frames, placement and window focus, message boxes, `.res` layouts, scheme fonts, key names,
//! viewport geometry and the localization token table. It reads Source formats through
//! source-assets and knows nothing about the game that uses it (see `docs/DESIGN.md`).
pub mod canvas;
pub mod card_browser;
pub mod console;
pub mod font;
pub mod input;
pub mod keys;
pub mod layout;
pub mod localize;
pub mod message_box;
#[cfg(windows)]
pub mod native_font;
pub mod placement;
pub mod scheme_fonts;
pub mod shell;
pub mod viewport;
pub mod widgets;
pub mod windows;

/// Embedded MIT-licensed fallback / developer font bytes (ProggyClean).
pub const PROGGY_CLEAN: &[u8] = include_bytes!("../assets/fonts/ProggyClean.ttf");
