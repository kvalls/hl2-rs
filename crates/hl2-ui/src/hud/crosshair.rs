//! Weapon reticle positioning, with the native Windows raster path preserved.
use super::{default_crosshair_rects, WeaponHud};
use glam::vec2;
use hl2_simulation::gameplay::Inventory;
use source_ui::canvas::{draw_texture_ex, DrawTextureParams, WHITE};

impl WeaponHud {
    pub fn draw_crosshair(&self, inv: &Inventory) {
        if inv.health <= 0. {
            return;
        }
        let Some(character) = self.weapon_crosshairs.get(&inv.active) else {
            // CHudCrosshair::ResetCrosshair uses the installed default sprite and opaque white.
            let (destination, source) = default_crosshair_rects(
                vec2(self.canvas.width(), self.canvas.height()),
                self.default_crosshair_rect,
                self.default_crosshair.size(),
            );
            if self.default_crosshair_additive {
                self.canvas.additive(true);
            }
            draw_texture_ex(
                &self.canvas,
                &self.default_crosshair,
                destination.x,
                destination.y,
                WHITE,
                DrawTextureParams {
                    source: Some(source),
                    dest_size: Some(destination.size()),
                },
            );
            self.canvas.additive(false);
            return;
        };
        self.canvas.additive(true);
        #[cfg(windows)]
        if let Some(glyph) = character
            .chars()
            .next()
            .filter(|_| character.chars().count() == 1)
            .and_then(|c| {
                let range = self.crosshairs.ranges.iter().find(|range| {
                    self.canvas.height() >= range.minimum && self.canvas.height() <= range.maximum
                })?;
                range.monochrome_crosshair().then_some(())?;
                self.native_crosshairs.get(&(c, range.tall.round() as u16))
            })
        {
            if let Some(texture) = &glyph.texture {
                // Integer icon dimensions and GDI's actual cell height, not the requested
                // height or fractional fontdue EM metrics. No rasterization occurs here.
                let center = vec2(
                    (self.canvas.width() * 0.5 + 0.5).trunc(),
                    (self.canvas.height() * 0.5 + 0.5).trunc(),
                );
                let origin = center - vec2((glyph.advance / 2) as f32, (glyph.height / 2) as f32)
                    + glyph.origin;
                source_ui::canvas::draw_texture(
                    &self.canvas,
                    texture,
                    origin.x,
                    origin.y,
                    self.crosshair_color,
                );
            }
            self.canvas.additive(false);
            return;
        }
        let Some(character_glyph) = character.chars().next() else {
            self.canvas.additive(false);
            return;
        };
        let bounds = self.crosshairs.face.glyph_bounds(
            character_glyph,
            self.crosshairs.size(),
            self.crosshairs.effects(),
        );
        self.crosshairs.draw(
            character,
            self.canvas.width() * 0.5 - bounds.center().x,
            self.canvas.height() * 0.5 - bounds.center().y,
            self.crosshair_color,
        );
        self.canvas.additive(false);
    }
}
