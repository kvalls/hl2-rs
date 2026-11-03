use super::*;

/// Metrics and bounds for the primary display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayMetrics {
    pub physical_width: u32,
    pub physical_height: u32,
    pub scale_factor: f32,
}

impl DisplayMetrics {
    /// Metrics of a monitor.
    pub fn of(monitor: &Monitor) -> Self {
        Self {
            physical_width: monitor.physical_width,
            physical_height: monitor.physical_height,
            scale_factor: monitor.scale_factor as f32,
        }
    }

    /// Queries display metrics from the primary monitor, if available.
    #[allow(dead_code)]
    pub fn from_monitors(monitors: &Query<&Monitor, With<PrimaryMonitor>>) -> Option<Self> {
        monitors.iter().next().map(Self::of)
    }

    /// Detects if a window currently in `WindowMode::Windowed` is actually occupying the full
    /// display (such as macOS native workspace fullscreen via the green zoom button).
    pub fn is_workspace_fullscreen(&self, window: &Window) -> bool {
        let tolerance = (window.scale_factor().max(1.0).ceil() as u32).max(2);
        window.mode == WindowMode::Windowed
            && window
                .resolution
                .physical_width()
                .abs_diff(self.physical_width)
                <= tolerance
            && window
                .resolution
                .physical_height()
                .abs_diff(self.physical_height)
                <= tolerance
    }
}

/// Resolutions offered on the Video page: the primary monitor's reported video modes plus
/// the standard list, filtered and sorted by [`hl2_ui::gameui::display::usable_modes`], and
/// the desktop size in pixels that windowed choices must fit (windowed mode shows one frame
/// pixel per display pixel). Falls back to the current window size when nothing is usable.
/// A `--fake-monitor` spec replaces the monitor's size and reported modes.
pub fn modes(
    monitors: &Query<&Monitor, With<PrimaryMonitor>>,
    window: &Window,
    fake: Option<&hl2_ui::gameui::display::MonitorSpec>,
) -> hl2_ui::gameui::display::DisplayModes {
    use hl2_ui::gameui::display::{DisplayModes, Resolution, usable_modes};
    let current = Resolution::new(
        window.resolution.physical_width(),
        window.resolution.physical_height(),
    );
    if let Some(fake) = fake {
        return fake.display_modes(current);
    }
    let monitor = monitors.iter().next();
    let reported = monitor
        .into_iter()
        .flat_map(|m| m.video_modes.iter())
        .map(|mode| Resolution::new(mode.physical_size.x, mode.physical_size.y));
    let size = monitor.map(|m| Resolution::new(m.physical_width, m.physical_height));
    DisplayModes {
        fullscreen: usable_modes(reported, size, current),
        desktop: size,
        current: Some(current),
    }
}
