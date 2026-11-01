//! Only the last active game camera copies the shared intermediate image into the game frame: each
//! earlier camera's copy would be overwritten by the next one.
use super::{GameFrame, PresentCamera};
use bevy::{
    camera::{Camera, CameraOutputMode, ClearColorConfig, RenderTarget},
    prelude::{Query, Res, Without},
    render::render_resource::BlendState,
};

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
