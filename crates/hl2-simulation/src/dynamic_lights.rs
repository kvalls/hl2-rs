//! `light_dynamic` (SDK server dynamiclight.cpp CDynamicLight, client c_dynamiclight.cpp
//! C_DynamicLight): a dlight the client renews every frame from the entity's networked fields.
//! A light with a cone (`_cone` above 0) lights models only (C_DynamicLight::ClientThink adds
//! DLIGHT_NO_WORLD_ILLUMINATION), and DynamicLightThink aims a light with a `target` at it
//! every 0.1 s. Model lighting takes it through the retail dlight conversion
//! ([`modkit_core::lighting::DynamicLight::spot_light`]). Not modeled: world illumination (a
//! light without a cone, and the spot's end dlight where a ray along the cone meets the world
//! within `distance`) and the light style's scale.
use crate::entities::{named, number};
use glam::Vec3;
use modkit_core::lighting::{DynamicCone, DynamicLight, WorldLight};
use modkit_core::World;
use std::collections::BTreeMap;

/// DLIGHT_NO_MODEL_ILLUMINATION.
const NO_MODEL_ILLUMINATION: u32 = 2;
/// DynamicLightThink's interval.
const THINK: f64 = 0.1;
/// MAX_DL_EXPONENT_VALUE with NUM_DL_EXPONENT_BITS 8; MIN_DL_EXPONENT_VALUE is its negative.
const MAX_EXPONENT: f32 = 127.;

#[derive(Clone, Debug, PartialEq)]
struct Light {
    /// `_light` through SetRenderColor (its fourth value is unused).
    color: [u8; 3],
    /// `brightness`, clamped by CDynamicLight::Spawn.
    exponent: i8,
    /// `distance`.
    radius: f32,
    cone: DynamicCone,
    /// m_ActualFlags (spawnflags): TurnOn restores them, TurnOff lights nothing.
    flags: u32,
    on: bool,
    target: Option<usize>,
    next_think: Option<f64>,
}

/// Every `light_dynamic` on the map, by entity id.
#[derive(Clone, Debug, Default)]
pub struct DynamicLights {
    lights: BTreeMap<usize, Light>,
}

/// AngleVectors' forward of a QAngle in degrees.
fn forward([pitch, yaw, _]: [f32; 3]) -> Vec3 {
    let (p, y) = (pitch.to_radians(), yaw.to_radians());
    Vec3::new(p.cos() * y.cos(), p.cos() * y.sin(), -p.sin())
}

fn triple(text: &str) -> [f32; 3] {
    let mut out = [0.; 3];
    for (slot, value) in out.iter_mut().zip(text.split_whitespace()) {
        *slot = value.parse().unwrap_or(0.);
    }
    out
}

impl DynamicLights {
    /// CDynamicLight::KeyValue and Spawn: on, its spawnflags as flags, `pitch` overriding the
    /// angles' pitch (negated) and, with a target, the first think 0.1 s after spawn.
    pub fn spawn(&mut self, world: &World, time: f64) {
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() != "light_dynamic" {
                continue;
            }
            let mut angles = triple(e.get("angles").unwrap_or(""));
            let pitch = number(e, "pitch", 0.);
            if pitch != 0. {
                angles[0] = -pitch;
            }
            let color = e
                .get("_light")
                .map_or([255.; 3], triple)
                .map(|c| c.clamp(0., 255.) as u8);
            let target = e.get("target").filter(|t| !t.is_empty());
            let exponent = number(e, "brightness", 0.).clamp(-MAX_EXPONENT, MAX_EXPONENT);
            self.lights.insert(
                id,
                Light {
                    color,
                    exponent: exponent as i8,
                    radius: number(e, "distance", 0.),
                    cone: DynamicCone {
                        direction: forward(angles),
                        inner: number(e, "_inner_cone", 0.),
                        outer: number(e, "_cone", 0.),
                    },
                    flags: number(e, "spawnflags", 0.) as u32,
                    on: true,
                    target: target.and_then(|t| named(world, t)),
                    next_think: target.map(|_| time + THINK),
                },
            );
        }
    }

    /// TurnOn, TurnOff and Toggle.
    pub fn input(&mut self, id: usize, input: &str) -> bool {
        let Some(light) = self.lights.get_mut(&id) else {
            return false;
        };
        light.on = match input {
            "turnon" => true,
            "turnoff" => false,
            "toggle" => !light.on,
            _ => return false,
        };
        true
    }

    /// DynamicLightThink: each due light faces its target's origin (VectorAngles, no roll).
    pub fn think(&mut self, time: f64, origin: impl Fn(usize) -> Option<Vec3>) {
        for (&id, light) in &mut self.lights {
            if light.next_think.is_none_or(|due| due > time) {
                continue;
            }
            light.next_think = Some(time + THINK);
            let aim = light
                .target
                .and_then(|target| Some(origin(target)? - origin(id)?));
            if let Some(direction) = aim.and_then(Vec3::try_normalize) {
                light.cone.direction = direction;
            }
        }
    }

    /// The model lights of the lights that are on, at their entities' origins (`None` for a
    /// removed entity), as world lights for the per-map minlight `floor`.
    pub fn model_lights(
        &self,
        origin: impl Fn(usize) -> Option<Vec3>,
        floor: f32,
    ) -> Vec<WorldLight> {
        self.lights
            .iter()
            .filter(|(_, l)| l.on && l.flags & NO_MODEL_ILLUMINATION == 0)
            .filter_map(|(&id, l)| {
                let light = DynamicLight {
                    origin: origin(id)?,
                    radius: l.radius,
                    color: l.color,
                    exponent: l.exponent,
                    minlight: 0.,
                };
                Some(light.spot_light(floor, l.cone))
            })
            .collect()
    }
}

/// An entity's origin unless it was removed.
fn live_origin(states: &[crate::entities::State], id: usize) -> Option<Vec3> {
    states.get(id).filter(|s| !s.killed).map(|s| s.origin)
}

impl crate::entities::Scene {
    /// env_beam strikes, light style frames and fades, and light_dynamic aims, after parented
    /// children moved.
    pub(crate) fn think_effects(&mut self) {
        self.beams.think(self.time);
        self.lightstyles.animate(self.time);
        let states = &self.states;
        self.dynamic_lights
            .think(self.time, |id| live_origin(states, id));
    }

    /// The model lights of the map's light_dynamic entities ([`DynamicLights::model_lights`]).
    pub fn dynamic_model_lights(&self, floor: f32) -> Vec<WorldLight> {
        self.dynamic_lights
            .model_lights(|id| live_origin(&self.states, id), floor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::lighting::EmitType;
    use modkit_core::Entity;

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    /// d1_trainstation_01's light_tracktrain2 and its target, plus a light that skips models.
    fn world() -> World {
        let entities = vec![
            entity(&[
                ("classname", "light_dynamic"),
                ("targetname", "light_tracktrain2"),
                ("target", "gman_head"),
                ("distance", "220"),
                ("brightness", "1"),
                ("_light", "79 232 198 450"),
                ("_inner_cone", "30"),
                ("_cone", "45"),
                ("angles", "0 0 0"),
            ]),
            entity(&[("classname", "info_target"), ("targetname", "gman_head")]),
            entity(&[
                ("classname", "light_dynamic"),
                ("spawnflags", "2"),
                ("brightness", "300"),
            ]),
        ];
        World {
            entities,
            ..World::default()
        }
    }

    #[test]
    fn a_dynamic_light_aims_at_its_target_every_tenth_and_switches() {
        let world = world();
        let mut lights = DynamicLights::default();
        lights.spawn(&world, 1.);
        let origins = [
            Vec3::new(-14636., -13864., -1192.),
            Vec3::new(-14576., -13860., -1212.),
        ];
        let origin = |id: usize| origins.get(id).copied().or(Some(Vec3::ZERO));
        // Before its first think it points along its angles.
        let light = &lights.model_lights(origin, 1.)[0];
        assert_eq!((light.kind, light.normal), (EmitType::Spot, Vec3::X));
        assert!((light.stopdot - 30f32.to_radians().cos()).abs() < 1e-6);
        assert!((light.stopdot2 - 45f32.to_radians().cos()).abs() < 1e-6);
        assert!((light.intensity - Vec3::new(79., 232., 198.) * 2. / 255.).length() < 1e-5);
        assert_eq!(lights.lights[&2].exponent, 127);
        lights.think(1.05, origin);
        assert_eq!(lights.model_lights(origin, 1.)[0].normal, Vec3::X);
        lights.think(1.1, origin);
        let toward = (origins[1] - origins[0]).normalize();
        assert!((lights.model_lights(origin, 1.)[0].normal - toward).length() < 1e-6);
        assert!(lights.lights[&0]
            .next_think
            .is_some_and(|t| (t - 1.2).abs() < 1e-9));
        // The light without model illumination never lights models.
        assert_eq!(lights.model_lights(origin, 1.).len(), 1);
        assert!(lights.input(0, "turnoff"));
        assert!(lights.model_lights(origin, 1.).is_empty());
        assert!(lights.input(0, "toggle"));
        assert_eq!(lights.model_lights(origin, 1.).len(), 1);
        assert!(!lights.input(1, "turnon"));
        // A removed light lights nothing.
        assert!(lights.model_lights(|_| None, 1.).is_empty());
    }
}
