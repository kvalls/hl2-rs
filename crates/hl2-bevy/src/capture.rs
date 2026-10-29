//! Screenshot readback for `--capture` and `--monitor-capture`: the observers that keep the captured images
//! in the run status, and saving them.
use crate::Status;
use bevy::{
    ecs::{observer::On, system::Res},
    image::Image,
    render::view::screenshot::ScreenshotCaptured,
};
use std::path::PathBuf;

pub(crate) fn capture_complete(event: On<ScreenshotCaptured>, status: Res<Status>) {
    let mut report = status.0.lock().expect("status lock");
    report.capture_completed = true;
    report.capture_size = Some([event.image.width(), event.image.height()]);
    report.captured_image = Some(event.image.clone());
}

pub(crate) fn monitor_capture_complete(event: On<ScreenshotCaptured>, status: Res<Status>) {
    let mut report = status.0.lock().expect("monitor capture");
    report.monitor_capture_completed = true;
    report.monitor_image = Some(event.image.clone());
}

pub(crate) fn save_capture(image: Option<Image>, path: Option<&PathBuf>) -> Option<String> {
    let path = path?;
    match image {
        Some(image) => match image.try_into_dynamic() {
            Ok(image) => image
                .to_rgb8()
                .save(path)
                .err()
                .map(|error| error.to_string()),
            Err(error) => Some(format!("screenshot conversion: {error}")),
        },
        None => Some("screenshot readback did not finish".into()),
    }
}

/// Diagnostic captures include the actual install frame, before any fixed simulation tick.
/// Ordinary play does no screenshot work. This exposes one-frame loading handoff regressions
/// that a movement-script capture, taken after the first tick, cannot observe.
pub(crate) fn installed_frame(commands: &mut bevy::prelude::Commands, capture: Option<&PathBuf>) {
    let Some(capture) = capture else {
        return;
    };
    let name = capture.file_stem().unwrap_or_default().to_string_lossy();
    let path = capture.with_file_name(format!("{name}-installed.png"));
    commands.queue(move |world: &mut bevy::prelude::World| {
        let Some(frame) = world.get_resource::<crate::video::GameFrame>() else {
            return;
        };
        let frame = frame.image.clone();
        world
            .spawn(bevy::render::view::screenshot::Screenshot::image(frame))
            .observe(move |event: On<ScreenshotCaptured>| {
                if let Some(error) = save_capture(Some(event.image.clone()), Some(&path)) {
                    bevy::log::error!("Map installation capture: {error}");
                }
            });
    });
}
