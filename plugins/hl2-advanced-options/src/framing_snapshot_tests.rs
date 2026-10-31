use super::{
    ViewmodelFraming, ViewmodelProfile, compact_viewmodel_json, tuning_json_with_viewmodel_profiles,
};
use std::collections::BTreeMap;

#[test]
fn partial_profiles_omit_zero_fields_without_changing_offsets() {
    let root = serde_json::json!({"viewmodel": {"weapons": {
        "weapon_rpg": {
            "portrait": {"offset": [-0.5, 7.9, -9.5]},
            "ultrawide": {"offset": [-5.5, 0.0, 0.0]}
        },
        "weapon_shotgun": {"portrait": {"offset": [-1.1, 1.3, -3.3]}},
        "weapon_smg1": {
            "portrait": {"offset": [-3.3, 1.8, -0.5]},
            "standard": {"offset": [0.0, 0.0, 0.2]}
        },
        "weapon_zero": {"standard": {}, "portrait": {"offset": [-0.0, 0.0, 0.0]}, "ultrawide": {"fov_offset": -0.0}},
        "weapon_fov": {"standard": {"fov_offset": 4.0}}
    }}});
    let text = compact_viewmodel_json(root.clone());
    assert!(!text.contains("\"fov_offset\": 0.0"), "{text}");
    assert!(
        text.contains("\"standard\": {\"offset\": [0.0, 0.0, 0.2]}"),
        "{text}"
    );
    assert!(!text.contains("weapon_zero"), "{text}");
    let output: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        compact_viewmodel_json(output.clone()),
        text,
        "compaction must be stable"
    );
    for weapon in ["weapon_rpg", "weapon_shotgun", "weapon_smg1", "weapon_fov"] {
        for profile in ["standard", "portrait", "ultrawide"] {
            let framing = |value: &serde_json::Value| {
                if value.is_null() {
                    ViewmodelFraming::ZERO
                } else {
                    serde_json::from_value::<ViewmodelFraming>(value.clone()).unwrap()
                }
            };
            assert_eq!(
                framing(&root["viewmodel"]["weapons"][weapon][profile]),
                framing(&output["viewmodel"]["weapons"][weapon][profile])
            );
        }
    }
}

#[test]
fn complete_session_snapshot_omits_zero_fov_in_untouched_and_edited_profiles() {
    let edits = BTreeMap::from([(
        ("weapon_smg1".to_owned(), ViewmodelProfile::Standard),
        ViewmodelFraming {
            fov_offset: 0.,
            offset: [0., 0., 0.2],
        },
    )]);
    let snapshot = tuning_json_with_viewmodel_profiles(&edits).unwrap();
    assert!(!snapshot.contains("\"fov_offset\": 0.0"), "{snapshot}");
    assert!(snapshot.contains("\"standard\": {\"offset\": [0.0, 0.0, 0.2]}"));
}
