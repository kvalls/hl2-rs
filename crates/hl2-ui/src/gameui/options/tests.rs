use super::sub_frame::{is_frame_chrome, FOOTER_GAP};
use super::*;
use crate::config::{AimAssist, HdrMode};
use crate::gameui::{
    display::{AspectRatio, Resolution},
    localize::Localization,
};
use source_ui::{
    canvas::Canvas,
    layout::Layout,
    widgets::{Fonts, Theme},
};
use std::collections::BTreeMap;

const MOUSE: &str = r##""Resource/OptionsSubMouse.res" {
        "ReverseMouse" { "ControlName" "CCvarNegateCheckButton" "fieldName" "ReverseMouse"
            "xpos" "28" "ypos" "9" "wide" "150" "tall" "28" "labelText" "#GameUI_ReverseMouse"
            "tooltiptext" "#GameUI_ReverseMouseLabel" }
        "Slider" { "ControlName" "CCvarSlider" "fieldName" "Slider" "xpos" "200" "ypos" "40"
            "wide" "200" "tall" "40" "leftText" "#GameUI_Low" "rightText" "#GameUI_High" }
        "SensitivityLabel" { "ControlName" "TextEntry" "fieldName" "SensitivityLabel"
            "xpos" "400" "ypos" "40" "wide" "50" "tall" "24" }
        "Autoaim" { "ControlName" "ComboBox" "fieldName" "Autoaim" "xpos" "28" "ypos" "172"
            "wide" "175" "tall" "24" }
        "MouseFilter" { "ControlName" "CCvarToggleCheckButton" "fieldName" "MouseFilter"
            "xpos" "28" "ypos" "216" "wide" "150" "tall" "28" "labelText" "#GameUI_MouseFilter"
            "tooltiptext" "#GameUI_MouseFilterLabel" }
    }"##;
const VIDEO: &str = r##""Resource/OptionsSubVideo.res" {
        "Resolution" { "ControlName" "ComboBox" "fieldName" "Resolution" "xpos" "36" "ypos" "48"
            "wide" "160" "tall" "24" }
        "AspectRatio" { "ControlName" "ComboBox" "fieldName" "AspectRatio" "xpos" "36"
            "ypos" "117" "wide" "160" "tall" "24" }
        "GammaButton" { "ControlName" "Button" "fieldName" "GammaButton" "xpos" "254"
            "ypos" "117" "wide" "214" "tall" "24" "enabled" "0" "labelText" "#GameUI_Brightness" }
        "AdvancedButton" { "ControlName" "Button" "fieldName" "AdvancedButton" "xpos" "254"
            "ypos" "190" "wide" "214" "tall" "24" "labelText" "#GameUI_AdvancedEllipsis" }
        "HDR" { "ControlName" "ComboBox" "fieldName" "HDR" "xpos" "36" "ypos" "264"
            "wide" "160" "tall" "24" "visible" "0" }
    }"##;
const KEYBOARD: &str = r##""Resource/OptionsSubKeyboard.res" {
        "listpanel_keybindlist" { "ControlName" "SectionedListPanel"
            "fieldName" "listpanel_keybindlist" "xpos" "8" "ypos" "10" "wide" "480" "tall" "258" }
        "Defaults" { "ControlName" "Button" "fieldName" "Defaults" "xpos" "8" "ypos" "276"
            "wide" "134" "tall" "24" "labelText" "#GameUI_UseDefaults" }
        "ChangeKeyButton" { "ControlName" "Button" "fieldName" "ChangeKeyButton" "xpos" "272"
            "ypos" "276" "wide" "106" "tall" "24" "labelText" "#GameUI_SetNewKey" }
        "KeyAdvancedButton" { "ControlName" "Button" "fieldName" "KeyAdvancedButton" "xpos" "148"
            "ypos" "276" "wide" "111" "tall" "24" "labelText" "#GameUI_AdvancedEllipsis" }
        "ClearKeyButton" { "ControlName" "Button" "fieldName" "ClearKeyButton" "xpos" "384"
            "ypos" "276" "wide" "105" "tall" "24" "labelText" "#GameUI_ClearKey" }
    }"##;
const DIFFICULTY: &str = r##""Resource/OptionsSubDifficulty.res" {
        "Skill1Radio" { "ControlName" "RadioButton" "fieldName" "Skill1Radio" "xpos" "20"
            "ypos" "20" "wide" "200" "tall" "24" "tabPosition" "1" "SubTabPosition" "1" }
        "Skill2Radio" { "ControlName" "RadioButton" "fieldName" "Skill2Radio" "xpos" "20"
            "ypos" "60" "wide" "200" "tall" "24" "tabPosition" "1" "SubTabPosition" "2" }
        "Skill3Radio" { "ControlName" "RadioButton" "fieldName" "Skill3Radio" "xpos" "20"
            "ypos" "100" "wide" "200" "tall" "24" "tabPosition" "1" "SubTabPosition" "3" }
    }"##;
const KEYBOARD_ADVANCED: &str = r##""resource/OptionsSubKeyboardAdvancedDlg.res" {
        "FastSwitchCheck" { "ControlName" "CheckButton" "fieldName" "FastSwitchCheck" "xpos" "20"
            "ypos" "38" "wide" "240" "tall" "24" "labelText" "#GameUI_FastSwitchCheck" }
        "ConsoleCheck" { "ControlName" "CheckButton" "fieldName" "ConsoleCheck" "xpos" "20"
            "ypos" "66" "wide" "244" "tall" "24" "labelText" "#GameUI_DeveloperConsoleCheck" }
        "Button1" { "ControlName" "Button" "fieldName" "Button1" "xpos" "95" "ypos" "100"
            "wide" "80" "tall" "24" "labelText" "#GameUI_OK" "Command" "OK" "Default" "1" }
        "Button2" { "ControlName" "Button" "fieldName" "Button2" "xpos" "182" "ypos" "100"
            "wide" "80" "tall" "24" "labelText" "#GameUI_Cancel" "Command" "Close" }
    }"##;
const GAMEPAD: &str = r##""Resource\OptionsSubMouse.res" {
        "JoyScale" { "ControlName" "CCvarSlider" "fieldName" "JoyScale" "xpos" "38" "ypos" "43"
            "wide" "260" "tall" "40" }
        "Autoaim" { "ControlName" "ComboBox" "fieldName" "Autoaim" "xpos" "38" "ypos" "116"
            "wide" "175" "tall" "24" }
        "AutoaimScaleLabel" { "ControlName" "Label" "fieldName" "AutoaimScaleLabel" "xpos" "38"
            "ypos" "165" "wide" "179" "tall" "24" "labelText" "#GameUI_AimAssistScale"
            "associate" "AutoaimScale" }
        "AutoaimScale" { "ControlName" "CCvarSlider" "fieldName" "AutoaimScale" "xpos" "38"
            "ypos" "189" "wide" "260" "tall" "40" "enabled" "0" }
        "WeaponSelection" { "ControlName" "ComboBox" "fieldName" "WeaponSelection" "xpos" "38"
            "ypos" "262" "wide" "175" "tall" "24" }
        "SteamInput" { "ControlName" "Button" "fieldName" "SteamInput" "xpos" "240" "ypos" "262"
            "wide" "240" "tall" "24" "labelText" "#GameUI_Open_SteamInput" "Command" "OpenSteamInput" }
    }"##;
const VIDEO_ADVANCED: &str = r##""resource/OptionsSubVideoAdvancedDlg.res" {
        "OptionsSubVideoAdvancedDlg" { "ControlName" "COptionsSubVideoAdvancedDlg"
            "fieldName" "OptionsSubVideoAdvancedDlg" "wide" "482" "tall" "358" }
        "frame_close" { "ControlName" "Button" "fieldName" "frame_close" "xpos" "457" "ypos" "8"
            "wide" "18" "tall" "18" "labelText" "r" }
        "VSync" { "ControlName" "ComboBox" "fieldName" "VSync" "xpos" "330" "ypos" "196"
            "wide" "132" "tall" "24" }
        "FovSlider" { "ControlName" "CCvarSlider" "fieldName" "FovSlider" "xpos" "176"
            "ypos" "252" "wide" "140" "tall" "50" "minvalue" "75" "maxvalue" "110" }
        "FOVValueLabel" { "ControlName" "Label" "fieldName" "FOVValueLabel" "xpos" "276"
            "ypos" "232" "wide" "32" "tall" "24" "labelText" "75" "textAlignment" "east" }
        "Label11" { "ControlName" "Label" "fieldName" "Label11" "xpos" "18" "ypos" "310"
            "wide" "280" "tall" "24" "labelText" "#GameUI_RecommendedSettings" }
        "Button1" { "ControlName" "Button" "fieldName" "Button1" "xpos" "290" "ypos" "316"
            "wide" "80" "tall" "24" "labelText" "#GameUI_OK" "Command" "OK" }
        "Button2" { "ControlName" "Button" "fieldName" "Button2" "xpos" "382" "ypos" "316"
            "wide" "80" "tall" "24" "labelText" "#GameUI_Cancel" "Command" "Close" }
    }"##;
const GAMMA: &str = r##""Resource/OptionsSubVideoGammaDlg.res" {
        "OptionsSubVideoGammaDlg" { "ControlName" "CGammaDialog" "fieldName" "OptionsSubVideoGammaDlg"
            "xpos" "0" "ypos" "0" "wide" "290" "tall" "396" }
        "Gamma" { "ControlName" "Slider" "fieldName" "Gamma" "xpos" "17" "ypos" "112"
            "wide" "200" "tall" "40" }
        "OKButton" { "ControlName" "Button" "fieldName" "OKButton" "xpos" "104" "ypos" "356"
            "wide" "72" "tall" "24" "labelText" "#vgui_ok" }
        "Button1" { "ControlName" "Button" "fieldName" "Button1" "xpos" "192" "ypos" "356"
            "wide" "72" "tall" "24" "labelText" "#vgui_Cancel" }
    }"##;

/// Canvas 1280x720 => scale 1, dialog at (384, 157), page at (392, 215).
const DIALOG: Vec2 = Vec2::new(384., 157.);

fn page(x: f32, y: f32) -> Vec2 {
    DIALOG + vec2(page_rect().x + x, page_rect().y + y)
}

pub(crate) fn resources() -> Arc<GameUiResources> {
    let mut localization = Localization::default();
    localization.insert("GameUI_Gamepad", "Gamepad");
    localization.insert("GameUI_Open_SteamInput", "Open Steam Input Configuration");
    localization.insert("GameUI_AdvancedEllipsis", "Advanced...");
    localization.insert("GameUI_ReverseMouseLabel", "Reverse mouse up-down axis");
    let mut layouts = BTreeMap::new();
    for (name, text) in [
        ("optionssubmouse", MOUSE),
        ("optionssubvideo", VIDEO),
        ("optionssubkeyboard", KEYBOARD),
        ("optionssubvideogammadlg", GAMMA),
        ("optionssubkeyboardadvanceddlg", KEYBOARD_ADVANCED),
        ("optionssubvideoadvanceddlg", VIDEO_ADVANCED),
        ("optionssubgamepad", GAMEPAD),
        ("optionssubdifficulty", DIFFICULTY),
    ] {
        layouts.insert(name.to_owned(), Layout::parse(text.as_bytes()).unwrap());
    }
    let key_rows = vec![
        KeyRow::Section("#Valve_Movement_Title".into()),
        KeyRow::Action {
            command: "+forward".into(),
            label: "#Valve_Move_Forward".into(),
        },
        KeyRow::Action {
            command: "+jump".into(),
            label: "#Valve_Jump".into(),
        },
    ];
    let default_bindings = BTreeMap::from([
        ("+forward".to_owned(), "KeyW".to_owned()),
        ("+jump".to_owned(), "Space".to_owned()),
    ]);
    Arc::new(GameUiResources {
        localization,
        layouts,
        key_rows,
        default_bindings,
        spoken_languages: vec!["english".into()],
        ..Default::default()
    })
}

struct Harness {
    dialog: OptionsDialog,
    canvas: Canvas,
    fonts: Fonts,
    theme: Theme,
}

impl Harness {
    fn new() -> Self {
        let mut dialog = OptionsDialog::default();
        dialog.set_resources(resources());
        dialog.set_display_modes(DisplayModes::new(vec![
            Resolution::new(1920, 1080),
            Resolution::new(1280, 720),
            Resolution::new(1024, 768),
        ]));
        dialog.open(Config::default());
        Self {
            dialog,
            canvas: Canvas::default(),
            fonts: Fonts::default(),
            theme: Theme::default(),
        }
    }

    fn frame(&mut self, input: Input) -> OptionsOutcome {
        let painter = Painter {
            canvas: &self.canvas,
            fonts: &self.fonts,
            theme: &self.theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };
        let outcome = self.dialog.input(&input, &painter);
        self.dialog.draw(&painter);
        self.canvas.drain();
        outcome
    }

    fn click(&mut self, at: Vec2) -> OptionsOutcome {
        let first = self.frame(Input {
            pointer: Some(at),
            click: true,
            held: true,
            ..Default::default()
        });
        let second = self.frame(Input {
            pointer: Some(at),
            released: true,
            ..Default::default()
        });
        if !matches!(first, OptionsOutcome::None) {
            first
        } else {
            second
        }
    }

    fn key(&mut self, key: Key) -> OptionsOutcome {
        self.frame(Input {
            keys: [key].into(),
            ..Default::default()
        })
    }

    fn tab(&mut self, tab: Tab) {
        let index = Tab::ALL.iter().position(|t| *t == tab).unwrap();
        let rect = self.dialog.tab_rects(&|t| t.chars().count() as f32 * 7.)[index];
        self.click(DIALOG + vec2(rect.x + 4., rect.y + 4.));
        assert_eq!(self.dialog.tab(), tab);
    }

    fn button(&mut self, index: usize) -> OptionsOutcome {
        let rect = button_rects()[index];
        self.click(DIALOG + vec2(rect.x + 4., rect.y + 4.))
    }
}

/// Name of the focused control of `panel` (page layers).
fn focused_name(h: &Harness, panel: Panel) -> Option<String> {
    let list = layout_widgets(
        &h.dialog.res,
        panel,
        page_rect(),
        &h.dialog.draft,
        &h.dialog.applied,
    );
    match h.dialog.focus {
        Some(Target::Control(i)) => list.get(i).map(|w| w.control.name.clone()),
        _ => None,
    }
}

fn shift_tab(h: &mut Harness) -> OptionsOutcome {
    h.frame(Input {
        keys: [Key::Tab].into(),
        shift: true,
        ..Default::default()
    })
}

#[test]
fn the_dialog_fits_square_and_portrait_screens() {
    let h = Harness::new();
    for canvas in [
        vec2(1024., 768.),
        vec2(1080., 1080.),
        vec2(1080., 1920.),
        vec2(720., 1280.),
    ] {
        let bounds = h.dialog.frame_bounds(canvas);
        assert!(
            bounds.x >= 0.
                && bounds.y >= 0.
                && bounds.x + bounds.w <= canvas.x
                && bounds.y + bounds.h <= canvas.y,
            "{canvas}: {bounds:?}"
        );
    }
}

#[test]
fn tab_order_reads_left_to_right_top_to_bottom() {
    // Two columns as on the installed Audio page, in a jumbled file order, with controls a
    // few units apart still sharing a row; the disabled combo is skipped.
    let layout = Layout::parse(
        br##""Resource/OptionsSubMouse.res" {
            "Autoaim" { "ControlName" "ComboBox" "fieldName" "Autoaim" "xpos" "254" "ypos" "116"
                "wide" "160" "tall" "24" }
            "MouseFilter" { "ControlName" "CCvarToggleCheckButton" "fieldName" "MouseFilter"
                "xpos" "254" "ypos" "10" "wide" "150" "tall" "28" }
            "SensitivityLabel" { "ControlName" "TextEntry" "fieldName" "SensitivityLabel"
                "xpos" "400" "ypos" "40" "wide" "50" "tall" "24" }
            "ReverseMouse" { "ControlName" "CCvarNegateCheckButton" "fieldName" "ReverseMouse"
                "xpos" "28" "ypos" "9" "wide" "150" "tall" "28" }
            "Slider" { "ControlName" "CCvarSlider" "fieldName" "Slider" "xpos" "200" "ypos" "40"
                "wide" "200" "tall" "40" }
            "MouseRaw" { "ControlName" "CCvarToggleCheckButton" "fieldName" "MouseRaw"
                "xpos" "28" "ypos" "120" "wide" "150" "tall" "28" }
            "AutoaimScale" { "ControlName" "ComboBox" "fieldName" "Autoaim" "xpos" "28"
                "ypos" "200" "wide" "160" "tall" "24" "enabled" "0" }
        }"##,
    )
    .unwrap();
    let mut res = (*resources()).clone();
    res.layouts.insert("optionssubmouse".into(), layout);
    let draft = Draft::new(Config::default());
    let list = layout_widgets(&res, Panel::Mouse, page_rect(), &draft, &draft.config);
    let names: Vec<&str> = control_order(&list, &draft.config)
        .into_iter()
        .map(|t| match t {
            Target::Control(i) => list[i].control.name.as_str(),
            _ => unreachable!(),
        })
        .collect();
    let mut expected = vec![
        "ReverseMouse",
        "MouseFilter",
        "Slider",
        "SensitivityLabel",
        "Autoaim",
    ];
    if !cfg!(target_os = "macos") {
        expected.insert(4, "MouseRaw");
    }
    assert_eq!(names, expected);
}

#[test]
fn tab_walks_the_tab_strip_page_and_footer_and_keys_drive_each_control() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    assert_eq!(
        h.dialog.focus,
        Some(Target::Tab(2)),
        "a click focuses the tab"
    );
    h.key(Key::Tab);
    assert_eq!(
        focused_name(&h, Panel::Mouse).as_deref(),
        Some("ReverseMouse")
    );
    let invert = h.dialog.draft().mouse.invert;
    h.key(Key::Space);
    assert_ne!(
        h.dialog.draft().mouse.invert,
        invert,
        "Space toggles a check box"
    );
    h.key(Key::Enter);
    assert_eq!(
        h.dialog.draft().mouse.invert,
        invert,
        "so does Enter, not OK"
    );

    h.key(Key::Tab);
    assert_eq!(focused_name(&h, Panel::Mouse).as_deref(), Some("Slider"));
    let sensitivity = h.dialog.draft().mouse.sensitivity;
    h.key(Key::Right);
    assert!(
        h.dialog.draft().mouse.sensitivity > sensitivity,
        "Right raises a slider"
    );
    h.key(Key::PageDown);
    assert!(
        h.dialog.draft().mouse.sensitivity < sensitivity,
        "Page Down lowers a tick"
    );

    h.key(Key::Tab);
    assert_eq!(
        focused_name(&h, Panel::Mouse).as_deref(),
        Some("SensitivityLabel")
    );
    assert!(h.dialog.editing.is_some(), "a focused text entry edits");
    h.frame(Input {
        keys: [Key::Backspace].into(),
        ..Default::default()
    });
    h.key(Key::Tab);
    assert!(h.dialog.editing.is_none(), "Tab commits the entry");
    assert_eq!(focused_name(&h, Panel::Mouse).as_deref(), Some("Autoaim"));
    let aim = h.dialog.draft().mouse.aim_assist;
    h.key(Key::Down);
    assert_ne!(
        h.dialog.draft().mouse.aim_assist,
        aim,
        "Down selects the next item"
    );
    assert!(h.dialog.combo.is_none(), "without opening the list");
    h.frame(Input {
        keys: [Key::Down].into(),
        alt: true,
        ..Default::default()
    });
    assert!(h.dialog.combo.is_some(), "Alt+Down opens it");
    h.key(Key::Enter);
    assert!(h.dialog.combo.is_none());

    h.key(Key::Tab);
    assert_eq!(
        focused_name(&h, Panel::Mouse).as_deref(),
        Some("MouseFilter")
    );
    h.key(Key::Tab);
    assert_eq!(h.dialog.focus, Some(Target::Ok));
    h.key(Key::Tab);
    assert_eq!(h.dialog.focus, Some(Target::Cancel));
    h.key(Key::Tab);
    assert_eq!(
        h.dialog.focus,
        Some(Target::Apply),
        "Apply is a stop while enabled"
    );
    h.key(Key::Tab);
    assert_eq!(
        h.dialog.focus,
        Some(Target::Tab(2)),
        "wraps to the tab strip"
    );
    shift_tab(&mut h);
    assert_eq!(h.dialog.focus, Some(Target::Apply));
    assert!(matches!(h.key(Key::Enter), OptionsOutcome::Apply(_)));
    assert!(!h.dialog.changed());
    h.frame(Input::default());
    assert_eq!(h.dialog.focus, None, "Apply left the order once disabled");
}

#[test]
fn arrows_switch_tabs_and_move_a_radio_group() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    h.key(Key::Left);
    assert_eq!(h.dialog.tab(), Tab::Keyboard);
    assert_eq!(h.dialog.focus, Some(Target::Tab(1)));
    h.key(Key::Left);
    assert_eq!(h.dialog.tab(), Tab::Gameplay);
    h.key(Key::Left);
    assert_eq!(h.dialog.tab(), Tab::Gameplay, "no wrap at the first tab");
    h.dialog.draft.config.gameplay.skill = 2;
    h.key(Key::Tab);
    assert_eq!(
        focused_name(&h, Panel::Gameplay).as_deref(),
        Some("Skill2Radio"),
        "the group's stop is its selected button"
    );
    h.key(Key::Down);
    assert_eq!(h.dialog.draft().gameplay.skill, 3);
    assert_eq!(
        focused_name(&h, Panel::Gameplay).as_deref(),
        Some("Skill3Radio")
    );
    h.key(Key::Right);
    assert_eq!(h.dialog.draft().gameplay.skill, 1, "the group wraps");
    h.key(Key::Up);
    assert_eq!(h.dialog.draft().gameplay.skill, 3);
    h.key(Key::Tab);
    assert_eq!(
        h.dialog.focus,
        Some(Target::Ok),
        "one stop for the whole group"
    );
}

#[test]
fn enter_acts_only_on_the_focused_control_and_the_key_list_takes_focus_back() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    assert_eq!(
        h.key(Key::Enter),
        OptionsOutcome::None,
        "no default button on a tab"
    );
    assert_eq!(h.dialog.tab(), Tab::Mouse);
    h.tab(Tab::Keyboard);
    let list_target = h.dialog.key_list_target();
    h.key(Key::Down);
    assert_eq!(
        h.dialog.keys.selected, None,
        "arrows stay with the focused tab"
    );
    h.click(page(30., 10. + KEY_HEADER + MENU_ROW * 1.5));
    assert_eq!(h.dialog.focus, list_target, "a row click focuses the list");
    h.key(Key::Enter);
    assert_eq!(h.dialog.keys.capture, Some(1));
    h.frame(Input {
        binding_keys: vec!["KeyQ".into()],
        ..Default::default()
    });
    assert_eq!(
        h.dialog.focus, list_target,
        "capture hands the keyboard back to the list"
    );
    h.key(Key::Down);
    assert_eq!(h.dialog.keys.selected, Some(2));
    // Without focus the selected row takes the dim out-of-focus selection.
    let fills = |h: &Harness| -> usize {
        drawn(h)
            .iter()
            .filter(|q| q.color == h.theme.selection_bg)
            .count()
    };
    assert_eq!(fills(&h), 1);
    h.key(Key::Tab);
    assert_eq!(fills(&h), 0);
}

#[test]
fn the_report_names_the_tab_focus_and_pending_changes() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    let report = h.dialog.report();
    assert_eq!(
        (
            report.tab.as_str(),
            report.focus.as_deref(),
            report.focus_visible
        ),
        ("Mouse", Some("tab"), false),
        "a click focuses without showing it"
    );
    h.key(Key::Tab);
    h.key(Key::Space);
    let report = h.dialog.report();
    assert_eq!(report.focus.as_deref(), Some("ReverseMouse"));
    assert!(report.focus_visible && report.changed && report.sub.is_none());
}

#[test]
fn a_held_space_presses_the_button_and_the_release_activates_it() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    shift_tab(&mut h);
    assert_eq!(
        h.dialog.focus,
        Some(Target::Cancel),
        "Apply is disabled: not a stop"
    );
    let down = Input {
        keys: [Key::Space].into(),
        held_keys: [Key::Space].into(),
        ..Default::default()
    };
    assert_eq!(h.frame(down), OptionsOutcome::None);
    assert_eq!(
        h.dialog.key_press.pressed(),
        Some(Target::Cancel),
        "drawn depressed"
    );
    let hold = Input {
        held_keys: [Key::Space].into(),
        ..Default::default()
    };
    assert_eq!(h.frame(hold), OptionsOutcome::None);
    assert_eq!(
        h.frame(Input::default()),
        OptionsOutcome::Close(None),
        "the release activates Cancel"
    );
}

#[test]
fn credits_close_takes_focus_and_enter_closes_on_release() {
    let mut h = Harness::new();
    h.dialog.command(Command::AudioCredits);
    assert_eq!(
        h.dialog.focus,
        Some(Target::Cancel),
        "Close holds the focus"
    );
    assert!(!h.dialog.focus_visible, "drawn only after Tab");
    h.key(Key::Tab);
    assert_eq!(h.dialog.focus, Some(Target::Cancel));
    assert!(h.dialog.focus_visible, "Tab shows the focus");
    assert!(
        drawn(&h).iter().any(|q| q.color == h.theme.focus_border),
        "the focus box is drawn"
    );
    let down = Input {
        keys: [Key::Enter].into(),
        held_keys: [Key::Enter].into(),
        ..Default::default()
    };
    h.frame(down);
    assert_eq!(
        h.dialog.sub,
        Some(Sub::AudioCredits),
        "a press does not close"
    );
    assert_eq!(h.dialog.key_press.pressed(), Some(Target::Cancel));
    h.frame(Input::default());
    assert_eq!(h.dialog.sub, None, "the release closes the credits only");
}

#[test]
fn a_sub_dialog_takes_focus_and_gives_it_back() {
    let mut h = Harness::new();
    h.tab(Tab::Video);
    while focused_name(&h, Panel::Video).as_deref() != Some("AdvancedButton") {
        h.key(Key::Tab);
    }
    h.key(Key::Space);
    assert_eq!(h.dialog.sub, Some(Sub::VideoAdvanced));
    assert_eq!(h.dialog.focus, None, "the sub-dialog starts unfocused");
    h.key(Key::Tab);
    assert!(matches!(h.dialog.focus, Some(Target::Control(_))));
    assert!(h.dialog.escape());
    assert_eq!(h.dialog.sub, None);
    assert_eq!(
        focused_name(&h, Panel::Video).as_deref(),
        Some("AdvancedButton")
    );
    assert_eq!(
        h.key(Key::Enter),
        OptionsOutcome::None,
        "Enter presses the button"
    );
    assert_eq!(h.dialog.sub, Some(Sub::VideoAdvanced));
}

#[test]
fn geometry_follows_property_dialog_rules() {
    assert_eq!(page_rect(), Rect::new(8., 58., 496., 308.));
    let [ok, cancel, apply] = button_rects();
    assert_eq!((ok.x, cancel.x, apply.x, ok.y), (264., 344., 424., 370.));
}

#[test]
fn gamepad_page_edits_its_own_settings() {
    let mut h = Harness::new();
    h.tab(Tab::Gamepad);
    assert!(h.dialog.take_log().is_empty(), "no longer a placeholder");
    let enabled = |h: &Harness, name: &str| {
        layout_widgets(
            &h.dialog.res,
            Panel::Gamepad,
            page_rect(),
            &h.dialog.draft,
            &Config::default(),
        )
        .iter()
        .find(|w| w.control.name == name)
        .unwrap()
        .enabled
    };
    assert!(!enabled(&h, "AutoaimScale"), "aim assist is off");
    assert!(
        !enabled(&h, "AutoaimScaleLabel"),
        "the label follows its associate"
    );
    h.click(page(38. + 80., 116. + 12.));
    h.click(page(38. + 80., 140. + MENU_ROW * 2.5));
    assert_eq!(h.dialog.draft().gamepad.aim_assist, AimAssist::Enhanced);
    assert_eq!(
        h.dialog.draft().mouse.aim_assist,
        AimAssist::Off,
        "not the mouse setting"
    );
    assert!(enabled(&h, "AutoaimScale") && enabled(&h, "AutoaimScaleLabel"));
    h.click(page(38. + 1., 43. + SLIDER_TRACK_Y));
    assert_eq!(h.dialog.draft().gamepad.sensitivity_scale, 0.5);
    h.click(page(38. + 80., 262. + 12.));
    h.click(page(38. + 80., 286. + MENU_ROW * 1.5));
    assert_eq!(
        h.dialog.draft().gamepad.weapon_selection,
        crate::config::WeaponSelectionStyle::Carousel
    );
    assert!(!enabled(&h, "SteamInput"));
    h.click(page(240. + 100., 262. + 12.));
    assert_eq!(
        h.dialog.take_log(),
        ["Open Steam Input Configuration: not implemented in HL2-RS yet."]
    );
}

#[test]
fn ok_cancel_apply_follow_draft_semantics() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    assert_eq!(
        h.button(0),
        OptionsOutcome::Close(None),
        "OK without changes"
    );
    h.dialog.open(Config::default());
    h.click(page(30., 20.));
    assert!(h.dialog.draft().mouse.invert);
    assert!(h.dialog.changed());
    let OptionsOutcome::Apply(applied) = h.button(2) else {
        panic!("Apply publishes the draft");
    };
    assert!(applied.mouse.invert);
    assert!(!h.dialog.changed());
    assert_eq!(
        h.button(2),
        OptionsOutcome::None,
        "Apply disabled without changes"
    );
    h.click(page(30., 20.));
    assert_eq!(
        h.button(1),
        OptionsOutcome::Close(None),
        "Cancel drops the draft"
    );
    h.dialog.open(applied.clone());
    h.click(page(30., 20.));
    let OptionsOutcome::Close(Some(closed)) = h.button(0) else {
        panic!("OK returns the changed draft");
    };
    assert!(!closed.mouse.invert);
}

#[test]
fn slider_drag_and_text_entry_edit_values() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    // Track spans x 204..388 of the page; the middle is half of 0.1..6.
    let start = page(296., 50.);
    h.frame(Input {
        pointer: Some(start),
        click: true,
        held: true,
        ..Default::default()
    });
    assert!((h.dialog.draft().mouse.sensitivity - 3.05).abs() < 0.01);
    h.frame(Input {
        pointer: Some(page(500., 90.)),
        held: true,
        ..Default::default()
    });
    assert_eq!(
        h.dialog.draft().mouse.sensitivity,
        6.,
        "drag continues off the track"
    );
    h.frame(Input::default());
    h.click(page(410., 50.));
    for _ in 0..4 {
        h.key(Key::Backspace);
    }
    h.frame(Input {
        text: "1x2.5".into(),
        ..Default::default()
    });
    h.key(Key::Enter);
    assert_eq!(
        h.dialog.draft().mouse.sensitivity,
        12.5,
        "typed values may exceed the slider"
    );
    h.click(page(410., 50.));
    h.frame(Input {
        text: "-".into(),
        keys: [Key::Backspace, Key::Backspace].into(),
        ..Default::default()
    });
    h.click(page(30., 20.));
    assert!(
        h.dialog.draft().mouse.sensitivity > 0.09,
        "invalid entries are clamped or kept"
    );
}

#[test]
fn combos_select_items_and_aspect_resets_resolution() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    h.click(page(40., 180.));
    assert!(h.dialog.combo.is_some());
    // Rows below the combo (y 196): Off, Basic, Enhanced.
    h.click(page(40., 196. + MENU_ROW * 1.5));
    assert!(h.dialog.combo.is_none());
    assert_eq!(h.dialog.draft().mouse.aim_assist, AimAssist::Basic);
    h.tab(Tab::Video);
    h.click(page(40., 125.));
    // Row 0 is now Fit Display (Flexible)
    h.click(page(40., 141. + MENU_ROW * 0.5));
    assert_eq!(h.dialog.draft.aspect, AspectRatio::Flexible);
    assert!(h.dialog.draft().video.flexible_aspect);
    // Open aspect combo again and pick row 1 (Normal 4:3)
    h.click(page(40., 125.));
    h.click(page(40., 141. + MENU_ROW * 1.5));
    assert_eq!(h.dialog.draft.aspect, AspectRatio::Normal);
    assert_eq!(
        (h.dialog.draft().video.width, h.dialog.draft().video.height),
        (1024, 768)
    );
    h.click(page(40., 270.));
    h.key(Key::Down);
    h.key(Key::Down);
    h.key(Key::Enter);
    assert_eq!(
        h.dialog.draft().video.hdr,
        HdrMode::Full,
        "HDR combo is shown on every platform"
    );
    h.click(page(40., 125.));
    h.click(page(450., 300.));
    assert!(h.dialog.combo.is_none(), "clicking outside closes the list");
    assert_eq!(h.dialog.draft.aspect, AspectRatio::Normal);
}

/// Quads of the dialog drawn on its own (no fonts, so no glyph quads).
fn drawn(h: &Harness) -> Vec<source_ui::canvas::Quad> {
    let painter = Painter {
        canvas: &h.canvas,
        fonts: &h.fonts,
        theme: &h.theme,
        origin: Vec2::ZERO,
        scale: 1.,
    };
    h.dialog.draw(&painter);
    h.canvas.drain()
}

/// Quads of the dialog drawn at a specific timestamp.
fn drawn_at(h: &Harness, time: f64) -> Vec<source_ui::canvas::Quad> {
    let painter = Painter {
        canvas: &h.canvas,
        fonts: &h.fonts,
        theme: &h.theme,
        origin: Vec2::ZERO,
        scale: 1.,
    };
    h.dialog
        .draw_with_occluders(&painter, &[], Some(time), None);
    h.canvas.drain()
}

#[test]
fn key_list_separates_groups_and_capture_highlights_only_the_key_cell() {
    let mut h = Harness::new();
    h.tab(Tab::Keyboard);
    // The clicked tab's dashed focus box shares the divider's color and height.
    h.dialog.focus = None;
    let theme = h.theme.clone();
    // Group separator under the section header row (row 0): one pixel high.
    let quads = drawn(&h);
    let dividers: Vec<_> = quads
        .iter()
        .filter(|q| q.color == theme.list_divider && q.destination.h == 1.)
        .collect();
    assert_eq!(dividers.len(), 1, "one section, one separator");
    let list = page(8., 10.);
    assert_eq!(
        dividers[0].destination.y,
        list.y + KEY_HEADER + MENU_ROW - 2.
    );

    // Selected row: the whole row is orange.
    h.click(page(30., 10. + KEY_HEADER + MENU_ROW * 1.5));
    let orange = |quads: &[source_ui::canvas::Quad]| {
        quads
            .iter()
            .filter(|q| q.color == theme.selection_bg)
            .map(|q| q.destination)
            .collect::<Vec<_>>()
    };
    let row = orange(&drawn(&h));
    assert_eq!(row.len(), 1);
    // Capturing: the key cell is orange and the action cell keeps a dimmer selection.
    h.click(page(280., 280.));
    assert_eq!(h.dialog.keys.capture, Some(1));
    let quads = drawn(&h);
    let dim: Vec<_> = quads
        .iter()
        .filter(|q| {
            q.color
                == source_ui::canvas::Color {
                    a: theme.selection_bg_dim.a * CAPTURE_ACTION_FADE,
                    ..theme.selection_bg_dim
                }
        })
        .map(|q| q.destination)
        .collect();
    let cell = orange(&quads);
    assert_eq!(dim.len(), 1);
    assert_eq!((dim[0].x, dim[0].y), (row[0].x, row[0].y));
    assert_eq!(
        dim[0].x + dim[0].w,
        cell[0].x,
        "action cell meets the key cell"
    );
    assert_eq!(cell.len(), 1);
    assert_eq!(cell[0].y, row[0].y);
    assert!(cell[0].x > row[0].x && cell[0].w < row[0].w);
    assert_eq!(
        cell[0].x + cell[0].w,
        row[0].x + row[0].w,
        "ends at the row edge"
    );
}

#[test]
fn external_display_mode_changes_reach_an_open_dialog() {
    use crate::config::DisplayMode;
    let mut h = Harness::new();
    h.dialog.open(Config {
        video: crate::config::VideoSettings {
            display_mode: DisplayMode::Windowed,
            ..Default::default()
        },
        ..Config::default()
    });
    h.dialog.sync_display_mode(DisplayMode::Fullscreen);
    assert_eq!(h.dialog.draft().video.display_mode, DisplayMode::Fullscreen);
    assert!(!h.dialog.changed(), "nothing left to apply");
    // A mode the user already chose in the draft is kept.
    h.dialog.draft.config.video.display_mode = DisplayMode::Windowed;
    h.dialog.sync_display_mode(DisplayMode::Fullscreen);
    assert_eq!(h.dialog.draft().video.display_mode, DisplayMode::Windowed);
    assert!(h.dialog.changed());
}

#[test]
fn key_list_scroll_bar_scrolls_while_held_and_blocks_row_clicks() {
    let mut h = Harness::new();
    let mut res = (*resources()).clone();
    res.key_rows = (0..40)
        .map(|i| KeyRow::Action {
            command: format!("+cmd{i}"),
            label: "#Valve_Jump".into(),
        })
        .collect();
    h.dialog.set_resources(Arc::new(res));
    h.dialog.open(Config::default());
    h.tab(Tab::Keyboard);
    let list = Rect::new(8., 10., 480., 258.);
    let bar = key_bar_rect(list);
    let layout = widgets::ScrollBarLayout::new(bar, 0, key_rows_visible(list), 40);
    let at = |p: Vec2| page(p.x, p.y);
    let nob = vec2(layout.nob.x + 5., layout.nob.y + 5.);
    // Press on the nob and drag: the list scrolls before the release.
    h.frame(Input {
        pointer: Some(at(nob)),
        click: true,
        held: true,
        ..Default::default()
    });
    h.frame(Input {
        pointer: Some(at(nob + vec2(0., 60.))),
        held: true,
        ..Default::default()
    });
    let dragged = h.dialog.keys.first;
    assert!(dragged > 0, "live drag");
    h.frame(Input {
        pointer: Some(at(nob + vec2(0., 60.))),
        released: true,
        ..Default::default()
    });
    assert_eq!(h.dialog.keys.first, dragged);
    assert_eq!(h.dialog.keys.selected, None, "the bar never selects a row");
    // The down arrow steps on press.
    let down = vec2(layout.down.x + 5., layout.down.y + 5.);
    h.click(at(down));
    assert_eq!(h.dialog.keys.first, dragged + 1);
}

#[test]
fn double_click_on_a_key_row_starts_capture() {
    let mut h = Harness::new();
    h.tab(Tab::Keyboard);
    let row = page(30., 10. + KEY_HEADER + MENU_ROW * 1.5);
    let click_at = |h: &mut Harness, time: f64| {
        for (click, held, released) in [(true, true, false), (false, false, true)] {
            h.frame(Input {
                pointer: Some(row),
                click,
                held,
                released,
                time: Some(time),
                ..Default::default()
            });
        }
    };
    click_at(&mut h, 1.);
    assert_eq!(
        (h.dialog.keys.selected, h.dialog.keys.capture),
        (Some(1), None)
    );
    click_at(&mut h, 1.8);
    assert_eq!(
        h.dialog.keys.capture, None,
        "slow second click only selects"
    );
    click_at(&mut h, 2.1);
    assert_eq!(h.dialog.keys.capture, Some(1), "double click edits the key");
    h.frame(Input {
        binding_keys: vec!["MOUSE2".into()],
        ..Default::default()
    });
    assert_eq!(h.dialog.draft().keys.get("+forward").unwrap(), "MOUSE2");
    // Without frame times (scripted input) repeated clicks never start capture.
    h.click(row);
    h.click(row);
    assert_eq!(h.dialog.keys.capture, None);
}

#[test]
fn key_capture_clear_and_defaults() {
    let mut h = Harness::new();
    h.tab(Tab::Keyboard);
    // Header 20, then the section row and +forward at row 1.
    h.click(page(30., 10. + KEY_HEADER + MENU_ROW * 1.5));
    assert_eq!(h.dialog.keys.selected, Some(1));
    h.click(page(280., 280.));
    assert_eq!(h.dialog.keys.capture, Some(1));
    h.frame(Input {
        binding_keys: vec!["MOUSE1".into()],
        ..Default::default()
    });
    assert_eq!(h.dialog.draft().keys.get("+forward").unwrap(), "MOUSE1");
    h.key(Key::Down);
    assert_eq!(h.dialog.keys.selected, Some(2));
    h.key(Key::Enter);
    assert!(h.dialog.escape(), "Escape cancels capture only");
    assert_eq!(h.dialog.draft().keys.get("+jump").unwrap(), "Space");
    h.key(Key::Enter);
    h.frame(Input {
        binding_keys: vec!["Escape".into(), "MOUSE1".into()],
        ..Default::default()
    });
    assert_eq!(h.dialog.draft().keys.get("+jump").unwrap(), "MOUSE1");
    assert_eq!(
        h.dialog.draft().keys.get("+forward").unwrap(),
        keys::UNBOUND,
        "a key runs one command"
    );
    h.click(page(390., 280.));
    assert_eq!(h.dialog.draft().keys.get("+jump").unwrap(), keys::UNBOUND);
    h.click(page(20., 280.));
    assert!(h.dialog.confirm.is_some());
    h.key(Key::Enter);
    assert_eq!(h.dialog.draft().keys.get("+forward").unwrap(), "KeyW");
    assert_eq!(h.dialog.draft().keys.get("+jump").unwrap(), "Space");
    assert_eq!(
        h.dialog.draft().keys.get("loadout").unwrap(),
        "F3",
        "other commands kept"
    );
}

impl Harness {
    /// Press at `from`, move with the button held to `to`, release there.
    fn drag(&mut self, from: Vec2, to: Vec2) {
        for (pointer, click, held, released) in [
            (from, true, true, false),
            (to, false, true, false),
            (to, false, false, true),
        ] {
            self.frame(Input {
                pointer: Some(pointer),
                click,
                held,
                released,
                ..Default::default()
            });
        }
    }
}

#[test]
fn the_caption_drags_the_dialog_and_its_controls_follow() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    h.dialog.draft.config.mouse.invert = true;
    // Caption: the top 23 units. Dragging it 100 left and 50 down moves OK with it.
    h.drag(DIALOG + vec2(100., 10.), DIALOG + vec2(0., 60.));
    let moved = DIALOG + vec2(-100., 50.);
    let [ok, _, _] = button_rects();
    let old_ok = DIALOG + vec2(ok.x + 10., ok.y + 10.);
    assert_eq!(
        h.click(old_ok),
        OptionsOutcome::None,
        "nothing at the old spot"
    );
    let outcome = h.click(moved + vec2(ok.x + 10., ok.y + 10.));
    assert!(
        matches!(outcome, OptionsOutcome::Close(Some(_))),
        "{outcome:?}"
    );
    // The placement stays for the next opening.
    h.dialog.open(Config::default());
    assert_eq!(
        h.dialog
            .placement
            .place(&main_frame(vec2(1280., 720.)))
            .origin,
        moved
    );
}

#[test]
fn escape_undoes_a_drag_and_sub_dialogs_move_alone() {
    let mut h = Harness::new();
    h.frame(Input {
        pointer: Some(DIALOG + vec2(100., 10.)),
        click: true,
        held: true,
        ..Default::default()
    });
    h.frame(Input {
        pointer: Some(DIALOG + vec2(300., 200.)),
        held: true,
        ..Default::default()
    });
    assert!(h.dialog.escape(), "Escape cancels the drag, not the dialog");
    let canvas = vec2(1280., 720.);
    assert_eq!(h.dialog.placement.place(&main_frame(canvas)).origin, DIALOG);
    // The brightness dialog (290 x 396, at 495, 162) moves without its parent.
    h.tab(Tab::Video);
    h.click(page(300., 125.));
    let gamma = vec2(495., 162.);
    h.drag(gamma + vec2(50., 10.), gamma + vec2(-150., 10.));
    let size = h.dialog.sub_frame_size(Sub::Gamma, |_| 0.);
    let placed = h
        .dialog
        .sub_placement
        .place(&sub_frame(canvas, size))
        .origin;
    assert_eq!(placed, gamma + vec2(-200., 0.));
    assert_eq!(h.dialog.placement.place(&main_frame(canvas)).origin, DIALOG);
    h.dialog.escape();
    h.click(page(300., 125.));
    let placed = h
        .dialog
        .sub_placement
        .place(&sub_frame(canvas, size))
        .origin;
    assert_eq!(placed, gamma, "a sub-dialog opens centered again");
}

#[test]
fn a_caption_press_with_a_list_open_only_closes_the_list() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    h.click(page(40., 180.));
    assert!(h.dialog.combo.is_some());
    h.drag(DIALOG + vec2(100., 10.), DIALOG + vec2(0., 60.));
    assert!(h.dialog.combo.is_none());
    assert_eq!(
        h.dialog
            .placement
            .place(&main_frame(vec2(1280., 720.)))
            .origin,
        DIALOG,
        "the press closed the list and did not drag"
    );
}

#[test]
fn video_advanced_opens_and_gamma_dialog_restores_on_cancel() {
    let mut h = Harness::new();
    h.tab(Tab::Video);
    h.click(page(300., 200.));
    assert_eq!(h.dialog.sub, Some(Sub::VideoAdvanced));
    assert!(h.dialog.take_log().is_empty(), "no longer a placeholder");
    assert!(h.dialog.escape(), "Escape closes the sub-dialog first");
    assert_eq!(h.dialog.sub, None);
    h.click(page(300., 125.));
    assert_eq!(h.dialog.sub, Some(Sub::Gamma));
    // Gamma frame 290x396 centered: origin (495, 162); slider track at y 122.
    let gamma = vec2(495., 162.);
    h.click(gamma + vec2(17. + 4., 122.));
    assert_eq!(h.dialog.draft().video.gamma, 1.6);
    assert_eq!(h.dialog.preview_gamma(), Some(1.6), "previewed live");
    h.click(gamma + vec2(200., 360.));
    assert!(h.dialog.sub.is_none());
    assert_eq!(h.dialog.preview_gamma(), None, "no preview once closed");
    assert_eq!(
        h.dialog.draft().video.gamma,
        2.2,
        "Cancel restores the gamma"
    );
    h.click(page(300., 125.));
    h.click(gamma + vec2(17. + 4., 122.));
    h.click(gamma + vec2(110., 360.));
    assert_eq!(
        h.dialog.draft().video.gamma,
        1.6,
        "OK keeps it in the draft"
    );
    // Unlocked in every display mode (retail: full screen only).
    h.dialog.open(Config {
        video: crate::config::VideoSettings {
            display_mode: crate::config::DisplayMode::Windowed,
            ..Default::default()
        },
        ..Config::default()
    });
    h.tab(Tab::Video);
    h.click(page(300., 125.));
    assert!(h.dialog.sub == Some(Sub::Gamma), "opens in windowed mode");
}

#[test]
fn keyboard_advanced_cancel_restores_and_ok_applies_only_its_settings() {
    let mut h = Harness::new();
    h.tab(Tab::Keyboard);
    h.dialog.draft.config.mouse.invert = true;
    h.click(page(148. + 50., 276. + 12.));
    assert_eq!(h.dialog.sub, Some(Sub::KeyboardAdvanced));
    // No frame record: the controls span x 20..264, y 38..124, so the frame is 284 x 144
    // (left margin repeated), centered at (498, 288).
    let size = h.dialog.sub_frame_size(Sub::KeyboardAdvanced, |_| 0.);
    assert_eq!(size, vec2(284., 144.));
    let origin = vec2(498., 288.);
    h.click(origin + vec2(30., 50.));
    assert!(h.dialog.draft().keyboard.fast_switch);
    h.click(origin + vec2(182. + 40., 112.));
    assert_eq!(h.dialog.sub, None);
    assert!(!h.dialog.draft().keyboard.fast_switch, "Cancel restores");
    assert!(h.dialog.draft().mouse.invert, "earlier page edits stay");

    h.click(page(148. + 50., 276. + 12.));
    h.click(origin + vec2(30., 78.));
    let outcome = h.click(origin + vec2(95. + 40., 112.));
    let OptionsOutcome::Apply(applied) = outcome else {
        panic!("advanced OK applies at once: {outcome:?}");
    };
    assert!(!applied.keyboard.developer_console);
    assert!(
        !applied.mouse.invert,
        "other unapplied edits are not applied"
    );
    assert!(h.dialog.draft().mouse.invert, "and stay in the draft");
    assert!(h.dialog.changed(), "so Apply stays enabled for them");
}

#[test]
fn footer_moves_below_content_it_overlaps_and_the_frame_grows() {
    // The installed macOS rows: the caption at 310 and OK/Cancel at 316 overlap the
    // Classic Effects combo (316..340).
    let overlapping = Layout::parse(
        br##""resource/OptionsSubVideoAdvancedDlg.res" {
            "ClassicEffectsLabel" { "ControlName" "Label" "fieldName" "ClassicEffectsLabel"
                "xpos" "22" "ypos" "292" "wide" "138" "tall" "24" "associate" "ClassicEffects" }
            "ClassicEffects" { "ControlName" "ComboBox" "fieldName" "ClassicEffects" "xpos" "22"
                "ypos" "316" "wide" "132" "tall" "24" }
            "Label11" { "ControlName" "Label" "fieldName" "Label11" "xpos" "18" "ypos" "310"
                "wide" "280" "tall" "24" "labelText" "#GameUI_RecommendedSettings" }
            "Button1" { "ControlName" "Button" "fieldName" "Button1" "xpos" "290" "ypos" "316"
                "wide" "80" "tall" "24" "Command" "OK" }
            "Button2" { "ControlName" "Button" "fieldName" "Button2" "xpos" "382" "ypos" "316"
                "wide" "80" "tall" "24" "Command" "Close" }
        }"##,
    )
    .unwrap();
    let controls: Vec<&Control> = overlapping.controls.iter().collect();
    let (shift, footer) = footer_reflow(Panel::VideoAdvanced, &controls, (482., 358.));
    assert_eq!(shift, 340. + FOOTER_GAP - 310.);
    assert_eq!(
        footer,
        [false, false, true, true, true],
        "caption and buttons"
    );
    // The Windows rows (caption 442, buttons 468) already clear the content.
    let mut clear = overlapping.clone();
    for (name, y) in [("Label11", 442.), ("Button1", 468.), ("Button2", 468.)] {
        clear.get_mut(name).unwrap().y = source_ui::layout::Coord::Start(y);
    }
    let controls: Vec<&Control> = clear.controls.iter().collect();
    assert_eq!(
        footer_reflow(Panel::VideoAdvanced, &controls, (482., 510.)).0,
        0.
    );
}

#[test]
fn the_fov_slider_ring_stays_clear_of_the_value_label_text() {
    let layout = Layout::parse(VIDEO_ADVANCED.as_bytes()).unwrap();
    let rect_of = |name: &str| {
        let c = layout.controls.iter().find(|c| c.name == name).unwrap();
        Rect::new(c.x.resolve(482.), c.y.resolve(358.), c.wide, c.tall)
    };
    let label = rect_of("FOVValueLabel");
    let text_bottom = label.y + (label.h - widgets::FONT_TALL) * 0.5 + widgets::FONT_TALL;
    let ring = widgets::slider_content(rect_of("FovSlider")).pad(widgets::FOCUS_PAD);
    assert!(ring.y >= text_bottom, "{ring:?} vs {text_bottom}");
}

#[test]
fn video_advanced_combo_and_fov_slider_apply_on_ok() {
    let mut h = Harness::new();
    h.tab(Tab::Video);
    h.click(page(300., 200.));
    assert_eq!(h.dialog.sub, Some(Sub::VideoAdvanced));
    // Frame record 482 x 358, centered at (399, 181).
    let origin = vec2(399., 181.);
    h.click(origin + vec2(330. + 66., 196. + 12.));
    assert!(
        h.dialog.combo.is_some(),
        "the combo opens inside the sub-dialog"
    );
    h.click(origin + vec2(330. + 66., 220. + MENU_ROW * 0.5));
    assert!(h.dialog.combo.is_none());
    assert!(!h.dialog.draft().video.advanced.vsync, "Disabled picked");
    // The FOV slider spans the layout's 75..110, not a page constant.
    h.click(origin + vec2(176. + 139., 252. + SLIDER_TRACK_Y));
    assert_eq!(h.dialog.draft().video.fov, 110.);
    let res = h.dialog.res.clone();
    let area = Rect::new(0., 0., 482., 358.);
    let widgets = layout_widgets(
        &res,
        Panel::VideoAdvanced,
        area,
        &h.dialog.draft,
        &Config::default(),
    );
    let label = widgets
        .iter()
        .find(|w| w.control.name == "FOVValueLabel")
        .unwrap();
    assert!(!label.interactive(), "the value label is display only");
    assert!(widgets.iter().all(|w| w.control.name != "frame_close"));
    let outcome = h.click(origin + vec2(290. + 40., 316. + 12.));
    let OptionsOutcome::Apply(applied) = outcome else {
        panic!("advanced OK applies at once: {outcome:?}");
    };
    assert!(!applied.video.advanced.vsync);
    assert_eq!(applied.video.fov, 110.);
}

#[test]
fn the_vertical_and_viewmodel_fov_sliders_are_added_and_apply_on_ok() {
    let mut res = (*resources()).clone();
    crate::gameui::resources::apply_layout_addenda(&mut res.layouts);
    let draft = Draft::new(Config::default());
    let widgets = layout_widgets(
        &res,
        Panel::VideoAdvanced,
        Rect::new(0., 0., 482., 510.),
        &draft,
        &draft.config,
    );
    let find = |name: &str| {
        widgets
            .iter()
            .find(|w| w.control.name == name)
            .unwrap_or_else(|| panic!("{name}"))
    };
    for (slider, label, value) in [
        (
            "VerticalFovSlider",
            "VerticalFovValueLabel",
            Value::VerticalFov,
        ),
        (
            "ViewmodelFovSlider",
            "ViewmodelFovValueLabel",
            Value::ViewmodelFov,
        ),
    ] {
        assert_eq!(find(slider).binding, Some(Binding::Slider(value)));
        assert!(find(slider).interactive());
        assert_eq!(find(label).binding, Some(Binding::ValueLabel(value)));
        assert!(
            !find(label).interactive(),
            "the value label is display only"
        );
    }
    // They sit under the Field of view slider and clear of the divider at 352.
    let fov = find("FovSlider").control.rect((482., 510.));
    let vertical = find("VerticalFovSlider").control.rect((482., 510.));
    assert!(vertical.1 >= fov.1 + find("FovSlider").control.tall);
    assert!(vertical.1 + find("VerticalFovSlider").control.tall <= 352.);
    // Their labels are content, not footer captions: the footer reflow must not move them.
    let controls: Vec<&Control> = res.layouts["optionssubvideoadvanceddlg"]
        .controls
        .iter()
        .collect();
    let (shift, footer) = footer_reflow(Panel::VideoAdvanced, &controls, (482., 358.));
    for (control, is_footer) in controls.iter().zip(&footer) {
        if control.name.to_ascii_lowercase().contains("fov") {
            assert!(!is_footer, "{} is not footer", control.name);
        }
    }
    assert!(
        shift > 0.,
        "the dialog grows to hold the new sliders on the short layout"
    );
    let mut from = Config::default();
    from.video.vertical_fov = 90.;
    from.video.viewmodel_fov = 70.;
    let mut to = Config::default();
    assert!(Sub::VideoAdvanced.apply_on_ok(&from, &mut to));
    assert_eq!((to.video.vertical_fov, to.video.viewmodel_fov), (90., 70.));
}

#[test]
#[ignore = "requires an owned Half-Life 2 installation"]
fn owned_install_pages_tab_in_reading_order() {
    let root = source_assets::install::discover().expect("installed game");
    let vfs = source_assets::vpk::Vfs::mount(&root).unwrap();
    let res = GameUiResources::load(&vfs);
    let draft = Draft::new(Config::default());
    let order = |panel: Panel| -> Vec<String> {
        let list = layout_widgets(&res, panel, page_rect(), &draft, &draft.config);
        control_order(&list, &draft.config)
            .into_iter()
            .filter_map(|t| match t {
                Target::Control(i) => Some(list[i].control.name.clone()),
                _ => None,
            })
            .collect()
    };
    // Left to right, then top to bottom, as the installed pages read.
    assert_eq!(
        order(Panel::Mouse),
        [
            "ReverseMouse",
            "MouseFilter",
            "Slider",
            "SensitivityLabel",
            "Autoaim"
        ]
    );
    assert_eq!(
        order(Panel::Audio),
        [
            "SFXSlider",
            "MusicSlider",
            "SpeakerSetup",
            "SoundQuality",
            "AudioSpokenLanguage",
            "CloseCaptionCheck",
            "snd_mute_losefocus",
            "ThirdPartySoundCredits"
        ]
    );
    assert_eq!(
        order(Panel::Video),
        [
            "Resolution",
            "DisplayModeCombo",
            "AspectRatio",
            "GammaButton",
            "HudAspectRatio",
            "AdvancedButton",
            "HDR",
            "ThirdPartyVideoCredits"
        ]
    );
    let gamepad = order(Panel::Gamepad);
    eprintln!("gamepad {gamepad:?}");
    let read: Vec<(f32, f32)> = {
        let list = layout_widgets(&res, Panel::Gamepad, page_rect(), &draft, &draft.config);
        gamepad
            .iter()
            .map(|n| {
                let r = list.iter().find(|w| &w.control.name == n).unwrap().rect;
                (r.y, r.x)
            })
            .collect()
    };
    assert!(
        read.windows(2)
            .all(|w| w[1].0 > w[0].0 + FOCUS_ROW_TOLERANCE
                || (w[1].0 - w[0].0).abs() <= FOCUS_ROW_TOLERANCE && w[1].1 > w[0].1),
        "Gamepad reads in rows: {gamepad:?}"
    );
}

#[test]
#[ignore = "requires an owned Half-Life 2 installation"]
fn owned_install_pages_bind_every_expected_control() {
    let root = source_assets::install::discover().expect("installed game");
    let vfs = source_assets::vpk::Vfs::mount(&root).unwrap();
    let res = Arc::new(GameUiResources::load(&vfs));
    let expected: &[(Panel, &[&str])] = &[
        (
            Panel::Gameplay,
            &["Skill1Radio", "Skill2Radio", "Skill3Radio", "MigrateButton"],
        ),
        (
            Panel::Mouse,
            &[
                "ReverseMouse",
                "MouseFilter",
                "MouseRaw",
                "MouseAccelerationCheckbox",
                "Slider",
                "SensitivityLabel",
                "MouseAccelerationSlider",
                "MouseAccelerationLabel",
                "Autoaim",
                "AutoaimScale",
            ],
        ),
        (
            Panel::Audio,
            &[
                "SFXSlider",
                "MusicSlider",
                "SpeakerSetup",
                "SoundQuality",
                "AudioSpokenLanguage",
                "CloseCaptionCheck",
                "snd_mute_losefocus",
                "ThirdPartySoundCredits",
            ],
        ),
        (
            Panel::Video,
            &[
                "Resolution",
                "AspectRatio",
                "DisplayModeCombo",
                "HudAspectRatio",
                "HDR",
                "GammaButton",
                "AdvancedButton",
                "ThirdPartyVideoCredits",
            ],
        ),
        (
            Panel::Keyboard,
            &[
                "listpanel_keybindlist",
                "Defaults",
                "KeyAdvancedButton",
                "ChangeKeyButton",
                "ClearKeyButton",
            ],
        ),
        (
            Panel::Gamma,
            &["Gamma", "GammaEntry", "OKButton", "Button1"],
        ),
        (Panel::AudioCredits, &["Button2"]),
        (Panel::VideoCredits, &["Button2"]),
        (
            Panel::KeyboardAdvanced,
            &["FastSwitchCheck", "ConsoleCheck"],
        ),
    ];
    let config = Config::default();
    let draft = Draft::new(config.clone());
    for (panel, names) in expected {
        let layout = res
            .layout(panel.layout())
            .unwrap_or_else(|| panic!("{panel:?}: {:?}", res.missing));
        for name in *names {
            assert!(layout.get(name).is_some(), "{panel:?} lacks {name}");
            assert!(
                settings::binding(*panel, name).is_some(),
                "{panel:?} {name}"
            );
        }
        for widget in layout_widgets(&res, *panel, page_rect(), &draft, &config) {
            for text in [&widget.control.label, &widget.control.left_text] {
                if text.starts_with('#') {
                    assert!(
                        res.localization.lookup(text).is_some(),
                        "{panel:?} {} label {text}",
                        widget.control.name
                    );
                }
            }
        }
    }
    let mut h = Harness::new();
    h.dialog.set_resources(res);
    h.dialog.open(config);
    for tab in Tab::ALL {
        h.dialog.select_tab(tab);
        h.frame(Input::default());
    }
    h.dialog.command(Command::GammaDialog);
    h.frame(Input::default());
    assert!(h.dialog.gamma_texture.is_some());
    // Sub-dialog OK/Cancel buttons bind through their layout `Command`, frame chrome is
    // skipped, and every interactive control lies below the title bar inside the frame
    // (label boxes only start there: the installed `Label10` box overhangs the frame by 2,
    // and the frame clips it).
    let subs: &[Sub] = &[Sub::Gamma, Sub::KeyboardAdvanced, Sub::VideoAdvanced];
    for &sub in subs {
        let panel = sub.panel();
        let size = h.dialog.sub_frame_size(sub, |_| 0.);
        let area = Rect::new(0., 0., size.x, size.y);
        let widgets = layout_widgets(&h.dialog.res, panel, area, &draft, &Config::default());
        for w in &widgets {
            let r = w.rect;
            let inside_end = !w.interactive() || (r.x + r.w <= size.x && r.y + r.h <= size.y);
            assert!(
                r.x >= 0. && r.y >= CLIENT_TOP && r.x < size.x && r.y < size.y && inside_end,
                "{panel:?} {} {r:?} outside {size:?}",
                w.control.name
            );
        }
        let overlaps = |a: Rect, b: Rect| {
            a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
        };
        let interactive: Vec<&Widget> = widgets.iter().filter(|w| w.interactive()).collect();
        for (i, a) in interactive.iter().enumerate() {
            for b in &interactive[i + 1..] {
                assert!(
                    !overlaps(a.rect, b.rect),
                    "{panel:?}: {} overlaps {}",
                    a.control.name,
                    b.control.name
                );
            }
        }
        let commands: Vec<_> = widgets
            .iter()
            .filter_map(|w| match w.binding {
                Some(Binding::Command(c @ (Command::SubOk | Command::SubCancel))) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(commands, [Command::SubOk, Command::SubCancel], "{panel:?}");
        assert!(widgets.iter().all(|w| !is_frame_chrome(w.control)));
    }
    h.dialog.command(Command::KeyAdvanced);
    h.frame(Input::default());
    assert_eq!(h.dialog.sub, Some(Sub::KeyboardAdvanced));
}

impl Harness {
    /// A press, one held frame and a release at `at`, like a real click spanning frames.
    fn slow_click(&mut self, at: Vec2) -> OptionsOutcome {
        let mut outcome = OptionsOutcome::None;
        for (click, held, released) in [
            (true, true, false),
            (false, true, false),
            (false, false, true),
        ] {
            let frame = self.frame(Input {
                pointer: Some(at),
                click,
                held,
                released,
                ..Default::default()
            });
            if !matches!(frame, OptionsOutcome::None) {
                outcome = frame;
            }
        }
        outcome
    }

    fn hover(&mut self, at: Vec2) {
        self.frame(Input {
            pointer: Some(at),
            ..Default::default()
        });
    }
}

/// Mouse page: the Autoaim list (rows from y 196: Off, Basic, Enhanced) lies over the
/// MouseFilter check box (y 216..244).
const AUTOAIM: (f32, f32) = (40., 180.);
const BASIC_OVER_FILTER: (f32, f32) = (40., 196. + MENU_ROW * 1.5);
const REVERSE_MOUSE: (f32, f32) = (30., 20.);

#[test]
fn open_dropdown_list_gates_input_to_elements_behind_it() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    let behind = page(BASIC_OVER_FILTER.0, BASIC_OVER_FILTER.1);
    let outside = page(REVERSE_MOUSE.0, REVERSE_MOUSE.1);

    // List closed: the check box under the list area takes hover, tooltip and clicks.
    h.hover(behind);
    assert!(matches!(h.dialog.armed, Some(Target::Control(_))));
    assert!(h.dialog.active_tooltip().is_some());
    h.slow_click(behind);
    assert!(
        h.dialog.draft().mouse.filter,
        "closed list: click reaches the box"
    );
    h.slow_click(behind);
    assert!(!h.dialog.draft().mouse.filter);
    let _ = h.dialog.take_sounds();

    // List open: hovering an entry over the box, or anything outside the list, arms
    // nothing, shows no tooltip and plays nothing.
    h.slow_click(page(AUTOAIM.0, AUTOAIM.1));
    assert!(h.dialog.combo.is_some());
    let _ = h.dialog.take_sounds();
    for at in [behind, outside] {
        h.hover(at);
        assert_eq!(h.dialog.armed, None);
        assert_eq!(h.dialog.pressed, None);
        assert_eq!(h.dialog.active_tooltip(), None);
        assert!(h.dialog.take_sounds().is_empty());
    }

    // Clicking the entry selects it and the click is consumed: the box behind it does
    // not toggle on the press, held or release frames.
    h.slow_click(behind);
    assert!(h.dialog.combo.is_none());
    assert_eq!(h.dialog.draft().mouse.aim_assist, AimAssist::Basic);
    assert!(
        !h.dialog.draft().mouse.filter,
        "selection click must not reach the box"
    );
    assert_eq!(h.dialog.take_sounds(), vec!["ui/buttonclickrelease.wav"]);

    // Clicking outside closes the list without activating what was clicked.
    h.slow_click(page(AUTOAIM.0, AUTOAIM.1));
    assert!(h.dialog.combo.is_some());
    h.slow_click(outside);
    assert!(h.dialog.combo.is_none(), "outside click closes the list");
    assert!(
        !h.dialog.draft().mouse.invert,
        "outside click activates nothing"
    );
    assert_eq!(h.dialog.draft().mouse.aim_assist, AimAssist::Basic);

    // Clicking outside with the list closed works as usual.
    h.slow_click(outside);
    assert!(h.dialog.draft().mouse.invert);
}

#[test]
fn dropdown_open_state_toggles_on_header_and_closes_in_the_release_phase() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    let header = page(AUTOAIM.0, AUTOAIM.1);
    let initial = h.dialog.draft().mouse.aim_assist;
    let press = |h: &mut Harness, at: Vec2, click: bool| {
        h.frame(Input {
            pointer: Some(at),
            click,
            held: true,
            ..Default::default()
        });
    };
    let release = |h: &mut Harness, at: Vec2| {
        h.frame(Input {
            pointer: Some(at),
            released: true,
            ..Default::default()
        });
    };

    // Closed -> open on a click, opening on release over the header.
    press(&mut h, header, true);
    assert!(h.dialog.combo.is_none(), "opens on release, not press");
    release(&mut h, header);
    assert!(h.dialog.combo.is_some());

    // Open -> closed on a header click: still open on press and hold, closed on release,
    // and it stays closed (one full press-release never closes and reopens).
    press(&mut h, header, true);
    press(&mut h, header, false);
    assert!(h.dialog.combo.is_some(), "header press does not close yet");
    release(&mut h, header);
    assert!(h.dialog.combo.is_none(), "header release closes the list");
    h.hover(header);
    assert!(
        h.dialog.combo.is_none(),
        "the same click does not reopen it"
    );
    assert_eq!(
        h.dialog.draft().mouse.aim_assist,
        initial,
        "no selection change"
    );
    // A quick two-frame header click toggles the same way.
    h.click(header);
    assert!(h.dialog.combo.is_some());
    h.click(header);
    assert!(h.dialog.combo.is_none());
    assert_eq!(h.dialog.draft().mouse.aim_assist, initial);

    // Open -> closed on entry selection, in the release phase.
    h.slow_click(header);
    let entry = page(BASIC_OVER_FILTER.0, BASIC_OVER_FILTER.1);
    press(&mut h, entry, true);
    assert!(h.dialog.combo.is_some(), "entry press does not select yet");
    release(&mut h, entry);
    assert!(h.dialog.combo.is_none());
    assert_eq!(h.dialog.draft().mouse.aim_assist, AimAssist::Basic);

    // Open -> closed on an outside click, in the release phase.
    h.slow_click(header);
    let outside = page(REVERSE_MOUSE.0, REVERSE_MOUSE.1);
    press(&mut h, outside, true);
    assert!(h.dialog.combo.is_some(), "outside press does not close yet");
    release(&mut h, outside);
    assert!(h.dialog.combo.is_none());
    assert!(!h.dialog.draft().mouse.invert);

    // Pressing the header and releasing elsewhere leaves the list open.
    h.slow_click(header);
    press(&mut h, header, true);
    release(&mut h, outside);
    assert!(h.dialog.combo.is_some());
}

#[test]
fn escape_closes_layers_innermost_first() {
    let mut h = Harness::new();
    h.tab(Tab::Video);
    h.click(page(300., 125.));
    h.click(page(40., 270.));
    assert!(h.dialog.sub.is_some());
    assert!(h.dialog.escape());
    assert!(h.dialog.sub.is_none());
    h.click(page(40., 270.));
    assert!(h.dialog.combo.is_some());
    assert!(h.dialog.escape());
    assert!(h.dialog.combo.is_none());
    assert!(!h.dialog.escape(), "Escape on the dialog closes it");
}

#[test]
fn dialogs_over_the_options_dialog_are_translucent_and_clip_underlying_content() {
    let mut h = Harness::new();
    // The frame fill of the topmost layer: the last untextured quad as wide as its frame.
    let frame_alpha = |h: &mut Harness, width: &dyn Fn(&OptionsDialog, &Painter) -> f32| {
        let painter = Painter {
            canvas: &h.canvas,
            fonts: &h.fonts,
            theme: &h.theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };
        let width = width(&h.dialog, &painter);
        h.dialog.draw(&painter);
        h.canvas
            .drain()
            .iter()
            .filter(|q| q.texture.is_none() && q.destination.w == width)
            .map(|q| q.color.a)
            .next_back()
            .expect("frame quad")
    };
    // Options alone: focused frame alpha
    let options_alpha = frame_alpha(&mut h, &|_, _| DIALOG_SIZE.x);
    assert!((options_alpha - h.theme.frame_bg_focused.a).abs() < 1e-4);

    for sub in [
        Sub::Gamma,
        Sub::AudioCredits,
        Sub::VideoCredits,
        Sub::KeyboardAdvanced,
        Sub::VideoAdvanced,
    ] {
        h.dialog.open_sub(sub);
        let width =
            |d: &OptionsDialog, p: &Painter| d.sub_frame_size(sub, |s| p.text_width(s, false)).x;
        let alpha = frame_alpha(&mut h, &width);
        assert!(
                (alpha - h.theme.frame_bg_focused.a).abs() < 1e-4,
                "{sub:?} stacked in-focus over options has focused frame background (alpha={alpha}, expected={})",
                h.theme.frame_bg_focused.a
            );

        // Verify parent options dialog frame background is out of focus:
        let painter = Painter {
            canvas: &h.canvas,
            fonts: &h.fonts,
            theme: &h.theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };
        h.dialog.draw(&painter);
        let options_frame = h
            .canvas
            .drain()
            .into_iter()
            .find(|q| q.texture.is_none() && q.destination.w == DIALOG_SIZE.x)
            .expect("options frame quad");
        assert!(
            (options_frame.color.a - h.theme.frame_bg_out_of_focus.a).abs() < 1e-4,
            "parent options dialog has out-of-focus frame background (alpha={}, expected={})",
            options_frame.color.a,
            h.theme.frame_bg_out_of_focus.a
        );
        h.dialog.cancel_sub();
    }

    // Test confirmation dialog over subdialog:
    h.dialog.open_sub(Sub::KeyboardAdvanced);
    h.dialog.command(Command::KeyDefaults);
    let confirm_width = {
        let painter = Painter {
            canvas: &h.canvas,
            fonts: &h.fonts,
            theme: &h.theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };
        let confirm = h.dialog.confirm.as_ref().expect("confirmation box");
        confirm.layout(|t| painter.text_width(t, false)).size.x
    };
    let sub_width = {
        let painter = Painter {
            canvas: &h.canvas,
            fonts: &h.fonts,
            theme: &h.theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };
        h.dialog
            .sub_frame_size(Sub::KeyboardAdvanced, |s| painter.text_width(s, false))
            .x
    };
    let painter = Painter {
        canvas: &h.canvas,
        fonts: &h.fonts,
        theme: &h.theme,
        origin: Vec2::ZERO,
        scale: 1.,
    };
    h.dialog.draw(&painter);
    let quads = h.canvas.drain();

    // 1. Confirmation box is focused:
    let confirm_frame = quads
        .iter()
        .rfind(|q| q.texture.is_none() && q.destination.w == confirm_width)
        .expect("confirmation frame quad");
    assert!(
        (confirm_frame.color.a - h.theme.frame_bg_focused.a).abs() < 1e-4,
        "confirmation box is focused (alpha={})",
        confirm_frame.color.a
    );

    // 2. Subdialog under confirmation is out of focus:
    let sub_frame_quad = quads
        .iter()
        .rfind(|q| q.texture.is_none() && q.destination.w == sub_width)
        .expect("subdialog frame quad");
    assert!(
        (sub_frame_quad.color.a - h.theme.frame_bg_out_of_focus.a).abs() < 1e-4,
        "subdialog under confirmation box is out of focus (alpha={})",
        sub_frame_quad.color.a
    );

    // 3. Parent options dialog is out of focus:
    let options_frame = quads
        .iter()
        .find(|q| q.texture.is_none() && q.destination.w == DIALOG_SIZE.x)
        .expect("options frame quad");
    assert!(
        (options_frame.color.a - h.theme.frame_bg_out_of_focus.a).abs() < 1e-4,
        "options dialog is out of focus (alpha={})",
        options_frame.color.a
    );

    h.dialog.confirm = None;
    h.dialog.cancel_sub();

    // Verify that underlying content behind the subdialog is clipped:
    let painter = Painter {
        canvas: &h.canvas,
        fonts: &h.fonts,
        theme: &h.theme,
        origin: Vec2::ZERO,
        scale: 1.,
    };
    // 1. Without subdialog: options content quads are drawn.
    let canvas_size = vec2(h.canvas.width(), h.canvas.height());
    h.dialog.draw(&painter);
    let quads_without_sub = h.canvas.drain();
    assert!(
        quads_without_sub.len() > 10,
        "options dialog draws content quads"
    );

    // 2. With subdialog open:
    h.dialog.open_sub(Sub::VideoAdvanced);
    let sub_size = h
        .dialog
        .sub_frame_size(Sub::VideoAdvanced, |s| painter.text_width(s, false));
    let sub_origin = h
        .dialog
        .sub_placement
        .place(&sub_frame(canvas_size, sub_size))
        .origin;
    let sub_rect = Rect::new(sub_origin.x, sub_origin.y, sub_size.x, sub_size.y);
    h.dialog.draw(&painter);
    let quads_with_sub = h.canvas.drain();

    // The options dialog's own frame background is still drawn (unclipped)
    assert!(
        quads_with_sub
            .iter()
            .any(|q| q.destination.w == DIALOG_SIZE.x && q.texture.is_none()),
        "options frame background is drawn across full dialog bounds"
    );

    // Find where the subdialog begins drawing its frame:
    let sub_start_idx = quads_with_sub
        .iter()
        .position(|q| q.destination.x == sub_rect.x + 5.0 && q.destination.y == sub_rect.y)
        .expect("subdialog top bar");

    // All underlying options content quads drawn before the subdialog frame (excluding the options
    // frame background at indices 0..7) are clipped outside sub_rect:
    for q in &quads_with_sub[7..sub_start_idx] {
        let dest = q.destination;
        let center = dest.center();
        assert!(
                !sub_rect.contains(center),
                "underlying quad {:?} center was not clipped despite being inside subdialog bounds {:?}",
                dest, sub_rect
            );
    }
}

#[test]
fn options_dialog_buttons_follow_press_cancel_and_release_activation() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);

    // 1. OK button interaction
    let ok_rect = button_rects()[0];
    let ok_pos = DIALOG + vec2(ok_rect.x + 4., ok_rect.y + 4.);
    let away_pos = DIALOG + vec2(10., 10.);

    // Hover over OK
    h.frame(Input {
        pointer: Some(ok_pos),
        ..Default::default()
    });
    assert_eq!(h.dialog.armed, Some(Target::Ok));
    assert_eq!(h.dialog.pressed, None);

    // Press down on OK
    let outcome = h.frame(Input {
        pointer: Some(ok_pos),
        click: true,
        held: true,
        ..Default::default()
    });
    assert_eq!(outcome, OptionsOutcome::None, "no action on press down");
    assert_eq!(h.dialog.pressed, Some(Target::Ok));

    // Move away while held: the press captured the mouse, so OK stays pressed
    h.frame(Input {
        pointer: Some(away_pos),
        held: true,
        ..Default::default()
    });
    assert_eq!(h.dialog.pressed, Some(Target::Ok), "captured on move away");
    assert_eq!(h.dialog.armed, None);

    // Release away: does not activate
    let outcome = h.frame(Input {
        pointer: Some(away_pos),
        released: true,
        ..Default::default()
    });
    assert_eq!(outcome, OptionsOutcome::None, "no action on release away");
    assert_eq!(h.dialog.pressed, None);

    // Press, drag off and back on, release over OK: activates (release-while-over)
    for (pointer, click) in [(ok_pos, true), (away_pos, false), (ok_pos, false)] {
        h.frame(Input {
            pointer: Some(pointer),
            click,
            held: true,
            ..Default::default()
        });
    }
    assert_eq!(h.dialog.pressed, Some(Target::Ok));
    let outcome = h.frame(Input {
        pointer: Some(ok_pos),
        released: true,
        ..Default::default()
    });
    assert_eq!(outcome, OptionsOutcome::Close(None));
    h.dialog.open(Config::default());
    h.tab(Tab::Mouse);

    // Click on OK properly (press then release over button)
    h.frame(Input {
        pointer: Some(ok_pos),
        click: true,
        held: true,
        ..Default::default()
    });
    assert_eq!(h.dialog.pressed, Some(Target::Ok));
    let outcome = h.frame(Input {
        pointer: Some(ok_pos),
        released: true,
        ..Default::default()
    });
    assert_eq!(outcome, OptionsOutcome::Close(None));

    // Dragging a captured press across the sensitivity slider neither arms nor moves the
    // slider, and returning to OK still activates it.
    let slider_pos = page(300., 40. + SLIDER_TRACK_Y);
    let sensitivity = h.dialog.draft().mouse.sensitivity;
    for (pointer, click) in [(ok_pos, true), (slider_pos, false), (ok_pos, false)] {
        h.frame(Input {
            pointer: Some(pointer),
            click,
            held: true,
            ..Default::default()
        });
        assert_eq!(h.dialog.pressed, Some(Target::Ok));
        assert!(h.dialog.armed.is_none() || h.dialog.armed == Some(Target::Ok));
    }
    assert_eq!(h.dialog.draft().mouse.sensitivity, sensitivity);
    let outcome = h.frame(Input {
        pointer: Some(ok_pos),
        released: true,
        ..Default::default()
    });
    assert_eq!(outcome, OptionsOutcome::Close(None));
    h.dialog.open(Config::default());
    h.tab(Tab::Mouse);

    // 2. CheckBox interaction (Invert mouse at page(30., 20.))
    h.dialog.open(Config::default());
    h.tab(Tab::Mouse);
    let chk_pos = page(30., 20.);
    assert!(!h.dialog.draft().mouse.invert);

    // Press down on checkbox
    h.frame(Input {
        pointer: Some(chk_pos),
        click: true,
        held: true,
        ..Default::default()
    });
    assert!(!h.dialog.draft().mouse.invert, "not toggled on press down");
    assert!(h.dialog.pressed.is_some());

    // Move away while held: still captured
    h.frame(Input {
        pointer: Some(away_pos),
        held: true,
        ..Default::default()
    });
    assert!(h.dialog.pressed.is_some());

    // Release away
    h.frame(Input {
        pointer: Some(away_pos),
        released: true,
        ..Default::default()
    });
    assert!(!h.dialog.draft().mouse.invert, "not toggled after cancel");

    // Click checkbox properly
    h.frame(Input {
        pointer: Some(chk_pos),
        click: true,
        held: true,
        ..Default::default()
    });
    h.frame(Input {
        pointer: Some(chk_pos),
        released: true,
        ..Default::default()
    });
    assert!(h.dialog.draft().mouse.invert, "toggled on release");
}

#[test]
fn options_dialog_sound_events_hover_and_activation() {
    let mut h = Harness::new();
    h.dialog.open(Config::default());
    let ok_rect = button_rects()[0];
    let ok_pos = DIALOG + vec2(ok_rect.x + 4., ok_rect.y + 4.);
    let away_pos = DIALOG + vec2(10., 10.);

    // 1. Hovering OK button does NOT emit rollover sound (dialog button)
    h.frame(Input {
        pointer: Some(ok_pos),
        ..Default::default()
    });
    assert!(h.dialog.take_sounds().is_empty());

    // 2. Hovering OK button again emits nothing
    h.frame(Input {
        pointer: Some(ok_pos),
        ..Default::default()
    });
    assert!(h.dialog.take_sounds().is_empty());

    // 3. Moving away emits nothing
    h.frame(Input {
        pointer: Some(away_pos),
        ..Default::default()
    });
    assert!(h.dialog.take_sounds().is_empty());

    // 4. Clicking and releasing OK button emits click sound
    h.frame(Input {
        pointer: Some(ok_pos),
        click: true,
        held: true,
        ..Default::default()
    });
    assert!(h.dialog.take_sounds().is_empty());
    h.frame(Input {
        pointer: Some(ok_pos),
        released: true,
        ..Default::default()
    });
    assert_eq!(h.dialog.take_sounds(), vec!["ui/buttonclickrelease.wav"]);
}

#[test]
fn options_dialog_close_button_behaves_like_cancel() {
    let mut h = Harness::new();
    h.dialog.open(Config::default());
    h.tab(Tab::Mouse);

    // Modify draft setting (invert mouse)
    let chk_pos = page(30., 20.);
    h.frame(Input {
        pointer: Some(chk_pos),
        click: true,
        held: true,
        ..Default::default()
    });
    h.frame(Input {
        pointer: Some(chk_pos),
        released: true,
        ..Default::default()
    });
    assert!(h.dialog.draft().mouse.invert);
    let _ = h.dialog.take_sounds();

    let close_rect = widgets::close_button_rect(DIALOG_SIZE);
    let close_pos = DIALOG
        + vec2(
            close_rect.x + close_rect.w / 2.,
            close_rect.y + close_rect.h / 2.,
        );

    // Hover over close button: arms Close without emitting rollover sound
    h.frame(Input {
        pointer: Some(close_pos),
        ..Default::default()
    });
    assert_eq!(h.dialog.armed, Some(Target::Close));
    assert!(h.dialog.take_sounds().is_empty());

    // Click and release close button: activates Cancel outcome (Close(None))
    h.frame(Input {
        pointer: Some(close_pos),
        click: true,
        held: true,
        ..Default::default()
    });
    assert_eq!(h.dialog.pressed, Some(Target::Close));

    let outcome = h.frame(Input {
        pointer: Some(close_pos),
        released: true,
        ..Default::default()
    });
    // Crucial: Close(None) discards draft changes, exactly like Cancel!
    assert_eq!(outcome, OptionsOutcome::Close(None));
    assert_eq!(h.dialog.take_sounds(), vec!["ui/buttonclickrelease.wav"]);
}

#[test]
fn options_subdialog_close_button_cancels_and_restores() {
    let mut h = Harness::new();
    h.dialog.open(Config::default());
    h.dialog.open_sub(Sub::Gamma);
    h.dialog.draft.config.video.gamma = 1.6;

    let canvas = vec2(1280., 720.);
    let size = GAMMA_SIZE;
    let gamma_origin = centered(canvas, size, 1.);
    let close_rect = widgets::close_button_rect(size);
    let close_pos = gamma_origin
        + vec2(
            close_rect.x + close_rect.w / 2.,
            close_rect.y + close_rect.h / 2.,
        );

    // Click and release on subdialog close button
    h.frame(Input {
        pointer: Some(close_pos),
        click: true,
        held: true,
        ..Default::default()
    });
    h.frame(Input {
        pointer: Some(close_pos),
        released: true,
        ..Default::default()
    });

    // Subdialog is closed and gamma is restored to saved value
    assert_eq!(h.dialog.sub, None);
    assert_eq!(h.dialog.draft().video.gamma, 2.2);
}

#[test]
fn credits_subdialog_sizing_and_no_scrolling() {
    let mut h = Harness::new();
    let mut loc = Localization::default();
    loc.insert(
            "GameUI_Miles_Audio",
            "Miles Sound System Copyright © 1991-2001 RAD Game Tools, Inc. Unmatched quality audio processing.",
        );
    loc.insert(
        "GameUI_Miles_Voice",
        "Miles Voice Compression System Copyright © 1991-2001 RAD Game Tools, Inc.",
    );
    loc.insert(
        "GameUI_Bink",
        "Uses Bink Video. Copyright © 1997-2004 by RAD Game Tools, Inc.",
    );
    loc.insert("GameUI_Close", "Close");
    h.dialog.res = Arc::new(GameUiResources {
        localization: loc,
        ..Default::default()
    });

    h.dialog.sub = Some(Sub::AudioCredits);
    let painter = Painter {
        canvas: &h.canvas,
        fonts: &h.fonts,
        theme: &h.theme,
        origin: Vec2::ZERO,
        scale: 1.,
    };

    // 1. Sizing is derived dynamically from content (Audio vs Video credits)
    let audio_items = h.dialog.credits_items(Sub::AudioCredits);
    let audio_content_h =
        h.dialog
            .measure_credits_height(&audio_items, CREDITS_SIZE.x - 32., |s| s.len() as f32 * 7.);
    let audio_size = h
        .dialog
        .sub_frame_size(Sub::AudioCredits, |s| s.len() as f32 * 7.);
    assert_eq!(audio_size.x, CREDITS_SIZE.x);
    assert_eq!(audio_size.y, 34. + audio_content_h + 46.);

    let video_items = h.dialog.credits_items(Sub::VideoCredits);
    let video_content_h =
        h.dialog
            .measure_credits_height(&video_items, CREDITS_SIZE.x - 32., |s| s.len() as f32 * 7.);
    let video_size = h
        .dialog
        .sub_frame_size(Sub::VideoCredits, |s| s.len() as f32 * 7.);
    assert_eq!(video_size.x, CREDITS_SIZE.x);
    assert_eq!(video_size.y, 34. + video_content_h + 46.);
    assert!(
        audio_size.y > video_size.y,
        "Audio credits with more items should be taller than video credits"
    );

    // 2. Draw audio credits and verify no scrollbar exists, nothing exceeds bounds, and nothing overlaps
    h.dialog.draw(&painter);
    let quads = h.canvas.drain();

    let real_audio_size = h
        .dialog
        .sub_frame_size(Sub::AudioCredits, |s| painter.text_width(s, false));
    let origin = centered(vec2(1280., 720.), real_audio_size, 1.);
    let dialog_bounds = Rect::new(origin.x, origin.y, real_audio_size.x, real_audio_size.y);
    let close_rect = Rect::new(
        origin.x + real_audio_size.x - 96.,
        origin.y + real_audio_size.y - 34.,
        80.,
        24.,
    );

    // Assert no scrollbar quad was rendered anywhere
    let sb_w = widgets::SCROLLBAR_WIDTH;
    let has_scrollbar = quads.iter().any(|q| {
        let r = q.destination;
        r.w == sb_w && r.h > 20.
    });
    assert!(
        !has_scrollbar,
        "Scrollbar must NOT be rendered; credits dialog is sized to content instead of scrolling"
    );

    // Assert Close button quad exists and is fully inside dialog bounds
    let has_close_button = quads.iter().any(|q| {
        let r = q.destination;
        r.x >= close_rect.x - 1.
            && r.x + r.w <= close_rect.x + close_rect.w + 1.
            && r.y >= close_rect.y - 1.
            && r.y + r.h <= close_rect.y + close_rect.h + 1.
    });
    assert!(
        has_close_button,
        "Expected Close button quad at bottom right inside dialog bounds"
    );

    // Assert every rendered subdialog quad is strictly within the dialog bounds (nothing exceeds bounds)
    let credits_frame_idx = quads
        .iter()
        .position(|q| q.destination.w == real_audio_size.x)
        .expect("audio credits frame quad");
    let subdialog_start_idx = credits_frame_idx.saturating_sub(1);
    let subdialog_quads = &quads[subdialog_start_idx..];
    for q in subdialog_quads {
        let r = q.destination;
        assert!(
            r.x >= dialog_bounds.x - 1. && r.x + r.w <= dialog_bounds.x + dialog_bounds.w + 1.,
            "Quad at ({}, {}) exceeds dialog horizontal bounds [{}, {}]",
            r.x,
            r.y,
            dialog_bounds.x,
            dialog_bounds.x + dialog_bounds.w
        );
        assert!(
            r.y >= dialog_bounds.y - 1. && r.y + r.h <= dialog_bounds.y + dialog_bounds.h + 1.,
            "Quad at ({}, {}) exceeds dialog vertical bounds [{}, {}]",
            r.x,
            r.y,
            dialog_bounds.y,
            dialog_bounds.y + dialog_bounds.h
        );
    }

    // 3. Close button dismisses credits subdialog
    let close_click_pos = vec2(close_rect.x + 10., close_rect.y + 10.);
    h.dialog.input(
        &Input {
            pointer: Some(close_click_pos),
            click: true,
            held: true,
            ..Default::default()
        },
        &painter,
    );
    h.dialog.input(
        &Input {
            pointer: Some(close_click_pos),
            released: true,
            ..Default::default()
        },
        &painter,
    );
    assert!(
        h.dialog.sub.is_none(),
        "Close button should dismiss credits subdialog"
    );
}

#[test]
fn credits_dialog_reports_when_exceeding_window_at_small_resolutions() {
    let mut h = Harness::new();
    h.dialog.sub = Some(Sub::AudioCredits);

    let painter = Painter {
        canvas: &h.canvas,
        fonts: &h.fonts,
        theme: &h.theme,
        origin: Vec2::ZERO,
        scale: 1.,
    };

    // Standard resolution fits cleanly
    assert!(
        h.dialog
            .credits_fits_window(Sub::AudioCredits, vec2(1280., 720.), |s| painter
                .text_width(s, false)),
        "Should fit window at 1280x720"
    );
    assert!(
        h.dialog
            .credits_fits_window(Sub::AudioCredits, vec2(1024., 768.), |s| painter
                .text_width(s, false)),
        "Should fit window at 1024x768"
    );

    // Very small resolution cannot fit window
    let small_canvas = vec2(400., 200.);
    assert!(
        !h.dialog
            .credits_fits_window(Sub::AudioCredits, small_canvas, |s| painter
                .text_width(s, false)),
        "Cannot fit window at 400x200"
    );

    // Running input on small canvas logs report message to console log
    let small_canvas_obj = Canvas::default();
    small_canvas_obj.resize(400., 200.);
    let small_painter = Painter {
        canvas: &small_canvas_obj,
        fonts: &h.fonts,
        theme: &h.theme,
        origin: Vec2::ZERO,
        scale: 1.,
    };
    h.dialog.input(&Input::default(), &small_painter);

    let log = h.dialog.take_log();
    assert!(
        log.iter().any(|msg| msg.contains("Credits dialog content")
            && msg.contains("cannot fit window bounds")),
        "Expected report in log when content cannot fit window at small resolution, got: {:?}",
        log
    );
}

#[test]
fn audio_options_controls_and_placeholder_handling() {
    let layout = Layout::parse(
            br##""Resource/OptionsSubAudio.res" {
                "SFXSlider" { "ControlName" "CCvarSlider" "fieldName" "SFXSlider" "xpos" "36" "ypos" "43" "wide" "175" "tall" "50" }
                "MusicSlider" { "ControlName" "CCvarSlider" "fieldName" "MusicSlider" "xpos" "254" "ypos" "43" "wide" "175" "tall" "50" }
                "SpeakerSetup" { "ControlName" "ComboBox" "fieldName" "SpeakerSetup" "xpos" "36" "ypos" "116" "wide" "175" "tall" "24" }
                "SoundQuality" { "ControlName" "ComboBox" "fieldName" "SoundQuality" "xpos" "254" "ypos" "116" "wide" "175" "tall" "24" }
                "CloseCaptionCheck" { "ControlName" "ComboBox" "fieldName" "CloseCaptionCheck" "xpos" "254" "ypos" "179" "wide" "175" "tall" "24" }
                "snd_mute_losefocus" { "ControlName" "CCvarToggleCheckButton" "fieldName" "snd_mute_losefocus" "xpos" "33" "ypos" "230" "wide" "430" "tall" "28" }
                "ThirdPartySoundCredits" { "ControlName" "URLButton" "fieldName" "ThirdPartySoundCredits" "xpos" "254" "ypos" "270" "wide" "200" "tall" "24" "command" "ShowThirdPartyAudioCredits" }
                "TestSpeakers" { "ControlName" "Button" "fieldName" "TestSpeakers" "xpos" "36" "ypos" "194" "wide" "160" "tall" "24" "command" "TestSpeakers" "visible" "1" }
            }"##,
        )
        .unwrap();

    let mut res = (*resources()).clone();
    res.layouts.insert("optionssubaudio".into(), layout);

    let draft = Draft::new(Config::default());
    let widgets = layout_widgets(&res, Panel::Audio, page_rect(), &draft, &draft.config);

    let find_widget = |name: &str| widgets.iter().find(|w| w.control.name == name).unwrap();

    assert!(find_widget("SFXSlider").enabled);
    assert!(find_widget("MusicSlider").enabled);
    assert!(find_widget("SpeakerSetup").enabled);
    assert!(find_widget("SoundQuality").enabled);
    assert!(find_widget("CloseCaptionCheck").enabled);
    assert!(find_widget("snd_mute_losefocus").enabled);
    assert!(find_widget("ThirdPartySoundCredits").enabled);

    // TestSpeakers is a placeholder: even when visible is 1, its enabled state must be false
    assert!(
        !find_widget("TestSpeakers").enabled,
        "TestSpeakers is a placeholder and must be disabled"
    );
}

#[test]
fn video_options_hdr_control_layout_and_selection() {
    let layout = Layout::parse(
            br##""Resource/OptionsSubVideo.res" {
                "HDR Label" { "ControlName" "Label" "fieldName" "HDR Label" "xpos" "36" "ypos" "239" "wide" "160" "tall" "24" "visible" "0" }
                "HDR" { "ControlName" "ComboBox" "fieldName" "HDR" "xpos" "36" "ypos" "264" "wide" "160" "tall" "24" "visible" "0" }
            }"##,
        )
        .unwrap();

    let mut res = (*resources()).clone();
    res.layouts.insert("optionssubvideo".into(), layout);

    let mut draft = Draft::new(Config::default());
    // force_visible ensures HDR and HDR Label are laid out even if layout sets visible 0 (like retail [$OSX])
    let widgets = layout_widgets(&res, Panel::Video, page_rect(), &draft, &draft.config);

    let find_widget = |name: &str| widgets.iter().find(|w| w.control.name == name).unwrap();
    // The widgets are included in layout_widgets because force_visible forces visible 1
    assert_eq!(find_widget("HDR Label").control.name, "HDR Label");
    assert_eq!(find_widget("HDR").control.name, "HDR");
    assert!(find_widget("HDR").enabled);

    let env = Env {
        localization: &res.localization,
        languages: &[],
        modes: &DisplayModes::default(),
        recommendation: None,
    };

    let items = Choice::Hdr.items(&env, &draft);
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], "GameUI_hdr_level0");
    assert_eq!(items[1], "GameUI_hdr_level1");
    assert_eq!(items[2], "GameUI_hdr_level2");

    assert_eq!(Choice::Hdr.selected(&env, &draft), Some(2));

    Choice::Hdr.select(&env, &mut draft, 1);
    assert_eq!(draft.config.video.hdr, HdrMode::Bloom);
    assert_eq!(Choice::Hdr.selected(&env, &draft), Some(1));

    Choice::Hdr.select(&env, &mut draft, 2);
    assert_eq!(draft.config.video.hdr, HdrMode::Full);
    assert_eq!(Choice::Hdr.selected(&env, &draft), Some(2));

    Choice::Hdr.select(&env, &mut draft, 0);
    assert_eq!(draft.config.video.hdr, HdrMode::None);
    assert_eq!(Choice::Hdr.selected(&env, &draft), Some(0));
}

#[test]
fn video_advanced_hdcontent_placeholder_and_effects_layout() {
    use crate::gameui::settings::{binding, Command, Panel};

    assert_eq!(
        binding(Panel::VideoAdvanced, "HDContentButton"),
        Some(Binding::Command(Command::HDContent))
    );
    assert!(
        Command::HDContent.placeholder(),
        "HDContent must be a placeholder"
    );
}

#[test]
fn gameplay_difficulty_options_and_migrate_placeholder() {
    use crate::gameui::settings::{binding, Command, Panel};

    assert_eq!(
        binding(Panel::Gameplay, "Skill1Radio"),
        Some(Binding::Skill(1))
    );
    assert_eq!(
        binding(Panel::Gameplay, "Skill2Radio"),
        Some(Binding::Skill(2))
    );
    assert_eq!(
        binding(Panel::Gameplay, "Skill3Radio"),
        Some(Binding::Skill(3))
    );
    assert_eq!(
        binding(Panel::Gameplay, "MigrateButton"),
        Some(Binding::Command(Command::MigrateData))
    );
    assert!(
        Command::MigrateData.placeholder(),
        "MigrateData must be a placeholder"
    );

    let layout = Layout::parse(
            br##""Resource/OptionsSubDifficulty.res" {
                "Skill1Radio" { "ControlName" "RadioButton" "fieldName" "Skill1Radio" "xpos" "34" "ypos" "58" "wide" "140" "tall" "24" "labelText" "#GameUI_SkillEasy" "visible" "1" "enabled" "1" }
                "Skill2Radio" { "ControlName" "RadioButton" "fieldName" "Skill2Radio" "xpos" "34" "ypos" "116" "wide" "140" "tall" "24" "labelText" "#GameUI_SkillNormal" "visible" "1" "enabled" "1" }
                "Skill3Radio" { "ControlName" "RadioButton" "fieldName" "Skill3Radio" "xpos" "34" "ypos" "176" "wide" "140" "tall" "24" "labelText" "#GameUI_SkillHard" "visible" "1" "enabled" "1" }
                "MigrateButton" { "ControlName" "Button" "fieldName" "MigrateButton" "xpos" "38" "ypos" "246" "wide" "240" "tall" "24" "labelText" "#GameUI_MigrateEpisodicData" "visible" "0" "enabled" "1" }
            }"##,
        )
        .unwrap();

    let mut res = (*resources()).clone();
    res.layouts.insert("optionssubdifficulty".into(), layout);
    let res = Arc::new(res);

    let config = Config::default();
    let draft = Draft::new(config.clone());
    let widgets = layout_widgets(&res, Panel::Gameplay, page_rect(), &draft, &config);

    // MigrateButton has visible "0" so it should not be in drawn_controls
    assert!(widgets.iter().all(|w| w.control.name != "MigrateButton"));

    // All 3 skill radio buttons are present and enabled
    let find_widget = |name: &str| widgets.iter().find(|w| w.control.name == name).unwrap();
    assert!(find_widget("Skill1Radio").enabled);
    assert!(find_widget("Skill2Radio").enabled);
    assert!(find_widget("Skill3Radio").enabled);

    // Test interaction via Harness
    let mut h = Harness::new();
    h.dialog.set_resources(res.clone());
    h.tab(Tab::Gameplay);

    // Default difficulty is 2
    assert_eq!(h.dialog.draft().gameplay.skill, 2);

    // Click Skill 1 (Easy)
    let skill1_rect = find_widget("Skill1Radio").rect;
    h.click(DIALOG + skill1_rect.center());
    assert_eq!(h.dialog.draft().gameplay.skill, 1);

    // Click Skill 3 (Hard)
    let skill3_rect = find_widget("Skill3Radio").rect;
    h.click(DIALOG + skill3_rect.center());
    assert_eq!(h.dialog.draft().gameplay.skill, 3);
}

#[test]
fn video_advanced_recommended_settings_displayed_with_asterisks_in_dialog_and_combos() {
    use crate::gameui::recommendation::{
        CpuSpecs, GpuDeviceType, GpuSpecs, HardwareTier, SystemRecommendation,
    };
    use crate::gameui::settings::AdvancedChoice;

    let mut h = Harness::new();
    let cpu = CpuSpecs::new(8);
    let gpu = GpuSpecs::new("Apple M1 Pro", GpuDeviceType::Integrated);
    let rec = SystemRecommendation::from_specs(cpu, gpu);
    assert_eq!(rec.tier, HardwareTier::High);

    h.dialog.set_system_recommendation(rec.clone());
    h.tab(Tab::Video);

    // Open Video Advanced sub-dialog
    h.click(page(300., 200.));
    assert_eq!(h.dialog.sub, Some(Sub::VideoAdvanced));

    let res = h.dialog.res.clone();
    let area = Rect::new(0., 0., 482., 358.);
    let widgets = layout_widgets(
        &res,
        Panel::VideoAdvanced,
        area,
        &h.dialog.draft,
        &Config::default(),
    );

    // Label11 displaying #GameUI_RecommendedSettings ("* Setting recommended for your system") is present
    let label11 = widgets
        .iter()
        .find(|w| w.control.name == "Label11")
        .unwrap();
    assert_eq!(label11.control.label, "#GameUI_RecommendedSettings");

    let modes = h.dialog.modes.clone();
    let env = OptionsDialog::make_env(&res, &modes, &rec);

    // ModelDetail: recommended is High (index 2) -> "gameui_high *"
    let model_items = Choice::Advanced(AdvancedChoice::ModelDetail).items(&env, &h.dialog.draft);
    assert_eq!(model_items.len(), 3);
    assert_eq!(model_items[0], "gameui_low");
    assert_eq!(model_items[1], "gameui_medium");
    assert_eq!(model_items[2], "gameui_high *");

    // Multicore: recommended is Enabled (index 1) -> "gameui_enabled *"
    let multicore_items = Choice::Advanced(AdvancedChoice::Multicore).items(&env, &h.dialog.draft);
    assert_eq!(multicore_items[0], "gameui_disabled");
    assert_eq!(multicore_items[1], "gameui_enabled *");

    // Filtering: recommended is Anisotropic 8X (index 4) -> "GameUI_Anisotropic8X *"
    let filtering_items = Choice::Advanced(AdvancedChoice::Filtering).items(&env, &h.dialog.draft);
    assert_eq!(filtering_items[4], "GameUI_Anisotropic8X *");
    assert_eq!(filtering_items[0], "GameUI_Bilinear");

    // Antialiasing: recommended is 4X (index 2) -> "GameUI_4X *"
    let aa_items = Choice::Advanced(AdvancedChoice::Antialiasing).items(&env, &h.dialog.draft);
    assert_eq!(aa_items[2], "GameUI_4X *");
    assert_eq!(aa_items[0], "GameUI_None");

    // HDR: recommended is Full (index 2) -> "GameUI_hdr_level2 *"
    let hdr_items = Choice::Hdr.items(&env, &h.dialog.draft);
    assert_eq!(hdr_items[2], "GameUI_hdr_level2 *");
    assert_eq!(hdr_items[0], "GameUI_hdr_level0");

    // When recommended option is selected, Choice::text returns the label with asterisk
    h.dialog.apply_recommended_video();
    assert_eq!(
        Choice::Advanced(AdvancedChoice::ModelDetail).text(&env, &h.dialog.draft),
        "gameui_high *"
    );
    assert_eq!(
        Choice::Advanced(AdvancedChoice::Multicore).text(&env, &h.dialog.draft),
        "gameui_enabled *"
    );
    assert_eq!(
        Choice::Hdr.text(&env, &h.dialog.draft),
        "GameUI_hdr_level2 *"
    );

    // When user switches away to a non-recommended setting, Choice::text does NOT have asterisk
    Choice::Advanced(AdvancedChoice::ModelDetail).select(&env, &mut h.dialog.draft, 0);
    assert_eq!(
        Choice::Advanced(AdvancedChoice::ModelDetail).text(&env, &h.dialog.draft),
        "gameui_low"
    );
}

#[test]
fn video_advanced_system_recommendation_scales_with_hardware_tiers() {
    use crate::gameui::recommendation::{
        CpuSpecs, GpuDeviceType, GpuSpecs, HardwareTier, SystemRecommendation,
    };
    use crate::gameui::settings::AdvancedChoice;

    let mut h = Harness::new();

    // 1. Low Tier Hardware (1 CPU core, software renderer)
    let low_rec = SystemRecommendation::from_specs(
        CpuSpecs::new(1),
        GpuSpecs::new("llvmpipe", GpuDeviceType::Cpu),
    );
    assert_eq!(low_rec.tier, HardwareTier::Low);
    h.dialog.set_system_recommendation(low_rec);
    let low_env = h.dialog.env();

    // On Low tier, Low settings get the asterisk
    let model_items =
        Choice::Advanced(AdvancedChoice::ModelDetail).items(&low_env, &h.dialog.draft);
    assert_eq!(model_items[0], "gameui_low *");
    assert_eq!(model_items[2], "gameui_high");

    let multicore_items =
        Choice::Advanced(AdvancedChoice::Multicore).items(&low_env, &h.dialog.draft);
    assert_eq!(multicore_items[0], "gameui_disabled *");
    assert_eq!(multicore_items[1], "gameui_enabled");

    let aa_items = Choice::Advanced(AdvancedChoice::Antialiasing).items(&low_env, &h.dialog.draft);
    assert_eq!(aa_items[0], "GameUI_None *");

    let hdr_items = Choice::Hdr.items(&low_env, &h.dialog.draft);
    assert_eq!(hdr_items[0], "GameUI_hdr_level0 *");

    // 2. Ultra Tier Hardware (16 CPU cores, RTX 4090)
    let ultra_rec = SystemRecommendation::from_specs(
        CpuSpecs::new(16),
        GpuSpecs::new("GeForce RTX 4090", GpuDeviceType::Discrete),
    );
    assert_eq!(ultra_rec.tier, HardwareTier::Ultra);
    h.dialog.set_system_recommendation(ultra_rec);
    let ultra_env = h.dialog.env();

    let model_ultra =
        Choice::Advanced(AdvancedChoice::ModelDetail).items(&ultra_env, &h.dialog.draft);
    assert_eq!(model_ultra[2], "gameui_high *");

    let tex_ultra =
        Choice::Advanced(AdvancedChoice::TextureDetail).items(&ultra_env, &h.dialog.draft);
    assert_eq!(tex_ultra[3], "gameui_ultra *");

    let filtering_ultra =
        Choice::Advanced(AdvancedChoice::Filtering).items(&ultra_env, &h.dialog.draft);
    assert_eq!(filtering_ultra[5], "GameUI_Anisotropic16X *");

    let aa_ultra =
        Choice::Advanced(AdvancedChoice::Antialiasing).items(&ultra_env, &h.dialog.draft);
    assert_eq!(aa_ultra[3], "GameUI_8X *");

    let hdr_ultra = Choice::Hdr.items(&ultra_env, &h.dialog.draft);
    assert_eq!(hdr_ultra[2], "GameUI_hdr_level2 *");

    let multicore_ultra =
        Choice::Advanced(AdvancedChoice::Multicore).items(&ultra_env, &h.dialog.draft);
    assert_eq!(multicore_ultra[1], "gameui_enabled *");

    // 3. Apple Silicon / Metal Hardware (e.g. M1 Pro) - 8X MSAA is gated
    let apple_rec = SystemRecommendation::from_specs(
        CpuSpecs::new(10),
        GpuSpecs::new("Apple M1 Pro", GpuDeviceType::Integrated),
    );
    h.dialog.set_system_recommendation(apple_rec);
    let apple_env = h.dialog.env();
    let aa_apple =
        Choice::Advanced(AdvancedChoice::Antialiasing).items(&apple_env, &h.dialog.draft);
    assert_eq!(
        aa_apple.len(),
        3,
        "8X MSAA must be gated on Apple Silicon / Metal hardware"
    );
    assert_eq!(aa_apple[2], "GameUI_4X *");
}

#[test]
fn video_advanced_performance_benchmark_tracking_and_accordance() {
    use crate::gameui::recommendation::{
        CpuSpecs, GpuDeviceType, GpuSpecs, HardwareTier, SystemRecommendation, VideoBenchmarkSuite,
    };

    // Low tier hardware: Low profile maintains 60 FPS target, Ultra profile degrades
    let low_rec = SystemRecommendation::from_specs(
        CpuSpecs::new(1),
        GpuSpecs::new("llvmpipe", GpuDeviceType::Cpu),
    );
    let low_report = VideoBenchmarkSuite::run_preset_benchmarks(&low_rec);
    assert_eq!(low_report.tier, HardwareTier::Low);
    assert!(
        low_report.recommended_profile.target_fps_achieved,
        "Low-tier recommended profile must achieve target 60 FPS: got {:.1}",
        low_report.recommended_profile.estimated_fps
    );
    assert!(low_report.recommended_profile.in_accordance);
    assert!(
        low_report.ultra_profile.estimated_fps < 40.0,
        "Ultra profile on Low hardware should drop significantly below budget: got {:.1}",
        low_report.ultra_profile.estimated_fps
    );

    // Ultra tier hardware: Recommended profile achieves high FPS with ultra fidelity
    let ultra_rec = SystemRecommendation::from_specs(
        CpuSpecs::new(16),
        GpuSpecs::new("GeForce RTX 4090", GpuDeviceType::Discrete),
    );
    let ultra_report = VideoBenchmarkSuite::run_preset_benchmarks(&ultra_rec);
    assert_eq!(ultra_report.tier, HardwareTier::Ultra);
    assert!(ultra_report.recommended_profile.target_fps_achieved);
    assert!(ultra_report.recommended_profile.in_accordance);
    assert!(
        ultra_report.recommended_profile.estimated_fps >= 100.0,
        "Ultra profile on Ultra hardware should exceed 100 FPS: got {:.1}",
        ultra_report.recommended_profile.estimated_fps
    );

    // Track isolated video setting impact metrics
    let impacts = &ultra_report.settings_impact;
    assert!(impacts["msaa_8x_vs_none_ms"] > 0.0);
    assert!(impacts["aniso_16x_vs_bilinear_ms"] > 0.0);
    assert!(impacts["shadow_high_vs_low_ms"] > 0.0);
    assert!(impacts["water_all_vs_simple_ms"] > 0.0);
    assert!(impacts["multicore_cpu_gain_ms"] > 0.0);
}

mod focus;
mod hints;
