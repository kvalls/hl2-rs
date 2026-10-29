//! Shared tier policy; legacy saved booleans remain backwards compatible.
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
impl super::AdvancedVideoSettings {
    pub fn motion_blur_mode(&self) -> MotionBlurMode {
        if !self.motion_blur {
            MotionBlurMode::Disabled
        } else if self.motion_blur_world {
            MotionBlurMode::Ultra
        } else if self.motion_blur_viewmodel {
            MotionBlurMode::Advanced
        } else {
            MotionBlurMode::Enabled
        }
    }
}
