#[test]
#[ignore = "requires an owned installed Half-Life 2 copy"]
fn owned_options_support_resources() {
    use crate::gameui::{
        resources::{GameUiResources, Reader},
        settings::Panel,
    };
    let root = source_assets::install::discover().unwrap();
    let vfs = source_assets::vpk::Vfs::mount(&root).unwrap();
    let reader = Reader::new(&vfs);
    let resources = GameUiResources::load(&vfs);
    for panel in [
        Panel::Mouse,
        Panel::Audio,
        Panel::VideoAdvanced,
        Panel::Gamepad,
    ] {
        for c in &resources.layout(panel.layout()).unwrap().controls {
            eprintln!(
                "{panel:?} {} {:?} visible={} enabled={} associate={:?} label={:?} tooltip={:?} rect={:?}",
                c.name, c.kind, c.visible, c.enabled, c.associate, c.label, c.tooltip, c.rect((512., 406.))
            );
        }
    }
    for path in [
        "resource/gameui_english.txt",
        "resource/valve_english.txt",
        "resource/vgui_english.txt",
        "resource/merged_loc_all.txt",
    ] {
        if let Some(text) = reader.read_text(path) {
            for line in text.lines().filter(|line| {
                line.to_ascii_lowercase().contains("not implemented")
                    || line.to_ascii_lowercase().contains("in development")
            }) {
                eprintln!("potential status token {path}: {line}");
            }
        }
    }
}

use super::*;
use source_ui::layout::Layout;

#[test]
fn platform_gates_apply_to_controls_and_unassociated_labels_only_on_their_platform() {
    let layout = Layout::parse(
        br#"Root {
        D3D9Ex { ControlName ComboBox }
        D3D9ExLabel { ControlName Label }
        MouseRaw { ControlName CheckButton }
        MouseAccelerationCheckbox { ControlName CheckButton }
        MouseAccelerationSlider { ControlName Slider }
    }"#,
    )
    .unwrap();
    for platform in [
        Platform::Windows,
        Platform::MacOs,
        Platform::Linux,
        Platform::Other,
    ] {
        for name in ["D3D9Ex", "D3D9ExLabel"] {
            assert_eq!(
                available(Panel::VideoAdvanced, layout.get(name).unwrap(), platform),
                platform == Platform::Windows
            );
        }
        for name in [
            "MouseRaw",
            "MouseAccelerationCheckbox",
            "MouseAccelerationSlider",
        ] {
            assert_eq!(
                available(Panel::Mouse, layout.get(name).unwrap(), platform),
                platform != Platform::MacOs
            );
        }
    }
}

#[test]
fn completed_features_have_no_development_status() {
    for advanced in [
        AdvancedChoice::ModelDetail,
        AdvancedChoice::TextureDetail,
        AdvancedChoice::ShadowDetail,
        AdvancedChoice::ColorCorrection,
        AdvancedChoice::Antialiasing,
        AdvancedChoice::Filtering,
        AdvancedChoice::VSync,
    ] {
        assert_eq!(
            development(Binding::Choice(Choice::Advanced(advanced))),
            None,
            "{advanced:?}"
        );
    }
    for binding in [
        Binding::Choice(Choice::Hdr),
        Binding::Slider(Value::Fov),
        Binding::Toggle(Toggle::Acceleration),
        Binding::Toggle(Toggle::ReverseMouse),
    ] {
        assert_eq!(development(binding), None, "{binding:?}");
    }
}

#[test]
fn unfinished_features_have_specific_localizable_status() {
    let localization = crate::gameui::localize::Localization::default();
    for binding in [
        Binding::Choice(Choice::Advanced(AdvancedChoice::WaterDetail)),
        Binding::Choice(Choice::Advanced(AdvancedChoice::ShaderDetail)),
        Binding::Choice(Choice::Speakers),
        Binding::Choice(Choice::AimAssist),
        Binding::Command(Command::SteamInput),
    ] {
        let token = development(binding).unwrap();
        assert!(token.starts_with("#HL2RS_"));
        assert_ne!(localization.text(token), token);
    }
}

#[test]
fn motion_blur_reports_the_experimental_ultra_tier() {
    assert_eq!(
        development(Binding::Choice(Choice::Advanced(
            AdvancedChoice::MotionBlur
        ))),
        Some(crate::gameui::localize::placeholders::STATUS_MOTION_BLUR)
    );
}
