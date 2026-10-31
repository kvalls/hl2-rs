//! Extra display choices and Fit Display framing.
pub const MODES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/display/modes.json"
));
pub const MIN_ASPECT: f32 = 9. / 16.;
pub const MAX_ASPECT: f32 = 32. / 9.;
pub fn clamp_supported_aspect(value: f32) -> f32 {
    value.clamp(MIN_ASPECT, MAX_ASPECT)
}

use hl2_ui::gameui::display::{AspectRatio, DisplayModes, Resolution};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::LazyLock};
pub const ASPECTS: &[AspectRatio] = &[
    AspectRatio::Flexible,
    AspectRatio::Normal,
    AspectRatio::Wide16x9,
    AspectRatio::Wide16x10,
    AspectRatio::Ultrawide21x9,
    AspectRatio::Ultrawide32x9,
    AspectRatio::Square1x1,
    AspectRatio::Portrait3x4,
    AspectRatio::Portrait10x16,
    AspectRatio::Portrait9x16,
];
#[derive(Deserialize)]
struct Data {
    min_size: (u32, u32),
    native_fit_step: u32,
    aspect_ratios: Vec<Entry>,
    standard_modes: BTreeMap<String, Vec<(u32, u32)>>,
}
#[derive(Deserialize)]
struct Entry {
    aspect: AspectRatio,
    ratio: (u32, u32),
    below: Option<f32>,
    tolerance: Option<f32>,
}
static DATA: LazyLock<Data> =
    LazyLock::new(|| serde_json::from_str(MODES).expect("authored display modes"));
pub fn aspect_of(width: u32, height: u32) -> AspectRatio {
    let ratio = width as f32 / height.max(1) as f32;
    DATA.aspect_ratios
        .iter()
        .find(|e| {
            e.tolerance
                .is_some_and(|t| (ratio - e.ratio.0 as f32 / e.ratio.1 as f32).abs() < t)
        })
        .or_else(|| {
            DATA.aspect_ratios
                .iter()
                .filter(|e| e.tolerance.is_none())
                .find(|e| e.below.is_none_or(|b| ratio < b))
        })
        .map_or(AspectRatio::Wide16x9, |e| e.aspect)
}
pub fn choices(modes: &DisplayModes, windowed: bool) -> Vec<Resolution> {
    let monitor = modes.desktop;
    let fits = DATA.aspect_ratios.iter().filter_map(|entry| {
        let monitor = monitor?;
        let (w, h) = entry.ratio;
        let k = (monitor.width / w).min(monitor.height / h) / DATA.native_fit_step
            * DATA.native_fit_step;
        (k > 0).then_some(Resolution::new(w * k, h * k))
    });
    let standards = DATA
        .standard_modes
        .values()
        .flatten()
        .map(|&(w, h)| Resolution::new(w, h));
    let mut choices: Vec<_> = modes
        .fullscreen
        .iter()
        .copied()
        .chain(standards)
        .chain(fits)
        .filter(|m| m.at_least(Resolution::new(DATA.min_size.0, DATA.min_size.1)))
        .filter(|m| monitor.is_none_or(|max| m.fits(max)))
        .collect();
    choices.sort_by(|a, b| b.cmp(a));
    choices.dedup();
    if choices.is_empty() {
        choices.extend(modes.current.or_else(|| modes.fullscreen.last().copied()));
    }
    let mut augmented = modes.clone();
    augmented.fullscreen = choices;
    augmented.choices(windowed)
}

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;
