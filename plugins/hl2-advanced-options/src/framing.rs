//! Viewmodel framing for frames outside the retail aspects, per weapon and aspect profile.
use hl2_ui::extensions::{FramingEdits, FramingProfiles, STANDARD_PROFILE};
use modkit_core::viewmodel::ViewmodelFraming;
use serde::Deserialize;
use std::sync::LazyLock;

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
            Self::Standard => STANDARD_PROFILE,
            Self::Portrait => "portrait",
            Self::Ultrawide => "ultrawide",
        }
    }
    fn parse(id: &str) -> Option<Self> {
        [Self::Standard, Self::Portrait, Self::Ultrawide]
            .into_iter()
            .find(|profile| profile.as_str() == id)
    }
}

/// This plugin's profiles, offered to the host and to any tuning tool through the core contract.
#[derive(Debug)]
pub struct AdvancedFraming;

impl FramingProfiles for AdvancedFraming {
    fn profile_at(&self, aspect: f32) -> &'static str {
        viewmodel_profile(aspect).as_str()
    }
    fn endpoint(&self, weapon: &str, profile: &str) -> ViewmodelFraming {
        ViewmodelProfile::parse(profile).map_or(ViewmodelFraming::ZERO, |profile| {
            viewmodel_profile_framing(weapon, profile)
        })
    }
    fn blended(
        &self,
        weapon: &str,
        aspect: f32,
        edit: Option<(&str, ViewmodelFraming)>,
    ) -> ViewmodelFraming {
        match edit.and_then(|(id, value)| Some((ViewmodelProfile::parse(id)?, value))) {
            Some((profile, value)) => {
                viewmodel_framing_with_profile(weapon, aspect, profile, value)
            }
            None => viewmodel_framing(weapon, aspect),
        }
    }
    fn snapshot(&self, edits: &FramingEdits) -> Result<String, serde_json::Error> {
        let edits = edits
            .iter()
            .filter_map(|((weapon, id), framing)| {
                Some(((weapon.clone(), ViewmodelProfile::parse(id)?), *framing))
            })
            .collect();
        tuning_json_with_viewmodel_profiles(&edits)
    }
}

#[derive(Debug, Deserialize)]
struct Root {
    viewmodel: ViewmodelTuning,
}

fn data() -> &'static ViewmodelTuning {
    static DATA: LazyLock<Root> = LazyLock::new(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/ui/tuning.json"
        )))
        .expect("advanced options framing data")
    });
    &DATA.viewmodel
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

/// Width / height of the narrowest supported frame (`9:16`): the portrait tuning reaches full
/// strength here, and a flexible frame letterboxes below it.
pub fn min_supported_aspect() -> f32 {
    let (width, height) = data().portrait_aspect;
    width as f32 / height.max(1) as f32
}

/// Width / height of the widest supported frame (`32:9`): the ultrawide tuning reaches full
/// strength here, and a flexible frame pillarboxes above it.
pub fn max_supported_aspect() -> f32 {
    let (width, height) = data().ultrawide_aspect;
    width as f32 / height.max(1) as f32
}

/// `aspect` limited to `min_supported_aspect()..=max_supported_aspect()` (non-finite or
/// non-positive values give the narrowest design aspect, `5:4`).
pub fn clamp_supported_aspect(aspect: f32) -> f32 {
    if !aspect.is_finite() || aspect <= 0.0 {
        return hl2_ui::tuning::narrowest_design_aspect();
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
/// (`width / height`). Both are `0` inside `hl2_ui::tuning::narrowest_design_aspect()..=hl2_ui::tuning::widest_design_aspect()`
/// (`5:4..=16:9`, retail aspects untouched). Below `5:4` the portrait strength rises to `1` at
/// [`min_supported_aspect`]; above `16:9` the ultrawide strength rises to `1` at
/// [`max_supported_aspect`]; past them both stay saturated. The ramp is the linear position
/// between the known aspects eased by [`cubic_bezier_ease`], so every aspect in between (the
/// flexible frame takes any) gets a smooth blend of the tuning values of the known ones.
pub fn aspect_factors(aspect: f32) -> (f32, f32) {
    if !aspect.is_finite() || aspect <= 0.0 {
        return (0.0, 0.0);
    }
    let narrow = hl2_ui::tuning::narrowest_design_aspect();
    let wide = hl2_ui::tuning::widest_design_aspect();
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

/// Aspect-adaptive viewmodel framing (`fov_offset` and Source `[forward, left, up]` offset) for
/// `weapon` at `aspect` (`width / height`).
///
/// Inside `hl2_ui::tuning::narrowest_design_aspect()..=hl2_ui::tuning::widest_design_aspect()` (`5:4..=16:9`) it uses the direct
/// `standard` profile (zero by default, so retail aspects remain untouched). Below `5:4` it blends
/// from that profile toward `portrait` at `portrait_aspect` (`9:16`); above `16:9` it blends toward
/// `ultrawide` at `ultrawide_aspect` (`32:9`), with the strengths of [`aspect_factors`].
pub fn viewmodel_framing(weapon: &str, aspect: f32) -> ViewmodelFraming {
    let vm = &data();
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
    if aspect < hl2_ui::tuning::narrowest_design_aspect() {
        ViewmodelProfile::Portrait
    } else if aspect > hl2_ui::tuning::widest_design_aspect() {
        ViewmodelProfile::Ultrawide
    } else {
        ViewmodelProfile::Standard
    }
}

/// The direct endpoint tuning for a weapon and named aspect profile.
pub fn viewmodel_profile_framing(weapon: &str, profile: ViewmodelProfile) -> ViewmodelFraming {
    let vm = &data();
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
    let vm = &data();
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
/// `fov_offset`/`offset` fields and all-zero profiles (and the weapons left empty by that),
/// since missing fields and profiles load as zero.
fn compact_viewmodel_json(mut root: serde_json::Value) -> String {
    let is_zero = |profile: &serde_json::Value| {
        profile["fov_offset"].as_f64().unwrap_or(0.0) == 0.0
            && profile["offset"]
                .as_array()
                .is_none_or(|v| v.iter().all(|x| x.as_f64() == Some(0.0)))
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
                let number = |value: &serde_json::Value| {
                    let value = value.as_f64().unwrap_or(0.0);
                    format!("{:?}", if value == 0.0 { 0.0 } else { value })
                };
                let mut fields = Vec::new();
                if profile["fov_offset"].as_f64().unwrap_or(0.0) != 0.0 {
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
#[path = "framing_snapshot_tests.rs"]
mod snapshot_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use hl2_ui::tuning::narrowest_design_aspect;

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
    fn viewmodel_edit_profiles_and_snapshot_round_trip() {
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
            accumulated_value["viewmodel"]["weapons"]["weapon_physcannon"]["portrait"]["fov_offset"],
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
    }
}

#[cfg(all(test, feature = "bevy"))]
mod projection_tests {
    #[test]
    fn viewmodel_framing_keeps_physcannon_and_rpg_in_frame_across_portrait_and_ultrawide() {
        let project = |p: [f32; 3], weapon: &str, aspect: f32| {
            let framing = crate::framing::viewmodel_framing(weapon, aspect);
            let (fov_v, _) =
                hl2_bevy::camera::viewmodel_projection(54., framing.fov_offset, 1000., aspect);
            let x = (p[0] + framing.offset[0]).max(0.1);
            let y = p[1] + framing.offset[1];
            let z = p[2] + framing.offset[2];
            let tan_v = (fov_v * 0.5).tan();
            let tan_h = aspect * tan_v;
            ((-y / x) / tan_h, (z / x) / tan_v)
        };
        // Gravity Gun left arm stump (X=24.5, Y=2.4, Z=-21.4) stays below the bottom screen edge
        // (ndc_y < -1.0) on 9:16 portrait just like on 16:9.
        let (_, stump_16_9) = project([24.5, 2.4, -21.4], "weapon_physcannon", 16. / 9.);
        let (_, stump_9_16) = project([24.5, 2.4, -21.4], "weapon_physcannon", 9. / 16.);
        assert!(stump_16_9 < -1.0 && stump_9_16 < -1.0);

        // Rocket Launcher mid-tube reaches into a 9:16 frame; the rear cutoff stays outside the
        // right edge up to 64:27. At 32:9 the owner-tuned offset pulls the cutoff to the edge, so
        // it only has to stay near it.
        let (mid_9_16, _) = project([30.0, -14.0, -8.0], "weapon_rpg", 9. / 16.);
        assert!(mid_9_16 < 0.7, "{mid_9_16}");
        for aspect in [16. / 9., 64. / 27.] {
            let (rear_x, _) = project([16.1, -14.0, -5.0], "weapon_rpg", aspect);
            assert!(rear_x > 1.0, "RPG rear cutoff hidden at {aspect}: {rear_x}");
        }
        let (rear_32_9, _) = project([16.1, -14.0, -5.0], "weapon_rpg", 32. / 9.);
        assert!(
            rear_32_9 > 0.9,
            "RPG rear cutoff far inside at 32:9: {rear_32_9}"
        );
    }
}
