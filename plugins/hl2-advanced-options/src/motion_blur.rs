//! Advanced tier selection from backwards-compatible settings.
pub use modkit_core::video::MotionBlurMode;
pub fn mode(enabled: bool, viewmodel: bool, world: bool) -> MotionBlurMode {
    if !enabled {
        MotionBlurMode::Disabled
    } else if world {
        MotionBlurMode::Ultra
    } else if viewmodel {
        MotionBlurMode::Advanced
    } else {
        MotionBlurMode::Enabled
    }
}
