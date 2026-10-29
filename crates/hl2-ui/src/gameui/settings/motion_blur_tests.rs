use super::AdvancedChoice;
use crate::{
    config::{AdvancedVideoSettings, Config},
    console::Console,
};

#[test]
fn motion_blur_has_distinct_retail_advanced_and_ultra_tiers() {
    let choice = AdvancedChoice::MotionBlur;
    let mut a = AdvancedVideoSettings::default();
    assert_eq!(choice.selected(&a), 1);
    assert_eq!(
        choice.tokens(),
        [
            "#gameui_disabled",
            "#gameui_enabled",
            "#GameUI_SettingAdvanced",
            crate::gameui::localize::placeholders::MOTION_BLUR_ULTRA
        ]
    );
    choice.select(&mut a, 2);
    assert!(a.motion_blur && a.motion_blur_viewmodel);
    assert_eq!(choice.selected(&a), 2);
    assert!(!a.motion_blur_world);
    assert!(a.motion_blur_mode().viewmodel());
    assert!(a.motion_blur_mode().retail_camera());
    assert!(!a.motion_blur_mode().world());
    choice.select(&mut a, 3);
    assert_eq!(choice.selected(&a), 3);
    assert!(a.motion_blur_mode().viewmodel() && a.motion_blur_mode().world());
    assert!(!a.motion_blur_mode().retail_camera());
    choice.select(&mut a, 4);
    assert_eq!(choice.selected(&a), 3);
    choice.select(&mut a, 1);
    assert!(a.motion_blur && !a.motion_blur_viewmodel);
    choice.select(&mut a, 0);
    assert!(!a.motion_blur && !a.motion_blur_viewmodel && !a.motion_blur_world);
}

#[test]
fn old_saves_remain_retail_and_advanced_round_trips() {
    for enabled in [false, true] {
        let bytes = format!(r#"{{"video":{{"advanced":{{"motion_blur":{enabled}}}}}}}"#);
        let config = Config::from_json(bytes.as_bytes()).unwrap();
        assert_eq!(config.video.advanced.motion_blur, enabled);
        assert!(!config.video.advanced.motion_blur_viewmodel);
        assert!(!config.video.advanced.motion_blur_world);
    }
    let mut config = Config::default();
    AdvancedChoice::MotionBlur.select(&mut config.video.advanced, 2);
    let bytes = serde_json::to_vec(&config).unwrap();
    let restored = Config::from_json(&bytes).unwrap();
    assert_eq!(
        AdvancedChoice::MotionBlur.selected(&restored.video.advanced),
        2
    );
    let legacy = Config::from_json(
        br#"{"video":{"advanced":{"motion_blur":true,"motion_blur_viewmodel":true}}}"#,
    )
    .unwrap();
    assert_eq!(
        AdvancedChoice::MotionBlur.selected(&legacy.video.advanced),
        2
    );
    assert!(!legacy.video.advanced.motion_blur_world);
    let mut ultra = restored.clone();
    AdvancedChoice::MotionBlur.select(&mut ultra.video.advanced, 3);
    let restored_ultra = Config::from_json(&serde_json::to_vec(&ultra).unwrap()).unwrap();
    assert_eq!(
        AdvancedChoice::MotionBlur.selected(&restored_ultra.video.advanced),
        3
    );
    let mut console = Console::default();
    console.config = restored_ultra;
    assert_eq!(
        console.cvar_value("mat_motion_blur_enabled").as_deref(),
        Some("1")
    );
    console.submit("mat_motion_blur_enabled 1");
    assert_eq!(
        AdvancedChoice::MotionBlur.selected(&console.config.video.advanced),
        1
    );
}

#[test]
#[ignore = "requires an owned installed Half-Life 2 copy"]
fn owned_motion_blur_advanced_token() {
    use crate::gameui::resources::GameUiResources;
    let root = source_assets::install::discover().unwrap();
    let vfs = source_assets::vpk::Vfs::mount(&root).unwrap();
    let resources = GameUiResources::load(&vfs);
    let tokens = AdvancedChoice::MotionBlur.tokens();
    assert_eq!(resources.localization.text(tokens[2]), "Advanced");
    assert_eq!(resources.localization.text(tokens[0]), "Disabled");
    assert_eq!(resources.localization.text(tokens[1]), "Enabled");
    assert_eq!(resources.localization.text(tokens[3]), "Ultra");
}
