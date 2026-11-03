//! The executable selects concrete implementations; the reusable host knows only contracts.
use bevy::prelude::App;
pub fn install(_app: &mut App) {
    #[cfg(feature = "advanced-options")]
    hl2_advanced_options::bevy::host::install(_app);
}
