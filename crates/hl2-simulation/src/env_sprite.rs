//! env_sprite (SDK shared/sprite.cpp CSprite; client c_sprite.cpp DrawSprite and
//! StandardGlowBlend). A named sprite starts hidden unless SF_SPRITE_STARTON (1); an unnamed one
//! shows; ShowSprite, HideSprite and ToggleSprite switch it. Drawn as a view-parallel quad the
//! size of its texture times `scale`, additive with rendercolor x renderamt:
//! kRenderTransAdd (5) depth-tested; kRenderGlow (3) without depth test, faded by
//! (1200 / distance)^2 (at most 1), hidden when the world blocks the eye's line to it (the
//! pixel-visibility query, here one brush ray) and grown by distance / 200 so it keeps its
//! screen size; kRenderWorldGlow (9) fades and hides the same but keeps its world size, as does
//! a glow with kRenderFxNoDissipation (renderfx 14), which also skips the fade. Other
//! render modes, frame animation, the proxy size and the HDR color scale are not modeled.
use crate::entities::number;
use crate::projectile_visuals::{Quad, GLOW_SUFFIX};
use glam::Vec3;
use modkit_core::{Entity, World};
use std::collections::BTreeMap;

/// Sprite materials of the installed maps' env_sprites that the effects load.
pub const MATERIALS: &[&str] = &["sprites/flare1", "sprites/glow01", "sprites/glow06"];
const STARTON: u32 = 1;
const GLOW: u8 = 3;
const ADD: u8 = 5;
const WORLD_GLOW: u8 = 9;
const NO_DISSIPATION: u8 = 14;

#[derive(Clone, Debug)]
struct Sprite {
    material: String,
    scale: f32,
    mode: u8,
    /// kRenderFxNoDissipation: no distance fade or growth.
    steady: bool,
    color: [f32; 3],
    on: bool,
}

#[derive(Clone, Debug, Default)]
pub struct EnvSprites {
    sprites: BTreeMap<usize, Sprite>,
}

/// `sprites/x.spr` and `materials/Sprites/x.vmt` name the material `sprites/x`.
pub fn material_name(model: &str) -> String {
    let lower = model.to_ascii_lowercase().replace('\\', "/");
    let name = lower.strip_prefix("materials/").unwrap_or(&lower);
    name.trim_end_matches(".spr")
        .trim_end_matches(".vmt")
        .to_owned()
}

fn spawn_one(e: &Entity) -> Option<Sprite> {
    let mode = number(e, "rendermode", 0.) as u8;
    if !matches!(mode, GLOW | ADD | WORLD_GLOW) {
        return None;
    }
    let rgb = modkit_core::parse_vec3(e.get("rendercolor").unwrap_or("255 255 255"))
        .unwrap_or(Vec3::splat(255.));
    let alpha = number(e, "renderamt", 255.).clamp(0., 255.) / 255.;
    let named = e.get("targetname").is_some_and(|n| !n.is_empty());
    Some(Sprite {
        material: material_name(e.get("model").filter(|m| !m.is_empty())?),
        scale: Some(number(e, "scale", 1.))
            .filter(|&s| s > 0.)
            .unwrap_or(1.),
        mode,
        steady: number(e, "renderfx", 0.) as u8 == NO_DISSIPATION,
        color: (rgb / 255. * alpha).to_array(),
        on: !named || number(e, "spawnflags", 0.) as u32 & STARTON != 0,
    })
}

impl EnvSprites {
    /// CSprite::Spawn of every env_sprite in a supported render mode.
    pub fn spawn(&mut self, world: &World) {
        self.sprites = (world.entities.iter().enumerate())
            .filter(|(_, e)| e.class() == "env_sprite")
            .filter_map(|(id, e)| Some((id, spawn_one(e)?)))
            .collect();
    }

    /// ShowSprite, HideSprite, ToggleSprite.
    pub fn input(&mut self, id: usize, input: &str) -> bool {
        let Some(sprite) = self.sprites.get_mut(&id) else {
            return false;
        };
        sprite.on = match input {
            "showsprite" => true,
            "hidesprite" => false,
            "togglesprite" => !sprite.on,
            _ => return false,
        };
        true
    }

    /// This frame's quads. `view` is (eye, right, up); `position` gives a sprite entity's
    /// current origin (parents move them); `size` a material's texture size; `visible`
    /// whether the world leaves the line from the eye to a point open.
    pub fn quads(
        &self,
        (eye, right, up): (Vec3, Vec3, Vec3),
        position: impl Fn(usize) -> Option<Vec3>,
        size: impl Fn(&str) -> Option<(f32, f32)>,
        visible: impl Fn(Vec3) -> bool,
        out: &mut Vec<Quad>,
    ) {
        for (&id, s) in &self.sprites {
            let (Some(center), Some((width, height))) =
                (s.on.then(|| position(id)).flatten(), size(&s.material))
            else {
                continue;
            };
            let distance = center.distance(eye);
            let (mut scale, mut blend) = (s.scale, 1.);
            if s.mode != ADD {
                if distance <= 0. || !visible(center) {
                    continue;
                }
                if !s.steady {
                    blend = (1200. * 1200. / (distance * distance)).min(1.);
                    if s.mode == GLOW {
                        scale *= distance / 200.;
                    }
                }
            }
            let color = s
                .color
                .map(|c| (c * blend * 255.).round().clamp(0., 255.) as u8);
            if color == [0; 3] {
                continue;
            }
            let (r, u) = (right * width * 0.5 * scale, up * height * 0.5 * scale);
            let material = if s.mode == GLOW {
                format!("{}{GLOW_SUFFIX}", s.material)
            } else {
                s.material.clone()
            };
            out.push(Quad {
                material,
                positions: [
                    center - r - u,
                    center - r + u,
                    center + r + u,
                    center + r - u,
                ],
                color: [color[0], color[1], color[2], 255],
                uv: [[0., 1.], [0., 0.], [1., 0.], [1., 1.]],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: [("classname", "env_sprite")]
                .iter()
                .chain(pairs)
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        }
    }

    #[test]
    fn named_glows_wait_for_show_keep_their_screen_size_and_hide_behind_walls() {
        let world = World {
            entities: vec![
                sprite(&[
                    ("targetname", "glow_1"),
                    ("model", "sprites/flare1.spr"),
                    ("rendermode", "3"),
                    ("origin", "400 0 0"),
                ]),
                sprite(&[
                    ("model", "materials/Sprites/light_glow03.vmt"),
                    ("rendermode", "5"),
                    ("renderamt", "128"),
                    ("origin", "0 0 100"),
                ]),
                sprite(&[("model", "sprites/glow01.spr"), ("rendermode", "0")]),
            ],
            ..Default::default()
        };
        let mut sprites = EnvSprites::default();
        sprites.spawn(&world);
        let view = (Vec3::ZERO, Vec3::Y, Vec3::Z);
        let at = |id: usize| Some(world.entities[id].origin());
        let size = |_: &str| Some((32., 32.));
        let mut out = Vec::new();
        sprites.quads(view, at, size, |_| true, &mut out);
        // Only the unnamed additive one shows, half bright, 32 units across.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].material, "sprites/light_glow03");
        assert_eq!(out[0].color, [128, 128, 128, 255]);
        assert!((out[0].positions[2].y - out[0].positions[0].y - 32.).abs() < 1e-4);
        assert!(sprites.input(0, "showsprite"));
        out.clear();
        sprites.quads(view, at, size, |_| true, &mut out);
        // The glow at 400 units is twice its texture size, without depth test.
        let glow = out
            .iter()
            .find(|q| q.material.ends_with(GLOW_SUFFIX))
            .expect("glow");
        assert!((glow.positions[2].y - glow.positions[0].y - 64.).abs() < 1e-3);
        out.clear();
        sprites.quads(view, at, size, |p| p.z > 50., &mut out);
        assert_eq!(out.len(), 1);
        // kRenderFxNoDissipation keeps the world size.
        sprites.sprites.get_mut(&0).expect("glow").steady = true;
        out.clear();
        sprites.quads(view, at, size, |_| true, &mut out);
        let glow = out
            .iter()
            .find(|q| q.material.ends_with(GLOW_SUFFIX))
            .expect("glow");
        assert!((glow.positions[2].y - glow.positions[0].y - 32.).abs() < 1e-3);
    }
}
