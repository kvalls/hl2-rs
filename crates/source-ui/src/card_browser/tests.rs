use super::*;
use crate::input::{Input, Key};
fn test_scale(canvas: Vec2) -> f32 {
    canvas.y.min(canvas.x / 1.25) / 720.
}
fn fixture(count: usize) -> CardBrowser {
    let cards = (0..count)
        .map(|i| Card {
            heading: format!("Item {i}"),
            title: format!("Name {i}"),
            image: None,
        })
        .collect();
    CardBrowser::new(
        BrowserSpec {
            cards,
            release_sound: Some("release".into()),
            ..Default::default()
        },
        Scaling {
            dialog_scale: test_scale,
            design_height: |s| s.y.min(s.x / 1.25),
        },
    )
}

#[test]
fn pointer_capture_selects_on_release_and_accepts_the_card_index() {
    let mut dialog = fixture(7);
    let canvas = vec2(960., 720.);
    let frame = dialog.frame(canvas);
    let pointer = |rect: Rect| Some(frame.origin + vec2(rect.x + 2., rect.y + 2.) * frame.scale);
    let card = dialog.panel_rect(1);
    assert_eq!(
        dialog.input(
            &Input {
                pointer: pointer(card),
                click: true,
                held: true,
                ..Default::default()
            },
            canvas
        ),
        Outcome::None
    );
    assert_eq!(dialog.selected, 0);
    dialog.input(
        &Input {
            pointer: None,
            held: true,
            ..Default::default()
        },
        canvas,
    );
    dialog.input(
        &Input {
            pointer: pointer(card),
            released: true,
            ..Default::default()
        },
        canvas,
    );
    assert_eq!(dialog.selected, 1);
    assert_eq!(dialog.take_sounds(), ["release"]);
    let play = dialog
        .targets()
        .find(|(t, _)| *t == Target::Accept)
        .unwrap()
        .1;
    dialog.input(
        &Input {
            pointer: pointer(play),
            click: true,
            held: true,
            ..Default::default()
        },
        canvas,
    );
    assert_eq!(
        dialog.input(
            &Input {
                pointer: pointer(play),
                released: true,
                ..Default::default()
            },
            canvas
        ),
        Outcome::Accept(1)
    );
}

#[test]
fn keyboard_pages_clamps_and_activates_only_on_release() {
    let mut dialog = fixture(7);
    let canvas = vec2(1280., 720.);
    for _ in 0..10 {
        dialog.input(
            &Input {
                keys: [Key::Right].into(),
                ..Default::default()
            },
            canvas,
        );
    }
    assert_eq!((dialog.selected, dialog.first), (6, 6));
    assert!(!dialog.targets().any(|(t, _)| t == Target::Next));
    assert_eq!(
        dialog.input(
            &Input {
                keys: [Key::Enter].into(),
                held_keys: [Key::Enter].into(),
                ..Default::default()
            },
            canvas
        ),
        Outcome::None
    );
    assert_eq!(dialog.input(&Input::default(), canvas), Outcome::Accept(6));
    dialog.open();
    assert_eq!((dialog.selected, dialog.first), (0, 0));
}

#[test]
fn cancel_tab_focus_and_scaled_frames_use_shared_controls() {
    let mut dialog = fixture(1);
    let canvas = vec2(1080., 1920.);
    let b = dialog.bounds(canvas);
    assert!(b.x >= 0. && b.y >= 0. && b.x + b.w <= canvas.x && b.y + b.h <= canvas.y);
    // One card, Play, Cancel, Close; Tab follows reading order without a default button.
    for _ in 0..2 {
        dialog.input(
            &Input {
                keys: [Key::Tab].into(),
                ..Default::default()
            },
            canvas,
        );
    }
    assert_eq!(dialog.focus, Some(Target::Cancel));
    assert_eq!(
        dialog.input(
            &Input {
                keys: [Key::Space].into(),
                ..Default::default()
            },
            canvas
        ),
        Outcome::Close
    );
}

#[test]
fn padded_card_texture_is_clipped_at_native_size_before_ui_scaling() {
    use crate::{
        canvas::Canvas,
        widgets::{Fonts, Painter, Theme},
    };
    let mut dialog = fixture(1);
    dialog.spec.cards[0].image = Some(Texture2D::from_rgba8(256, 128, &vec![255; 256 * 128 * 4]));
    let canvas = Canvas::default();
    let theme = Theme::default();
    let fonts = Fonts::default();
    let p = Painter {
        canvas: &canvas,
        fonts: &fonts,
        theme: &theme,
        origin: Vec2::ZERO,
        scale: 1.,
    };
    dialog.draw(&p, vec2(1024., 768.));
    let commands = canvas.drain();
    let quad = commands
        .iter()
        .find(|q| q.texture.is_some())
        .expect("card image");
    assert_eq!(quad.source, Rect::new(0., 0., 152., 86.));
    let scale = test_scale(vec2(1024., 768.));
    assert_eq!(quad.destination.w, (152. * scale).round());
    assert_eq!(quad.destination.h, (86. * scale).round());
}

#[test]
fn single_card_layout_centers_the_card_and_sizes_decorative_rules_to_the_frame() {
    use crate::{
        canvas::Canvas,
        widgets::{Fonts, Painter, Theme},
    };
    let mut dialog = fixture(1);
    dialog.spec.layout.size = vec2(300., 296.);
    dialog.spec.layout.dividers = vec![Rect::new(24., 34., 548., 2.)];
    dialog.spec.layout.accept = Rect::new(63., 252., 124., 24.);
    dialog.spec.layout.cancel = Rect::new(200., 252., 72., 24.);
    assert_eq!(dialog.panel_rect(0).x, 66.);
    let canvas = Canvas::default();
    let theme = Theme::default();
    let fonts = Fonts::default();
    let size = vec2(1024., 768.);
    dialog.draw(
        &Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        },
        size,
    );
    let bounds = dialog.bounds(size);
    for q in canvas.drain() {
        assert!(q.destination.x >= bounds.x && q.destination.y >= bounds.y);
        assert!(q.destination.x + q.destination.w <= bounds.x + bounds.w + 1.);
        assert!(q.destination.y + q.destination.h <= bounds.y + bounds.h + 1.);
    }
}

#[test]
fn occlusion_preserves_selection_and_cancels_hidden_key_activation() {
    let mut dialog = fixture(5);
    let canvas = vec2(960., 720.);
    dialog.input(
        &Input {
            keys: [Key::Right].into(),
            ..Default::default()
        },
        canvas,
    );
    assert_eq!(dialog.selected, 1);
    dialog.input(
        &Input {
            keys: [Key::Enter].into(),
            held_keys: [Key::Enter].into(),
            ..Default::default()
        },
        canvas,
    );
    dialog.occlude(true);
    assert_eq!(dialog.selected, 1);
    assert_eq!(dialog.input(&Input::default(), canvas), Outcome::None);
    assert_eq!(
        dialog.input(
            &Input {
                keys: [Key::Enter].into(),
                ..Default::default()
            },
            canvas
        ),
        Outcome::Accept(1)
    );
}

#[test]
fn footer_toggle_checks_on_click_and_draws_card_badges_when_checked() {
    use crate::{
        canvas::Canvas,
        widgets::{Fonts, Painter, Theme},
    };
    let mut dialog = fixture(2);
    dialog.spec.layout.card.badge = Some(ScaledImageLayout {
        rect: Rect::new(10., 100., 43., 42.),
        scale: 0.5,
    });
    dialog.spec.layout.toggle = Some(BrowserToggleLayout {
        backing: Some(Rect::new(22., 254., 308., 40.)),
        icon: Some(ScaledImageLayout {
            rect: Rect::new(25., 258., 43., 42.),
            scale: 0.5,
        }),
        check_box: Rect::new(60., 262., 264., 24.),
    });
    dialog.spec.toggle_label = Some("Toggle option".into());
    dialog.spec.toggle_icon = Some(Texture2D::from_rgba8(64, 64, &vec![255; 64 * 64 * 4]));
    let size = vec2(960., 720.);
    let frame = dialog.frame(size);
    let pointer = Some(frame.origin + vec2(70., 270.) * frame.scale);

    let canvas = Canvas::default();
    let theme = Theme::default();
    let fonts = Fonts::default();
    let p = Painter {
        canvas: &canvas,
        fonts: &fonts,
        theme: &theme,
        origin: Vec2::ZERO,
        scale: 1.,
    };
    dialog.draw(&p, size);
    let unchecked_textures = canvas
        .drain()
        .iter()
        .filter(|q| q.texture.is_some())
        .count();
    assert_eq!(unchecked_textures, 1);

    dialog.input(
        &Input {
            pointer,
            click: true,
            held: true,
            ..Default::default()
        },
        size,
    );
    dialog.input(
        &Input {
            pointer,
            released: true,
            ..Default::default()
        },
        size,
    );
    assert!(dialog.toggle_checked);

    dialog.draw(&p, size);
    let checked_textures = canvas
        .drain()
        .iter()
        .filter(|q| q.texture.is_some())
        .count();
    assert_eq!(checked_textures, 3);

    dialog.open();
    assert!(!dialog.toggle_checked);
}
