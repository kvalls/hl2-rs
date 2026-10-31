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
