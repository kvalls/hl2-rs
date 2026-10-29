use super::*;

#[test]
fn options_tooltips_lookup_and_missing_case() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);

    // 1. Direct tip_for_control lookup:
    // Present tooltip:
    assert_eq!(
        h.dialog.tip_for_control(Panel::Mouse, "ReverseMouse"),
        Some("Reverse mouse up-down axis".to_string())
    );
    // Missing tooltip:
    assert_eq!(h.dialog.tip_for_control(Panel::Mouse, "Slider"), None);
    assert_eq!(
        h.dialog.tip_for_control(Panel::Mouse, "SensitivityLabel"),
        None
    );
    // Missing control:
    assert_eq!(h.dialog.tip_for_control(Panel::Mouse, "NonExistent"), None);

    // 2. Hover tip lookup via tooltip_at:
    // ReverseMouse is at (28, 9, 150, 28) on page
    let rev_ptr = page(35., 15.);
    assert_eq!(
        h.dialog.tooltip_at(vec2(1280., 720.), rev_ptr),
        Some("Reverse mouse up-down axis".to_string())
    );
    // Slider is at (200, 40, 200, 40) on page (has no tooltip)
    let slider_ptr = page(250., 50.);
    assert_eq!(h.dialog.tooltip_at(vec2(1280., 720.), slider_ptr), None);
    // Blank space on page
    assert_eq!(h.dialog.tooltip_at(vec2(1280., 720.), page(10., 10.)), None);

    // 3. Active tooltip lifecycle in frame input:
    // Hovering ReverseMouse sets active_tooltip
    h.frame(Input {
        pointer: Some(rev_ptr),
        ..Default::default()
    });
    let tip = h.dialog.active_tooltip().unwrap();
    assert_eq!(tip.text, "Reverse mouse up-down axis");
    assert_eq!(tip.pointer, rev_ptr);
    assert!(tip.development.is_none());

    // Hovering Slider (missing tip) clears active_tooltip
    h.frame(Input {
        pointer: Some(slider_ptr),
        ..Default::default()
    });
    assert_eq!(h.dialog.active_tooltip(), None);

    // Hovering ReverseMouse again restores active_tooltip
    h.frame(Input {
        pointer: Some(rev_ptr),
        ..Default::default()
    });
    assert!(h.dialog.active_tooltip().is_some());

    // Mouse click dismisses active_tooltip
    h.frame(Input {
        pointer: Some(rev_ptr),
        click: true,
        held: true,
        ..Default::default()
    });
    assert_eq!(h.dialog.active_tooltip(), None);

    // Moving outside page clears active_tooltip
    h.frame(Input {
        pointer: Some(vec2(10., 10.)),
        ..Default::default()
    });
    assert_eq!(h.dialog.active_tooltip(), None);
}

#[test]
fn development_only_and_stacked_hints_reuse_cached_text_and_keep_completed_controls_clean() {
    use super::super::hints::DEVELOPMENT_FILL;
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    let point = page(40., 180.);
    h.frame(Input {
        pointer: Some(point),
        ..Default::default()
    });
    let tip = h.dialog.active_tooltip().unwrap();
    assert!(tip.text.is_empty());
    let address = tip.development.as_ref().unwrap().as_ptr();
    h.frame(Input {
        pointer: Some(point + vec2(2., 1.)),
        ..Default::default()
    });
    assert_eq!(
        h.dialog
            .active_tooltip()
            .unwrap()
            .development
            .as_ref()
            .unwrap()
            .as_ptr(),
        address
    );
    let report = h.dialog.report();
    assert!(report.development_controls.contains(&"Autoaim".into()));
    assert!(!report.development_controls.contains(&"ReverseMouse".into()));
    assert!(!report.development_controls.contains(&"Slider".into()));
    let mut res = (*resources()).clone();
    let control = res
        .layouts
        .get_mut("optionssubmouse")
        .unwrap()
        .controls
        .iter_mut()
        .find(|c| c.name == "Autoaim")
        .unwrap();
    control.tooltip = "#GameUI_ReverseMouseLabel".into();
    h.dialog.set_resources(Arc::new(res));
    h.frame(Input {
        pointer: Some(point),
        ..Default::default()
    });
    let tip = h.dialog.active_tooltip().unwrap();
    assert_eq!(tip.text, "Reverse mouse up-down axis");
    assert!(tip.development.is_some());
    let quads = drawn(&h);
    let orange = quads
        .iter()
        .find(|q| q.color == h.theme.tooltip_bg)
        .unwrap()
        .destination;
    let blue = quads
        .iter()
        .rfind(|q| q.color == DEVELOPMENT_FILL && q.destination.h > 4.)
        .unwrap()
        .destination;
    assert_eq!(blue.y, orange.y + orange.h + widgets::TOOLTIP_GAP);
    h.frame(Input {
        pointer: Some(page(40., 15.)),
        ..Default::default()
    });
    assert!(h.dialog.active_tooltip().unwrap().development.is_none());
    h.tab(Tab::Keyboard);
    assert!(h.dialog.active_tooltip().is_none());
}

#[test]
fn disabled_subdialog_controls_show_status_and_popups_occlude_it() {
    let mut h = Harness::new();
    let mut res = (*resources()).clone();
    let added = Layout::parse(br#"Root { WaterDetail { ControlName ComboBox xpos 20 ypos 50 wide 150 tall 24 enabled 0 } }"#).unwrap();
    res.layouts
        .get_mut("optionssubvideoadvanceddlg")
        .unwrap()
        .controls
        .extend(added.controls);
    h.dialog.set_resources(Arc::new(res));
    h.dialog.open_sub(Sub::VideoAdvanced);
    let canvas = vec2(1280., 720.);
    let size = h.dialog.sub_frame_size(Sub::VideoAdvanced, |_| 0.);
    let origin = h
        .dialog
        .sub_placement
        .place(&sub_frame(canvas, size))
        .origin;
    h.frame(Input {
        pointer: Some(origin + vec2(30., 60.)),
        time: Some(1.),
        ..Default::default()
    });
    assert!(h.dialog.active_tooltip().unwrap().development.is_some());
    h.canvas.set_alpha_multiplier(0.25);
    h.dialog.sub_alpha = 1.;
    let quads = drawn(&h);
    assert!(
        quads
            .iter()
            .any(|q| q.color == super::super::hints::DEVELOPMENT_FILL && q.destination.h > 4.),
        "focused subdialog tooltip does not inherit the inactive parent's fade"
    );
    assert_eq!(
        h.canvas.alpha_multiplier(),
        0.25,
        "parent alpha is restored"
    );
    h.canvas.set_alpha_multiplier(1.);
    h.dialog.escape();
    assert!(h.dialog.active_tooltip().is_none());
    h.tab(Tab::Mouse);
    h.click(page(40., 180.));
    assert!(h.dialog.report().list_open);
    h.frame(Input {
        pointer: Some(page(40., 180.)),
        ..Default::default()
    });
    assert!(h.dialog.active_tooltip().is_none());
}

#[test]
fn applying_visible_changes_keeps_hidden_platform_settings() {
    let mut h = Harness::new();
    let mut config = Config::default();
    config.mouse.acceleration = true;
    config.mouse.raw_input = false;
    config.video.advanced.d3d9ex = false;
    h.dialog.open(config);
    h.tab(Tab::Mouse);
    if cfg!(target_os = "macos") {
        assert!(h
            .dialog
            .report()
            .controls
            .iter()
            .all(|c| c != "MouseRaw" && !c.starts_with("MouseAcceleration")));
    }
    h.dialog.draft.config.mouse.invert = true;
    let OptionsOutcome::Apply(applied) = h.dialog.apply() else {
        panic!("visible change applies")
    };
    assert!(applied.mouse.invert && applied.mouse.acceleration);
    assert!(!applied.mouse.raw_input && !applied.video.advanced.d3d9ex);
}

#[test]
#[cfg(not(target_os = "macos"))]
fn mouse_options_dynamic_enabling_and_draft_update() {
    let layout = Layout::parse(
            br##""Resource/OptionsSubMouse.res" {
                "ReverseMouse" { "ControlName" "CCvarNegateCheckButton" "fieldName" "ReverseMouse"
                    "xpos" "28" "ypos" "9" "wide" "150" "tall" "28" }
                "MouseAccelerationCheckbox" { "ControlName" "CheckButton" "fieldName" "MouseAccelerationCheckbox"
                    "xpos" "30" "ypos" "100" "wide" "150" "tall" "24" }
                "MouseAccelerationSlider" { "ControlName" "CCvarSlider" "fieldName" "MouseAccelerationSlider"
                    "xpos" "200" "ypos" "100" "wide" "200" "tall" "40" }
                "MouseAccelerationLabel" { "ControlName" "TextEntry" "fieldName" "MouseAccelerationLabel"
                    "xpos" "400" "ypos" "100" "wide" "50" "tall" "24" }
                "Autoaim" { "ControlName" "ComboBox" "fieldName" "Autoaim"
                    "xpos" "28" "ypos" "172" "wide" "175" "tall" "24" }
                "AutoaimScaleLabel" { "ControlName" "Label" "fieldName" "AutoaimScaleLabel"
                    "xpos" "28" "ypos" "221" "wide" "179" "tall" "24" "associate" "AutoaimScale" }
                "AutoaimScale" { "ControlName" "CCvarSlider" "fieldName" "AutoaimScale"
                    "xpos" "28" "ypos" "245" "wide" "200" "tall" "40" }
            }"##,
        )
        .unwrap();

    let mut res = (*resources()).clone();
    res.layouts.insert("optionssubmouse".into(), layout);

    // 1. Default config: acceleration is false, aim_assist is Off
    let mut draft = Draft::new(Config::default());
    assert!(!draft.config.mouse.acceleration);
    assert_eq!(draft.config.mouse.aim_assist, AimAssist::Off);

    let widgets = layout_widgets(&res, Panel::Mouse, page_rect(), &draft, &draft.config);
    let is_enabled = |name: &str| {
        widgets
            .iter()
            .find(|w| w.control.name == name)
            .unwrap()
            .enabled
    };

    assert!(is_enabled("ReverseMouse"));
    assert!(is_enabled("MouseAccelerationCheckbox"));
    assert!(!is_enabled("MouseAccelerationSlider"));
    assert!(!is_enabled("MouseAccelerationLabel"));
    assert!(is_enabled("Autoaim"));
    assert!(!is_enabled("AutoaimScale"));
    assert!(!is_enabled("AutoaimScaleLabel"));

    // 2. Enable acceleration and aim assist
    draft.config.mouse.acceleration = true;
    draft.config.mouse.aim_assist = AimAssist::Basic;

    let widgets = layout_widgets(&res, Panel::Mouse, page_rect(), &draft, &draft.config);
    let is_enabled = |name: &str| {
        widgets
            .iter()
            .find(|w| w.control.name == name)
            .unwrap()
            .enabled
    };

    assert!(is_enabled("MouseAccelerationSlider"));
    assert!(is_enabled("MouseAccelerationLabel"));
    assert!(is_enabled("AutoaimScale"));
    assert!(is_enabled("AutoaimScaleLabel"));
}

#[test]
fn options_occluders_only_active_when_settled_and_not_closing() {
    let mut h = Harness::new();
    let sub_rect = Rect::new(10., 10., 100., 100.);
    let confirm_rect = Rect::new(20., 20., 50., 50.);
    let sub = Some(sub_rect);
    let confirm = Some(confirm_rect);
    let external = [Rect::new(30., 30., 40., 40.)];

    // Settled with full alpha: both sub and confirm occlude
    h.dialog.sub_alpha = 1.0;
    h.dialog.confirm_alpha = 1.0;
    h.dialog.closing_sub = None;
    h.dialog.closing_confirm = None;
    let occ = h.dialog.dialog_occluders(sub, confirm, &external);
    assert_eq!(occ.len(), 3);
    assert_eq!(occ[0], sub_rect);
    assert_eq!(occ[1], confirm_rect);
    assert_eq!(occ[2], external[0]);

    // Subdialog closing or fading out: sub occluder is suppressed
    h.dialog.closing_sub = Some(Sub::Gamma);
    let occ = h.dialog.dialog_occluders(sub, confirm, &external);
    assert_eq!(occ.len(), 2);
    assert_eq!(occ[0], confirm_rect);
    assert_eq!(occ[1], external[0]);

    // Subdialog fading (alpha < 0.999): sub occluder is suppressed
    h.dialog.closing_sub = None;
    h.dialog.sub_alpha = 0.5;
    let occ = h.dialog.dialog_occluders(sub, confirm, &external);
    assert_eq!(occ.len(), 2);

    // Confirm box closing or fading: confirm occluder is suppressed
    h.dialog.sub_alpha = 1.0;
    h.dialog.closing_confirm = Some(MessageBox::new(
        String::new(),
        String::new(),
        String::new(),
        None,
    ));
    let occ = h.dialog.dialog_occluders(sub, confirm, &external);
    assert_eq!(occ.len(), 2);
    assert_eq!(occ[0], sub_rect);

    h.dialog.closing_confirm = None;
    h.dialog.confirm_alpha = 0.8;
    let occ = h.dialog.dialog_occluders(sub, confirm, &external);
    assert_eq!(occ.len(), 2);
    assert_eq!(occ[0], sub_rect);
}
