//! Portable contributions installed by the application, never discovered by core.
use hl2_ui::{
    config::{AdvancedVideoSettings, VideoSettings},
    extensions::{Extensions, VideoExtension, VideoPolicy},
    gameui::display::{AspectRatio, DisplayModes, Resolution},
};
#[derive(Debug)]
pub struct AdvancedPolicy;
impl VideoExtension for AdvancedPolicy {
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

    fn flexible(&self, video: &VideoSettings) -> bool {
        video.flexible_aspect
    }
    fn vertical_fov(&self, video: &VideoSettings) -> f32 {
        video.vertical_fov
    }
    fn viewmodel_fov(&self, video: &VideoSettings) -> f32 {
        video.viewmodel_fov
    }
    fn vertical_range(&self) -> (f32, f32) {
        crate::fov::vertical_fov_range()
    }
    fn viewmodel_range(&self) -> (f32, f32) {
        crate::fov::viewmodel_fov_range()
    }
    fn blur_mode(&self, video: &AdvancedVideoSettings) -> modkit_core::video::MotionBlurMode {
        crate::motion_blur::mode(
            video.motion_blur,
            video.motion_blur_viewmodel,
            video.motion_blur_world,
        )
    }
    fn blur_tokens(&self) -> &'static [&'static str] {
        crate::BLUR_TOKENS
    }
    fn select_blur(&self, video: &mut AdvancedVideoSettings, index: usize) {
        if index < crate::BLUR_TOKENS.len() {
            video.motion_blur = index != 0;
            video.motion_blur_viewmodel = index >= 2;
            video.motion_blur_world = index == 3;
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
    extensions.video = VideoPolicy::new(AdvancedPolicy);
    extensions.layouts.extend_from_slice(crate::LAYOUT_ADDENDA);
    extensions
        .localization
        .extend_from_slice(crate::localize::PLACEHOLDER_STRINGS);
    extensions
        .after_apply
        .push(crate::video_confirm::after_apply);
    Ok(())
}
