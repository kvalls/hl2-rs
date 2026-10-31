//! Viewmodel framing profiles: which one applies at an aspect, and its value per weapon.
pub use modkit_core::viewmodel::ViewmodelFraming;
use std::collections::BTreeMap;

/// The profile every aspect uses unless a policy offers others.
pub const STANDARD_PROFILE: &str = "standard";

/// Temporary edits, keyed by weapon classname and profile id.
pub type FramingEdits = BTreeMap<(String, &'static str), ViewmodelFraming>;

/// Implemented by a policy with its own framing data; the defaults leave the retail view alone.
pub trait FramingProfiles: std::fmt::Debug + Send + Sync {
    /// The profile id that applies at `aspect` (`width / height`).
    fn profile_at(&self, _aspect: f32) -> &'static str {
        STANDARD_PROFILE
    }
    /// A profile's stored value for `weapon`.
    fn endpoint(&self, _weapon: &str, _profile: &str) -> ViewmodelFraming {
        ViewmodelFraming::ZERO
    }
    /// The live framing at `aspect`, with one profile's value replaced by `edit`.
    fn blended(
        &self,
        _weapon: &str,
        _aspect: f32,
        edit: Option<(&str, ViewmodelFraming)>,
    ) -> ViewmodelFraming {
        edit.map_or(ViewmodelFraming::ZERO, |(_, value)| value)
    }
    /// Text a tuner can copy to keep `edits`.
    fn snapshot(&self, edits: &FramingEdits) -> Result<String, serde_json::Error> {
        let mut weapons = serde_json::Map::new();
        for ((weapon, profile), framing) in edits {
            let entry = weapons
                .entry(weapon.clone())
                .or_insert_with(|| serde_json::json!({}));
            entry[*profile] = serde_json::json!({
                "fov_offset": framing.fov_offset,
                "offset": framing.offset,
            });
        }
        serde_json::to_string_pretty(&serde_json::json!({ "viewmodel": weapons }))
    }
}

/// No framing data: the view retail shows at every aspect.
#[derive(Debug)]
pub(super) struct RetailFraming;
impl FramingProfiles for RetailFraming {}

#[cfg(test)]
mod tests {
    use super::*;

    const EDIT: ViewmodelFraming = ViewmodelFraming {
        fov_offset: 1.5,
        offset: [1., 2., 3.],
    };

    #[test]
    fn without_a_contributor_every_aspect_keeps_the_retail_view() {
        for aspect in [9. / 16., 4. / 3., 16. / 9., 32. / 9.] {
            assert_eq!(RetailFraming.profile_at(aspect), STANDARD_PROFILE);
            assert_eq!(
                RetailFraming.blended("weapon_rpg", aspect, None),
                ViewmodelFraming::ZERO
            );
        }
        assert_eq!(
            RetailFraming.endpoint("weapon_rpg", STANDARD_PROFILE),
            ViewmodelFraming::ZERO
        );
    }

    #[test]
    fn a_tuning_edit_works_without_a_contributor() {
        assert_eq!(
            RetailFraming.blended("weapon_rpg", 4. / 3., Some((STANDARD_PROFILE, EDIT))),
            EDIT
        );
        let edits = FramingEdits::from([(("weapon_rpg".to_owned(), STANDARD_PROFILE), EDIT)]);
        let text = RetailFraming.snapshot(&edits).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            json["viewmodel"]["weapon_rpg"]["standard"]["fov_offset"],
            1.5
        );
    }

    /// An independent contributor with a profile core has never heard of.
    #[derive(Debug)]
    struct Tall;
    impl FramingProfiles for Tall {
        fn profile_at(&self, aspect: f32) -> &'static str {
            if aspect < 1. {
                "tall"
            } else {
                STANDARD_PROFILE
            }
        }
        fn endpoint(&self, _weapon: &str, profile: &str) -> ViewmodelFraming {
            if profile == "tall" {
                EDIT
            } else {
                ViewmodelFraming::ZERO
            }
        }
    }

    #[test]
    fn a_contributor_chooses_its_own_profile_ids() {
        let profiles: &dyn FramingProfiles = &Tall;
        let profile = profiles.profile_at(0.5);
        assert_eq!(profile, "tall");
        assert_eq!(profiles.endpoint("weapon_rpg", profile), EDIT);
        assert_eq!(profiles.profile_at(16. / 9.), STANDARD_PROFILE);
    }
}
