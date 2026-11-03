//! Generic card rendering over caller-provided content and geometry.
use super::{CardBrowser, ScaledImageLayout, Target};
use crate::{
    canvas::{draw_texture_ex, Color, DrawTextureParams, Rect, Texture2D, WHITE},
    widgets::{truncate_ellipsis, Align, ButtonState, FrameSpec, Painter},
};
use glam::{vec2, Vec2};
use std::borrow::Cow;

impl Painter<'_> {
    /// Text width in layout units for an explicit character-cell height `tall`.
    pub fn text_width_sized(&self, text: &str, tall: f32, bold: bool) -> f32 {
        self.font(bold).map_or(
            text.lines()
                .map(|l| l.chars().count() as f32 * 7.)
                .fold(0.0, f32::max),
            |f| {
                text.lines()
                    .map(|l| f.width_cell(l, tall))
                    .fold(0.0, f32::max)
            },
        )
    }

    /// Truncates `text` with an ellipsis so its width at cell height `tall` fits within `max_w`.
    pub fn truncate_ellipsis_sized<'b>(
        &self,
        text: &'b str,
        max_w: f32,
        tall: f32,
        bold: bool,
    ) -> Cow<'b, str> {
        truncate_ellipsis(text, max_w, |s| self.text_width_sized(s, tall, bold))
    }

    /// Draws one line of text at cell height `tall` vertically centered in `rect`.
    pub fn text_in_sized(
        &self,
        text: &str,
        rect: Rect,
        tall: f32,
        align: Align,
        color: Color,
        bold: bool,
    ) {
        let width = self.text_width_sized(text, tall, bold);
        let x = match align {
            Align::Left => rect.x,
            Align::Center => rect.x + (rect.w - width) * 0.5,
            Align::Right => rect.x + rect.w - width,
        };
        if let Some(font) = self.font(bold) {
            let at = vec2(x, rect.y + (rect.h - tall) * 0.5);
            for (i, line) in text.lines().enumerate() {
                let p = self.origin + (at + vec2(0., i as f32 * tall)) * self.scale;
                font.draw_cell(line, p.round(), tall * self.scale, color);
            }
        }
    }
}

impl CardBrowser {
    fn state(&self, target: Target) -> ButtonState {
        ButtonState::active(
            true,
            self.tracker.armable(self.hover) == Some(target),
            self.tracker.is_pressed(target)
                || self.keys.pressed() == Some(target)
                || target == Target::Close && self.escape_held,
        )
    }

    pub fn draw(&self, painter: &Painter<'_>, canvas: Vec2) {
        let frame = self.frame(canvas);
        let origin = self.placement.place(&frame).origin;
        let p = Painter {
            origin,
            scale: frame.scale,
            ..*painter
        };
        p.frame_spec(
            FrameSpec::new(frame.size, &self.spec.title),
            Some(self.state(Target::Close)),
        );
        for &rect in &self.spec.layout.dividers {
            let wide = rect.w.min((frame.size.x - 2. * rect.x).max(0.));
            p.sunken(Rect::new(rect.x, rect.y, wide, rect.h));
        }
        if let Some(toggle) = &self.spec.layout.toggle {
            if let Some(backing) = toggle.backing {
                p.rounded_fill(backing, Color::new(0., 0., 0., 196. / 255.), 5.0);
            }
            if let (Some(icon), Some(tex)) = (toggle.icon, &self.spec.toggle_icon) {
                Self::draw_scaled_image(&p, icon, tex);
            }
        }
        for (target, rect) in self.targets() {
            match target {
                Target::Card(index) => self.draw_card(&p, index, rect),
                Target::Close => {}
                Target::Toggle => {
                    if let Some(label) = &self.spec.toggle_label {
                        p.check_box(rect, label, self.toggle_checked, self.state(target));
                    }
                }
                button => {
                    let label = match button {
                        Target::Previous => &self.spec.labels[0],
                        Target::Next => &self.spec.labels[1],
                        Target::Accept => &self.spec.labels[2],
                        _ => &self.spec.labels[3],
                    };
                    p.button(rect, label, self.state(target));
                }
            }
            if self.focus_visible && self.focus == Some(target) {
                p.focus_ring(rect);
            }
        }
    }

    fn draw_scaled_image(p: &Painter<'_>, spec: ScaledImageLayout, texture: &Texture2D) {
        let scale = if spec.scale > 0. { spec.scale } else { 1. };
        let w = (texture.width() * scale).min(spec.rect.w);
        let h = (texture.height() * scale).min(spec.rect.h);
        p.image(Rect::new(spec.rect.x, spec.rect.y, w, h), texture);
    }

    fn panel_control(rect: Rect, panel: Rect) -> Rect {
        Rect::new(panel.x + rect.x, panel.y + rect.y, rect.w, rect.h)
    }

    fn draw_card(&self, p: &Painter<'_>, index: usize, panel: Rect) {
        let card = &self.spec.cards[index];
        let border = Self::panel_control(self.spec.layout.card.border, panel);
        let selected = index == self.selected;
        let color = if selected {
            self.spec.selection.unwrap_or(p.theme.selection_bg)
        } else {
            self.spec.fill.unwrap_or(p.theme.frame_bg)
        };
        p.fill(border, color);
        let image = Self::panel_control(self.spec.layout.card.image, panel);
        if let Some(texture) = &card.image {
            // Retail ImagePanel does not scale these padded, power-of-two textures:
            // the panel clips the top-left image region before proportional UI scaling.
            let source = Rect::new(
                0.,
                0.,
                image.w.min(texture.width()),
                image.h.min(texture.height()),
            );
            let destination = p.map(Rect::new(image.x, image.y, source.w, source.h));
            draw_texture_ex(
                p.canvas,
                texture,
                destination.x,
                destination.y,
                WHITE,
                DrawTextureParams {
                    source: Some(source),
                    dest_size: Some(vec2(destination.w, destination.h)),
                },
            );
        }
        if self.toggle_checked {
            if let (Some(badge), Some(tex)) = (self.spec.layout.card.badge, &self.spec.toggle_icon)
            {
                Self::draw_scaled_image(
                    p,
                    ScaledImageLayout {
                        rect: Self::panel_control(badge.rect, panel),
                        scale: badge.scale,
                    },
                    tex,
                );
            }
        }
        let heading = Self::panel_control(self.spec.layout.card.heading, panel);
        let title = Self::panel_control(self.spec.layout.card.title, panel);
        let font_tall = self.spec.layout.card.font_tall;
        let text_color = if selected {
            self.spec.selection.unwrap_or(p.theme.selection_bg)
        } else {
            self.spec.text.unwrap_or(p.theme.text)
        };
        p.text_in_sized(
            &card.heading,
            heading,
            font_tall,
            Align::Left,
            text_color,
            true,
        );
        let title_text = self
            .display_titles
            .get(index)
            .map_or(card.title.as_str(), String::as_str);
        p.text_in_sized(title_text, title, font_tall, Align::Left, text_color, true);
        if self.state(Target::Card(index)).pressed {
            p.border(image, p.theme.border_dark, p.theme.border_bright);
        }
    }
}
