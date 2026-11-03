//! GameMenu row rendering; content and style remain supplied by owned resources.
use super::{draw_cell_text_outlined, Console, Mode};
use crate::gameui::menu::MenuGeometry;
use glam::{vec2, Vec2};
use source_ui::{canvas::Color, font::TextOutline, windows::Layer};

/// Native draws the title labels' GDI text 4 px below the Main.TitleN.Y cell at every
/// resolution (hl2_complete oracle: HL2EP2 at 720p and 1080p, pause and main menu; the older
/// halflife2.ttf 720p capture shows the same 4 px). CBasePanel::PerformLayout places the labels
/// at the scaled TitleN.Y and vgui Label adds no inset, so the source of the offset is unfound.
const TITLE_TEXT_DROP: f32 = 4.;

impl Console {
    pub(super) fn draw_menu_buttons(
        &self,
        shown: Mode,
        geometry: MenuGeometry,
        alpha_mult: f32,
        base_focus: f32,
    ) {
        let menu = if shown == Mode::MainMenu {
            &self.main_menu
        } else {
            &self.pause_menu
        };
        // With the keyboard elsewhere (a window has the focus), a row is armed only under
        // the pointer or while pressed.
        let menu_keyboard = self.shell.windows.focus() == Layer::Base;
        let modal_open = self.modal_dialog_open();
        for (i, entry) in menu.entries().iter().enumerate() {
            if entry.spacer {
                continue;
            }
            let is_pressed = !modal_open && self.menu_tracker.is_pressed(i);
            let selected = if menu_keyboard {
                menu.selection == Some(i)
            } else {
                self.last_menu_hover == Some(i)
            };
            let armed = !modal_open && (is_pressed || selected);
            let color = if !entry.enabled {
                self.theme.text_disabled
            } else if armed {
                self.style.menu_armed
            } else {
                self.style.menu
            };
            let border_color = if !entry.enabled {
                Color::new(0.0, 0.0, 0.0, 0.25)
            } else if armed {
                Color::new(0.0, 0.0, 0.0, 0.65)
            } else {
                Color::new(0.0, 0.0, 0.0, 0.45)
            };
            let color = if alpha_mult >= 1.0 - 1e-4 {
                color
            } else {
                Color::new(color.r, color.g, color.b, color.a * alpha_mult)
            };
            let border_alpha = if base_focus > 1e-4 {
                border_color.a * base_focus
            } else {
                0.0
            };
            let button_outline = if border_alpha > 1e-4 {
                TextOutline::new(Color::new(0.0, 0.0, 0.0, border_alpha), 1.0)
            } else {
                TextOutline::NONE
            };
            let offset = if is_pressed { vec2(1., 1.) } else { Vec2::ZERO };
            draw_cell_text_outlined(
                &entry.label,
                geometry.row_text_pos(i) + offset,
                geometry.font_tall,
                self.menu_font.as_ref(),
                self.menu_metrics,
                color,
                button_outline,
            );
        }
    }

    pub(super) fn draw_menu_titles(
        &self,
        geometry: MenuGeometry,
        alpha_mult: f32,
        base_focus: f32,
    ) {
        let title_color = Color::new(1., 1., 1., alpha_mult);
        let sub_color = if alpha_mult >= 1.0 - 1e-4 {
            Color::from_rgba(255, 255, 255, 200)
        } else {
            Color::new(1., 1., 1., (200.0 / 255.0) * alpha_mult)
        };
        let title_outline = if base_focus >= 1.0 - 1e-4 {
            TextOutline::DEFAULT
        } else if base_focus > 1e-4 {
            TextOutline::new(
                Color::new(
                    TextOutline::DEFAULT.color.r,
                    TextOutline::DEFAULT.color.g,
                    TextOutline::DEFAULT.color.b,
                    TextOutline::DEFAULT.color.a * base_focus,
                ),
                1.0,
            )
        } else {
            TextOutline::NONE
        };
        let sub_outline = if base_focus >= 1.0 - 1e-4 {
            TextOutline::DEFAULT
        } else if base_focus > 1e-4 {
            TextOutline::new(
                Color::new(
                    TextOutline::DEFAULT.color.r,
                    TextOutline::DEFAULT.color.g,
                    TextOutline::DEFAULT.color.b,
                    TextOutline::DEFAULT.color.a * base_focus,
                ),
                1.0,
            )
        } else {
            TextOutline::NONE
        };

        if let Some(face) = &self.title_face {
            self.draw_face_menu_titles(
                face,
                geometry,
                title_color,
                sub_color,
                title_outline,
                sub_outline,
            );
        } else {
            self.draw_fallback_menu_titles(
                geometry,
                title_color,
                sub_color,
                title_outline,
                sub_outline,
            );
        }
    }

    fn draw_face_menu_titles(
        &self,
        face: &fontdue::Font,
        geometry: MenuGeometry,
        title_color: Color,
        sub_color: Color,
        title_outline: TextOutline,
        sub_outline: TextOutline,
    ) {
        let title = self.current_title();
        let (em, _) = self.title_metrics.size_for(geometry.title_tall);
        for (line_idx, line) in title.lines().enumerate() {
            let mut pen = match line_idx {
                0 => geometry.title,
                1 => geometry.title2,
                _ => {
                    geometry.title2
                        + vec2(
                            0.,
                            (line_idx - 1) as f32 * (geometry.title2.y - geometry.title.y).max(1.),
                        )
                }
            } + vec2(0., TITLE_TEXT_DROP);
            if line_idx == 0 {
                let mut chars = Vec::with_capacity(line.len());
                for character in line.chars() {
                    chars.push((character, pen));
                    // GDI supplies integer ABC advances. Fractional accumulation visibly widens this logo.
                    pen.x += face.metrics(character, em).advance_width.round();
                }
                if title_outline.is_visible() {
                    for &(character, char_pen) in &chars {
                        draw_cell_text_outlined(
                            &character.to_string(),
                            char_pen,
                            geometry.title_tall,
                            self.title_font.as_ref(),
                            self.title_metrics,
                            Color::new(0., 0., 0., 0.),
                            title_outline,
                        );
                    }
                }
                for &(character, char_pen) in &chars {
                    draw_cell_text_outlined(
                        &character.to_string(),
                        char_pen,
                        geometry.title_tall,
                        self.title_font.as_ref(),
                        self.title_metrics,
                        title_color,
                        TextOutline::NONE,
                    );
                }
            } else {
                // Subtitle line (e.g. episode or lost coast submenu subtitle).
                let is_lostcoast =
                    line.to_ascii_lowercase().contains("lost coast") || line.contains("lost coasT");
                if is_lostcoast && self.lostcoast_title_font.is_some() {
                    draw_cell_text_outlined(
                        line,
                        pen,
                        geometry.title_tall,
                        self.lostcoast_title_font.as_ref(),
                        self.lostcoast_title_metrics,
                        sub_color,
                        sub_outline,
                    );
                } else if self.title_font_can_render(line) {
                    draw_cell_text_outlined(
                        line,
                        pen,
                        geometry.title_tall,
                        self.title_font.as_ref(),
                        self.title_metrics,
                        sub_color,
                        sub_outline,
                    );
                } else {
                    let sub_tall = (geometry.title_tall * 0.55).round().max(14.);
                    draw_cell_text_outlined(
                        line,
                        pen,
                        sub_tall,
                        self.menu_font.as_ref(),
                        self.menu_metrics,
                        sub_color,
                        sub_outline,
                    );
                }
            }
        }
    }

    fn draw_fallback_menu_titles(
        &self,
        geometry: MenuGeometry,
        title_color: Color,
        sub_color: Color,
        title_outline: TextOutline,
        sub_outline: TextOutline,
    ) {
        let title = self.current_title();
        for (line_idx, line) in title.lines().enumerate() {
            let pen = match line_idx {
                0 => geometry.title,
                1 => geometry.title2,
                _ => {
                    geometry.title2
                        + vec2(
                            0.,
                            (line_idx - 1) as f32 * (geometry.title2.y - geometry.title.y).max(1.),
                        )
                }
            };
            let tall = if line_idx == 0 {
                geometry.title_tall
            } else {
                (geometry.title_tall * 0.55).round().max(14.)
            };
            draw_cell_text_outlined(
                line,
                pen,
                tall,
                self.menu_font.as_ref(),
                self.menu_metrics,
                if line_idx == 0 {
                    title_color
                } else {
                    sub_color
                },
                if line_idx == 0 {
                    title_outline
                } else {
                    sub_outline
                },
            );
        }
    }
}
