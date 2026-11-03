use super::*;

/// Every character is 6 pixels wide (bold 7), lines are 12 tall.
struct Fixed;
impl Measure for Fixed {
    fn tall(&self) -> f32 {
        12.
    }
    fn width(&self, style: Style, text: &str) -> f32 {
        let w = if matches!(style, Style::Bold | Style::BoldItalic) {
            7.
        } else {
            6.
        };
        text.chars().count() as f32 * w
    }
}

fn on() -> Settings {
    Settings {
        closecaption: true,
        ..Settings::default()
    }
}

fn item(stream: &str, from_player: bool) -> Item {
    Item {
        stream: stream.into(),
        time_to_live: 1.,
        initial_life_span: 1.,
        added_time: 0.,
        predisplay: 0.,
        valid: true,
        from_player,
        work: None,
        width: 0.,
        height: 0.,
    }
}

#[test]
fn options_setting_maps_to_the_caption_cvars() {
    use crate::config::Captions;
    let none = Settings::from_config(Captions::None);
    assert!(!none.closecaption);
    let all = Settings::from_config(Captions::ClosedCaptions);
    assert!(all.closecaption && !all.subtitles);
    let subtitles = Settings::from_config(Captions::Subtitles);
    assert!(subtitles.closecaption && subtitles.subtitles);
    assert_eq!((all.linger_time, all.predisplay_time), (1., 0.25));
}

#[test]
fn commands_split_like_the_sdk() {
    let chars: Vec<char> = "<clr:255,255,200>Hi".chars().collect();
    let mut i = 0;
    assert_eq!(
        split_command(&chars, &mut i),
        Some(("clr".into(), "255,255,200".into()))
    );
    assert_eq!(chars[i], '>');
    assert_eq!(
        commands("<sfx><norepeat:30>[Gunfire]<len:2.5>"),
        [
            ("sfx".to_string(), String::new()),
            ("norepeat".into(), "30".into()),
            ("len".into(), "2.5".into())
        ]
    );
    assert_eq!(float_command("<len:2.5s>", "len"), Some(2.5));
    assert_eq!(float_command("x", "len"), None);
    assert_eq!(leading_float("-3"), -3.);
}

#[test]
fn items_get_duration_plus_linger_and_wait_for_the_predisplay_delay() {
    let mut cc = CloseCaption::new();
    cc.process("<clr:255,255,200><I>Ahem...is this on?", 4., false, &on());
    assert_eq!(cc.items.len(), 1);
    let item = &cc.items[0];
    assert_eq!(item.time_to_live, 5.);
    assert_eq!(item.predisplay, 0.25);
    cc.tick(0.2);
    assert_eq!(cc.items[0].time_to_live, 5., "still waiting");
    cc.tick(0.1);
    cc.tick(1.);
    assert!((cc.items[0].time_to_live - 4.).abs() < 1e-5);
    for _ in 0..5 {
        cc.tick(1.);
    }
    assert!(cc.is_empty(), "expired");
}

#[test]
fn delay_splits_phrases_and_len_overrides_the_life() {
    let mut cc = CloseCaption::new();
    cc.process("First.<delay:2>Second.<len:3>", 1., false, &on());
    let streams: Vec<_> = cc.streams().collect();
    assert_eq!(streams, ["First.", "Second.<len:3>"]);
    assert_eq!(cc.items[0].predisplay, 0.25);
    assert_eq!(cc.items[1].predisplay, 2.25);
    assert_eq!(cc.items[1].time_to_live, 3.);
    assert_eq!(cc.items[1].initial_life_span, 3.);
    // A later, shorter caption lives as long as the previous one with the difference
    // marked as added time.
    cc.process("Third.", 0.5, false, &on());
    assert_eq!(cc.items[2].time_to_live, 3.);
    assert_eq!(cc.items[2].added_time, 1.5);
}

#[test]
fn subtitles_drop_sound_effects_and_disabled_captions_clear_the_queue() {
    let mut cc = CloseCaption::new();
    cc.process("<sfx>[Explosion]", 1., false, &on());
    assert_eq!(cc.items[0].predisplay, 0., "sfx show instantly");
    let subtitles = Settings {
        subtitles: true,
        ..on()
    };
    cc.process("<sfx>[Gunfire]", 1., false, &subtitles);
    assert_eq!(cc.len(), 1);
    cc.process("Voice.", 1., false, &subtitles);
    assert_eq!(cc.len(), 2);
    cc.process("Voice.", 1., false, &Settings::default());
    assert!(cc.is_empty());
    cc.process("-->untranslated", 1., false, &on());
    cc.process("   ", 1., false, &on());
    assert!(cc.is_empty());
    cc.process("!!!pending", 1., false, &on());
    assert!(!cc.items[0].valid);
}

#[test]
fn norepeat_and_same_tick_repeats_are_suppressed() {
    let mut cc = CloseCaption::new();
    let lookup = |token: &str| match token {
        "barney.yell" => Some("<norepeat:10>Hey!".to_owned()),
        "alyx.hi" => Some("Hi.".to_owned()),
        "alyx.hi_female" => Some("Hi there.".to_owned()),
        _ => None,
    };
    let request = |token: &str| Request {
        token: token.into(),
        duration: 1.,
        from_player: false,
        gender: None,
    };
    assert!(cc.emit(&lookup, &request("Barney.Yell"), &on(), 1.));
    assert!(cc.emit(&lookup, &request("barney.yell"), &on(), 5.));
    assert_eq!(cc.len(), 1, "within the norepeat interval");
    cc.tick(0.);
    assert!(cc.emit(&lookup, &request("barney.yell"), &on(), 11.5));
    assert_eq!(cc.len(), 2);
    cc.emit(&lookup, &request("alyx.hi"), &on(), 12.);
    cc.emit(&lookup, &request("alyx.hi"), &on(), 12.);
    assert_eq!(cc.len(), 3, "same tick");
    cc.tick(0.);
    cc.emit(
        &lookup,
        &Request {
            gender: Some("female"),
            ..request("alyx.hi")
        },
        &on(),
        12.1,
    );
    assert_eq!(cc.streams().last(), Some("Hi there."));
    assert!(!cc.emit(&lookup, &request("missing.token"), &on(), 13.));
    assert_eq!(cc.missing, ["missing.token"]);
}

#[test]
fn work_units_follow_color_style_and_wrap_at_spaces() {
    let mut unit = item("<clr:255,255,200><I>aaa bbb ccc<B>dd", false);
    compute_work(&mut unit, 50., &Fixed);
    let work = unit.work.unwrap();
    let texts: Vec<_> = work.iter().map(|w| w.text.as_str()).collect();
    // 50 pixels hold eight 6-pixel characters: "aaa bbb " then wrap at the space.
    assert_eq!(texts, ["aaa bbb", "ccc", "dd"]);
    assert_eq!(work[0].color, [255, 255, 200, 255]);
    assert_eq!(work[0].style, Style::Italic);
    assert_eq!((work[0].y, work[1].y), (0., 12.));
    assert_eq!(work[0].width, 48., "the trailing space keeps its width");
    assert_eq!(work[2].style, Style::BoldItalic);
    assert_eq!(work[2].x, 18.);
    assert_eq!(unit.height, 24.);
}

#[test]
fn color_stack_pops_and_player_colors_pick_the_speaker() {
    let mut unit = item("<clr:1,2,3>a<clr:4,5,6>b<clr>c<cr>d", false);
    compute_work(&mut unit, 500., &Fixed);
    let work = unit.work.unwrap();
    let colors: Vec<_> = work.iter().map(|w| (w.text.as_str(), w.color)).collect();
    assert_eq!(
        colors,
        [
            ("a", [1, 2, 3, 255]),
            ("b", [4, 5, 6, 255]),
            ("c", [1, 2, 3, 255]),
            ("d", [1, 2, 3, 255])
        ]
    );
    assert_eq!(work[3].y, 12., "<cr> starts a new line");
    for (from_player, expected) in [(true, [9, 9, 9, 255]), (false, [7, 7, 7, 255])] {
        let mut unit = item("<playerclr:9,9,9:7,7,7>x", from_player);
        compute_work(&mut unit, 500., &Fixed);
        assert_eq!(unit.work.unwrap()[0].color, expected);
    }
}

#[test]
fn paint_grows_the_box_and_fades_the_item_in() {
    let layout = Layout {
        top_offset: 0.,
        bg_alpha: 128.,
        ..Layout::default()
    };
    let mut cc = CloseCaption::new();
    cc.process("<clr:255,255,200>Hello", 2., false, &on());
    cc.tick(0.3);
    let background = [0., 0., 0., 76. / 255.];
    let paint = cc.paint((500., 136.), 1., &layout, &Fixed, background, 10.);
    // Growth starts at the old height (0); the box starts opaque (current alpha 1).
    let (rect, color) = paint.background.unwrap();
    assert_eq!(rect, [0., 124., 500., 12.]);
    assert!((color[3] - 128. / 255.).abs() < 1e-6);
    let paint = cc.paint((500., 136.), 1., &layout, &Fixed, background, 10.25);
    let (rect, color) = paint.background.unwrap();
    assert_eq!(rect, [0., 112., 500., 24.]);
    assert!((color[3] - 128. / 255.).abs() < 1e-6);
    let run = &paint.runs[0];
    assert_eq!(run.text, "Hello");
    assert_eq!((run.x, run.y), (9., 118.));
    assert_eq!(run.color[3], 0., "hidden during ItemHiddenTime");
    cc.tick(0.3);
    let paint = cc.paint((500., 136.), 1., &layout, &Fixed, background, 10.55);
    let run = &paint.runs[0];
    assert!((run.color[3] - 0.6667).abs() < 1e-3, "{}", run.color[3]);
    assert_eq!(&run.color[..3], &[1., 1., 200. / 255.]);
}

#[test]
fn old_items_are_popped_when_the_panel_is_full() {
    let layout = Layout {
        top_offset: 0.,
        ..Layout::default()
    };
    let mut cc = CloseCaption::new();
    for n in 0..12 {
        cc.process(&format!("line {n}"), 5., false, &on());
    }
    cc.tick(0.3);
    let paint = cc.paint((500., 136.), 1., &layout, &Fixed, [0.; 4], 1.);
    // 136 - 2 * 6 = 124 pixels hold ten 12-pixel lines.
    assert_eq!(cc.items.iter().filter(|i| i.time_to_live == 0.).count(), 2);
    assert!(paint.runs.iter().all(|r| r.text != "line 0"));
    cc.tick(0.01);
    assert_eq!(cc.len(), 10);
}

#[test]
fn caption_faces_come_from_registered_files_with_the_requested_slant() {
    let file = |family: &str, weight: u16, italic: bool| FontFile {
        bytes: Vec::new(),
        info: source_ui::scheme_fonts::FontFileInfo {
            families: vec![family.into()],
            weight,
        },
        italic,
    };
    let files = [
        file("HalfLife2", 400, false),
        file("DejaVu Sans", 400, false),
        file("DejaVu Sans", 700, false),
        file("DejaVu Sans", 700, true),
        file("DejaVu Sans", 400, true),
        file("GorDIN", 600, false),
    ];
    let font = |name: &str, weight, italic| FontDefinition {
        name: name.into(),
        tall: 12.,
        weight,
        italic,
    };
    let picked = |definition| {
        let f = pick_file(&files, &definition).unwrap();
        (f.info.families[0].clone(), f.info.weight, f.italic)
    };
    // Tahoma is not installed: the registered substitute family, by slant then weight.
    assert_eq!(
        picked(font("Tahoma", 500, false)),
        ("DejaVu Sans".into(), 400, false)
    );
    assert_eq!(
        picked(font("Tahoma", 500, true)),
        ("DejaVu Sans".into(), 400, true)
    );
    assert_eq!(
        picked(font("Tahoma", 900, false)),
        ("DejaVu Sans".into(), 700, false)
    );
    assert_eq!(
        picked(font("Tahoma", 900, true)),
        ("DejaVu Sans".into(), 700, true)
    );
    // An installed family is used as named, even without an italic file.
    assert_eq!(
        picked(font("GorDIN", 600, true)),
        ("GorDIN".into(), 600, false)
    );
}

#[test]
#[ignore = "requires an owned Half-Life 2 installation"]
fn owned_episode_one_caption_panel_reads_layout_scheme_and_fonts() {
    let root = source_assets::install::discover().expect("installed game");
    let vfs = source_assets::vpk::Vfs::mount_game(&root, "episodic").unwrap();
    let parse = |path: &str| {
        source_assets::keyvalues::parse_resource(&vfs.read(path).unwrap().unwrap())
            .unwrap()
            .remove(0)
    };
    let scheme = parse("resource/ClientScheme.res");
    let layout = parse("scripts/HudLayout.res");
    let hud = CaptionHud::load(&vfs, &scheme, &layout, Default::default(), LANGUAGE);
    assert!(hud.errors.is_empty(), "{:?}", hud.errors);
    let lengths: Vec<_> = hud.captions.dictionaries.iter().map(|d| d.len()).collect();
    assert_eq!(lengths, [3153, 3970]);
    assert_eq!(
        hud.layout,
        Layout {
            xpos: "c-250".into(),
            ypos: 276.,
            wide: 500.,
            tall: 136.,
            bg_alpha: 128.,
            grow_time: 0.25,
            item_hidden_time: 0.2,
            item_fade_in_time: 0.15,
            item_fade_out_time: 0.3,
            top_offset: 0.,
        }
    );
    assert_eq!(hud.background, [0., 0., 0., 76. / 255.]);
    assert_eq!(hud.fonts.len(), 4);
    assert!(hud.fonts.iter().all(|(_, _, tall)| *tall == 12.));
    // 768 lines: 12 * 1.6 = 19 pixel caption text.
    let (_, tall) = hud.face(Style::Italic, 768. / 480.).unwrap();
    assert_eq!(tall, 19.);
}

#[test]
fn durations_are_quantized_like_the_message() {
    assert!((message_duration(4.36) - 4.3).abs() < 1e-5);
    assert!((message_duration(36.8) - 25.5).abs() < 1e-5);
}

#[test]
fn long_styled_commentary_stays_inside_its_viewport_and_reflows() {
    for scale in [1., 1.5, 2.] {
        let layout = Layout {
            top_offset: 20.,
            grow_time: 0.,
            ..Layout::default()
        };
        let mut cc = CloseCaption::new();
        cc.process(
            &"one <B>two<B> three four five six seven eight nine ten ".repeat(30),
            60.,
            false,
            &on(),
        );
        cc.tick(0.5);
        let size = (240. * scale, 136. * scale);
        let mut shown = std::collections::BTreeSet::new();
        for step in 0..60 {
            cc.tick(0.5);
            let paint = cc.paint(
                size,
                scale,
                &layout,
                &Fixed,
                [0.; 4],
                1. + step as f64 * 0.5,
            );
            let (rect, _) = paint.background.unwrap();
            assert!(rect[1] >= layout.top_offset * scale);
            assert!(rect[1] + rect[3] <= size.1);
            for run in paint.runs {
                assert!(run.y >= rect[1] && run.y + run.height <= size.1);
                assert!(run.x + run.width <= size.0, "{run:?}");
                shown.insert((run.text, run.y as i32));
            }
        }
        assert!(shown.len() > 10);
        let old_height = cc.items[0].height;
        cc.paint((size.0 * 2., size.1), scale, &layout, &Fixed, [0.; 4], 32.);
        assert!(
            cc.items[0].height < old_height,
            "cached work must rewrap on resize"
        );
    }
}

#[test]
fn panning_counts_lines_instead_of_color_and_style_runs() {
    let mut plain = item(&"one two three four ".repeat(25), false);
    let mut styled = item(
        &"<clr:255,0,0>one<clr:0,255,0> two three four ".repeat(25),
        false,
    );
    compute_work(&mut plain, 120., &Fixed);
    compute_work(&mut styled, 120., &Fixed);
    // Match the same viewport overflow independently of style-run count.
    styled.height = plain.height;
    assert_eq!(
        super::paint::pan(&plain, 60., 12., 0.5),
        super::paint::pan(&styled, 60., 12., 0.5)
    );
}

#[test]
fn a_style_boundary_at_the_right_edge_wraps_before_the_next_glyph() {
    let mut unit = item("aaaa<B>bbbbbbbb", false);
    compute_work(&mut unit, 24., &Fixed);
    for run in unit.work.as_ref().unwrap() {
        assert!(run.x + run.width <= 24., "{run:?}");
    }
}

#[test]
fn an_idle_caption_queue_emits_no_invisible_background() {
    let mut cc = CloseCaption::new();
    for time in [1., 1.25, 1.5] {
        cc.paint((500., 136.), 1., &Layout::default(), &Fixed, [0.; 4], time);
    }
    assert_eq!(
        cc.paint((500., 136.), 1., &Layout::default(), &Fixed, [0.; 4], 2.),
        Paint::default()
    );
}
