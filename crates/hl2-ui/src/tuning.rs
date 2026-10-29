//! UI tuning values embedded from `assets/ui/tuning.json` and parsed once: numbers that shape
//! the UI but come from no game resource.

use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Debug, Deserialize)]
struct Tuning {
    /// Width:height of the narrowest screen the 480-line UI design fits ([`crate::proportional`]).
    narrowest_design_aspect: (u32, u32),
    /// Width:height of the widest screen the menu keeps its retail size on; wider screens grow
    /// the menu ([`crate::proportional::widescreen_menu_scale`]).
    widest_design_aspect: (u32, u32),
    /// Largest growth of the menu on wide screens.
    widescreen_menu_scale_max: f32,
    /// Rows of the tallest menu: the stack whose bottom stays put when the menu grows.
    menu_stack_rows: u32,
    /// Duration of UI focus and window fade animations in seconds (matching sourcescheme Frame.FocusTransitionEffectTime).
    ui_fade_duration: f32,
    /// Unfocused title/subtitle opacity factor (ratio of Frame.OutOfFocusBgColor to Frame.BgColor).
    unfocused_alpha_factor: f32,
    /// Authentic backdrop overlay opacity from MainMenu.Backdrop in sourcescheme.res (156 / 255).
    backdrop_alpha: f32,
    /// Unverified fallback opacity for mid-gameplay pause menu dimming overlay.
    gameplay_dim_alpha: f32,
    /// Seconds before an unconfirmed video change reverts (`gameui::video_confirm`).
    video_confirm_seconds: f64,
    /// Smallest and largest explicit vertical field of view of the Video > Advanced slider
    /// (below the smallest the slider means automatic).
    vertical_fov_range: (f32, f32),
    /// Smallest and largest viewmodel field of view of the Video > Advanced slider.
    viewmodel_fov_range: (f32, f32),
    /// Aspect-adaptive additive world 4:3 base FOV offset (degrees) outside `5:4..=16:9`.
    world_fov_offset: WorldFovOffset,
    /// Width:height of each centered HUD aspect of Video > HUD Aspect Ratio
    /// ([`crate::proportional::hud_frame`]).
    hud_aspects: HudAspects,
    /// Aspect-adaptive and per-weapon viewmodel framing outside `5:4..=16:9`.
    viewmodel: ViewmodelTuning,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct WorldFovOffset {
    portrait: f32,
    ultrawide: f32,
}

/// The centered HUD aspects, width:height, named like [`crate::config::HudAspect`].
#[derive(Debug, Deserialize)]
struct HudAspects {
    normal: (u32, u32),
    wide16x9: (u32, u32),
    wide16x10: (u32, u32),
}

/// Aspect-adaptive viewmodel framing outside `narrowest_design_aspect..=widest_design_aspect`
/// (`5:4..=16:9`), with a default profile and per-weapon overrides.
#[derive(Debug, Deserialize)]
struct ViewmodelTuning {
    /// Width:height where the portrait framing reaches full strength (e.g. `9:16`).
    portrait_aspect: (u32, u32),
    /// Width:height where the ultrawide framing reaches full strength (e.g. `32:9`).
    ultrawide_aspect: (u32, u32),
    /// Framing applied to weapons without a per-weapon entry.
    #[serde(default)]
    default: ViewmodelWeaponTuning,
    /// Per-weapon framing keyed by weapon classname (e.g. `"weapon_physcannon"`, `"weapon_rpg"`).
    weapons: std::collections::BTreeMap<String, ViewmodelWeaponTuning>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct ViewmodelWeaponTuning {
    #[serde(default)]
    standard: ViewmodelFraming,
    #[serde(default)]
    portrait: ViewmodelFraming,
    #[serde(default)]
    ultrawide: ViewmodelFraming,
}

/// Direct tuning profile selected by the active frame aspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ViewmodelProfile {
    Standard,
    Portrait,
    Ultrawide,
}

impl ViewmodelProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Portrait => "portrait",
            Self::Ultrawide => "ultrawide",
        }
    }
}

/// Additive viewmodel horizontal field-of-view offset (degrees, measured at the 4:3 reference
/// aspect) and local camera-space translation offset (`[forward, left, up]` in Source units) for
/// frames narrower than `5:4` or wider than `16:9`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ViewmodelFraming {
    pub fov_offset: f32,
    pub offset: [f32; 3],
}

impl ViewmodelFraming {
    pub const ZERO: Self = Self {
        fov_offset: 0.0,
        offset: [0.0; 3],
    };
}

fn data() -> &'static Tuning {
    static DATA: LazyLock<Tuning> = LazyLock::new(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/ui/tuning.json"
        )))
        .expect("embedded UI tuning data is valid")
    });
    &DATA
}

/// Width / height of the narrowest screen the UI design fits.
pub fn narrowest_design_aspect() -> f32 {
    let (width, height) = data().narrowest_design_aspect;
    width as f32 / height as f32
}

/// Width / height of the widest screen the menu keeps its retail size on.
pub fn widest_design_aspect() -> f32 {
    let (width, height) = data().widest_design_aspect;
    width as f32 / height as f32
}

/// Largest growth of the menu on wide screens.
pub fn widescreen_menu_scale_max() -> f32 {
    data().widescreen_menu_scale_max
}

/// Rows of the tallest menu.
pub fn menu_stack_rows() -> f32 {
    data().menu_stack_rows as f32
}

/// Seconds before an unconfirmed video change reverts.
pub fn video_confirm_seconds() -> f64 {
    data().video_confirm_seconds
}

/// Duration of UI focus and window fade animations in seconds.
pub fn ui_fade_duration() -> f32 {
    data().ui_fade_duration
}

/// Unfocused title and subtitle alpha multiplier.
pub fn unfocused_alpha_factor() -> f32 {
    data().unfocused_alpha_factor
}

/// Authentic backdrop overlay opacity from MainMenu.Backdrop in sourcescheme.res (156 / 255).
pub fn backdrop_alpha() -> f32 {
    data().backdrop_alpha
}

/// Fallback opacity for mid-gameplay pause menu dimming overlay.
pub fn gameplay_dim_alpha() -> f32 {
    data().gameplay_dim_alpha
}

/// Smallest and largest explicit vertical field of view (degrees).
pub fn vertical_fov_range() -> (f32, f32) {
    data().vertical_fov_range
}

/// Smallest and largest viewmodel field of view (degrees).
pub fn viewmodel_fov_range() -> (f32, f32) {
    data().viewmodel_fov_range
}

/// Width / height the HUD is laid out in on a wider screen, or `None` for Fit Display.
pub fn hud_aspect_ratio(aspect: crate::config::HudAspect) -> Option<f32> {
    use crate::config::HudAspect;
    let (width, height) = match aspect {
        HudAspect::Fit => return None,
        HudAspect::Normal => data().hud_aspects.normal,
        HudAspect::Wide16x9 => data().hud_aspects.wide16x9,
        HudAspect::Wide16x10 => data().hud_aspects.wide16x10,
    };
    Some(width as f32 / height as f32)
}

/// Width / height of the narrowest supported frame (`9:16`): the portrait tuning reaches full
/// strength here, and a flexible frame letterboxes below it.
pub fn min_supported_aspect() -> f32 {
    let (width, height) = data().viewmodel.portrait_aspect;
    width as f32 / height.max(1) as f32
}

/// Width / height of the widest supported frame (`32:9`): the ultrawide tuning reaches full
/// strength here, and a flexible frame pillarboxes above it.
pub fn max_supported_aspect() -> f32 {
    let (width, height) = data().viewmodel.ultrawide_aspect;
    width as f32 / height.max(1) as f32
}

/// `aspect` limited to `min_supported_aspect()..=max_supported_aspect()` (non-finite or
/// non-positive values give the narrowest design aspect, `5:4`).
pub fn clamp_supported_aspect(aspect: f32) -> f32 {
    if !aspect.is_finite() || aspect <= 0.0 {
        return narrowest_design_aspect();
    }
    aspect.clamp(min_supported_aspect(), max_supported_aspect())
}

/// Ease of the 1D cubic Bézier with control values `0, 0, 1, 1` (`3t² - 2t³`, smoothstep) on
/// `t` clamped to `0..=1`. It keeps the end values `0` and `1` and the midpoint `0.5` of a linear
/// ramp, but starts and ends with zero slope, so a tuning blended by it has no visible kink where
/// it leaves the untouched `5:4..=16:9` plateau or where it saturates at `9:16` / `32:9`.
pub fn cubic_bezier_ease(t: f32) -> f32 {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    t * t * (3.0 - 2.0 * t)
}

/// Strengths `(portrait, ultrawide)` in `0..=1` of the aspect-adaptive tunings at `aspect`
/// (`width / height`). Both are `0` inside `narrowest_design_aspect()..=widest_design_aspect()`
/// (`5:4..=16:9`, retail aspects untouched). Below `5:4` the portrait strength rises to `1` at
/// [`min_supported_aspect`]; above `16:9` the ultrawide strength rises to `1` at
/// [`max_supported_aspect`]; past them both stay saturated. The ramp is the linear position
/// between the known aspects eased by [`cubic_bezier_ease`], so every aspect in between (the
/// flexible frame takes any) gets a smooth blend of the tuning values of the known ones.
pub fn aspect_factors(aspect: f32) -> (f32, f32) {
    if !aspect.is_finite() || aspect <= 0.0 {
        return (0.0, 0.0);
    }
    let narrow = narrowest_design_aspect();
    let wide = widest_design_aspect();
    if aspect < narrow {
        let span = (narrow - min_supported_aspect()).max(f32::EPSILON);
        (cubic_bezier_ease((narrow - aspect) / span), 0.0)
    } else if aspect > wide {
        let span = (max_supported_aspect() - wide).max(f32::EPSILON);
        (0.0, cubic_bezier_ease((aspect - wide) / span))
    } else {
        (0.0, 0.0)
    }
}

/// Aspect-adaptive additive world 4:3 base FOV offset (degrees) at `aspect` (`width / height`).
///
/// Inside `narrowest_design_aspect()..=widest_design_aspect()` (`5:4..=16:9`) this is `0.0` so
/// standard retail aspects are untouched. Below `5:4` it blends toward
/// `world_fov_offset.portrait` at `9:16` (reducing vertical fish-eye distortion); above `16:9`
/// toward `world_fov_offset.ultrawide` at `32:9` (reducing peripheral horizontal stretching),
/// with the strengths of [`aspect_factors`].
pub fn world_fov_offset(aspect: f32) -> f32 {
    let (portrait, ultrawide) = aspect_factors(aspect);
    let offsets = data().world_fov_offset;
    offsets.portrait * portrait + offsets.ultrawide * ultrawide
}

/// Aspect-adaptive viewmodel framing (`fov_offset` and Source `[forward, left, up]` offset) for
/// `weapon` at `aspect` (`width / height`).
///
/// Inside `narrowest_design_aspect()..=widest_design_aspect()` (`5:4..=16:9`) it uses the direct
/// `standard` profile (zero by default, so retail aspects remain untouched). Below `5:4` it blends
/// from that profile toward `portrait` at `portrait_aspect` (`9:16`); above `16:9` it blends toward
/// `ultrawide` at `ultrawide_aspect` (`32:9`), with the strengths of [`aspect_factors`].
pub fn viewmodel_framing(weapon: &str, aspect: f32) -> ViewmodelFraming {
    let vm = &data().viewmodel;
    let profile = vm.weapons.get(weapon).copied().unwrap_or(vm.default);
    let (portrait, ultrawide) = aspect_factors(aspect);
    if portrait > 0.0 {
        blend(profile.standard, profile.portrait, portrait)
    } else if ultrawide > 0.0 {
        blend(profile.standard, profile.ultrawide, ultrawide)
    } else {
        profile.standard
    }
}

/// Direct profile selected at `aspect`, independent of its interpolation strength.
pub fn viewmodel_profile(aspect: f32) -> ViewmodelProfile {
    if !aspect.is_finite() || aspect <= 0.0 {
        return ViewmodelProfile::Standard;
    }
    if aspect < narrowest_design_aspect() {
        ViewmodelProfile::Portrait
    } else if aspect > widest_design_aspect() {
        ViewmodelProfile::Ultrawide
    } else {
        ViewmodelProfile::Standard
    }
}

/// The direct endpoint tuning for a weapon and named aspect profile.
pub fn viewmodel_profile_framing(weapon: &str, profile: ViewmodelProfile) -> ViewmodelFraming {
    let vm = &data().viewmodel;
    let tuning = vm.weapons.get(weapon).copied().unwrap_or(vm.default);
    match profile {
        ViewmodelProfile::Standard => tuning.standard,
        ViewmodelProfile::Portrait => tuning.portrait,
        ViewmodelProfile::Ultrawide => tuning.ultrawide,
    }
}

/// Live framing with one direct profile replaced by the debug editor's current value.
pub fn viewmodel_framing_with_profile(
    weapon: &str,
    aspect: f32,
    edited_profile: ViewmodelProfile,
    edited_value: ViewmodelFraming,
) -> ViewmodelFraming {
    let vm = &data().viewmodel;
    let tuning = vm.weapons.get(weapon).copied().unwrap_or(vm.default);
    let standard = if edited_profile == ViewmodelProfile::Standard {
        edited_value
    } else {
        tuning.standard
    };
    let portrait = if edited_profile == ViewmodelProfile::Portrait {
        edited_value
    } else {
        tuning.portrait
    };
    let ultrawide = if edited_profile == ViewmodelProfile::Ultrawide {
        edited_value
    } else {
        tuning.ultrawide
    };
    let (portrait_strength, ultrawide_strength) = aspect_factors(aspect);
    if portrait_strength > 0.0 {
        blend(standard, portrait, portrait_strength)
    } else if ultrawide_strength > 0.0 {
        blend(standard, ultrawide, ultrawide_strength)
    } else {
        standard
    }
}

fn blend(from: ViewmodelFraming, to: ViewmodelFraming, amount: f32) -> ViewmodelFraming {
    let amount = amount.clamp(0.0, 1.0);
    ViewmodelFraming {
        fov_offset: from.fov_offset + (to.fov_offset - from.fov_offset) * amount,
        offset: std::array::from_fn(|i| from.offset[i] + (to.offset[i] - from.offset[i]) * amount),
    }
}

/// Pretty-printed complete tuning asset with one weapon/profile replaced by the live debug value.
pub fn tuning_json_with_viewmodel_profile(
    weapon: &str,
    profile: ViewmodelProfile,
    framing: ViewmodelFraming,
) -> Result<String, serde_json::Error> {
    let mut edits = std::collections::BTreeMap::new();
    edits.insert((weapon.to_owned(), profile), framing);
    tuning_json_with_viewmodel_profiles(&edits)
}

/// Pretty-printed tuning asset with every direct weapon/profile edit from one debug session merged.
pub fn tuning_json_with_viewmodel_profiles(
    edits: &std::collections::BTreeMap<(String, ViewmodelProfile), ViewmodelFraming>,
) -> Result<String, serde_json::Error> {
    let mut root: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/ui/tuning.json"
    )))?;
    let viewmodel = &mut root["viewmodel"];

    for ((weapon, profile), framing) in edits {
        let target = if weapon.starts_with("weapon_") {
            let defaults = viewmodel["default"].clone();
            let target = &mut viewmodel["weapons"][weapon];
            if target.is_null() {
                *target = defaults;
            }
            target
        } else {
            &mut viewmodel["default"]
        };
        let two_decimals = |value: f32| (f64::from(value) * 100.0).round() / 100.0;
        target[profile.as_str()] = serde_json::json!({
            "fov_offset": two_decimals(framing.fov_offset),
            "offset": framing.offset.map(two_decimals),
        });
    }
    Ok(compact_viewmodel_json(root))
}

/// Pretty-prints the tuning asset with each viewmodel profile on one line, omitting zero
/// `fov_offset`/`offset` fields and all-zero profiles (and the weapons or default block left empty
/// by that), since missing fields and profiles load as zero.
fn compact_viewmodel_json(mut root: serde_json::Value) -> String {
    let is_zero = |profile: &serde_json::Value| {
        profile["fov_offset"].as_f64() == Some(0.0)
            && profile["offset"]
                .as_array()
                .is_some_and(|v| v.iter().all(|x| x.as_f64() == Some(0.0)))
    };
    let mut inline = Vec::new();
    let mut compact = |block: &mut serde_json::Value| {
        let Some(map) = block.as_object_mut() else {
            return;
        };
        for name in ["standard", "portrait", "ultrawide"] {
            if map.get(name).is_some_and(is_zero) {
                map.remove(name);
            } else if let Some(profile) = map.get_mut(name) {
                let number =
                    |value: &serde_json::Value| format!("{:?}", value.as_f64().unwrap_or(0.0));
                let mut fields = Vec::new();
                if profile["fov_offset"].as_f64() != Some(0.0) {
                    fields.push(format!(
                        "\"fov_offset\": {}",
                        number(&profile["fov_offset"])
                    ));
                }
                let offset: Vec<String> = profile["offset"]
                    .as_array()
                    .map_or_else(Vec::new, |v| v.iter().map(number).collect());
                if offset.iter().any(|x| x != "0.0") {
                    fields.push(format!("\"offset\": [{}]", offset.join(", ")));
                }
                inline.push(format!("{{{}}}", fields.join(", ")));
                *profile = serde_json::Value::String(format!("@@{}@@", inline.len() - 1));
            }
        }
    };
    let viewmodel = &mut root["viewmodel"];
    compact(&mut viewmodel["default"]);
    if let Some(weapons) = viewmodel["weapons"].as_object_mut() {
        for weapon in weapons.values_mut() {
            compact(weapon);
        }
        weapons.retain(|_, weapon| weapon.as_object().is_none_or(|m| !m.is_empty()));
    }
    let mut text = serde_json::to_string_pretty(&root).unwrap_or_default();
    for (index, line) in inline.iter().enumerate() {
        text = text.replace(&format!("\"@@{index}@@\""), line);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuning_data_parses_with_usable_values() {
        assert!(narrowest_design_aspect() >= 1.0, "a landscape design");
        assert!(video_confirm_seconds() >= 5.0, "time to read and answer");
        assert!(widest_design_aspect() > narrowest_design_aspect());
        assert!(widescreen_menu_scale_max() >= 1.0 && menu_stack_rows() >= 1.0);
        assert_eq!(hud_aspect_ratio(crate::config::HudAspect::Fit), None);
        for aspect in &crate::config::HudAspect::ALL[1..] {
            assert!(
                hud_aspect_ratio(*aspect).is_some_and(|r| r >= 1.),
                "{aspect:?}"
            );
        }
        for (min, max) in [vertical_fov_range(), viewmodel_fov_range()] {
            assert!(0. < min && min < max && max < 180., "{min} {max}");
        }
        for aspect in [5. / 4., 4. / 3., 16. / 10., 16. / 9.] {
            assert_eq!(world_fov_offset(aspect), 0.0, "{aspect}");
        }
        assert!(world_fov_offset(9. / 16.) < -10.0);
        assert!(world_fov_offset(32. / 9.) < -5.0);
    }

    #[test]
    fn viewmodel_framing_leaves_retail_aspects_untouched_and_adapts_portrait_and_ultrawide() {
        for aspect in [5. / 4., 4. / 3., 16. / 10., 16. / 9.] {
            for weapon in ["weapon_crowbar", "weapon_physcannon", "weapon_rpg"] {
                assert_eq!(
                    viewmodel_framing(weapon, aspect),
                    ViewmodelFraming::ZERO,
                    "{weapon} at {aspect} is untouched"
                );
            }
        }
        // Portrait and ultrawide values are owner-tuned data and not pinned here.
    }

    #[test]
    fn viewmodel_debug_profiles_and_snapshot_round_trip() {
        let normal = viewmodel_profile(4. / 3.);
        let portrait = viewmodel_profile(9. / 16.);
        let ultrawide = viewmodel_profile(32. / 9.);
        assert_eq!(normal, ViewmodelProfile::Standard);
        assert_eq!(portrait, ViewmodelProfile::Portrait);
        assert_eq!(ultrawide, ViewmodelProfile::Ultrawide);

        let adjusted = ViewmodelFraming {
            fov_offset: 0.12,
            offset: [0.01, -0.02, -11.49],
        };
        let snapshot = tuning_json_with_viewmodel_profile(
            "weapon_physcannon",
            ViewmodelProfile::Portrait,
            adjusted,
        )
        .expect("valid tuning snapshot");
        let json: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
        assert_eq!(
            json["viewmodel"]["weapons"]["weapon_physcannon"]["portrait"]["fov_offset"],
            0.12
        );
        assert_eq!(
            json["viewmodel"]["weapons"]["weapon_physcannon"]["portrait"]["offset"],
            serde_json::json!([0.01, -0.02, -11.49])
        );
        assert!(
            json["viewmodel"]["weapons"]["weapon_physcannon"]["ultrawide"].is_null(),
            "zero profiles are omitted"
        );
        assert!(snapshot.contains("\n  \"viewmodel\": {"));

        let accumulated = std::collections::BTreeMap::from([
            (
                ("weapon_physcannon".to_owned(), ViewmodelProfile::Portrait),
                adjusted,
            ),
            (
                ("weapon_pistol".to_owned(), ViewmodelProfile::Ultrawide),
                ViewmodelFraming {
                    fov_offset: 2.5,
                    offset: [1.0, 2.0, 3.0],
                },
            ),
        ]);
        let accumulated_json = tuning_json_with_viewmodel_profiles(&accumulated)
            .expect("valid accumulated tuning snapshot");
        let accumulated_value: serde_json::Value = serde_json::from_str(&accumulated_json).unwrap();
        assert_eq!(
            accumulated_value["viewmodel"]["weapons"]["weapon_physcannon"]["portrait"]
                ["fov_offset"],
            0.12
        );
        assert_eq!(
            accumulated_value["viewmodel"]["weapons"]["weapon_pistol"]["ultrawide"]["offset"],
            serde_json::json!([1.0, 2.0, 3.0])
        );

        let standard_edit = ViewmodelFraming {
            fov_offset: 3.0,
            offset: [1.0, 2.0, 3.0],
        };
        assert_eq!(
            viewmodel_framing_with_profile(
                "weapon_physcannon",
                4. / 3.,
                ViewmodelProfile::Standard,
                standard_edit,
            ),
            standard_edit
        );
        assert_eq!(
            viewmodel_framing_with_profile(
                "weapon_physcannon",
                9. / 16.,
                ViewmodelProfile::Portrait,
                adjusted,
            ),
            adjusted
        );

        let unknown_weapon_snapshot = tuning_json_with_viewmodel_profile(
            "weapon_pistol",
            ViewmodelProfile::Standard,
            standard_edit,
        )
        .expect("valid per-weapon tuning snapshot");
        let unknown: serde_json::Value = serde_json::from_str(&unknown_weapon_snapshot).unwrap();
        assert_eq!(
            unknown["viewmodel"]["weapons"]["weapon_pistol"]["standard"]["fov_offset"],
            3.0
        );
        assert!(
            unknown["viewmodel"]["weapons"]["weapon_pistol"]["ultrawide"].is_null(),
            "untouched zero profiles are omitted"
        );
        assert!(unknown["viewmodel"]["default"]["standard"].is_null());
        let root = serde_json::json!({"viewmodel": {
            "default": {"standard": {"fov_offset": 0.0, "offset": [0.0, 0.0, 0.0]}},
            "weapons": {
                "weapon_a": {
                    "portrait": {"fov_offset": 0.0, "offset": [-5.1, 0.0, -3.2]},
                    "ultrawide": {"fov_offset": 8.0, "offset": [0.0, 0.0, 0.0]},
                    "standard": {"fov_offset": 0.0, "offset": [0.0, 0.0, 0.0]}
                },
                "weapon_b": {"portrait": {"fov_offset": 0.0, "offset": [0.0, 0.0, 0.0]}}
            }
        }});
        let text = compact_viewmodel_json(root);
        assert!(
            text.contains(
                "\"weapon_a\": {\n        \"portrait\": {\"offset\": [-5.1, 0.0, -3.2]},\n        \"ultrawide\": {\"fov_offset\": 8.0}\n      }"
            ),
            "{text}"
        );
        assert!(
            !text.contains("weapon_b") && !text.contains("standard"),
            "{text}"
        );
    }

    #[test]
    fn viewmodel_profile_fields_may_be_omitted() {
        let framing: ViewmodelFraming =
            serde_json::from_str(r#"{"offset": [-5.1, 0.0, -3.2]}"#).unwrap();
        assert_eq!(framing.fov_offset, 0.0);
        assert_eq!(framing.offset, [-5.1, 0.0, -3.2]);
        let framing: ViewmodelFraming = serde_json::from_str(r#"{"fov_offset": 4.0}"#).unwrap();
        assert_eq!(framing.offset, [0.0; 3]);
    }

    #[test]
    fn aspect_math_clamps_to_the_supported_range_and_blends_smoothly() {
        let (min, max) = (min_supported_aspect(), max_supported_aspect());
        assert!((min - 9. / 16.).abs() < 1e-6 && (max - 32. / 9.).abs() < 1e-6);
        assert_eq!(clamp_supported_aspect(0.3), min);
        assert_eq!(clamp_supported_aspect(5.0), max);
        assert_eq!(clamp_supported_aspect(1.5), 1.5);
        assert_eq!(clamp_supported_aspect(f32::NAN), narrowest_design_aspect());

        assert_eq!(cubic_bezier_ease(0.), 0.);
        assert_eq!(cubic_bezier_ease(1.), 1.);
        assert_eq!(cubic_bezier_ease(0.5), 0.5);
        assert_eq!(cubic_bezier_ease(-1.), 0.);
        assert_eq!(cubic_bezier_ease(2.), 1.);

        assert_eq!(aspect_factors(16. / 10.), (0., 0.));
        assert_eq!(aspect_factors(min), (1., 0.));
        assert_eq!(aspect_factors(0.4), (1., 0.), "saturated below 9:16");
        assert_eq!(aspect_factors(max), (0., 1.));
        assert_eq!(aspect_factors(6.), (0., 1.), "saturated above 32:9");

        // Monotonic and without jumps across every aspect a flexible frame can take.
        let mut previous = world_fov_offset(min);
        let steps = 2000;
        for i in 1..=steps {
            let aspect = min + (max - min) * i as f32 / steps as f32;
            let offset = world_fov_offset(aspect);
            assert!(
                (offset - previous).abs() < 0.1,
                "{aspect}: {previous} -> {offset}"
            );
            previous = offset;
        }
        // Zero slope where the tuning leaves the retail plateau.
        let edge = widest_design_aspect();
        assert!(world_fov_offset(edge + 0.01).abs() < 0.01);
        let edge = narrowest_design_aspect();
        assert!(world_fov_offset(edge - 0.01).abs() < 0.02);
    }
}
