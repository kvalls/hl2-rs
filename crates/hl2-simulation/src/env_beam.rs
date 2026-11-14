//! env_beam between two named entities (SDK EnvBeam.cpp CEnvBeam; client view_beams.cpp and
//! beamdraw.cpp DrawSegs). With `life` 0 it is a server-side CBeam drawn every frame while
//! active; otherwise each strike is a temporary beam lasting `life`, struck again after
//! life + StrikeTime s (plus 0..StrikeTime at random with SF_BEAM_RANDOM). A named beam starts
//! off unless SF_BEAM_STARTON; TurnOn, TurnOff, Toggle and StrikeOnce drive it.
//!
//! Drawing follows DrawSegs, additive (kRenderTransAdd): fractal noise (Noise over 128
//! divisions, made again every frame) moves each segment across the beam by up to
//! NoiseAmplitude x length / 100 units, 0.25 segments a unit (+3) above amplitude 0.5, the
//! texture tiles once a 100 units and scrolls TextureScroll lengths a second, the full width is
//! twice BoltWidth (SF_BEAM_TAPEROUT narrows it to 0 at the end) and the color is rendercolor
//! x renderamt. Not modeled: rings, strikes without LightningEnd (random area or point), sparks,
//! decals, shade in/out, the sprite's frame animation and the HDR color scale.
use crate::entities::number;
use crate::projectile_visuals::Quad;
use glam::Vec3;
use modkit_core::{Entity, World};
use std::collections::BTreeMap;

/// Beam sprites the installed maps' env_beams use (`texture` without `.spr`).
pub const MATERIALS: &[&str] = &["sprites/plasma", "sprites/laserbeam"];
const NOISE_DIVISIONS: usize = 128;
const STARTON: u32 = 1;
const RANDOM: u32 = 4;
const RING: u32 = 8;
const TAPEROUT: u32 = 512;

/// One env_beam's keyvalues, read once.
#[derive(Clone, Debug)]
struct Spec {
    start: Option<usize>,
    end: Option<usize>,
    material: String,
    life: f32,
    restrike: f32,
    random: bool,
    width: f32,
    taper: bool,
    amplitude: f32,
    speed: f32,
    color: [u8; 3],
}

#[derive(Clone, Debug)]
struct Beam {
    spec: Spec,
    active: bool,
    /// The current strike's end (striking beams).
    strike_until: f64,
    /// StrikeThink's next think.
    next_strike: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct EnvBeams {
    beams: BTreeMap<usize, Beam>,
    seed: u64,
}

fn named(world: &World, name: &str) -> Option<usize> {
    (!name.is_empty()).then_some(())?;
    world.entities.iter().position(|e| {
        e.get("targetname")
            .is_some_and(|t| t.eq_ignore_ascii_case(name))
    })
}

fn spec(world: &World, e: &Entity) -> Spec {
    let flags = number(e, "spawnflags", 0.) as u32;
    let renderamt = number(e, "renderamt", 255.).clamp(0., 255.) / 255.;
    let rgb = modkit_core::parse_vec3(e.get("rendercolor").unwrap_or("255 255 255"))
        .unwrap_or(Vec3::splat(255.));
    let texture = e.get("texture").unwrap_or("");
    Spec {
        start: named(world, e.get("LightningStart").unwrap_or("")),
        end: named(world, e.get("LightningEnd").unwrap_or("")),
        material: texture
            .trim_end_matches(".spr")
            .trim_end_matches(".vmt")
            .to_owned(),
        life: number(e, "life", 0.),
        restrike: number(e, "StrikeTime", 1.),
        random: flags & RANDOM != 0,
        width: number(e, "BoltWidth", 2.).min(102.3),
        taper: flags & TAPEROUT != 0,
        amplitude: number(e, "NoiseAmplitude", 0.).min(64.),
        speed: number(e, "TextureScroll", 35.).clamp(0., 100.),
        color: (rgb * renderamt)
            .to_array()
            .map(|c| c.round().clamp(0., 255.) as u8),
    }
}

/// Fractal noise of power-of-two length, normalized to +/- `scale` (view_beams.cpp Noise).
fn noise(values: &mut [f32], scale: f32, random: &mut impl FnMut() -> f32) {
    let divs = values.len() - 1;
    if divs < 2 {
        return;
    }
    let half = divs / 2;
    values[half] = (values[0] + values[divs]) * 0.5 + scale * (random() * 2. - 1.);
    if half > 1 {
        noise(&mut values[half..], scale * 0.5, random);
        noise(&mut values[..=half], scale * 0.5, random);
    }
}

impl EnvBeams {
    /// CEnvBeam::Spawn of every env_beam of the map.
    pub fn spawn(&mut self, world: &World, time: f64) {
        self.beams.clear();
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() != "env_beam" || e.get("texture").is_none_or(str::is_empty) {
                continue;
            }
            let spec = spec(world, e);
            let flags = number(e, "spawnflags", 0.) as u32;
            let named = e.get("targetname").is_some_and(|n| !n.is_empty());
            let server = spec.life == 0. && flags & RING == 0;
            let on = !named || flags & STARTON != 0;
            let beam = Beam {
                active: server && on,
                strike_until: f64::NEG_INFINITY,
                next_strike: (!server && on).then_some(time + 1.),
                spec,
            };
            self.beams.insert(id, beam);
        }
    }

    /// TurnOn (`Some(true)`), TurnOff (`Some(false)`), Toggle (`None`) or StrikeOnce.
    pub fn input(&mut self, id: usize, input: &str, time: f64) -> bool {
        let Some(beam) = self.beams.get_mut(&id) else {
            return false;
        };
        let server = beam.spec.life == 0.;
        let on = match input {
            "turnon" => true,
            "turnoff" => false,
            "toggle" => !(beam.active || beam.next_strike.is_some()),
            "strikeonce" => {
                beam.strike_until = time + f64::from(beam.spec.life);
                return true;
            }
            _ => return false,
        };
        if server {
            beam.active = on;
        } else {
            beam.next_strike = on.then_some(time);
            beam.active = false;
        }
        true
    }

    /// StrikeThink: striking beams strike when due.
    pub fn think(&mut self, time: f64) {
        for beam in self.beams.values_mut() {
            let Some(due) = beam.next_strike.filter(|&due| due <= time) else {
                continue;
            };
            self.seed = self
                .seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let extra = if beam.spec.random {
                (self.seed >> 40) as f32 / (1u64 << 24) as f32 * beam.spec.restrike
            } else {
                beam.spec.restrike
            };
            let life = f64::from(beam.spec.life);
            beam.strike_until = due + life;
            beam.next_strike = (life > 0.).then_some(due + life + f64::from(extra));
        }
    }

    /// This frame's beam quads: `position` gives an entity's origin, `view` is
    /// (eye, forward), `random` a uniform 0..1 source for the noise.
    pub fn quads(
        &self,
        time: f64,
        (eye, forward): (Vec3, Vec3),
        position: impl Fn(usize) -> Option<Vec3>,
        random: &mut impl FnMut() -> f32,
        out: &mut Vec<Quad>,
    ) {
        let mut values = [0f32; NOISE_DIVISIONS + 1];
        for beam in self.beams.values() {
            let s = &beam.spec;
            if !(beam.active || time < beam.strike_until) {
                continue;
            }
            let (Some(a), Some(b)) = (s.start.and_then(&position), s.end.and_then(&position))
            else {
                continue;
            };
            values.fill(0.);
            if s.amplitude != 0. {
                noise(&mut values, 1., random);
            }
            segments(s, (a, b), (eye, forward), time, &values, out);
        }
    }
}

/// DrawSegs: the beam from `a` to `b` as quads between its noisy segment points.
fn segments(
    s: &Spec,
    (a, b): (Vec3, Vec3),
    (eye, forward): (Vec3, Vec3),
    time: f64,
    noise: &[f32],
    out: &mut Vec<Quad>,
) {
    let delta = b - a;
    let length = delta.length();
    if length < 1e-3 {
        return;
    }
    let per_unit = if s.amplitude >= 0.5 { 0.25 } else { 0.075 };
    let mut count = (length * per_unit + 3.) as usize;
    let half_width = s.width;
    if length / ((count - 1) as f32) < half_width * 1.414 {
        count = ((length / (half_width * 1.414)) as usize + 1).max(2);
    }
    count = count.min(NOISE_DIVISIONS);
    let div = 1. / (count - 1) as f32;
    let v_step = length * 0.01 * div;
    let scale = s.amplitude * length * 0.01;
    let perp = forward.cross(delta / length).normalize_or_zero();
    let noise_step = ((NOISE_DIVISIONS - 1) as f32 * div * 65536.) as usize;
    let points: Vec<(Vec3, f32, f32)> = (0..count)
        .map(|i| {
            let fraction = i as f32 * div;
            let offset = noise[(i * noise_step) >> 16] * scale;
            let width = if s.taper {
                half_width * (1. - fraction)
            } else {
                half_width
            } * 2.;
            let v = ((time * f64::from(s.speed)).rem_euclid(1.)) as f32 + v_step * i as f32;
            (a + delta * fraction + perp * offset, width, v)
        })
        .collect();
    let side = |i: usize| {
        let (p, width, _) = points[i];
        let tangent = points[(i + 1).min(count - 1)].0 - points[i.saturating_sub(1)].0;
        tangent.cross(p - eye).normalize_or_zero() * width * 0.5
    };
    let color = [s.color[0], s.color[1], s.color[2], 255];
    for i in 0..count - 1 {
        let (p0, _, v0) = points[i];
        let (p1, _, v1) = points[i + 1];
        let (n0, n1) = (side(i), side(i + 1));
        out.push(Quad {
            material: s.material.clone(),
            positions: [p0 - n0, p0 + n0, p1 + n1, p1 - n1],
            color,
            uv: [[0., v0], [1., v0], [1., v1], [0., v1]],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beam(life: f32, flags: u32) -> Beam {
        Beam {
            spec: Spec {
                start: Some(0),
                end: Some(1),
                material: "sprites/plasma".into(),
                life,
                restrike: 2.,
                random: false,
                width: 3.,
                taper: false,
                amplitude: 8.,
                speed: 10.,
                color: [255, 255, 255],
            },
            active: flags & STARTON != 0 && life == 0.,
            strike_until: f64::NEG_INFINITY,
            next_strike: None,
        }
    }

    #[test]
    fn a_toggled_server_beam_draws_noisy_segments_while_on() {
        let mut beams = EnvBeams::default();
        beams.beams.insert(5, beam(0., 0));
        let at = |id: usize| [Some(Vec3::ZERO), Some(Vec3::new(0., 0., -400.))][id];
        let mut seed = 1u32;
        let mut random = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let view = (Vec3::new(300., 0., -200.), Vec3::NEG_X);
        let mut out = Vec::new();
        beams.quads(0., view, at, &mut random, &mut out);
        assert!(out.is_empty());
        assert!(beams.input(5, "turnon", 0.));
        beams.quads(0., view, at, &mut random, &mut out);
        // 0.25 a unit + 3 segments, fewer where a segment would be shorter than 1.414 x the
        // half width (3): 95 points, 94 quads 6 units wide.
        assert_eq!(out.len(), 94);
        let q = &out[60];
        assert!(((q.positions[1] - q.positions[0]).length() - 6.).abs() < 0.1);
        // Noise moves points across the beam (y): fractal noise under 2 x 8 % of 400 units.
        assert!(out.iter().any(|q| q.positions[0].y.abs() > 0.5));
        assert!(out.iter().all(|q| q.positions[0].y.abs() < 64. + 3.1));
        // The texture tiles once a 100 units along the beam.
        assert!((out[93].uv[2][1] - out[0].uv[0][1] - 4.).abs() < 0.01);
    }

    #[test]
    fn a_striking_beam_strikes_for_its_life_then_waits_its_strike_time() {
        let mut beams = EnvBeams::default();
        beams.beams.insert(5, beam(4., 0));
        beams.input(5, "turnon", 10.);
        beams.think(10.);
        let lit = |b: &EnvBeams, t: f64| t < b.beams[&5].strike_until;
        assert!(lit(&beams, 13.9) && !lit(&beams, 14.1));
        beams.think(14.5);
        assert!(!lit(&beams, 14.6));
        beams.think(16.);
        assert!(lit(&beams, 16.5));
    }
}
