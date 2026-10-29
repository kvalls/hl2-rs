//! Window composition shared with chapter selection.
use super::{Console, Window};
use glam::Vec2;
use source_ui::{canvas::Rect, widgets::Painter, windows::Layer};

impl Console {
    pub(super) fn draw_windows(&self, time: f64) {
        if self.is_background_loading() {
            self.draw_loading();
            return;
        }
        let confirm_box_to_draw = self
            .confirm_box
            .as_ref()
            .or(self.closing_confirm_box.as_ref());
        let quit_bounds = confirm_box_to_draw.map(|(confirm, _)| {
            let (origin, scale, layout) = self.quit_geometry(confirm);
            Rect::new(
                origin.x,
                origin.y,
                layout.size.x * scale,
                layout.size.y * scale,
            )
        });

        let active_windows: Vec<Window> = self.shell.windows.order().collect();
        for (idx, &window) in active_windows.iter().enumerate() {
            let mut occluders = Vec::new();
            for &above in &active_windows[idx + 1..] {
                if !self.window_occludes(above, time) {
                    continue;
                }
                match above {
                    Window::Console => occluders.push(self.window_bounds(Window::Console)),
                    Window::Options | Window::NewGame => occluders.push(self.window_bounds(above)),
                    Window::Confirmation if self.closing_confirm_box.is_none() => {
                        if let Some(bounds) = quit_bounds {
                            occluders.push(bounds);
                        }
                    }
                    _ => {}
                }
            }

            match window {
                Window::NewGame => self.draw_chapters(time, &occluders),
                Window::Options => {
                    let theme = self.window_theme(window, time);
                    let alpha = self
                        .shell
                        .windows
                        .window_composite_alpha(Window::Options, time);
                    let old_alpha = self.canvas.set_alpha_multiplier(alpha);
                    let options_focused =
                        self.shell.windows.focus() == Layer::Window(Window::Options);
                    self.options.draw_with_occluders(
                        &Painter {
                            theme: &theme,
                            ..self.painter(Vec2::ZERO, 1.)
                        },
                        &occluders,
                        Some(time),
                        Some(options_focused),
                    );
                    self.canvas.set_alpha_multiplier(old_alpha);
                }
                Window::Console => {
                    let alpha = self
                        .shell
                        .windows
                        .window_composite_alpha(Window::Console, time);
                    let old_alpha = self.canvas.set_alpha_multiplier(alpha);
                    self.draw_console_with_occluders(time, &occluders);
                    self.canvas.set_alpha_multiplier(old_alpha);
                }
                Window::Confirmation => self.draw_confirmation(time, &occluders),
                Window::SubDialog => {}
            }
        }
        self.draw_loading();
    }

    pub(super) fn draw_loading(&self) {
        if let Some(loading) = &self.loading_dialog {
            let scale = crate::proportional::dialog_scale(self.canvas_size());
            loading.draw(&self.painter(Vec2::ZERO, scale), &self.gameui.localization);
        }
    }

    fn draw_confirmation(&self, time: f64, occluders: &[Rect]) {
        let Some((confirm, _)) = self
            .confirm_box
            .as_ref()
            .or(self.closing_confirm_box.as_ref())
        else {
            return;
        };
        let (origin, scale, layout) = self.quit_geometry(confirm);
        let theme = self.window_theme(Window::Confirmation, time);
        let alpha = self
            .shell
            .windows
            .window_composite_alpha(Window::Confirmation, time);
        let old_alpha = self.canvas.set_alpha_multiplier(alpha);
        for &rect in occluders {
            self.canvas.push_occluder(rect);
        }
        confirm.draw(
            &Painter {
                theme: &theme,
                ..self.painter(origin, scale)
            },
            &layout,
        );
        for _ in occluders {
            self.canvas.pop_occluder();
        }
        self.canvas.set_alpha_multiplier(old_alpha);
    }

    fn window_occludes(&self, window: Window, time: f64) -> bool {
        !self.shell.windows.is_closing(window)
            && self.shell.windows.window_composite_alpha(window, time) >= 0.999
    }
}
