//! Shared push-button padding and painting.
use super::{Align, ButtonBorder, ButtonState, Painter, Rect};

/// Horizontal inset shared by push-button measurement and painting.
pub const BUTTON_TEXT_INSET: f32 = 6.;

impl Painter<'_> {
    /// Label width in layout units, conservatively fitted to its actual raster size.
    pub fn button_label_width(&self, label: &str) -> f32 {
        let nominal = self.text_width(label, false);
        self.font(false).map_or(nominal, |font| {
            let scale = self.scale.max(f32::EPSILON);
            let raster = font.button_label_width(label, super::FONT_TALL * scale) / scale;
            nominal.max(raster)
        })
    }

    /// A push button drawn from [`Theme::button_style`]: raised border, sunken while pressed,
    /// selection border when default, and the scheme's armed colors while hovered.
    pub fn button(&self, rect: Rect, label: &str, state: ButtonState) {
        let style = self.theme.button_style(state);
        if style.bg.a > 0. {
            self.fill(rect, style.bg);
        }
        match style.border {
            ButtonBorder::Depressed => self.sunken(rect),
            ButtonBorder::Raised => self.raised(rect),
            ButtonBorder::KeyFocus => {
                self.raised(rect);
                let outer = Rect::new(rect.x - 1., rect.y - 1., rect.w + 2., rect.h + 2.);
                self.border(
                    outer,
                    self.theme.border_selection,
                    self.theme.border_selection,
                );
            }
        }
        let offset = style.text_offset;
        let text_rect = Rect::new(
            rect.x + BUTTON_TEXT_INSET + offset,
            rect.y + offset,
            rect.w - 2. * BUTTON_TEXT_INSET,
            rect.h,
        );
        if state.enabled {
            self.text_in(label, text_rect, Align::Left, style.text, false);
        } else {
            self.label(label, text_rect, Align::Left, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ButtonState, Painter, Rect};
    use crate::canvas::Canvas;
    use crate::font::FontFace;
    use crate::message_box::MessageBox;
    use crate::widgets::{Fonts, Theme};
    use glam::Vec2;

    #[test]
    fn confirmation_buttons_extend_without_changing_the_rendered_label() {
        let canvas = Canvas::default();
        let fonts = Fonts {
            text: Some(FontFace::load(crate::PROGGY_CLEAN, canvas.clone()).unwrap()),
            ..Default::default()
        };
        let theme = Theme::default();
        for scale in [1., 1.25, 1.5, 1.75, 2., 2.625, 3.] {
            let painter = Painter {
                canvas: &canvas,
                fonts: &fonts,
                theme: &theme,
                origin: Vec2::ZERO,
                scale,
            };
            let base = Painter {
                scale: 1.,
                ..painter
            };
            for label in [
                "Quit game",
                "Beenden und zum Desktop zurückkehren",
                "Quitter le jeu et retourner au bureau",
            ] {
                let message = MessageBox::new("T".into(), "?".into(), label.into(), None);
                let old = message.layout(|text| base.text_width(text, false));
                let fitted = message.layout_for(&painter);
                let old_rect = Rect::new(0., 0., old.buttons[0].w, old.buttons[0].h);
                let rect = Rect::new(0., 0., fitted.buttons[0].w, fitted.buttons[0].h);
                assert!(rect.w >= old_rect.w);
                painter.button(old_rect, label, ButtonState::new(true));
                let before: Vec<_> = canvas
                    .drain()
                    .into_iter()
                    .filter(|q| q.texture.is_some())
                    .map(|q| q.destination)
                    .collect();
                painter.button(rect, label, ButtonState::new(true));
                let after: Vec<_> = canvas
                    .drain()
                    .into_iter()
                    .filter(|q| q.texture.is_some())
                    .map(|q| q.destination)
                    .collect();
                assert_eq!(
                    before, after,
                    "label glyph size and placement must stay unchanged"
                );
                let left = after.iter().map(|q| q.x).fold(f32::INFINITY, f32::min);
                let right = rect.w * scale - after.iter().map(|q| q.x + q.w).fold(0., f32::max);
                assert!(
                    right + 1. >= left,
                    "{label}, scale={scale}, left={left}, right={right}"
                );
            }
        }
    }
}
