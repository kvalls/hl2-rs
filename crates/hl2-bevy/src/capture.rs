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

/// Saves a screenshot as an unfiltered, uncompressed RGB PNG (`source_assets::png`): quick to
/// write and for the checks to read; returns the error, if any.
pub(crate) fn save_capture(image: Option<Image>, path: Option<&PathBuf>) -> Option<String> {
    let path = path?;
    let Some(image) = image else {
        return Some("screenshot readback did not finish".into());
    };
    let rgb = match image.try_into_dynamic() {
        Ok(image) => image.to_rgb8(),
        Err(error) => return Some(format!("screenshot conversion: {error}")),
    };
    match source_assets::png::encode_rgb(rgb.width(), rgb.height(), rgb.as_raw()) {
        Ok(png) => std::fs::write(path, png)
            .err()
            .map(|error| error.to_string()),
        Err(error) => Some(error),
    }
}

/// An observer saving its screenshot to `path` like `--capture` (see [`save_capture`]).
pub(crate) fn save_to(path: PathBuf) -> impl FnMut(On<ScreenshotCaptured>) {
    move |event: On<ScreenshotCaptured>| {
        if let Some(error) = save_capture(Some(event.image.clone()), Some(&path)) {
            bevy::log::error!("capture {}: {error}", path.display());
        }
    }
}

/// `--capture-every`: saves the frame as `<capture>-<seconds>s.png` each time the scene clock
/// passes another interval, so one run gives a time-lapse to set beside native frames.
pub(crate) fn timelapse(
    commands: &mut bevy::prelude::Commands,
    control: &mut crate::CaptureControl,
    (path, every): (&PathBuf, f64),
    scene_time: f64,
    source: impl Fn() -> bevy::render::view::screenshot::Screenshot,
) {
    if every <= 0.0 {
        return;
    }
    let interval = (scene_time / every).floor() as u64;
    if interval < control.next_timelapse {
        return;
    }
    control.next_timelapse = interval + 1;
    let name = path.file_stem().unwrap_or_default().to_string_lossy();
    // Whole seconds keep their short names; shorter intervals name milliseconds too.
    let at = interval as f64 * every;
    let file = if every >= 1.0 {
        path.with_file_name(format!("{name}-{at:05.0}s.png"))
    } else {
        path.with_file_name(format!("{name}-{at:09.3}s.png"))
    };
    commands.spawn(source()).observe(save_to(file));
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
            .observe(save_to(path));
    });
}
