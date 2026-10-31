pub use modkit_core::video::MotionBlurMode;
impl super::AdvancedVideoSettings {
    pub fn motion_blur_mode(&self) -> MotionBlurMode {
        self.policy.0.blur_mode(self)
    }
}
