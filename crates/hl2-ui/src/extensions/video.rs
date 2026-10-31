//! Injected video policy. Serialized preferences remain independent of installed extensions.
use crate::{
    config::{AdvancedVideoSettings, VideoSettings},
    gameui::display::{AspectRatio, DisplayModes, Resolution},
};
use modkit_core::video::MotionBlurMode;
use std::sync::{Arc, LazyLock};

pub trait VideoExtension: std::fmt::Debug + Send + Sync {
    fn frame_size(
        &self,
        configured: glam::UVec2,
        _window: glam::UVec2,
        _flexible: bool,
    ) -> glam::UVec2 {
        configured.max(glam::UVec2::ONE)
    }
    fn viewport(
        &self,
        target: glam::UVec2,
        window: glam::UVec2,
        scale: f32,
        _flexible: bool,
    ) -> source_ui::viewport::ViewportBounds {
        source_ui::viewport::ViewportBounds::compute(target, window, scale)
    }
    fn window_size(
        &self,
        resolution: glam::UVec2,
        room: Option<glam::Vec2>,
        _flexible: bool,
    ) -> glam::Vec2 {
        source_ui::viewport::windowed_size(resolution, room)
    }
    fn projection(&self, view: f32, _default: f32, aspect: f32, _vertical: f32) -> f32 {
        2. * ((view.to_radians() / 2.).tan() / aspect.min(4. / 3.)).atan()
    }
    fn flexible(&self, _video: &VideoSettings) -> bool {
        false
    }
    fn vertical_fov(&self, _video: &VideoSettings) -> f32 {
        0.
    }
    fn viewmodel_fov(&self, _video: &VideoSettings) -> f32 {
        hl2_simulation::viewmodel_effects::VIEWMODEL_FOV
    }
    fn vertical_range(&self) -> (f32, f32) {
        (0., 0.)
    }
    fn viewmodel_range(&self) -> (f32, f32) {
        (54., 54.)
    }
    fn blur_mode(&self, video: &AdvancedVideoSettings) -> MotionBlurMode {
        if video.motion_blur {
            MotionBlurMode::Enabled
        } else {
            MotionBlurMode::Disabled
        }
    }
    fn blur_tokens(&self) -> &'static [&'static str] {
        &["#gameui_disabled", "#gameui_enabled"]
    }
    fn select_blur(&self, video: &mut AdvancedVideoSettings, index: usize) {
        if index < 2 {
            video.motion_blur = index == 1;
        }
    }
    fn aspects(&self) -> &'static [AspectRatio] {
        &AspectRatio::ALL
    }
    fn aspect_of(&self, width: u32, height: u32) -> AspectRatio {
        AspectRatio::of(width, height)
    }
    fn modes(&self, modes: &DisplayModes, windowed: bool) -> Vec<Resolution> {
        modes.choices(windowed)
    }
    fn presented(&self, resolution: Resolution) -> Resolution {
        resolution
    }
}
#[derive(Debug)]
struct Retail;
impl VideoExtension for Retail {}

#[derive(Clone, Debug)]
pub struct VideoPolicy(pub Arc<dyn VideoExtension>);
impl Default for VideoPolicy {
    fn default() -> Self {
        static RETAIL: LazyLock<Arc<dyn VideoExtension>> = LazyLock::new(|| Arc::new(Retail));
        Self(RETAIL.clone())
    }
}
impl PartialEq for VideoPolicy {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl VideoPolicy {
    pub fn new(policy: impl VideoExtension + 'static) -> Self {
        Self(Arc::new(policy))
    }
    pub fn resolution_choices(&self, modes: &[Resolution], aspect: AspectRatio) -> Vec<Resolution> {
        if aspect == AspectRatio::Flexible {
            return modes.to_vec();
        }
        let bucket: Vec<_> = modes
            .iter()
            .copied()
            .filter(|m| self.0.aspect_of(m.width, m.height) == aspect)
            .collect();
        if bucket.is_empty() {
            modes.to_vec()
        } else {
            bucket
        }
    }
    pub fn choose_resolution(
        &self,
        modes: &[Resolution],
        aspect: AspectRatio,
        previous: Resolution,
    ) -> Option<Resolution> {
        let choices = self.resolution_choices(modes, aspect);
        if choices.contains(&previous) {
            Some(previous)
        } else {
            choices.first().copied()
        }
    }
}
