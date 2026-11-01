//! Portable contributions installed by the application, never discovered by core.
use hl2_ui::{
    config::{AdvancedVideoSettings, VideoSettings},
    extensions::{Extensions, VideoExtension, VideoPolicy},
    gameui::display::{AspectRatio, DisplayModes, Resolution},
    gameui::settings::CustomValue,
};
#[derive(Debug)]
pub struct AdvancedPolicy;
impl VideoExtension for AdvancedPolicy {
    fn render_budget(&self, video: &VideoSettings) -> hl2_ui::extensions::RenderBudget {
        if crate::settings::potato(video) {
            crate::settings::POTATO_BUDGET
        } else {
            Default::default()
        }
    }
    fn frame_size(
        &self,
        configured: glam::UVec2,
        window: glam::UVec2,
        flexible: bool,
    ) -> glam::UVec2 {
        if flexible {
            crate::flexible_frame::flexible_frame_resolution(configured, window)
        } else {
            configured.max(glam::UVec2::ONE)
        }
    }
    fn viewport(
        &self,
        target: glam::UVec2,
        window: glam::UVec2,
        scale: f32,
        flexible: bool,
    ) -> source_ui::viewport::ViewportBounds {
        crate::flexible_frame::compute_with_mode(target, window, scale, flexible)
    }
    fn window_size(
        &self,
        resolution: glam::UVec2,
        room: Option<glam::Vec2>,
        flexible: bool,
    ) -> glam::Vec2 {
        if flexible {
            crate::flexible_frame::flexible_windowed_size(resolution, room)
        } else {
            source_ui::viewport::windowed_size(resolution, room)
        }
    }
    fn projection(&self, view: f32, default: f32, aspect: f32, vertical: f32) -> f32 {
        crate::fov::vertical_fov(view, default, aspect, vertical)
    }

    fn follows_window(&self, video: &VideoSettings) -> bool {
        crate::settings::fit_display(video)
    }
    fn projection_parameter(&self, video: &VideoSettings) -> f32 {
        crate::settings::vertical_fov(video)
    }
    fn viewmodel_fov(&self, video: &VideoSettings) -> f32 {
        crate::settings::viewmodel_fov(video)
    }
    fn retail_camera_blur(&self, video: &AdvancedVideoSettings) -> bool {
        crate::motion_blur::selected(video).retail_camera()
    }
    fn selected_blur(&self, video: &AdvancedVideoSettings) -> usize {
        crate::motion_blur::selected(video) as usize
    }
    fn blur_status(&self) -> Option<&'static str> {
        Some("#HL2RS_StatusMotionBlur")
    }
    fn framing_profiles(&self) -> &'static dyn hl2_ui::extensions::FramingProfiles {
        &crate::framing::AdvancedFraming
    }
    fn custom_values(&self) -> &'static [CustomValue] {
        &crate::settings::SLIDERS
    }
    fn stored_aspect(&self, video: &VideoSettings) -> Option<AspectRatio> {
        crate::settings::fit_display(video).then_some(crate::display::FIT_DISPLAY)
    }
    fn store_aspect(&self, video: &mut VideoSettings, aspect: AspectRatio) {
        crate::settings::set_fit_display(video, aspect.follows_window());
    }
    fn blur_tokens(&self) -> &'static [&'static str] {
        crate::BLUR_TOKENS
    }
    fn select_blur(&self, video: &mut AdvancedVideoSettings, index: usize) {
        if index < crate::BLUR_TOKENS.len() {
            video.motion_blur = index != 0;
            crate::settings::set_blur_tier(video, index >= 2, index == 3);
        }
    }
    fn aspects(&self) -> &'static [AspectRatio] {
        crate::display::ASPECTS
    }
    fn aspect_of(&self, width: u32, height: u32) -> AspectRatio {
        crate::display::aspect_of(width, height)
    }
    fn modes(&self, modes: &DisplayModes, windowed: bool) -> Vec<Resolution> {
        crate::display::choices(modes, windowed)
    }
    fn presented(&self, resolution: Resolution) -> Resolution {
        let size = crate::flexible_frame::flexible_presented_resolution(glam::UVec2::new(
            resolution.width,
            resolution.height,
        ));
        Resolution::new(size.x, size.y)
    }
}
pub fn register(extensions: &mut Extensions) -> Result<(), String> {
    extensions.commands(&crate::commands::SPECS)?;
    extensions.select_video("hl2-advanced-options", VideoPolicy::new(AdvancedPolicy))?;
    extensions.layouts.extend_from_slice(crate::LAYOUT_ADDENDA);
    extensions
        .localization
        .extend_from_slice(crate::localize::PLACEHOLDER_STRINGS);
    extensions
        .after_apply
        .push(crate::video_confirm::after_apply);
    Ok(())
}
