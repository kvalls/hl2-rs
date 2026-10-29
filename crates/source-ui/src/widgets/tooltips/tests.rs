use super::*;
use crate::{
    canvas::Canvas,
    widgets::{Fonts, Theme},
};

#[test]
fn stacked_panels_keep_order_gap_and_screen_clamping() {
    let canvas = Canvas::default();
    let fonts = Fonts::default();
    let theme = Theme::default();
    let p = Painter {
        canvas: &canvas,
        fonts: &fonts,
        theme: &theme,
        origin: Vec2::ZERO,
        scale: 1.,
    };
    let blue = Color::from_rgba(110, 220, 255, 240);
    let prepared = p.prepare_tooltip(&[
        TooltipSection {
            text: "Retail hint",
            fill: theme.tooltip_bg,
        },
        TooltipSection {
            text: "Development report with enough words to wrap onto several lines",
            fill: blue,
        },
    ]);
    let bounds = Rect::new(0., 0., 640., 360.);
    for pointer in [
        vec2(10., 10.),
        vec2(635., 355.),
        vec2(635., 10.),
        vec2(10., 355.),
    ] {
        let rects: Vec<_> = prepared.panels(pointer, bounds).collect();
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[1].y, rects[0].y + rects[0].h + TOOLTIP_GAP);
        for r in &rects {
            assert!(r.x >= 0. && r.y >= 0. && r.x + r.w <= bounds.w && r.y + r.h <= bounds.h);
        }
        p.draw_tooltip(&prepared, pointer, bounds);
        let quads = canvas.drain();
        assert!(quads.iter().any(|q| q.color == theme.tooltip_bg));
        assert!(quads.iter().any(|q| q.color == blue));
    }
}

#[test]
fn empty_retail_section_does_not_leave_a_gap_or_orange_fill() {
    let canvas = Canvas::default();
    let fonts = Fonts::default();
    let theme = Theme::default();
    let p = Painter {
        canvas: &canvas,
        fonts: &fonts,
        theme: &theme,
        origin: Vec2::ZERO,
        scale: 1.,
    };
    let blue = Color::from_rgba(110, 220, 255, 240);
    let prepared = p.prepare_tooltip(&[
        TooltipSection {
            text: "",
            fill: theme.tooltip_bg,
        },
        TooltipSection {
            text: "Status",
            fill: blue,
        },
    ]);
    assert_eq!(
        prepared
            .panels(Vec2::ZERO, Rect::new(0., 0., 640., 360.))
            .count(),
        1
    );
    p.draw_tooltip(&prepared, Vec2::ZERO, Rect::new(0., 0., 640., 360.));
    assert!(canvas.drain().iter().all(|q| q.color != theme.tooltip_bg));
    p.tooltip("", Vec2::ZERO, Rect::default());
    assert!(canvas.drain().is_empty());
}
