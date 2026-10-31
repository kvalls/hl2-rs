impl super::AdvancedVideoSettings {
    /// Whether the retail camera blur pass runs under the selected video policy.
    pub fn retail_camera_blur(&self) -> bool {
        self.policy.0.retail_camera_blur(self)
    }
}
