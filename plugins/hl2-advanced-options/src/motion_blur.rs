//! Advanced tier selection from backwards-compatible settings.
use hl2_ui::config::AdvancedVideoSettings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionBlurMode {
    Disabled,
    Enabled,
    Advanced,
    Ultra,
}
impl MotionBlurMode {
    pub fn viewmodel(self) -> bool {
        matches!(self, Self::Advanced | Self::Ultra)
    }
    pub fn world(self) -> bool {
        self == Self::Ultra
    }
    pub fn retail_camera(self) -> bool {
        matches!(self, Self::Enabled | Self::Advanced)
    }
}
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
pub fn selected(video: &AdvancedVideoSettings) -> MotionBlurMode {
    mode(
        video.motion_blur,
        crate::settings::blur_viewmodel(video),
        crate::settings::blur_world(video),
    )
}
