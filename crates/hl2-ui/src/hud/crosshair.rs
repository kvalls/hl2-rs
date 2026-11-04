//! Weapon reticle positioning, with the native Windows raster path preserved.
use super::{
    default_crosshair_rects, keyvalues, number, sprite_filter, texture, Context, Entry, Rect,
    Result, Texture2D, Vfs, WeaponHud,
};
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
        // Without GDI: the same hard-edged glyph at whole pixels, on the native dot positions.
        let (width_scale, x_offset) = crate::tuning::weapon_crosshair();
        if let Some(texture) = character.chars().next().and_then(|c| {
            self.crosshairs
                .face
                .monochrome_glyph(c, self.crosshairs.size(), width_scale)
        }) {
            let size = texture.size();
            source_ui::canvas::draw_texture(
                &self.canvas,
                &texture,
                (self.canvas.width() * 0.5 + 0.5).trunc() - (size.x * 0.5).trunc() + x_offset,
                (self.canvas.height() * 0.5 + 0.5).trunc() - (size.y * 0.5).trunc(),
                self.crosshair_color,
            );
        }
        self.canvas.additive(false);
    }
}
/// The default crosshair of scripts/hud_textures.txt: its texture, its rectangle and whether it
/// draws additively.
pub(super) fn load_default(vfs: &Vfs) -> Result<(Texture2D, Rect, bool)> {
    let hud_textures = keyvalues::parse_resource(
        &vfs.read("scripts/hud_textures.txt")?
            .context("HUD textures missing")?,
    )?;
    let default_icon = hud_textures
        .first()
        .and_then(|root| root.get("TextureData"))
        .and_then(|textures| textures.get("crosshair_default"))
        .context("Default crosshair resource missing")?;
    let default_file = default_icon
        .get("file")
        .and_then(Entry::text)
        .context("Default crosshair material missing")?;
    let default_crosshair = texture(vfs, default_file)?;
    let default_base = vfs
        .base_texture(default_file)?
        .context("Default crosshair base texture missing")?;
    let default_vtf = vfs
        .read(&format!(
            "materials/{}.vtf",
            default_base
                .trim_start_matches("materials/")
                .trim_end_matches(".vtf")
        ))?
        .context("Default crosshair VTF missing")?;
    default_crosshair.set_filter(sprite_filter(&default_vtf)?);
    let default_crosshair_additive = vfs
        .material_value(default_file, "$additive")?
        .is_some_and(|value| value.parse::<i32>().is_ok_and(|value| value != 0));
    let rect = Rect::new(
        number(default_icon, "x", 0.),
        number(default_icon, "y", 48.),
        number(default_icon, "width", 24.),
        number(default_icon, "height", 24.),
    );
    Ok((default_crosshair, rect, default_crosshair_additive))
}
