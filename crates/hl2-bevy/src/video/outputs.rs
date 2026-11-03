//! Only the last active game camera copies the shared intermediate image into the game frame: each
//! earlier camera's copy would be overwritten by the next one.
use super::{GameFrame, PresentCamera, apply_video_config_to_window};
use bevy::asset::Assets;
use bevy::image::{Image, ImageSampler, ImageSamplerDescriptor};
use bevy::render::render_resource::TextureFormat;
use bevy::window::{PrimaryWindow, Window};
use bevy::{
    camera::{Camera, CameraOutputMode, ClearColorConfig, RenderTarget},
    prelude::{Query, Res, ResMut, UVec2, With, Without},
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
    image.sampler = frame_sampler(nearest);
    image
}

fn frame_sampler(nearest: bool) -> ImageSampler {
    ImageSampler::Descriptor(if nearest {
        ImageSamplerDescriptor::nearest()
    } else {
        ImageSamplerDescriptor::linear()
    })
}

/// Rewrites the frame's sampler only when the budget changed it: a rewrite re-creates the image.
pub(super) fn sync_frame_sampler(frame: &GameFrame, images: &mut Assets<Image>) {
    let wanted = frame_sampler(frame.budget.nearest);
    if images
        .get(&frame.image)
        .is_some_and(|image| image.sampler != wanted)
        && let Some(mut image) = images.get_mut(&frame.image)
    {
        image.sampler = wanted;
    }
}

pub(crate) fn apply_video_settings(
    mut sim: ResMut<crate::simulation::Simulation>,
    mut frame: Option<ResMut<GameFrame>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
) {
    // A budget changes without any window setting changing, so it does not wait for one.
    if let (Some(budget), Some(frame)) = (sim.pending_budget.take(), frame.as_deref_mut())
        && take_budget(frame, budget)
    {
        sync_frame_sampler(frame, &mut images);
    }
    let Some(cfg) = sim.pending_video_config.take() else {
        return;
    };
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    apply_video_config_to_window(&mut window, &cfg, frame.as_deref_mut());
}

/// Whether the frame's budget changed.
fn take_budget(frame: &mut GameFrame, budget: RenderBudget) -> bool {
    let changed = frame.budget != budget;
    frame.budget = budget;
    changed
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
    fn only_the_last_camera_writes_the_frame() {
        assert!(writes_frame(2, 2));
        assert!(!writes_frame(1, 2));
        assert!(!writes_frame(-2, 2));
    }
}
