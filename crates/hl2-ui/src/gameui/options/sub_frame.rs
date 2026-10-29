//! Layout and reflow calculations for options sub-dialog frames.
use super::{control_binding, settings, Binding, Command, ControlKind, Panel, Sub};
use crate::gameui::resources::GameUiResources;
use crate::gameui::settings::support;
use glam::{vec2, Vec2};
use source_ui::layout::Control;

/// Frame size of a sub-dialog: as authored ([`authored_frame_size`]), taller by the footer
/// reflow ([`footer_reflow`]) when content reaches into the footer.
pub(super) fn frame_size(res: &GameUiResources, sub: Sub) -> Vec2 {
    let size = authored_frame_size(res, sub);
    let Some(layout) = res.layout(sub.panel().layout()) else {
        return size;
    };
    let controls = drawn_controls(sub.panel(), layout);
    let (shift, _) = footer_reflow(sub.panel(), &controls, (size.x, size.y));
    size + vec2(0., shift)
}

/// Frame size of a sub-dialog as authored: its layout's frame record; for a layout without one,
/// the extent of its controls with the left margin repeated on the right and bottom; else the
/// fallback.
pub(super) fn authored_frame_size(res: &GameUiResources, sub: Sub) -> Vec2 {
    let Some(layout) = res.layout(sub.panel().layout()) else {
        return sub.fallback_size();
    };
    if let Some(frame) = layout
        .controls
        .iter()
        .find(|c| matches!(c.kind, ControlKind::Other(_)) && c.wide >= 200. && c.tall >= 100.)
    {
        return vec2(frame.wide, frame.tall);
    }
    let rects: Vec<(f32, f32, f32, f32)> = layout
        .controls
        .iter()
        .filter(|c| c.visible && !is_frame_chrome(c))
        .map(|c| c.rect((0., 0.)))
        .collect();
    let left = rects.iter().map(|r| r.0).fold(f32::INFINITY, f32::min);
    let right = rects.iter().map(|r| r.0 + r.2).fold(0., f32::max);
    let bottom = rects.iter().map(|r| r.1 + r.3).fold(0., f32::max);
    if rects.is_empty() || !left.is_finite() {
        return sub.fallback_size();
    }
    vec2(right + left, bottom + left)
}

/// Space kept between a dialog's lowest content control and its footer when the footer moves.
pub(crate) const FOOTER_GAP: f32 = 8.;

/// The controls a layout draws (visible, not frame chrome or the frame record itself).
pub(super) fn drawn_controls(panel: Panel, layout: &source_ui::layout::Layout) -> Vec<&Control> {
    layout
        .controls
        .iter()
        .filter(|c| {
            (c.visible || settings::force_visible(panel, &c.name))
                && support::available(panel, c, support::Platform::current())
        })
        .filter(|c| !matches!(c.kind, ControlKind::Other(_)) && !is_frame_chrome(c))
        .collect()
}

/// A dialog footer that overlaps content moves below it.
///
/// VGUI places controls absolutely. The installed Video > Advanced layout keeps older macOS
/// (`[$OSX]`) footer positions that predate its Classic Effects row, so OK/Cancel land on that
/// combo. The footer is the OK/Cancel buttons plus the unbound caption over them (a label
/// without `associate`, at most two button heights above the button row: the installed caption
/// sits 26 units above it on Windows, 6 on macOS). When content reaches
/// into the footer, the footer moves down to `FOOTER_GAP` below the lowest content control, and
/// the frame grows by the same amount. Returns the shift and whether each control is footer.
pub(super) fn footer_reflow(
    panel: Panel,
    controls: &[&Control],
    parent: (f32, f32),
) -> (f32, Vec<bool>) {
    let is_button = |c: &Control| {
        matches!(
            control_binding(panel, c),
            Some(Binding::Command(Command::SubOk | Command::SubCancel))
        )
    };
    let buttons: Vec<(f32, f32)> = controls
        .iter()
        .filter(|c| is_button(c))
        .map(|c| (c.rect(parent).1, c.tall))
        .collect();
    let Some((button_top, button_tall)) =
        buttons.iter().copied().min_by(|a, b| a.0.total_cmp(&b.0))
    else {
        return (0., vec![false; controls.len()]);
    };
    let footer: Vec<bool> = controls
        .iter()
        .map(|c| {
            is_button(c)
                || (c.kind == ControlKind::Label
                    && c.associate.is_empty()
                    && control_binding(panel, c).is_none()
                    && c.rect(parent).1 >= button_top - 2. * button_tall)
        })
        .collect();
    let footer_top = controls
        .iter()
        .zip(&footer)
        .filter(|(_, f)| **f)
        .map(|(c, _)| c.rect(parent).1)
        .fold(f32::INFINITY, f32::min);
    let content_bottom = controls
        .iter()
        .zip(&footer)
        .filter(|(_, f)| !**f)
        .map(|(c, _)| c.rect(parent).1 + c.tall)
        .fold(0., f32::max);
    ((content_bottom + FOOTER_GAP - footer_top).max(0.), footer)
}

/// Controls a VGUI `Frame` creates for itself (`frame_caption`, `frame_close`, resize grips):
/// the dialog frame draws its own title bar and close button.
pub(crate) fn is_frame_chrome(control: &Control) -> bool {
    control
        .name
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("frame_"))
}
