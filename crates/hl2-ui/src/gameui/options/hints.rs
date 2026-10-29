//! Development status overlays share retail hover/occlusion and generic tooltip geometry.
use super::{
    confirm_frame, layout_widgets, main_frame, page_rect, sub_frame, OptionsDialog, Panel, Widget,
};
use crate::gameui::settings::support;
use glam::Vec2;
use source_ui::{
    canvas::{Color, Rect},
    input::Input,
    widgets::{Painter, PreparedTooltip, TooltipSection},
};

pub const DEVELOPMENT_FILL: Color = Color::from_rgba(110, 220, 255, 240);

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveTooltip {
    /// Original retail text, empty for a development-only hint.
    pub text: String,
    pub development: Option<String>,
    pub pointer: Vec2,
    target: (Panel, usize),
    prepared: PreparedTooltip,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct HintReport {
    pub retail: String,
    pub development: Option<String>,
}

impl From<&ActiveTooltip> for HintReport {
    fn from(tip: &ActiveTooltip) -> Self {
        Self {
            retail: tip.text.clone(),
            development: tip.development.clone(),
        }
    }
}

impl OptionsDialog {
    /// Returns the localized retail tooltip without changing its colour or contents.
    pub fn tip_for_control(&self, panel: Panel, control_name: &str) -> Option<String> {
        let control = self.res.layout(panel.layout())?.get(control_name)?;
        if control.tooltip.trim().is_empty() {
            return None;
        }
        let text = self.res.localization.localize(&control.tooltip);
        (!text.trim().is_empty()).then_some(text)
    }

    /// Retail text lookup retained for callers that do not request development status.
    pub fn tooltip_at(&self, canvas: Vec2, pointer: Vec2) -> Option<String> {
        let panel = self.tab.panel()?;
        let scale = crate::proportional::dialog_scale(canvas);
        let origin = self.placement.place(&main_frame(canvas)).origin;
        let dialog_pointer = (pointer - origin) / scale;
        let page = page_rect();
        if !page.contains(dialog_pointer) {
            return None;
        }
        let list = layout_widgets(&self.res, panel, page, &self.draft, &self.applied);
        let widget = list
            .iter()
            .rev()
            .find(|w| w.rect.contains(dialog_pointer))?;
        self.tip_for_control(panel, &widget.control.name)
    }

    pub fn active_tooltip(&self) -> Option<&ActiveTooltip> {
        self.active_tooltip.as_ref()
    }

    pub(super) fn update_tooltip(
        &mut self,
        panel: Panel,
        list: &[Widget<'_>],
        pointer: Option<Vec2>,
        input: &Input,
        measure: &Painter,
    ) {
        let hovered = pointer
            .filter(|_| !input.click && !input.held)
            .and_then(|p| {
                list.iter()
                    .enumerate()
                    .rev()
                    .find(|(_, w)| w.rect.contains(p))
            });
        let Some(((index, widget), canvas_pointer)) = hovered.zip(input.pointer) else {
            self.active_tooltip = None;
            return;
        };
        let target = (panel, index);
        if let Some(active) = self
            .active_tooltip
            .as_mut()
            .filter(|tip| tip.target == target)
        {
            active.pointer = canvas_pointer;
            return;
        }
        let text = self
            .tip_for_control(panel, &widget.control.name)
            .unwrap_or_default();
        let development = support::related_binding(panel, widget.control)
            .and_then(support::development)
            .map(|token| self.res.localization.text(token));
        if text.is_empty() && development.is_none() {
            self.active_tooltip = None;
            return;
        }
        let prepared = measure.prepare_tooltip(&[
            TooltipSection {
                text: &text,
                fill: measure.theme.tooltip_bg,
            },
            TooltipSection {
                text: development.as_deref().unwrap_or_default(),
                fill: DEVELOPMENT_FILL,
            },
        ]);
        self.active_tooltip = Some(ActiveTooltip {
            text,
            development,
            pointer: canvas_pointer,
            target,
            prepared,
        });
    }

    pub(super) fn draw_tooltip(&self, base: &Painter, canvas: Vec2, scale: f32) {
        if let Some(tip) = &self.active_tooltip {
            let painter = Painter {
                canvas: base.canvas,
                fonts: base.fonts,
                theme: base.theme,
                origin: Vec2::ZERO,
                scale,
            };
            // The focused subdialog must not inherit the inactive Options parent's fade.
            let old_alpha = self
                .sub
                .map(|_| base.canvas.set_alpha_multiplier(self.sub_alpha));
            painter.draw_tooltip(
                &tip.prepared,
                tip.pointer / scale,
                Rect::new(0., 0., canvas.x / scale, canvas.y / scale),
            );
            if let Some(alpha) = old_alpha {
                base.canvas.set_alpha_multiplier(alpha);
            }
        }
    }

    pub(super) fn mark_development(&self, painter: &Painter, widget: &Widget<'_>) {
        if widget.binding.and_then(support::development).is_some() {
            painter.fill(
                Rect::new(
                    widget.rect.x + widget.rect.w - 5.,
                    widget.rect.y + 1.,
                    4.,
                    4.,
                ),
                DEVELOPMENT_FILL,
            );
        }
    }

    pub(super) fn dialog_occluders(
        &self,
        sub: Option<Rect>,
        confirm: Option<Rect>,
        external: &[Rect],
    ) -> Vec<Rect> {
        let mut occluders = Vec::with_capacity(2 + external.len());
        if self.closing_sub.is_none() && self.sub_alpha >= 0.999 {
            if let Some(r) = sub {
                occluders.push(r);
            }
        }
        if self.closing_confirm.is_none() && self.confirm_alpha >= 0.999 {
            if let Some(r) = confirm {
                occluders.push(r);
            }
        }
        occluders.extend_from_slice(external);
        occluders
    }

    pub(super) fn overlay_bounds(
        &self,
        base: &Painter,
        canvas: Vec2,
        scale: f32,
    ) -> (Option<Rect>, Option<Rect>) {
        let sub = self.sub.or(self.closing_sub).map(|sub| {
            let size = self.sub_frame_size(sub, |s| base.text_width(s, false));
            let o = self.sub_placement.place(&sub_frame(canvas, size)).origin;
            Rect::new(o.x, o.y, size.x * scale, size.y * scale)
        });
        let confirm = self
            .confirm
            .as_ref()
            .or(self.closing_confirm.as_ref())
            .map(|confirm| {
                let layout = confirm.layout(|t| base.text_width(t, false));
                let o = self
                    .confirm_placement
                    .place(&confirm_frame(canvas, layout.size))
                    .origin;
                Rect::new(o.x, o.y, layout.size.x * scale, layout.size.y * scale)
            });
        (sub, confirm)
    }
}
