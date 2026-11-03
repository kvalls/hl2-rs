//! Closed-caption canvas presentation.
use super::{vec2, Color, Rect, WeaponHud};
impl WeaponHud {
    /// The closed caption panel (CHudCloseCaption::Paint): its box through the HUD's rounded
    /// panel background, then the text runs. Hidden while `closecaption` is off.
    pub fn draw_captions(&self, settings: &crate::closecaption::Settings, time: f64) {
        self.captions.tick(time);
        if !settings.closecaption {
            return;
        }
        let _frame = self.framed();
        let size = (self.canvas.width(), self.canvas.height());
        let Some((panel, paint)) = self.captions.paint(size, time) else {
            return;
        };
        let color = |c: [f32; 4]| Color::new(c[0], c[1], c[2], c[3]);
        if let Some((rect, background)) = paint.background {
            self.rounded_box(
                Rect::new(panel.x + rect[0], panel.y + rect[1], rect[2], rect[3]),
                color(background),
            );
        }
        let scale = crate::proportional::scale(vec2(size.0, size.1));
        for run in &paint.runs {
            let Some((face, tall)) = self.captions.face(run.style, scale) else {
                continue;
            };
            let (x, y) = (panel.x + run.x, panel.y + run.y);
            if let Some(highlight) = run.highlight {
                self.canvas
                    .rectangle(x, y, run.width, run.height, color(highlight));
            }
            face.draw_cell(&run.text, vec2(x, y), tall, color(run.color));
        }
    }

    /// A ScreenFade message (DamageEffect or env_fade) on the game clock.
    pub fn screen_fade(&self, fade: hl2_simulation::player_damage::ScreenFade, time: f64) {
        self.fades.borrow_mut().add(fade, time);
    }
    pub fn clear_fades(&self) {
        self.fades.borrow_mut().clear();
    }
}
