//! Health and ammunition arcs around the view crosshair.
use super::{alpha, color, number, HudFont, QuickInfoState};
use anyhow::{Context, Result};
use glam::{vec2, Vec2};
use hl2_simulation::gameplay::{Inventory, Weapon};
use source_assets::{
    keyvalues::{self, Entry},
    vpk::Vfs,
};
use source_ui::{
    canvas::{
        draw_texture_ex, texture, Canvas, Color, DrawTextureParams, MutexCell, Rect, Texture2D,
    },
    font::FontFace,
};
use std::collections::BTreeMap;

pub(super) struct QuickInfoHud {
    canvas: Canvas,
    font: HudFont,
    center: Texture2D,
    center_rect: Rect,
    left: String,
    left_empty: String,
    right: String,
    right_empty: String,
    pub(super) normal: Color,
    pub(super) caution: Color,
    pub(super) state: MutexCell<QuickInfoState>,
}
impl QuickInfoHud {
    pub(super) fn load(vfs: &Vfs, fonts: &Entry, colors: &Entry, canvas: Canvas) -> Result<Self> {
        let face = FontFace::load(
            &vfs.read("resource/HL2crosshairs.ttf")?
                .context("QuickInfo font missing")?,
            canvas.clone(),
        )?;
        let resources = keyvalues::parse_resource(
            &vfs.read("scripts/hud_textures.txt")?
                .context("HUD textures missing")?,
        )?;
        let textures = resources
            .first()
            .and_then(|e| e.get("TextureData"))
            .context("HUD TextureData missing")?;
        let character = |name| -> Result<String> {
            Ok(textures
                .get(name)
                .and_then(|e| e.get("character"))
                .and_then(Entry::text)
                .context("QuickInfo glyph missing")?
                .into())
        };
        let center = textures
            .get("crosshair")
            .context("QuickInfo center missing")?;
        let file = center
            .get("file")
            .and_then(Entry::text)
            .context("QuickInfo center material missing")?;
        Ok(Self {
            canvas,
            font: HudFont::read(fonts, "QuickInfo", face, 28.),
            center: texture(vfs, file)?,
            center_rect: Rect::new(
                number(center, "x", 0.),
                number(center, "y", 0.),
                number(center, "width", 40.),
                number(center, "height", 40.),
            ),
            left: character("crosshair_left_full")?,
            left_empty: character("crosshair_left_empty")?,
            right: character("crosshair_right_full")?,
            right_empty: character("crosshair_right_empty")?,
            normal: color(colors, "Normal", Color::from_rgba(255, 208, 64, 255)),
            caution: color(colors, "Caution", Color::from_rgba(255, 48, 0, 255)),
            state: MutexCell::new(QuickInfoState::default()),
        })
    }
    fn progress(&self, full: &str, empty: &str, origin: Vec2, percentage: f32, color: Color) {
        let p = percentage.clamp(0., 1.);
        (p > 0.).then(|| self.font.draw_cropped(empty, origin, vec2(0., p), color));
        (p < 1.).then(|| self.font.draw_cropped(full, origin, vec2(p, 1.), color));
    }
    fn arc_origins(&self) -> [Vec2; 2] {
        let size = self.font.size();
        let effects = self.font.effects();
        let bounds = [&self.left, &self.right].map(|glyph| {
            self.font.face.glyph_bounds(
                glyph.chars().next().expect("installed quick-info glyph"),
                size,
                effects,
            )
        });
        let mut origins = [
            -self.font.width(&self.left) * 2.,
            self.font.width(&self.right),
        ];
        // Preserve authored spacing, but account for side bearings when centering the pair.
        let left = origins[0] + bounds[0].x;
        let right = origins[1] + bounds[1].x + bounds[1].w;
        let offset = self.canvas.width() * 0.5 - (left + right) * 0.5;
        origins.iter_mut().for_each(|x| *x += offset);
        std::array::from_fn(|i| {
            vec2(
                origins[i],
                (self.canvas.height() * 0.5).trunc() - bounds[i].center().y,
            )
        })
    }
    pub(super) fn draw(&self, inv: &Inventory, weapons: &BTreeMap<String, Weapon>, time: f64) {
        let Some(weapon) = weapons.get(&inv.active) else {
            return;
        };
        if inv.health <= 0. || !time.is_finite() {
            return;
        }
        let ammo = inv.owned.get(&inv.active).copied().unwrap_or(-1);
        let mut state = self.state.borrow_mut();
        let dt = state.observe(inv.health as i32, ammo, weapon.magazine, time);
        let opacity = state.alpha.clamp(0., 255.) / 255.;
        let x = (self.canvas.width() * 0.5).trunc();
        let y = (self.canvas.height() * 0.5).trunc() - self.font.size().round() * 0.5;
        self.canvas.additive(true);
        draw_texture_ex(
            &self.canvas,
            &self.center,
            x,
            y,
            alpha(
                Color {
                    a: 138. / 255.,
                    ..self.normal
                },
                opacity,
            ),
            DrawTextureParams {
                source: Some(self.center_rect),
                ..Default::default()
            },
        );
        let [left_origin, right_origin] = self.arc_origins();
        let warn_health = state.warn_health;
        if let Some((color, full)) = QuickInfoState::warning_color(
            warn_health,
            &mut state.health_warning,
            time,
            dt,
            self.normal,
            self.caution,
        ) {
            let origin = left_origin;
            let color = alpha(color, opacity);
            if full {
                self.font.draw(&self.left, origin.x, origin.y, color);
            } else {
                self.progress(
                    &self.left,
                    &self.left_empty,
                    origin,
                    1. - inv.health / 100.,
                    color,
                );
            }
        }
        let warn_ammo = state.warn_ammo;
        if let Some((color, full)) = QuickInfoState::warning_color(
            warn_ammo,
            &mut state.ammo_warning,
            time,
            dt,
            self.normal,
            self.caution,
        ) {
            let origin = right_origin;
            let color = alpha(color, opacity);
            if full {
                self.font.draw(&self.right, origin.x, origin.y, color);
            } else {
                let percentage = if weapon.magazine <= 0 {
                    0.
                } else {
                    1. - ammo as f32 / weapon.magazine as f32
                };
                self.progress(&self.right, &self.right_empty, origin, percentage, color);
            }
        }
        self.canvas.additive(false);
    }
}

#[cfg(test)]
mod tests;
