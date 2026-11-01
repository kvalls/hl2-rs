//! Only the last active game camera copies the shared intermediate image into the game frame: each
//! earlier camera's copy would be overwritten by the next one.
use super::{GameFrame, PresentCamera};
use bevy::image::{Image, ImageSampler, ImageSamplerDescriptor};
use bevy::render::render_resource::TextureFormat;
use bevy::{
    camera::{Camera, CameraOutputMode, ClearColorConfig, RenderTarget},
    prelude::{Query, Res, UVec2, Without},
    render::render_resource::BlendState,
};
use hl2_ui::extensions::RenderBudget;

/// Every game camera, routed into the frame by `present_frame`.
pub(crate) type FrameCameras<'w, 's> = Query<
    'w,
    's,
    (&'static mut Camera, &'static mut RenderTarget),
    (
        Without<crate::monitors::MonitorCamera>,
        Without<PresentCamera>,
    ),
>;

/// A render-target image for the game frame, linearly filtered when scaled.
pub fn frame_image(resolution: UVec2, nearest: bool) -> Image {
    let size = resolution.max(UVec2::ONE);
    let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::Descriptor(if nearest {
        ImageSamplerDescriptor::nearest()
    } else {
        ImageSamplerDescriptor::linear()
    });
    image
}

/// `size` within the budget's height, keeping its shape.
pub(super) fn budgeted_size(size: UVec2, budget: RenderBudget) -> UVec2 {
    match budget.max_frame_height {
        Some(height) if size.y > height => {
            let width = (size.x as f32 * height as f32 / size.y as f32).round() as u32;
            UVec2::new(width.max(1), height.max(1))
        }
        _ => size,
    }
}

/// Whether a camera of `order` writes the frame when the active cameras end at `last`.
fn writes_frame(order: isize, last: isize) -> bool {
    order == last
}

/// Runs after everything that turns game cameras on and off.
pub(crate) fn skip_covered_outputs(mut cameras: FrameCameras, frame: Option<Res<GameFrame>>) {
    let Some(frame) = frame else {
        return;
    };
    let routed =
        |target: &RenderTarget| matches!(target, RenderTarget::Image(t) if t.handle == frame.image);
    let last = cameras
        .iter()
        .filter(|(camera, target)| camera.is_active && routed(target))
        .map(|(camera, _)| camera.order)
        .max();
    let Some(last) = last else {
        return;
    };
    for (mut camera, target) in &mut cameras {
        if !camera.is_active || !routed(&target) {
            continue;
        }
        let writes = writes_frame(camera.order, last);
        if writes == matches!(camera.output_mode, CameraOutputMode::Skip) {
            camera.output_mode = if writes {
                // The copy replaces the frame: nothing beneath it was written this frame.
                CameraOutputMode::Write {
                    blend_state: Some(BlendState::REPLACE),
                    clear_color: ClearColorConfig::None,
                }
            } else {
                CameraOutputMode::Skip
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_caps_the_frame_height_and_keeps_its_shape() {
        let budget = RenderBudget {
            max_frame_height: Some(480),
            ..Default::default()
        };
        assert_eq!(
            budgeted_size(UVec2::new(1920, 1080), budget),
            UVec2::new(853, 480)
        );
        assert_eq!(
            budgeted_size(UVec2::new(640, 360), budget),
            UVec2::new(640, 360)
        );
        assert_eq!(
            budgeted_size(UVec2::new(1920, 1080), RenderBudget::default()),
            UVec2::new(1920, 1080)
        );
    }

    #[test]
    fn only_the_last_camera_writes_the_frame() {
        assert!(writes_frame(2, 2));
        assert!(!writes_frame(1, 2));
        assert!(!writes_frame(-2, 2));
    }
}
