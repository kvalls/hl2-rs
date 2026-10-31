use super::{ASPECTS, AspectRatio, DisplayModes, Resolution, aspect_of, choices};
use hl2_ui::{
    extensions::VideoPolicy,
    gameui::display::{MonitorSpec, min_size},
};
fn r(w: u32, h: u32) -> Resolution {
    Resolution::new(w, h)
}
fn usable_modes(
    reported: impl IntoIterator<Item = Resolution>,
    desktop: Option<Resolution>,
    current: Resolution,
) -> Vec<Resolution> {
    choices(
        &DisplayModes {
            fullscreen: reported.into_iter().collect(),
            desktop,
            current: Some(current),
        },
        false,
    )
}
fn resolution_choices(modes: &[Resolution], aspect: AspectRatio) -> Vec<Resolution> {
    VideoPolicy::new(crate::ui::AdvancedPolicy).resolution_choices(modes, aspect)
}
fn choose_after_aspect_change(
    modes: &[Resolution],
    aspect: AspectRatio,
    current: Resolution,
) -> Option<Resolution> {
    VideoPolicy::new(crate::ui::AdvancedPolicy).choose_resolution(modes, aspect, current)
}
#[test]
fn aspect_buckets_match_the_video_page_grouping() {
    assert_eq!(aspect_of(1024, 768), AspectRatio::Normal);
    assert_eq!(aspect_of(1280, 1024), AspectRatio::Normal, "5:4 joins 4:3");
    assert_eq!(aspect_of(1680, 1050), AspectRatio::Wide16x10);
    assert_eq!(
        aspect_of(1512, 982),
        AspectRatio::Wide16x9,
        "1.54 is wider than 16:10's window"
    );
    assert_eq!(aspect_of(1920, 1080), AspectRatio::Wide16x9);
    assert_eq!(aspect_of(2160, 1080), AspectRatio::Wide16x9, "2:1");
    for (w, h) in [(2560, 1080), (3440, 1440), (3840, 1600), (5120, 2160)] {
        assert_eq!(aspect_of(w, h), crate::display::ULTRAWIDE_21X9, "{w}x{h}");
    }
    assert_eq!(aspect_of(3840, 1080), crate::display::ULTRAWIDE_32X9);
    assert_eq!(aspect_of(1080, 1080), crate::display::SQUARE_1X1);
    assert_eq!(aspect_of(768, 1024), crate::display::PORTRAIT_3X4);
    assert_eq!(aspect_of(1080, 1350), crate::display::PORTRAIT_3X4, "4:5");
    assert_eq!(aspect_of(1200, 1920), crate::display::PORTRAIT_10X16);
    assert_eq!(aspect_of(1080, 1920), crate::display::PORTRAIT_9X16);
    assert_eq!(
        aspect_of(1170, 2532),
        crate::display::PORTRAIT_9X16,
        "phone"
    );
    assert_eq!(aspect_of(5120, 1440), crate::display::ULTRAWIDE_32X9);
    assert_eq!(
        aspect_of(10, 0),
        crate::display::ULTRAWIDE_32X9,
        "zero height is safe"
    );
}

#[test]
fn ultrawide_panels_get_their_own_aspect_choices() {
    let modes = usable_modes([r(3440, 1440)], Some(r(3440, 1440)), r(1920, 1080));
    let wide = resolution_choices(&modes, crate::display::ULTRAWIDE_21X9);
    assert_eq!(wide[0], r(3440, 1440));
    assert!(wide.contains(&r(2560, 1080)));
    assert!(!modes.contains(&r(5120, 1440)), "larger than the panel");
    assert_eq!(
        resolution_choices(&modes, AspectRatio::Wide16x9)[0],
        r(2560, 1440),
        "16:9 no longer lists ultrawide modes"
    );
    let super_wide = usable_modes([], Some(r(5120, 1440)), r(1920, 1080));
    let thirty_two = resolution_choices(&super_wide, crate::display::ULTRAWIDE_32X9);
    assert_eq!(thirty_two, [r(5120, 1440), r(3840, 1080)]);
    assert_eq!(
        resolution_choices(&super_wide, AspectRatio::Normal)[0],
        r(1920, 1440),
        "4:3 up to the panel's height"
    );
}

#[test]
fn an_8k_panel_offers_every_aspect_up_to_its_size() {
    let panel = r(7680, 4320);
    let reported = [r(7680, 4320), r(3840, 2160), r(1920, 1080), r(1280, 720)];
    let modes = DisplayModes {
        fullscreen: usable_modes(reported, Some(panel), r(3840, 2160)),
        desktop: Some(panel),
        current: None,
    };
    let top = |aspect| resolution_choices(&choices(&modes, false), aspect)[0];
    assert_eq!(top(AspectRatio::Normal), r(5760, 4320));
    assert_eq!(top(AspectRatio::Wide16x9), r(7680, 4320));
    assert_eq!(top(AspectRatio::Wide16x10), r(6912, 4320));
    assert_eq!(top(crate::display::ULTRAWIDE_21X9), r(7680, 3240));
    assert_eq!(top(crate::display::ULTRAWIDE_32X9), r(7680, 2160));
    let all = choices(&modes, false);
    assert!(all.contains(&r(5120, 2880)) && all.contains(&r(640, 480)));
    assert_eq!(
        choices(&modes, true),
        all,
        "every mode fits an 8K desktop in pixels"
    );
}

#[test]
fn a_rotated_monitor_offers_portrait_and_landscape_modes() {
    let panel = r(1080, 1920);
    let modes = usable_modes([panel], Some(panel), r(1080, 1920));
    let top = |aspect| resolution_choices(&modes, aspect)[0];
    assert_eq!(top(crate::display::PORTRAIT_9X16), r(1080, 1920));
    assert_eq!(top(crate::display::PORTRAIT_10X16), r(1050, 1680));
    assert_eq!(top(crate::display::PORTRAIT_3X4), r(1080, 1440));
    assert_eq!(top(crate::display::SQUARE_1X1), r(1080, 1080));
    assert_eq!(top(AspectRatio::Normal), r(1056, 792), "landscape fits too");
    assert!(
        modes
            .iter()
            .all(|m| m.fits(panel) && m.at_least(min_size()))
    );
    assert!(
        r(480, 640).at_least(min_size()) && !r(480, 480).at_least(min_size()),
        "portrait counts with its sides swapped"
    );
    assert_eq!(
        "1080x1920".parse::<MonitorSpec>().unwrap().size,
        panel,
        "portrait monitors can be faked"
    );
}

#[test]
fn the_flexible_choice_lists_every_mode_and_keeps_the_resolution() {
    let modes = [r(1920, 1080), r(1680, 1050), r(1024, 768), r(1080, 1920)];
    assert_eq!(
        resolution_choices(&modes, crate::display::FIT_DISPLAY),
        modes
    );
    assert_eq!(
        choose_after_aspect_change(&modes, crate::display::FIT_DISPLAY, r(1024, 768)),
        Some(r(1024, 768))
    );
    assert_eq!(ASPECTS.first(), Some(&crate::display::FIT_DISPLAY));
    assert_eq!(
        ASPECTS[1..],
        ASPECTS[1..],
        "the fixed choices keep their combo positions"
    );
    assert!(!ASPECTS[1..].contains(&crate::display::FIT_DISPLAY));
    assert!(
        ASPECTS[1..]
            .iter()
            .all(|&a| a != crate::display::FIT_DISPLAY)
    );
}
