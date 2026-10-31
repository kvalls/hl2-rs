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

use hl2_ui::gameui::display::{AspectChoice, AspectRatio, DisplayModes, Resolution};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::LazyLock};
pub const FIT_DISPLAY: AspectRatio = AspectRatio::Custom(&AspectChoice {
    id: "flexible",
    token: "#GameUI_HUDAspectRatio_FitDisplay",
    ratio: (16, 9),
    follows_window: true,
});
pub const ULTRAWIDE_21X9: AspectRatio = AspectRatio::Custom(&AspectChoice {
    id: "ultrawide21x9",
    token: "#HL2RS_AspectUltrawide21x9",
    ratio: (64, 27),
    follows_window: false,
});
pub const ULTRAWIDE_32X9: AspectRatio = AspectRatio::Custom(&AspectChoice {
    id: "ultrawide32x9",
    token: "#HL2RS_AspectUltrawide32x9",
    ratio: (32, 9),
    follows_window: false,
});
pub const SQUARE_1X1: AspectRatio = AspectRatio::Custom(&AspectChoice {
    id: "square1x1",
    token: "#HL2RS_AspectSquare1x1",
    ratio: (1, 1),
    follows_window: false,
});
pub const PORTRAIT_3X4: AspectRatio = AspectRatio::Custom(&AspectChoice {
    id: "portrait3x4",
    token: "#HL2RS_AspectPortrait3x4",
    ratio: (3, 4),
    follows_window: false,
});
pub const PORTRAIT_10X16: AspectRatio = AspectRatio::Custom(&AspectChoice {
    id: "portrait10x16",
    token: "#HL2RS_AspectPortrait10x16",
    ratio: (10, 16),
    follows_window: false,
});
pub const PORTRAIT_9X16: AspectRatio = AspectRatio::Custom(&AspectChoice {
    id: "portrait9x16",
    token: "#HL2RS_AspectPortrait9x16",
    ratio: (9, 16),
    follows_window: false,
});
pub const ASPECTS: &[AspectRatio] = &[
    FIT_DISPLAY,
    AspectRatio::Normal,
    AspectRatio::Wide16x9,
    AspectRatio::Wide16x10,
    ULTRAWIDE_21X9,
    ULTRAWIDE_32X9,
    SQUARE_1X1,
    PORTRAIT_3X4,
    PORTRAIT_10X16,
    PORTRAIT_9X16,
];
/// The choice an authored `modes.json` id names.
fn choice(id: &str) -> AspectRatio {
    match id {
        "normal" => AspectRatio::Normal,
        "wide16x10" => AspectRatio::Wide16x10,
        "wide16x9" => AspectRatio::Wide16x9,
        other => ASPECTS
            .iter()
            .copied()
            .find(|aspect| matches!(aspect, AspectRatio::Custom(c) if c.id == other))
            .expect("authored aspect id"),
    }
}
#[derive(Deserialize)]
struct Data {
    min_size: (u32, u32),
    native_fit_step: u32,
    aspect_ratios: Vec<Entry>,
    standard_modes: BTreeMap<String, Vec<(u32, u32)>>,
}
#[derive(Deserialize)]
struct Entry {
    aspect: String,
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
        .map_or(AspectRatio::Wide16x9, |e| choice(&e.aspect))
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
