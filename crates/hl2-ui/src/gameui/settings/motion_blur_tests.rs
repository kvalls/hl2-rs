use super::AdvancedChoice;
use crate::config::{AdvancedVideoSettings, Config};

#[test]
fn core_motion_blur_has_the_retail_tiers_only() {
    let choice = AdvancedChoice::MotionBlur;
    let mut a = AdvancedVideoSettings::default();
    assert_eq!(choice.selected(&a), 1);
    assert_eq!(choice.tokens(), ["#gameui_disabled", "#gameui_enabled"]);
    choice.select(&mut a, 2);
    assert_eq!(choice.selected(&a), 1, "no tier beyond the retail choices");
    choice.select(&mut a, 0);
    assert!(!a.motion_blur);
    assert_eq!(choice.selected(&a), 0);
}

#[test]
fn old_saves_remain_retail_and_dormant_tiers_survive_without_the_plugin() {
    for enabled in [false, true] {
        let bytes = format!(r#"{{"video":{{"advanced":{{"motion_blur":{enabled}}}}}}}"#);
        let config = Config::from_json(bytes.as_bytes()).unwrap();
        assert_eq!(config.video.advanced.motion_blur, enabled);
        assert!(!config.video.advanced.motion_blur_viewmodel);
        assert!(!config.video.advanced.motion_blur_world);
    }
    let mut saved = Config::from_json(
        br#"{"video":{"advanced":{"motion_blur":true,"motion_blur_viewmodel":true,"motion_blur_world":true}}}"#,
    )
    .unwrap();
    assert_eq!(
        AdvancedChoice::MotionBlur.selected(&saved.video.advanced),
        1,
        "applied as the retail tier"
    );
    AdvancedChoice::MotionBlur.select(&mut saved.video.advanced, 1);
    let restored = Config::from_json(&serde_json::to_vec(&saved).unwrap()).unwrap();
    assert!(restored.video.advanced.motion_blur_viewmodel);
    assert!(restored.video.advanced.motion_blur_world);
}

#[test]
#[ignore = "requires an owned installed Half-Life 2 copy"]
fn owned_motion_blur_tokens() {
    use crate::gameui::resources::GameUiResources;
    let root = source_assets::install::discover().unwrap();
    let vfs = source_assets::vpk::Vfs::mount(&root).unwrap();
    let resources = GameUiResources::load(&vfs);
    let tokens = AdvancedChoice::MotionBlur.tokens();
    assert_eq!(resources.localization.text(tokens[0]), "Disabled");
    assert_eq!(resources.localization.text(tokens[1]), "Enabled");
    assert_eq!(
        resources.localization.text("#GameUI_SettingAdvanced"),
        "Advanced",
        "the installed token the advanced-options plugin reuses"
    );
}
