use super::*;
use crate::gameui::options::{Sub, Tab, Target};

#[test]
fn options_slider_text_entry_cursor_blinks_when_focused_and_hides_when_unfocused() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);

    let has_caret = |quads: &[source_ui::canvas::Quad]| {
        quads.iter().any(|q| {
            q.destination.x >= 793.0
                && q.destination.x <= 841.0
                && q.destination.y == 259.0
                && q.destination.w == 1.0
                && q.destination.h == 16.0
        })
    };

    // Before clicking the text entry, it is not focused and shows no caret.
    let quads_init = drawn_at(&h, 0.0);
    assert!(
        !has_caret(&quads_init),
        "unfocused slider entry must not draw caret"
    );

    // Click the mouse sensitivity text entry field at (410, 50).
    h.click(page(410., 50.));
    assert!(
        h.dialog.editing.is_some(),
        "clicking entry activates editing"
    );

    // In focus: caret blinks at 2.0 Hz (visible at 0.0s, invisible at 0.5s, visible at 1.0s).
    let quads_0 = drawn_at(&h, 0.0);
    assert!(
        has_caret(&quads_0),
        "caret must be visible at phase on (t=0.0)"
    );

    let quads_half = drawn_at(&h, 0.5);
    assert!(!has_caret(&quads_half), "caret must blink off at t=0.5");

    let quads_1 = drawn_at(&h, 1.0);
    assert!(has_caret(&quads_1), "caret must blink back on at t=1.0");

    // Parent dialog losing focus (e.g. dev console opened on top of Options):
    // Caret must be strictly hidden even during the phase-on times!
    h.dialog.parent_focused = false;
    let quads_unfocused_0 = drawn_at(&h, 0.0);
    assert!(
        !has_caret(&quads_unfocused_0),
        "caret must be hidden when parent dialog is out of focus (t=0.0)"
    );
    let quads_unfocused_1 = drawn_at(&h, 1.0);
    assert!(
        !has_caret(&quads_unfocused_1),
        "caret must be hidden when parent dialog is out of focus (t=1.0)"
    );
    h.dialog.parent_focused = true;

    // When a modal subdialog (e.g. Video Advanced) is open over Options, parent Options
    // is in the background and its child inputs must not show carets.
    h.dialog.sub = Some(Sub::Gamma);
    let quads_sub = drawn_at(&h, 0.0);
    assert!(
        !has_caret(&quads_sub),
        "caret must be hidden when subdialog is open over Options"
    );
    h.dialog.sub = None;

    // When editing is committed / finished, caret must disappear.
    h.dialog.commit_entry();
    assert!(h.dialog.editing.is_none());
    let quads_committed = drawn_at(&h, 0.0);
    assert!(
        !has_caret(&quads_committed),
        "caret must be hidden when entry is committed"
    );
}

#[test]
fn unfocused_options_dialog_disables_button_hover() {
    let mut h = Harness::new();
    h.tab(Tab::Keyboard);

    let [_, cancel, _] = crate::gameui::options::button_rects();
    let cancel_pos = DIALOG + cancel.center();

    // 1. When parent_focused is false (Options dialog is out of focus):
    h.dialog.parent_focused = false;
    h.hover(cancel_pos);

    assert_eq!(
        h.dialog.armed, None,
        "buttons must not arm when dialog is out of focus"
    );

    // In draw_with_occluders with parent_focused = false, close button and buttons must not be armed
    let quads_unfocused = drawn_at(&h, 0.0);
    assert!(!quads_unfocused.is_empty());

    // 2. When parent_focused is true (Options dialog has focus):
    h.dialog.parent_focused = true;
    h.hover(cancel_pos);

    assert_eq!(
        h.dialog.armed,
        Some(Target::Cancel),
        "buttons must arm on hover when dialog is in focus"
    );
}

#[test]
fn focus_rings_enclose_check_boxes_and_mark_tabs_and_sliders() {
    let mut h = Harness::new();
    h.tab(Tab::Mouse);
    assert!(
        drawn(&h).iter().all(|q| q.color != h.theme.focus_border),
        "focus taken by a click is not drawn"
    );
    shift_tab(&mut h);
    h.key(Key::Tab);
    let boxes = |h: &Harness| -> Vec<Rect> {
        drawn(h)
            .into_iter()
            .filter(|q| q.color == h.theme.focus_border && q.destination.w == 1.)
            .map(|q| q.destination)
            .collect()
    };
    let tab = h.dialog.tab_rects(&|t| t.chars().count() as f32 * 7.)[2];
    assert!(
        boxes(&h).iter().all(|d| d.y >= DIALOG.y + tab.y),
        "the focused tab has a box"
    );
    assert!(!boxes(&h).is_empty());
    h.key(Key::Tab);
    let res = h.dialog.res.clone();
    let list = layout_widgets(
        &res,
        Panel::Mouse,
        page_rect(),
        &h.dialog.draft,
        &h.dialog.applied,
    );
    let check = list
        .iter()
        .find(|w| w.control.name == "ReverseMouse")
        .unwrap()
        .rect;
    let left = boxes(&h).iter().map(|d| d.x).fold(f32::INFINITY, f32::min);
    let top = boxes(&h).iter().map(|d| d.y).fold(f32::INFINITY, f32::min);
    let square = DIALOG.x + check.x + 2.;
    let square_top = DIALOG.y + check.y + (check.h - widgets::CHECK_SIZE) * 0.5;
    assert!(left < square, "the ring encloses the check box");
    assert_eq!(
        square - left,
        square_top - top,
        "as far beside the box as above it"
    );
    h.key(Key::Tab);
    let slider = list
        .iter()
        .find(|w| w.control.name == "Slider")
        .unwrap()
        .rect;
    assert!(
        boxes(&h)
            .iter()
            .all(|d| d.y >= DIALOG.y + slider.y - widgets::FOCUS_PAD
                && d.y <= DIALOG.y + slider.y + slider.h + widgets::FOCUS_PAD),
        "the focused slider has a padded ring"
    );
    assert!(!boxes(&h).is_empty());
}
