//! This plugin's saved preferences, stored beside the core video settings under their existing
//! names so old files keep working and a build without the plugin preserves them.
use hl2_ui::{
    config::{AdvancedVideoSettings, Config, VideoSettings},
    extensions::{RETAIL_VIEWMODEL_FOV, RenderBudget},
    gameui::{
        localize::Localization,
        settings::{AdvancedChoice, CustomValue, Panel},
    },
};
use source_ui::widgets::SliderRange;

const FIT_DISPLAY: &str = "flexible_aspect";
const VERTICAL_FOV: &str = "vertical_fov";
const VIEWMODEL_FOV: &str = "viewmodel_fov";
const BLUR_VIEWMODEL: &str = "motion_blur_viewmodel";
const BLUR_WORLD: &str = "motion_blur_world";

/// Fit Display: the frame takes the window's shape within `9:16..=32:9`, and width and height
/// only set its detail and the requested windowed size.
pub fn fit_display(video: &VideoSettings) -> bool {
    video.extensions.bool(FIT_DISPLAY).unwrap_or(false)
}
pub fn set_fit_display(video: &mut VideoSettings, on: bool) {
    video.extensions.set_bool(FIT_DISPLAY, on);
}
/// Vertical field of view in degrees; 0 is automatic.
pub fn vertical_fov(video: &VideoSettings) -> f32 {
    video.extensions.f32(VERTICAL_FOV).unwrap_or(0.)
}
pub fn set_vertical_fov(video: &mut VideoSettings, degrees: f32) {
    video.extensions.set_f32(VERTICAL_FOV, degrees);
}
/// The viewmodel's horizontal 4:3 field of view in degrees.
pub fn viewmodel_fov(video: &VideoSettings) -> f32 {
    video
        .extensions
        .f32(VIEWMODEL_FOV)
        .unwrap_or(RETAIL_VIEWMODEL_FOV)
}
pub fn set_viewmodel_fov(video: &mut VideoSettings, degrees: f32) {
    video.extensions.set_f32(VIEWMODEL_FOV, degrees);
}
/// Opt-in animation blur; false preserves retail camera-only blur and old saves.
pub fn blur_viewmodel(video: &AdvancedVideoSettings) -> bool {
    video.extensions.bool(BLUR_VIEWMODEL).unwrap_or(false)
}
/// Ultra-only camera, world and independent projectile reconstruction.
pub fn blur_world(video: &AdvancedVideoSettings) -> bool {
    video.extensions.bool(BLUR_WORLD).unwrap_or(false)
}
pub fn set_blur_tier(video: &mut AdvancedVideoSettings, viewmodel: bool, world: bool) {
    video.extensions.set_bool(BLUR_VIEWMODEL, viewmodel);
    video.extensions.set_bool(BLUR_WORLD, world);
}

/// The combos with a Potato item, below retail's lowest, and where each is saved.
const POTATO: [(AdvancedChoice, &str); 5] = [
    (AdvancedChoice::ModelDetail, "potato_model"),
    (AdvancedChoice::TextureDetail, "potato_texture"),
    (AdvancedChoice::ShaderDetail, "potato_shader"),
    (AdvancedChoice::ShadowDetail, "potato_shadow"),
    (AdvancedChoice::Filtering, "potato_filtering"),
];
const POTATO_SKY: [f32; 3] = [0.45, 0.48, 0.5];

pub fn has_potato(choice: AdvancedChoice) -> bool {
    POTATO.iter().any(|(c, _)| *c == choice)
}
/// An item counts only while its combo's own value is the lowest: a cvar or an edited file that
/// raises the value leaves Potato.
pub fn potato(video: &AdvancedVideoSettings, choice: AdvancedChoice) -> bool {
    choice.selected_retail_lowest(video)
        && POTATO
            .iter()
            .find(|(c, _)| *c == choice)
            .and_then(|(_, key)| video.extensions.bool(key))
            .unwrap_or(false)
}
pub fn set_potato(video: &mut AdvancedVideoSettings, choice: AdvancedChoice, on: bool) {
    if let Some((_, key)) = POTATO.iter().find(|(c, _)| *c == choice) {
        video.extensions.set_bool(key, on);
    }
}
/// Every combo at its lowest own row, with its Potato item on or off.
pub fn set_all_potato(video: &mut AdvancedVideoSettings, on: bool) {
    for (choice, _) in POTATO {
        choice.select(video, usize::from(!on));
    }
}
/// What the selected Potato items give up, each only its own option's work.
pub fn potato_budget(video: &AdvancedVideoSettings) -> RenderBudget {
    let shader = potato(video, AdvancedChoice::ShaderDetail);
    RenderBudget {
        flat_sky: shader.then_some(POTATO_SKY),
        nearest: potato(video, AdvancedChoice::Filtering),
        plain_materials: shader,
        no_shadows: potato(video, AdvancedChoice::ShadowDetail),
        extra_picmip: i32::from(potato(video, AdvancedChoice::TextureDetail)),
        extra_root_lod: usize::from(potato(video, AdvancedChoice::ModelDetail)),
    }
}

/// Below half the smallest explicit field of view the slider means automatic.
fn set_vertical_slider(config: &mut Config, value: f32) {
    let (min, max) = crate::fov::vertical_fov_range();
    let degrees = if value < min * 0.5 {
        0.
    } else {
        value.clamp(min, max)
    };
    set_vertical_fov(&mut config.video, degrees);
}

fn set_viewmodel_slider(config: &mut Config, value: f32) {
    let (min, max) = crate::fov::viewmodel_fov_range();
    set_viewmodel_fov(&mut config.video, value.clamp(min, max));
}

/// The Video > Advanced sliders of this plugin's layout addendum.
pub static SLIDERS: [CustomValue; 2] = [
    CustomValue {
        id: VERTICAL_FOV,
        panel: Panel::VideoAdvanced,
        slider: "VerticalFovSlider",
        value_label: "VerticalFovValueLabel",
        // The low end is automatic (0), then the explicit range.
        range: |_| SliderRange::new(0., crate::fov::vertical_fov_range().1),
        get: |config| vertical_fov(&config.video),
        set: set_vertical_slider,
        label: |value, loc: &Localization| (value == 0.).then(|| loc.text("#Spec_Auto")),
    },
    CustomValue {
        id: VIEWMODEL_FOV,
        panel: Panel::VideoAdvanced,
        slider: "ViewmodelFovSlider",
        value_label: "ViewmodelFovValueLabel",
        range: |_| {
            let (min, max) = crate::fov::viewmodel_fov_range();
            SliderRange::new(min, max)
        },
        get: |config| viewmodel_fov(&config.video),
        set: set_viewmodel_slider,
        label: |_, _| None,
    },
];
