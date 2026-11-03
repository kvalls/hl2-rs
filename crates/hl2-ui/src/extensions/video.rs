//! Injected video policy. Serialized preferences remain independent of installed extensions.
use crate::{
    config::{AdvancedVideoSettings, VideoSettings},
    gameui::display::{AspectRatio, DisplayModes, Resolution},
    gameui::settings::{AdvancedChoice, CustomValue},
};
use std::sync::{Arc, LazyLock};

/// Work a video policy gives up beyond the settings' own levels. The default gives up nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderBudget {
    /// No sky cameras: this linear color fills what the world does not cover.
    pub flat_sky: Option<[f32; 3]>,
    /// The frame is presented, and base textures are sampled, without filtering.
    pub nearest: bool,
    /// Materials load without environment maps.
    pub plain_materials: bool,
    /// No dynamic shadows of any kind.
    pub no_shadows: bool,
    /// Further texture mips skipped and model levels of detail dropped.
    pub extra_picmip: i32,
    pub extra_root_lod: usize,
}

pub trait VideoExtension: std::fmt::Debug + Send + Sync {
    /// Read when the video settings change; the host keeps the answer.
    fn render_budget(&self, _video: &VideoSettings) -> RenderBudget {
        RenderBudget::default()
    }
    /// An item this policy lists below a Video > Advanced combo's lowest.
    fn below_lowest_token(&self, _choice: AdvancedChoice) -> Option<&'static str> {
        None
    }
    fn is_below_lowest(&self, _video: &AdvancedVideoSettings, _choice: AdvancedChoice) -> bool {
        false
    }
    fn set_below_lowest(
        &self,
        _video: &mut AdvancedVideoSettings,
        _choice: AdvancedChoice,
        _on: bool,
    ) {
    }
    fn frame_size(
        &self,
        configured: glam::UVec2,
        _window: glam::UVec2,
        _follows_window: bool,
    ) -> glam::UVec2 {
        configured.max(glam::UVec2::ONE)
    }
    fn viewport(
        &self,
        target: glam::UVec2,
        window: glam::UVec2,
        scale: f32,
        _follows_window: bool,
    ) -> source_ui::viewport::ViewportBounds {
        source_ui::viewport::ViewportBounds::compute(target, window, scale)
    }
    fn window_size(
        &self,
        resolution: glam::UVec2,
        room: Option<glam::Vec2>,
        _follows_window: bool,
    ) -> glam::Vec2 {
        source_ui::viewport::windowed_size(resolution, room)
    }
    fn projection(&self, view: f32, _default: f32, aspect: f32, _vertical: f32) -> f32 {
        2. * ((view.to_radians() / 2.).tan() / aspect.min(4. / 3.)).atan()
    }
    fn follows_window(&self, _video: &VideoSettings) -> bool {
        false
    }
    /// A value the policy reads once per config change and receives back in `projection`.
    fn projection_parameter(&self, _video: &VideoSettings) -> f32 {
        0.
    }
    fn viewmodel_fov(&self, _video: &VideoSettings) -> f32 {
        hl2_simulation::viewmodel_effects::VIEWMODEL_FOV
    }
    fn retail_camera_blur(&self, video: &AdvancedVideoSettings) -> bool {
        video.motion_blur
    }
    /// The selected row of the motion blur combo.
    fn selected_blur(&self, video: &AdvancedVideoSettings) -> usize {
        usize::from(video.motion_blur)
    }
    /// A development status token for the motion blur combo.
    fn blur_status(&self) -> Option<&'static str> {
        None
    }
    /// The viewmodel framing profiles this policy offers.
    fn framing_profiles(&self) -> &'static dyn super::FramingProfiles {
        &super::framing::RetailFraming
    }
    /// Sliders and their value labels this policy adds to the settings dialogs.
    fn custom_values(&self) -> &'static [CustomValue] {
        &[]
    }
    /// The aspect choice stored outside width and height, if one is selected.
    fn stored_aspect(&self, _video: &VideoSettings) -> Option<AspectRatio> {
        None
    }
    /// Records an aspect choice that width and height cannot express.
    fn store_aspect(&self, _video: &mut VideoSettings, _aspect: AspectRatio) {}
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
/// The viewmodel field of view without a policy that changes it.
pub const RETAIL_VIEWMODEL_FOV: f32 = hl2_simulation::viewmodel_effects::VIEWMODEL_FOV;

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
        if aspect.follows_window() {
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
