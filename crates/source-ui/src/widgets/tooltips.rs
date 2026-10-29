//! Prepared, coloured tooltip panels, positioned and clamped as one stack.
use super::{tooltip_rect, wrap, Painter, FONT_TALL};
use crate::canvas::{Color, Rect};
use glam::{vec2, Vec2};

const PAD: Vec2 = vec2(6., 4.);
pub const TOOLTIP_GAP: f32 = 2.;

pub struct TooltipSection<'a> {
    pub text: &'a str,
    pub fill: Color,
}

#[derive(Clone, Debug, PartialEq)]
struct Section {
    lines: Vec<String>,
    width: f32,
    height: f32,
    fill: Color,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreparedTooltip {
    sections: Vec<Section>,
    size: Vec2,
}

impl PreparedTooltip {
    /// Panel rectangles keep their order and spacing even when the stack flips above the mouse.
    pub fn panels(&self, pointer: Vec2, bounds: Rect) -> impl Iterator<Item = Rect> + '_ {
        let placed = tooltip_rect(pointer, self.size, bounds);
        self.sections.iter().scan(placed.y, move |y, section| {
            let rect = Rect::new(placed.x, *y, section.width, section.height);
            *y += section.height + TOOLTIP_GAP;
            Some(rect)
        })
    }
}

impl Painter<'_> {
    /// Wrap/measure on hover changes, then reuse the result for subsequent frames.
    pub fn prepare_tooltip(&self, sections: &[TooltipSection<'_>]) -> PreparedTooltip {
        let mut tooltip = PreparedTooltip::default();
        for section in sections.iter().filter(|s| !s.text.trim().is_empty()) {
            let lines = wrap(section.text, 280., |s| self.text_width(s, false));
            let width = lines
                .iter()
                .map(|s| self.text_width(s, false))
                .fold(0., f32::max);
            let height = lines.len() as f32 * FONT_TALL + PAD.y * 2.;
            if !tooltip.sections.is_empty() {
                tooltip.size.y += TOOLTIP_GAP;
            }
            tooltip.size.x = tooltip.size.x.max(width + PAD.x * 2.);
            tooltip.size.y += height;
            tooltip.sections.push(Section {
                lines,
                width: width + PAD.x * 2.,
                height,
                fill: section.fill,
            });
        }
        tooltip
    }

    pub fn draw_tooltip(&self, tooltip: &PreparedTooltip, pointer: Vec2, bounds: Rect) {
        for (section, rect) in tooltip.sections.iter().zip(tooltip.panels(pointer, bounds)) {
            self.fill(rect, section.fill);
            self.border(rect, self.theme.tooltip_border, self.theme.tooltip_border);
            for (i, line) in section.lines.iter().enumerate() {
                self.text(
                    line,
                    vec2(rect.x + PAD.x, rect.y + PAD.y + i as f32 * FONT_TALL),
                    self.theme.tooltip_text,
                    false,
                );
            }
        }
    }

    /// A single retail-coloured tooltip, preserving the existing API.
    pub fn tooltip(&self, text: &str, pointer: Vec2, bounds: Rect) {
        let prepared = self.prepare_tooltip(&[TooltipSection {
            text,
            fill: self.theme.tooltip_bg,
        }]);
        self.draw_tooltip(&prepared, pointer, bounds);
    }
}

#[cfg(test)]
mod tests;
