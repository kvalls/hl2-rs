//! Behavior the plugin's injected video policy adds to the core settings, cvars and projection.
use crate::fov::{vertical_fov, world_fov_offset};
use crate::ui::AdvancedPolicy;
use hl2_ui::config::Config;
use hl2_ui::console::{Console, Mode};
use hl2_ui::extensions::{Extensions, VideoExtension, VideoPolicy};
use hl2_ui::gameui::display::{AspectRatio, DisplayModes, Resolution};
use hl2_ui::gameui::localize::Localization;
use hl2_ui::gameui::settings::{AdvancedChoice, Binding, Choice, Draft, Env, enabled};

fn console() -> Console {
    let mut console = Console::default();
    let mut extensions = Extensions::default();
    crate::register(&mut extensions).unwrap();
    extensions.install(&mut console);
    console
}

/// Injects the policy the way the application does; stored settings never carry it.
fn with_policy(mut config: Config) -> Config {
    config.video.policy = VideoPolicy::new(AdvancedPolicy);
    config.video.advanced.policy = VideoPolicy::new(AdvancedPolicy);
    config
}

fn derived(view: f32, aspect: f32) -> f32 {
    2. * ((view.to_radians() / 2.).tan() / aspect.min(4. / 3.)).atan()
}

#[test]
fn an_explicit_vertical_fov_replaces_the_derived_one_and_scales_with_zoom() {
    for aspect in [5. / 4., 4. / 3., 16. / 10., 16. / 9.] {
        assert_eq!(
            vertical_fov(75., 75., aspect, 0.),
            derived(75., aspect),
            "retail aspects are untouched in automatic mode"
        );
    }
    // Outside 5:4..=16:9, automatic mode applies `world_fov_offset` to tame vertical fish-eye in
    // portrait and horizontal peripheral stretching in ultrawide.
    assert!(vertical_fov(75., 75., 9. / 16., 0.) < derived(75., 9. / 16.));
    assert!(vertical_fov(75., 75., 9. / 16., 0.) > derived(75., 4. / 3.));
    assert!(vertical_fov(75., 75., 32. / 9., 0.) < derived(75., 32. / 9.));

    for aspect in [9. / 16., 1., 4. / 3., 16. / 9., 32. / 9.] {
        let base = derived(75., aspect);
        let auto = vertical_fov(75., 75., aspect, 0.);
        let explicit = vertical_fov(75., 75., aspect, 90.);
        assert!(
            (explicit.to_degrees() - 90.).abs() < 1e-3,
            "{aspect}: {explicit}"
        );
        // Zoom narrows automatic and explicit FOV in the same proportion as the derived one.
        let derived_ratio = (derived(20., aspect) / 2.).tan() / (base / 2.).tan();
        let auto_ratio = (vertical_fov(20., 75., aspect, 0.) / 2.).tan() / (auto / 2.).tan();
        assert!((auto_ratio - derived_ratio).abs() < 1e-5, "{aspect}");
        let zoomed = vertical_fov(20., 75., aspect, 90.);
        let ratio = (zoomed / 2.).tan() / (explicit / 2.).tan();
        assert!((ratio - derived_ratio).abs() < 1e-5, "{aspect}");
        assert!(zoomed < explicit);
    }
}

#[test]
fn ultrawide_gameplay_trims_vertical_fov_without_full_fish_eye() {
    let horizontal = |vertical: f32, aspect: f32| 2. * ((vertical / 2.).tan() * aspect).atan();
    let v_4_3 = derived(75., 4. / 3.);
    let h_16_9 = horizontal(v_4_3, 16. / 9.);
    for aspect in [3440. / 1440., 64. / 27., 32. / 9.] {
        assert!(world_fov_offset(aspect) < 0., "{aspect}");
        let game = vertical_fov(75., 75., aspect, 0.);
        assert!(game < v_4_3, "{aspect}: trims vertical FOV");
        assert!(
            horizontal(game, aspect) > h_16_9
                && horizontal(game, aspect) < horizontal(v_4_3, aspect),
            "{aspect}: expands beyond 16:9 without full 4:3 fish-eye"
        );
    }
}

#[test]
fn fit_display_replaces_a_low_fixed_frame_with_current_physical_resolution() {
    // The host configures Fit Display from the presented size, not the previous fixed mode.
    let native = AdvancedPolicy.presented(Resolution::new(3024, 1964));
    assert_eq!(native, Resolution::new(3024, 1964));
    let size = AdvancedPolicy.frame_size(
        glam::UVec2::new(native.width, native.height),
        glam::UVec2::new(3024, 1964),
        true,
    );
    assert_eq!(size, glam::UVec2::new(3024, 1964));
}

#[test]
fn a_flexible_window_limits_each_side_on_its_own() {
    let room = Some(glam::Vec2::new(3456., 1964.));
    let size = AdvancedPolicy.window_size(glam::UVec2::new(3456, 2160), room, true);
    assert_eq!(size, glam::Vec2::new(3456., 1964.));
}

#[test]
fn fov_cvars_query_set_and_clamp() {
    let mut console = console();
    console.set_mode(Mode::Console);

    assert_eq!(console.cvar_value("vertical_fov").as_deref(), Some("0"));
    console.submit("vertical_fov 60");
    assert_eq!(crate::settings::vertical_fov(&console.config.video), 60.0);
    assert_eq!(console.cvar_value("vertical_fov").as_deref(), Some("60"));
    console.submit("vertical_fov 0");
    assert_eq!(crate::settings::vertical_fov(&console.config.video), 0.0);
    console.submit("vertical_fov 150");
    assert_eq!(crate::settings::vertical_fov(&console.config.video), 100.0);
    console.submit("vertical_fov 9");
    assert_eq!(crate::settings::vertical_fov(&console.config.video), 40.0);

    assert_eq!(console.cvar_value("viewmodel_fov").as_deref(), Some("54"));
    console.submit("viewmodel_fov 65");
    assert_eq!(crate::settings::viewmodel_fov(&console.config.video), 65.0);
    assert_eq!(console.cvar_value("viewmodel_fov").as_deref(), Some("65"));
    console.submit("viewmodel_fov 20");
    assert_eq!(crate::settings::viewmodel_fov(&console.config.video), 40.0);
    console.submit("viewmodel_fov 120");
    assert_eq!(crate::settings::viewmodel_fov(&console.config.video), 80.0);
}

#[test]
fn motion_blur_has_distinct_retail_advanced_and_ultra_tiers() {
    let choice = AdvancedChoice::MotionBlur;
    let mut a = with_policy(Config::default()).video.advanced;
    assert_eq!(choice.selected(&a), 1);
    assert_eq!(
        AdvancedPolicy.blur_tokens(),
        [
            "#gameui_disabled",
            "#gameui_enabled",
            "#GameUI_SettingAdvanced",
            "#HL2RS_MotionBlurUltra"
        ]
    );
    choice.select(&mut a, 2);
    assert!(a.motion_blur && crate::settings::blur_viewmodel(&a));
    assert_eq!(choice.selected(&a), 2);
    assert!(!crate::settings::blur_world(&a));
    assert!(crate::motion_blur::selected(&a).viewmodel());
    assert!(crate::motion_blur::selected(&a).retail_camera());
    assert!(!crate::motion_blur::selected(&a).world());
    choice.select(&mut a, 3);
    assert_eq!(choice.selected(&a), 3);
    assert!(
        crate::motion_blur::selected(&a).viewmodel() && crate::motion_blur::selected(&a).world()
    );
    assert!(!crate::motion_blur::selected(&a).retail_camera());
    choice.select(&mut a, 4);
    assert_eq!(choice.selected(&a), 3);
    choice.select(&mut a, 1);
    assert!(a.motion_blur && !crate::settings::blur_viewmodel(&a));
    choice.select(&mut a, 0);
    assert!(
        !a.motion_blur && !crate::settings::blur_viewmodel(&a) && !crate::settings::blur_world(&a)
    );
}

#[test]
fn advanced_tiers_round_trip_through_saved_settings() {
    let reload = |config: &Config| {
        with_policy(Config::from_json(&serde_json::to_vec(config).unwrap()).unwrap())
    };
    let mut config = with_policy(Config::default());
    AdvancedChoice::MotionBlur.select(&mut config.video.advanced, 2);
    let restored = reload(&config);
    assert_eq!(
        AdvancedChoice::MotionBlur.selected(&restored.video.advanced),
        2
    );
    let legacy = with_policy(
        Config::from_json(
            br#"{"video":{"advanced":{"motion_blur":true,"motion_blur_viewmodel":true}}}"#,
        )
        .unwrap(),
    );
    assert_eq!(
        AdvancedChoice::MotionBlur.selected(&legacy.video.advanced),
        2
    );
    assert!(!crate::settings::blur_world(&legacy.video.advanced));
    let mut ultra = restored.clone();
    AdvancedChoice::MotionBlur.select(&mut ultra.video.advanced, 3);
    let restored_ultra = reload(&ultra);
    assert_eq!(
        AdvancedChoice::MotionBlur.selected(&restored_ultra.video.advanced),
        3
    );
    let mut console = console();
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
fn flexible_aspect_disables_resolution_and_shows_fit_display() {
    let mut loc = Localization::default();
    loc.insert("#GameUI_HUDAspectRatio_FitDisplay", "Fit Display");
    let modes = DisplayModes {
        fullscreen: vec![Resolution::new(1920, 1080), Resolution::new(1280, 720)],
        desktop: Some(Resolution::new(1920, 1080)),
        current: Some(Resolution::new(1600, 900)),
    };
    let langs = ["english".to_owned()];
    let env = Env {
        localization: &loc,
        languages: &langs,
        modes: &modes,
        recommendation: None,
    };
    let mut draft = Draft::new(with_policy(Config::default()));
    let res_binding = Some(Binding::Choice(Choice::Resolution));
    assert!(enabled(res_binding, true, &draft, &draft.config));
    assert_eq!(
        Choice::Resolution.items(&env, &draft),
        ["1920 x 1080", "1600 x 900", "1366 x 768", "1280 x 720"]
    );

    let index_of = |aspect| {
        crate::display::ASPECTS
            .iter()
            .position(|a| *a == aspect)
            .unwrap()
    };
    assert_eq!(
        index_of(crate::display::FIT_DISPLAY),
        0,
        "Fit Display is in the first place"
    );
    Choice::Aspect.select(&env, &mut draft, index_of(crate::display::FIT_DISPLAY));
    assert_eq!(draft.aspect, crate::display::FIT_DISPLAY);
    assert!(crate::settings::fit_display(&draft.config.video));
    assert_eq!(
        (draft.config.video.width, draft.config.video.height),
        (1600, 900)
    );

    // Resolution is disabled and shows the presented size; selecting it is a no-op.
    assert!(!enabled(res_binding, true, &draft, &draft.config));
    assert_eq!(Choice::Resolution.items(&env, &draft), ["1600 x 900"]);
    assert_eq!(Choice::Resolution.selected(&env, &draft), Some(0));
    assert_eq!(Choice::Resolution.text(&env, &draft), "1600 x 900");
    Choice::Resolution.select(&env, &mut draft, 0);

    // A window beyond 32:9 or taller than 9:16 reads without its black stripes.
    for (current, expected) in [
        (Resolution::new(8000, 1000), "3556 x 1000"),
        (Resolution::new(500, 2000), "500 x 889"),
    ] {
        let striped = DisplayModes {
            fullscreen: vec![],
            desktop: None,
            current: Some(current),
        };
        let env = Env {
            modes: &striped,
            ..env
        };
        assert_eq!(Choice::Resolution.items(&env, &draft), [expected]);
        assert_eq!(Choice::Resolution.text(&env, &draft), expected);
    }

    Choice::Aspect.select(&env, &mut draft, index_of(AspectRatio::Wide16x9));
    assert_eq!(draft.aspect, AspectRatio::Wide16x9);
    assert!(!crate::settings::fit_display(&draft.config.video));
    assert!(enabled(res_binding, true, &draft, &draft.config));
    assert_eq!(
        Choice::Resolution.items(&env, &draft),
        ["1920 x 1080", "1600 x 900", "1366 x 768", "1280 x 720"]
    );
}

#[test]
fn every_plugin_placeholder_has_text_and_an_hl2rs_token() {
    for (key, text) in crate::localize::PLACEHOLDER_STRINGS {
        assert!(key.starts_with("HL2RS_"), "{key}");
        assert!(!text.is_empty(), "{key}");
    }
    for token in [
        "#HL2RS_AspectUltrawide21x9",
        "#HL2RS_AspectUltrawide32x9",
        "#HL2RS_AspectSquare1x1",
        "#HL2RS_AspectPortrait3x4",
        "#HL2RS_AspectPortrait10x16",
        "#HL2RS_AspectPortrait9x16",
        "#HL2RS_KeepVideoSettings",
        "#HL2RS_VerticalFov",
        "#HL2RS_ViewmodelFov",
        "#HL2RS_MotionBlurUltra",
        "#HL2RS_StatusMotionBlur",
    ] {
        let key = token.trim_start_matches('#');
        assert!(
            crate::localize::PLACEHOLDER_STRINGS
                .iter()
                .any(|(k, _)| *k == key),
            "{token}"
        );
    }
}

#[test]
fn installed_localization_wins_over_plugin_placeholders() {
    let mut resources = hl2_ui::gameui::resources::GameUiResources::default();
    resources
        .localization
        .insert("#HL2RS_VerticalFov", "Installed");
    let mut extensions = Extensions::default();
    crate::register(&mut extensions).unwrap();
    extensions.prepare_resources(&mut resources);
    assert_eq!(
        resources.localization.text("#HL2RS_VerticalFov"),
        "Installed"
    );
    assert_eq!(
        resources.localization.text("#HL2RS_ViewmodelFov"),
        "Viewmodel FOV"
    );
}

#[test]
fn a_second_video_policy_is_rejected_rather_than_replacing_the_first() {
    let mut extensions = Extensions::default();
    crate::register(&mut extensions).unwrap();
    let error = extensions
        .select_video("another", VideoPolicy::new(AdvancedPolicy))
        .unwrap_err();
    assert!(
        error.contains("hl2-advanced-options") && error.contains("another"),
        "{error}"
    );
}

#[test]
fn malformed_fov_cvars_change_nothing() {
    let mut console = console();
    console.set_mode(Mode::Console);
    for command in [
        "vertical_fov x",
        "vertical_fov 1 1",
        "viewmodel_fov x",
        "viewmodel_fov 1 1",
    ] {
        let before = console.config.clone();
        console.submit(command);
        assert_eq!(console.config, before, "{command}");
    }
}

#[test]
fn old_save_files_keep_their_values_and_format() {
    let bytes = br#"{"video":{"flexible_aspect":true,"vertical_fov":90.0,"viewmodel_fov":70.0,
        "advanced":{"motion_blur":true,"motion_blur_viewmodel":true,"motion_blur_world":true}}}"#;
    let config = with_policy(Config::from_json(bytes).unwrap());
    assert!(crate::settings::fit_display(&config.video));
    assert!(config.video.frame_follows_window());
    assert_eq!(config.video.projection_parameter(), 90.);
    assert_eq!(config.video.effective_viewmodel_fov(), 70.);
    assert_eq!(
        crate::motion_blur::selected(&config.video.advanced),
        crate::motion_blur::MotionBlurMode::Ultra
    );
    assert!(!config.video.advanced.retail_camera_blur());
    assert_eq!(
        Draft::new(config.clone()).aspect,
        crate::display::FIT_DISPLAY
    );
    let saved = serde_json::to_value(&config).unwrap();
    assert_eq!(saved["video"]["flexible_aspect"], true);
    assert_eq!(saved["video"]["vertical_fov"], 90.0);
    assert_eq!(saved["video"]["advanced"]["motion_blur_world"], true);
}

#[test]
fn the_motion_blur_cvar_returns_to_the_retail_tiers() {
    let mut console = console();
    console.set_mode(Mode::Console);
    crate::settings::set_blur_tier(&mut console.config.video.advanced, true, true);
    console.submit("mat_motion_blur_enabled 1");
    assert_eq!(
        crate::motion_blur::selected(&console.config.video.advanced),
        crate::motion_blur::MotionBlurMode::Enabled
    );
}
