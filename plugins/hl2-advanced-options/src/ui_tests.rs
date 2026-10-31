use hl2_ui::{
    console::{Console, Effect, Mode},
    extensions::Extensions,
};
use source_ui::input::{Input, Key};
fn console() -> Console {
    let mut c = Console::default();
    let mut e = Extensions::default();
    crate::register(&mut e).unwrap();
    e.install(&mut c);
    c
}
/// A main-menu console at 1280x720 whose Options applied a change to `width` x `height`
/// at UI time 10; returns the applied effects.
fn applied_video_change(width: u32, height: u32) -> (Console, Vec<Effect>) {
    let mut console = console();
    console.set_mode(Mode::MainMenu);
    console.config.video.width = 1280;
    console.config.video.height = 720;
    let _ = console.tick(10.);
    let mut config = console.config.clone();
    config.video.width = width;
    config.video.height = height;
    let effects = console.apply_settings(config);
    (console, effects)
}

fn applied_size(effects: &[Effect]) -> Option<(u32, u32)> {
    effects.iter().find_map(|e| match e {
        Effect::ApplyConfig(c) => Some((c.video.width, c.video.height)),
        _ => None,
    })
}

fn keep_video_open(console: &Console) -> bool {
    console
        .confirm_report()
        .is_some_and(|r| r["kind"] == "keep_video")
}

#[test]
fn an_aspect_change_applies_and_asks_to_keep_it() {
    let (console, effects) = applied_video_change(1024, 768);
    assert_eq!(applied_size(&effects), Some((1024, 768)), "applied at once");
    assert!(keep_video_open(&console));
    assert!(console.modal_dialog_open());
    let (mut loaded, _) = applied_video_change(1024, 768);
    loaded.level_loaded(true);
    assert!(
        keep_video_open(&loaded),
        "a background load does not decide it"
    );
    let (same_aspect, _) = applied_video_change(1920, 1080);
    assert!(
        same_aspect.confirm_report().is_none(),
        "16:9 to 16:9 needs no confirmation"
    );
}

#[test]
fn the_keep_video_box_counts_down_and_reverts_at_the_timeout() {
    let (mut console, _) = applied_video_change(1024, 768);
    let seconds = crate::fov::video_confirm_seconds();
    let text = |c: &Console| {
        c.confirm_report().unwrap()["text"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let first = text(&console);
    assert!(
        first.contains(&format!("{}", seconds.ceil() as u32)),
        "{first}"
    );
    assert!(console.tick(10. + seconds - 2.5).is_empty());
    assert!(text(&console).contains('3'), "{}", text(&console));
    let effects = console.tick(10. + seconds);
    assert_eq!(applied_size(&effects), Some((1280, 720)), "reverted");
    assert!(console.confirm_report().is_none());
    assert_eq!(
        (console.config.video.width, console.config.video.height),
        (1280, 720)
    );
}

#[test]
fn ok_keeps_the_change_and_cancel_escape_revert_it() {
    let enter = |console: &mut Console| {
        let mut effects = console.input(&Input {
            keys: [Key::Enter].into(),
            held_keys: [Key::Enter].into(),
            ..Default::default()
        });
        effects.extend(console.input(&Input::default()));
        effects
    };
    // Cancel has the focus: Enter reverts.
    let (mut console, _) = applied_video_change(1024, 768);
    assert_eq!(applied_size(&enter(&mut console)), Some((1280, 720)));
    assert!(console.confirm_report().is_none());
    // Tab to OK, Enter keeps it: nothing reverts later.
    let (mut console, _) = applied_video_change(1024, 768);
    let _ = console.input(&Input {
        keys: [Key::Tab].into(),
        ..Default::default()
    });
    assert_eq!(applied_size(&enter(&mut console)), None);
    assert!(console.confirm_report().is_none());
    assert!(console.tick(1000.).is_empty(), "no timeout after OK");
    assert_eq!(console.config.video.width, 1024);
    // Escape reverts; the effect comes with the next tick.
    let (mut console, _) = applied_video_change(1080, 1920);
    console.escape();
    assert!(console.confirm_report().is_none());
    assert_eq!(applied_size(&console.tick(11.)), Some((1280, 720)));
}

trait Presented {
    fn presented(self) -> Self;
}
impl Presented for hl2_ui::gameui::display::Resolution {
    fn presented(self) -> Self {
        use hl2_ui::extensions::VideoExtension;
        crate::ui::AdvancedPolicy.presented(self)
    }
}
use hl2_ui::gameui::display::Resolution;
#[test]
fn resolution_without_stripes_for_flexible() {
    // Fits within supported 9:16..=32:9 range: unchanged
    assert_eq!(
        Resolution::new(1920, 1080).presented(),
        Resolution::new(1920, 1080)
    );
    assert_eq!(
        Resolution::new(5120, 1440).presented(),
        Resolution::new(5120, 1440)
    );
    assert_eq!(
        Resolution::new(1080, 1920).presented(),
        Resolution::new(1080, 1920)
    );

    // Exceeds 32:9: pillarbox black stripes excluded
    assert_eq!(
        Resolution::new(8000, 1000).presented(),
        Resolution::new(3556, 1000)
    );

    // Taller than 9:16: letterbox black stripes excluded
    assert_eq!(
        Resolution::new(500, 2000).presented(),
        Resolution::new(500, 889)
    );
}
#[test]
fn injected_sliders_use_plugin_ranges() {
    use hl2_ui::gameui::{localize::Localization, settings::Value};
    // The vertical field of view: automatic at the slider's low end, else an explicit
    // value in the tuned range.
    let (vertical_min, vertical_max) = crate::fov::vertical_fov_range();
    let mut v = console().config;
    assert_eq!(
        Value::Custom(&crate::settings::SLIDERS[0]).get(&v),
        0.,
        "automatic by default"
    );
    Value::Custom(&crate::settings::SLIDERS[0]).set(&mut v, vertical_min - 1.);
    assert_eq!(
        crate::settings::vertical_fov(&v.video),
        vertical_min,
        "just below the range clamps up"
    );
    Value::Custom(&crate::settings::SLIDERS[0]).set(&mut v, vertical_min * 0.4);
    assert_eq!(
        crate::settings::vertical_fov(&v.video),
        0.,
        "the low end is automatic"
    );
    Value::Custom(&crate::settings::SLIDERS[0]).set(&mut v, 999.);
    assert_eq!(crate::settings::vertical_fov(&v.video), vertical_max);
    let mut loc = Localization::default();
    loc.insert("#Spec_Auto", "Auto");
    Value::Custom(&crate::settings::SLIDERS[0]).set(&mut v, 0.);
    assert_eq!(
        Value::Custom(&crate::settings::SLIDERS[0]).label(&v, &loc),
        "Auto"
    );
    Value::Custom(&crate::settings::SLIDERS[0]).set(&mut v, 90.);
    assert_eq!(
        Value::Custom(&crate::settings::SLIDERS[0]).label(&v, &loc),
        "90"
    );
    assert_eq!(Value::Fov.label(&v, &loc), "75");
    // The viewmodel field of view keeps to its tuned range.
    let (view_min, view_max) = crate::fov::viewmodel_fov_range();
    assert_eq!(
        Value::Custom(&crate::settings::SLIDERS[1]).get(&v),
        54.,
        "retail default"
    );
    Value::Custom(&crate::settings::SLIDERS[1]).set(&mut v, 1.);
    assert_eq!(crate::settings::viewmodel_fov(&v.video), view_min);
    Value::Custom(&crate::settings::SLIDERS[1]).set(&mut v, 500.);
    assert_eq!(crate::settings::viewmodel_fov(&v.video), view_max);
}

#[test]
fn advanced_tuning_has_usable_ranges() {
    use crate::fov::{vertical_fov_range, viewmodel_fov_range, world_fov_offset};
    assert_eq!(crate::fov::video_confirm_seconds(), 15.);
    for (min, max) in [vertical_fov_range(), viewmodel_fov_range()] {
        assert!(0. < min && min < max && max < 180., "{min} {max}");
    }
    for aspect in [5. / 4., 4. / 3., 16. / 10., 16. / 9.] {
        assert_eq!(world_fov_offset(aspect), 0.0, "{aspect}");
    }
    assert!(world_fov_offset(9. / 16.) < -10.0);
    assert!(world_fov_offset(32. / 9.) < -5.0);
}
