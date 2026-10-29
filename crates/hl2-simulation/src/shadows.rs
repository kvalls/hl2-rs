//! Dynamic shadows of studio models on the world (Source SDK `game/client/clientshadowmgr.cpp`): the
//! map's `shadow_control` settings, each caster's projection frame ([`frame`]) and the shadow's
//! decal geometry clipped onto the world's triangles ([`receivers`]). Source units, Z up. The
//! host rasterizes and draws them; this module has no engine.

pub mod allocator;
pub mod frame;
pub mod receivers;
pub mod silhouette;

use glam::Vec3;
use modkit_core::{lighting::EmitType, World};

/// Shadow texels per unit of caster size (`TEXEL_SIZE_PER_CASTER_SIZE`).
pub const TEXELS_PER_UNIT: f32 = 2.;
/// How much a shadow lightens at the end of its cast distance, of 255 (`MAX_FALLOFF_AMOUNT`).
pub const MAX_FALLOFF: f32 = 240.;

/// The map's shadow settings: `shadow_control`, else the engine's defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowControl {
    /// Unit direction from caster toward receiver.
    pub direction: Vec3,
    /// Shadow colour (bytes, gamma space).
    pub color: [u8; 3],
    /// Cast distance beyond the caster, units.
    pub distance: f32,
    /// `disableallshadows`: no render-to-texture shadows (blobs remain).
    pub disabled: bool,
}

/// The forward vector of Source angles in degrees (pitch positive down).
pub fn forward(pitch: f32, yaw: f32) -> Vec3 {
    let (sp, cp) = pitch.to_radians().sin_cos();
    let (sy, cy) = yaw.to_radians().sin_cos();
    Vec3::new(cp * cy, cp * sy, -sp)
}

fn parse3(text: Option<&str>) -> Option<Vec3> {
    text.and_then(modkit_core::parse_vec3)
        .map(|v| Vec3::from_array(v.to_array()))
}

impl ShadowControl {
    /// The settings of the map's first `shadow_control` (`C_ShadowControl`: `angles`, forward of
    /// (80, 30) when all zero, a legacy `direction` only without angles; colour 64 64 64 and distance
    /// 50 when missing), or without one `CClientShadowMgr`'s level defaults: direction (0.1, 0.1, -1),
    /// distance 50 and a colour from the level's ambient light (`3 * ambient + 0.3`, at most 1).
    pub fn from_world(world: &World) -> Self {
        let Some(e) = world
            .entities
            .iter()
            .find(|e| e.class() == "shadow_control")
        else {
            return Self {
                direction: Vec3::new(0.1, 0.1, -1.).normalize(),
                color: ambient_shadow_color(sky_ambient(world)),
                distance: 50.,
                disabled: false,
            };
        };
        let direction = match parse3(e.get("angles")) {
            Some(a) if a == Vec3::ZERO => forward(80., 30.),
            Some(a) => forward(a.x, a.y),
            None => parse3(e.get("direction"))
                .filter(|d| d.length_squared() > 0.)
                .unwrap_or(Vec3::new(0.2, 0.2, -2.))
                .normalize(),
        };
        let color = parse3(e.get("color"))
            .map_or([64; 3], |c| c.to_array().map(|v| v.clamp(0., 255.) as u8));
        let number = |key: &str| e.get(key).and_then(|v| v.trim().parse::<f32>().ok());
        Self {
            direction,
            color,
            distance: number("distance").unwrap_or(50.),
            disabled: number("disableallshadows").is_some_and(|v| v != 0.),
        }
    }
}

/// The level's ambient light: the sky ambient world light's intensity (the engine's ambient light
/// colour; inferred, the engine's query is not public), black without one.
fn sky_ambient(world: &World) -> Vec3 {
    world
        .lighting
        .as_deref()
        .and_then(|l| {
            l.lights
                .iter()
                .find(|light| light.kind == EmitType::SkyAmbient)
        })
        .map_or(Vec3::ZERO, |light| {
            Vec3::from_array(light.intensity.to_array())
        })
}

/// `CClientShadowMgr::LevelInitPreEntity`: `3 * ambient + 0.3`, 255 above 1, else truncated bytes.
pub fn ambient_shadow_color(ambient: Vec3) -> [u8; 3] {
    (ambient * 3. + Vec3::splat(0.3))
        .to_array()
        .map(|c| if c > 1. { 255 } else { (255. * c) as u8 })
}

/// The receiver shader's colour: gamma-to-linear with the 2.2 curve (from 0.95 up, 1).
pub fn linear_color(color: [u8; 3]) -> [f32; 3] {
    color.map(|v| {
        let x = v as f32 / 255.;
        if x >= 0.95 {
            1.
        } else {
            x.powf(2.2)
        }
    })
}

#[cfg(test)]
mod tests;
