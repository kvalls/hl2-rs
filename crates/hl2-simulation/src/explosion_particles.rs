//! Shared owned-material explosion emitters. Emission ranges are corroborated
//! with the retail client and pinned SDK; RNG, ambient lighting and collision
//! sampling are not complete Source particle-manager equivalents.
use crate::{
    physics::Physics,
    projectile_visuals::Quad,
    projectiles::{Effect, EffectKind},
};
use glam::Vec3;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const MATERIALS: &[&str] = &[
    "particle/particle_noisesphere",
    "effects/fire_cloud2",
    "effects/fire_embers1",
    "effects/fire_embers2",
    "effects/spark",
    "effects/yellowflare_noz",
    "sprites/lgtning",
    "effects/fleck_cement1",
    "effects/fleck_cement2",
];
const MAX_PARTICLES: usize = 4096;
const STEP: f64 = 0.015;
pub const UV: [[f32; 2]; 4] = [[0., 1.], [0., 0.], [1., 0.], [1., 1.]];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Kind {
    Smoke,
    Fire,
    Ember,
    Spark,
    Glow,
    Debris,
}
#[derive(Clone, Copy)]
enum Motion {
    Explosion,
    Free,
    Trail {
        gravity: f32,
        drag: f32,
        bounce: f32,
        collide: bool,
    },
}
#[derive(Clone)]
struct Particle {
    kind: Kind,
    material: &'static str,
    position: Vec3,
    velocity: Vec3,
    born: f64,
    updated: f64,
    life: f32,
    sizes: [f32; 2],
    alpha: [f32; 2],
    color: Vec3,
    roll: f32,
    roll_rate: f32,
    motion: Motion,
    trail: f32,
    near_clip: bool,
}
struct Ring {
    center: Vec3,
    born: f64,
    life: f32,
    diameter: f32,
    alpha: f32,
}
#[derive(Default, Serialize)]
pub struct Diagnostics {
    pub emitted: BTreeMap<Kind, u64>,
    pub live: BTreeMap<Kind, usize>,
    pub events: u64,
    pub rings_emitted: u64,
    pub rings_live: usize,
    pub collisions: u64,
    pub capacity_rejections: u64,
}
#[derive(Default)]
pub struct Particles {
    particles: Vec<Particle>,
    rings: Vec<Ring>,
    seen: BTreeSet<u64>,
    pub diagnostics: Diagnostics,
    last_time: Option<f64>,
}
struct Random(u64);
impl Random {
    fn unit(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u32 << 24) as f32
    }
    fn float(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
    fn int(&mut self, lo: i32, hi: i32) -> i32 {
        lo + ((hi - lo + 1) as f32 * self.unit()) as i32
    }
    fn vector(&mut self, lo: f32, hi: f32) -> Vec3 {
        Vec3::new(self.float(lo, hi), self.float(lo, hi), self.float(lo, hi))
    }
}
impl Particle {
    fn new(kind: Kind, material: &'static str, effect: &Effect, life: f32) -> Self {
        Self {
            kind,
            material,
            position: effect.position,
            velocity: Vec3::ZERO,
            born: effect.at,
            updated: effect.at,
            life,
            sizes: [1.; 2],
            alpha: [1., 0.],
            color: Vec3::ONE,
            roll: 0.,
            roll_rate: 0.,
            motion: Motion::Free,
            trail: 0.,
            near_clip: false,
        }
    }
    fn update(&mut self, dt: f32, physics: &Physics) -> bool {
        let mut collided = false;
        match self.motion {
            Motion::Explosion => {
                let direction = self.velocity.normalize_or_zero();
                self.velocity *= (-18.420681 * dt).exp();
                if self.velocity.length_squared() < 1024. {
                    self.velocity = direction * 32.;
                }
                self.position += self.velocity * dt;
                self.roll += self.roll_rate * dt;
                self.roll_rate *= 1. - 8. * dt;
                if self.roll_rate.abs() < 0.5 {
                    self.roll_rate = if self.roll_rate > 0. { 0.5 } else { -0.5 };
                }
            }
            Motion::Free => {
                self.position += self.velocity * dt;
                self.roll += self.roll_rate * dt;
            }
            Motion::Trail {
                gravity,
                drag,
                bounce,
                collide,
            } => {
                if self.velocity == Vec3::ZERO {
                    return false;
                }
                self.velocity.z -= gravity * dt;
                let end = self.position + self.velocity * dt;
                if let Some(hit) = collide
                    .then(|| physics.projectile_ray(self.position, end, &[]))
                    .flatten()
                {
                    self.position = hit.position + hit.normal * 0.01;
                    if hit.normal.z >= 0.5 && self.velocity.z.abs() <= 48. {
                        self.velocity = Vec3::ZERO;
                        self.roll_rate = 0.;
                    } else {
                        self.velocity = (self.velocity
                            - 2. * self.velocity.dot(hit.normal) * hit.normal)
                            * bounce;
                        self.roll_rate *= -0.25;
                    }
                    collided = true;
                } else {
                    self.position = end;
                }
                self.velocity *= (1. - drag * dt).max(0.);
                self.roll += self.roll_rate * dt;
            }
        }
        collided
    }
}

impl Particles {
    fn add(&mut self, p: Particle) {
        if self.particles.len() >= MAX_PARTICLES {
            self.diagnostics.capacity_rejections += 1;
            return;
        }
        *self.diagnostics.emitted.entry(p.kind).or_default() += 1;
        self.particles.push(p);
    }
    pub fn sync(&mut self, effects: &[Effect], time: f64, physics: &Physics) {
        if !time.is_finite() {
            return;
        }
        if self.last_time.is_some_and(|t| time < t) {
            *self = Self::default();
        }
        self.last_time = Some(time);
        self.seen.retain(|id| effects.iter().any(|e| e.id == *id));
        for effect in effects {
            if !self.seen.insert(effect.id) {
                continue;
            }
            self.diagnostics.events += 1;
            let mut rng = Random(
                effect.id.wrapping_mul(0x9e3779b97f4a7c15) ^ effect.at.to_bits() ^ 0x953732fb,
            );
            match effect.kind {
                EffectKind::GrenadeExplosion => self.grenade(effect, physics, &mut rng),
                EffectKind::BallImpact => self.electric(effect, 2, &mut rng),
                EffectKind::BallExplosion => {
                    self.electric(effect, 4, &mut rng);
                    for (life, alpha) in [(0.2, 32. / 255.), (0.5, 64. / 255.)] {
                        if self.rings.len() >= 64 {
                            self.diagnostics.capacity_rejections += 1;
                            continue;
                        }
                        self.rings.push(Ring {
                            center: effect.position,
                            born: effect.at,
                            life,
                            diameter: effect.radius,
                            alpha,
                        });
                        self.diagnostics.rings_emitted += 1;
                    }
                }
                // Sprite-only (projectile_visuals) or not drawn yet.
                EffectKind::BoltGlow | EffectKind::Sparks => {}
            }
        }
        for particle in &mut self.particles {
            let until = time.min(particle.born + f64::from(particle.life));
            // At most eight seconds of catch-up for the longest electric sparks.
            for _ in 0..540 {
                let dt = (until - particle.updated).min(STEP);
                if dt <= 1e-9 {
                    break;
                }
                if particle.update(dt as f32, physics) {
                    self.diagnostics.collisions += 1;
                }
                particle.updated += dt;
            }
        }
        self.particles.retain(|p| time < p.born + f64::from(p.life));
        self.rings.retain(|r| time < r.born + f64::from(r.life));
        self.diagnostics.live.clear();
        for p in &self.particles {
            *self.diagnostics.live.entry(p.kind).or_default() += 1;
        }
        self.diagnostics.rings_live = self.rings.len();
    }

    fn electric(&mut self, e: &Effect, magnitude: i32, rng: &mut Random) {
        let normal = e.normal.normalize_or_zero();
        let count = (magnitude * magnitude) as f32 * rng.float(2., 4.);
        for _ in 0..count as usize {
            let mut p = Particle::new(
                Kind::Spark,
                "effects/spark",
                e,
                magnitude as f32 * rng.float(1., 2.),
            );
            let mut direction = rng.vector(-1., 1.);
            direction.z = rng.float(0.5, 1.);
            p.velocity = (direction + normal * 2.).normalize_or_zero() * rng.float(64., 300.);
            p.sizes = [rng.float(2., 5.); 2];
            p.alpha = [1.; 2];
            p.trail = rng.float(0.02, 0.05);
            p.motion = Motion::Trail {
                gravity: 800.,
                drag: 0.,
                bounce: 0.3,
                collide: true,
            };
            self.add(p);
        }
        let count = magnitude * rng.int(16, 32);
        for _ in 0..count {
            let mut p = Particle::new(
                Kind::Spark,
                "effects/spark",
                e,
                magnitude as f32 * rng.float(0.1, 0.2),
            );
            p.velocity = (rng.vector(-1., 1.) + normal).normalize_or_zero() * rng.float(128., 256.);
            p.sizes = [rng.float(2., 4.); 2];
            p.alpha = [1.; 2];
            p.trail = rng.float(0.02, 0.03);
            p.motion = Motion::Trail {
                gravity: 400.,
                drag: 0.,
                bounce: 0.5,
                collide: false,
            };
            self.add(p);
        }
        for outer in [false, true] {
            let mut p = Particle::new(Kind::Glow, "effects/yellowflare_noz", e, 0.2);
            let value = if outer {
                rng.int(32, 64) as f32 / 255.
            } else {
                1.
            };
            p.color = Vec3::splat(value);
            p.alpha = [if outer { value } else { 1. }, if outer { 0. } else { 1. }];
            p.sizes = [
                (magnitude
                    * if outer {
                        rng.int(32, 64)
                    } else {
                        rng.int(4, 8)
                    }) as f32,
                0.,
            ];
            p.roll = rng.int(0, 360) as f32;
            p.roll_rate = if outer { rng.float(-1., 1.) } else { 0. };
            self.add(p);
        }
        let mut p = Particle::new(Kind::Smoke, "particle/particle_noisesphere", e, 1.);
        p.position += Vec3::new(rng.float(-4., 4.), rng.float(-4., 4.), 0.);
        p.velocity = Vec3::new(rng.float(-16., 16.), rng.float(-16., 16.), 16.);
        p.color = Vec3::new(1., 1., 200. / 255.);
        p.alpha = [rng.int(16, 32) as f32 / 255., 0.];
        p.sizes[0] = rng.int(4, 8) as f32;
        p.sizes[1] = p.sizes[0] * 4.;
        p.roll = rng.int(0, 360) as f32;
        p.roll_rate = rng.float(-2., 2.);
        self.add(p);
    }

    fn grenade(&mut self, e: &Effect, physics: &Physics, rng: &mut Random) {
        let direction = force_direction(e.position, e.magnitude, physics);
        // Retail clamps core force to 2; spread is therefore 0.7.
        let spread = 0.7;
        for (count, life, offset, size, speed, roll) in [
            (4, [2., 3.], 0., [72., 72.], [1., 750.], 2.),
            (8, [0.5, 1.], 16., [32., 64.], [1., 2000.], 8.),
            (32, [0.5, 1.5], 4., [16., 32.], [500., 2000.], 8.),
        ] {
            for index in 0..count {
                let mut p = Particle::new(
                    Kind::Smoke,
                    "particle/particle_noisesphere",
                    e,
                    rng.float(life[0], life[1]),
                );
                let dir = if count == 32 {
                    let angle = std::f32::consts::TAU * (index + 1) as f32 / 32.;
                    let dir = Vec3::new(angle.cos(), angle.sin(), 0.);
                    p.position += rng.vector(-offset, offset) + dir * rng.float(8., 16.);
                    dir
                } else {
                    p.position += rng.vector(-offset, offset);
                    (rng.vector(-spread, spread) + direction * rng.float(1., 6.))
                        .normalize_or_zero()
                };
                let deviation = if count == 32 {
                    spread
                } else {
                    spread * dir.dot(direction).abs()
                };
                p.velocity = dir * rng.float(speed[0], speed[1]) * 2. * deviation;
                // Ambient cubes/WorldGetLightForPoint are still missing. Preserve
                // dark smoke explicitly rather than treating it as additive fire.
                p.color = Vec3::splat(rng.float(0.15, 0.3));
                p.sizes[0] = rng.int(size[0] as i32, size[1] as i32) as f32;
                p.sizes[1] = p.sizes[0] * if count == 32 { 4. } else { 2. };
                p.alpha = [
                    if count == 32 {
                        rng.int(16, 32) as f32 / 255.
                    } else {
                        1.
                    },
                    0.,
                ];
                p.roll = rng.int(0, 360) as f32;
                p.roll_rate = rng.float(-roll, roll);
                p.motion = Motion::Explosion;
                p.near_clip = true;
                self.add(p);
            }
        }
        for fire in [false, true] {
            for _ in 0..if fire { 32 } else { 16 } {
                let material = if fire {
                    "effects/fire_cloud2"
                } else if rng.int(0, 1) == 0 {
                    "effects/fire_embers1"
                } else {
                    "effects/fire_embers2"
                };
                let mut p = Particle::new(
                    if fire { Kind::Fire } else { Kind::Ember },
                    material,
                    e,
                    if fire {
                        rng.float(0.2, 0.4)
                    } else {
                        rng.float(2., 3.)
                    },
                );
                p.position +=
                    rng.vector(if fire { -48. } else { -32. }, if fire { 48. } else { 32. });
                let range = if fire { spread * 0.75 } else { spread * 2. };
                let dir = (rng.vector(-range, range) + direction).normalize_or_zero();
                let deviation = spread * dir.dot(direction).abs();
                let speed = if fire {
                    rng.float(400., 800.)
                } else {
                    rng.float(1., 400.)
                };
                p.velocity = dir * speed * 8. * deviation * deviation;
                p.color = Vec3::splat(
                    if fire {
                        rng.int(128, 255)
                    } else {
                        rng.int(192, 255)
                    } as f32
                        / 255.,
                );
                let initial = rng.int(if fire { 32 } else { 8 }, if fire { 85 } else { 16 }) as f32
                    * deviation;
                p.sizes[0] = initial
                    .clamp(if fire { 32. } else { 4. }, if fire { 85. } else { 32. })
                    .floor();
                p.sizes[1] = if fire {
                    (p.sizes[0] * 1.5).floor()
                } else {
                    p.sizes[0]
                };
                p.roll = rng.int(0, 360) as f32;
                p.roll_rate = rng.float(if fire { -16. } else { -8. }, if fire { 16. } else { 8. });
                p.motion = Motion::Explosion;
                p.near_clip = true;
                self.add(p);
            }
        }
        for _ in 0..rng.int(8, 16) {
            let mut p = Particle::new(Kind::Fire, "effects/fire_cloud2", e, rng.float(0.1, 0.15));
            p.velocity =
                (rng.vector(-1., 1.) + direction).normalize_or_zero() * rng.float(1500., 2500.);
            p.sizes = [rng.float(2., 16.); 2];
            p.alpha = [1.; 2];
            p.trail = rng.float(0.05, 0.1);
            p.motion = Motion::Trail {
                gravity: 200.,
                drag: 8.,
                bounce: 0.5,
                collide: false,
            };
            self.add(p);
        }
        for _ in 0..rng.int(16, 32) {
            let mat = if rng.int(0, 1) == 0 {
                "effects/fleck_cement1"
            } else {
                "effects/fleck_cement2"
            };
            let mut p = Particle::new(Kind::Debris, mat, e, 3.);
            p.position += direction * 16. + rng.vector(-8., 8.);
            let dir = (direction + rng.vector(-1., 1.)).normalize_or_zero();
            let size = rng.int(1, 3) as f32;
            p.sizes = [size; 2];
            let deviation = 0.8 * dir.dot(direction).abs();
            p.velocity = dir * rng.float(64., 256.) * (4. - size) * 8. * deviation * deviation;
            p.color = Vec3::splat((0.25 * rng.float(0.5, 1.5)).min(1.));
            p.roll = rng.float(0., 360.);
            p.roll_rate = rng.float(0., 360.);
            p.motion = Motion::Trail {
                gravity: 800.,
                drag: 0.,
                bounce: 0.5,
                collide: true,
            };
            self.add(p);
        }
    }

    pub fn draw(&self, eye: Vec3, direction: Vec3, time: f64, physics: &Physics) -> Vec<Quad> {
        let mut result = Vec::new();
        let (right, up) = crate::projectile_visuals::basis(direction);
        let mut sorted: Vec<_> = self.particles.iter().collect();
        sorted.sort_by(|a, b| {
            (b.position - eye)
                .dot(direction)
                .total_cmp(&(a.position - eye).dot(direction))
        });
        for p in sorted {
            let age = ((time - p.born).max(0.) as f32 / p.life).clamp(0., 1.);
            let size = p.sizes[0] + (p.sizes[1] - p.sizes[0]) * age;
            let near = if p.near_clip {
                (((p.position - eye).dot(direction) - 64.) / 64.).clamp(0., 1.)
            } else {
                1.
            };
            let ramp = if matches!(p.motion, Motion::Explosion) {
                (1. - age).powi(2)
            } else {
                1.
            };
            let alpha = if matches!(p.motion, Motion::Explosion) {
                ramp
            } else {
                p.alpha[0] + (p.alpha[1] - p.alpha[0]) * age
            } * near;
            if alpha <= 0. || size <= 0. {
                continue;
            }
            // Native no-Z glows require pixel visibility gating. Keep this
            // bounded substitute depth-tested and suppress a blocked center.
            if p.kind == Kind::Glow && physics.projectile_ray(eye, p.position, &[]).is_some() {
                continue;
            }
            let color = rgba(p.color * ramp, alpha);
            let positions = if p.trail > 0. {
                let delta = p.velocity * (p.trail * (1. - age)).max(0.01);
                let side = delta.cross(p.position - eye).normalize_or_zero()
                    * size.min(delta.length())
                    * 0.5;
                if side.length_squared() == 0. {
                    continue;
                }
                [
                    p.position - side,
                    p.position + side,
                    p.position + delta + side,
                    p.position + delta - side,
                ]
            } else {
                let (sin, cos) = p.roll.to_radians().sin_cos();
                let r = (right * cos + up * sin) * size;
                let u = (-right * sin + up * cos) * size;
                [
                    p.position - r - u,
                    p.position - r + u,
                    p.position + r + u,
                    p.position + r - u,
                ]
            };
            result.push(Quad {
                material: p.material.into(),
                positions,
                color,
                uv: if p.trail > 0. {
                    [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
                } else {
                    UV
                },
            });
        }
        for ring in &self.rings {
            let age = ((time - ring.born).max(0.) as f32 / ring.life).clamp(0., 1.);
            let radius = (ring.diameter + (1024. - ring.diameter) * age) * 0.5;
            let color = rgba(Vec3::new(1., 1., 225. / 255.), ring.alpha * (1. - age));
            let diameter = radius * 2.;
            let segments =
                (((diameter * 0.075 + 3.) * std::f32::consts::PI) as usize).clamp(6, 1024);
            let texture_length = (diameter * 0.01 * std::f32::consts::PI).max(0.5) / 8.;
            let point = |n: usize| {
                let angle = std::f32::consts::TAU * n as f32 / segments as f32;
                ring.center + Vec3::new(angle.cos(), -angle.sin(), 0.) * radius
            };
            let screen = |p: Vec3| {
                let delta = p - eye;
                let depth = delta.dot(direction).max(1.);
                glam::Vec2::new(delta.dot(right), delta.dot(up)) / depth
            };
            let edges: Vec<_> = (0..segments)
                .map(|i| {
                    let p = point(i);
                    let previous = point((i + segments - 1) % segments);
                    let tangent = screen(p) - screen(previous);
                    // Adjacent beam segments share their edge, as in the native triangle strip.
                    // DrawRing width is a half-width; unlike Tracer_Draw it is not halved.
                    let side = (up * tangent.x - right * tangent.y).normalize_or_zero() * 64.;
                    [p - side, p + side]
                })
                .collect();
            for i in 0..segments {
                let a = edges[i];
                let b = edges[(i + 1) % segments];
                let v0 = texture_length * i as f32 / segments as f32;
                let v1 = texture_length * (i + 1) as f32 / segments as f32;
                result.push(Quad {
                    material: "sprites/lgtning".into(),
                    positions: [a[0], a[1], b[1], b[0]],
                    color,
                    uv: [[1., v0], [0., v0], [0., v1], [1., v1]],
                });
            }
        }
        result
    }
}
fn rgba(color: Vec3, alpha: f32) -> [u8; 4] {
    [color.x, color.y, color.z, alpha].map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
}
pub fn force_direction(origin: Vec3, magnitude: f32, physics: &Physics) -> Vec3 {
    let magnitude = magnitude.max(0.);
    let mut direction = Vec3::ZERO;
    for axis in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
        if let Some(hit) = physics.projectile_ray(origin, origin + axis * magnitude, &[]) {
            let fraction = if magnitude > 0. {
                origin.distance(hit.position) / magnitude
            } else {
                1.
            };
            direction -= axis * (1. - fraction.clamp(0., 1.));
        }
    }
    if direction == Vec3::ZERO {
        Vec3::Z
    } else {
        direction.normalize_or_zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::World;
    fn event(id: u64, kind: EffectKind) -> Effect {
        Effect {
            id,
            kind,
            position: Vec3::ZERO,
            normal: Vec3::Z,
            at: 0.,
            radius: 16.,
            magnitude: 100.,
        }
    }
    #[test]
    fn grenade_has_distinct_long_smoke_short_fire_and_embers() {
        let physics = Physics::new(&World::default());
        let mut p = Particles::default();
        let event = event(1, EffectKind::GrenadeExplosion);
        p.sync(&[event], 0., &physics);
        assert_eq!(p.diagnostics.emitted[&Kind::Smoke], 44);
        assert_eq!(p.diagnostics.emitted[&Kind::Ember], 16);
        assert!((40..=48).contains(&p.diagnostics.emitted[&Kind::Fire]));
        assert!((16..=32).contains(&p.diagnostics.emitted[&Kind::Debris]));
        p.sync(&[event], 0.5, &physics);
        assert!(!p.diagnostics.live.contains_key(&Kind::Fire));
        assert!(p.diagnostics.live[&Kind::Smoke] >= 4);
        assert_eq!(p.diagnostics.live[&Kind::Ember], 16);
        p.sync(&[event], 3.01, &physics);
        assert!(p.particles.is_empty());
    }
    #[test]
    fn ball_expiry_has_two_timed_rings_and_sparks_outlive_the_flash() {
        let physics = Physics::new(&World::default());
        let mut p = Particles::default();
        let event = event(1, EffectKind::BallExplosion);
        p.sync(&[event], 0., &physics);
        assert_eq!(p.diagnostics.rings_emitted, 2);
        assert_eq!(p.diagnostics.live[&Kind::Glow], 2);
        assert!((96..=191).contains(&p.diagnostics.emitted[&Kind::Spark]));
        p.sync(&[event], 0.1, &physics);
        let quads = p.draw(Vec3::new(-1000., 0., 300.), Vec3::X, 0.1, &physics);
        let ring: Vec<_> = quads
            .iter()
            .filter(|q| q.material == "sprites/lgtning")
            .collect();
        assert_eq!(ring.len(), 191); // SDK DrawRing segment count at these two diameters.
        assert_eq!(ring[0].uv[0][1], 0.);
        assert_eq!(ring[0].uv[2][1], ring[1].uv[0][1]);
        assert_eq!(ring[0].positions[2], ring[1].positions[1]);
        assert_eq!(ring[0].positions[3], ring[1].positions[0]);
        assert!(ring[0].uv[2][1] < 0.02);
        let first = (ring[0].positions[0] + ring[0].positions[1]) * 0.5;
        assert!((first.x - 260.).abs() < 0.0001); // Source ring parameters are diameters.
        p.sync(&[event], 0.25, &physics);
        assert_eq!(p.diagnostics.rings_live, 1);
        assert!(!p.diagnostics.live.contains_key(&Kind::Glow));
        p.sync(&[event], 0.6, &physics);
        assert_eq!(p.diagnostics.rings_live, 0);
        assert!(p.diagnostics.live[&Kind::Spark] > 0);
        p.sync(&[event], 8.01, &physics);
        assert!(p.particles.is_empty());
    }
    #[test]
    fn repeated_paused_frames_do_not_reemit_or_advance_particles() {
        let physics = Physics::new(&World::default());
        let mut p = Particles::default();
        let event = event(5, EffectKind::GrenadeExplosion);
        p.sync(&[event], 0.15, &physics);
        let before = p.draw(Vec3::new(-400., 0., 0.), Vec3::X, 0.15, &physics);
        let emitted = p.diagnostics.emitted.clone();
        for _ in 0..30 {
            p.sync(&[event], 0.15, &physics);
        }
        assert_eq!(p.diagnostics.events, 1);
        assert_eq!(emitted, p.diagnostics.emitted);
        assert_eq!(
            before,
            p.draw(Vec3::new(-400., 0., 0.), Vec3::X, 0.15, &physics)
        );
        assert!(before
            .iter()
            .all(|q| q.positions.iter().all(|p| p.is_finite())));
    }
    #[test]
    fn delayed_presentation_matches_fixed_tick_catchup() {
        let physics = Physics::new(&World::default());
        let mut regular = Particles::default();
        let mut delayed = Particles::default();
        let event = event(2, EffectKind::BallExplosion);
        regular.sync(&[event], 0., &physics);
        for tick in 1..=20 {
            regular.sync(&[event], tick as f64 * 0.015, &physics);
        }
        delayed.sync(&[event], 0.3, &physics);
        assert_eq!(regular.particles.len(), delayed.particles.len());
        for (a, b) in regular.particles.iter().zip(&delayed.particles) {
            assert!(a.position.distance(b.position) < 0.0001);
            assert!(a.velocity.distance(b.velocity) < 0.0001);
        }
    }
    #[test]
    fn particle_budget_is_bounded_and_reset_clock_starts_new_map_state() {
        let physics = Physics::new(&World::default());
        let mut p = Particles::default();
        let effects: Vec<_> = (1..=50)
            .map(|id| event(id, EffectKind::BallExplosion))
            .collect();
        p.sync(&effects, 0., &physics);
        assert!(p.particles.len() <= MAX_PARTICLES);
        assert_eq!(p.rings.len(), 64);
        assert!(p.diagnostics.capacity_rejections > 0);
        p.sync(&[], 1., &physics);
        p.sync(&[], 0., &physics);
        assert!(p.particles.is_empty());
        assert_eq!(p.diagnostics.events, 0);
    }
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_explosion_materials_decode_with_correct_blend_modes() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        for name in MATERIALS {
            let sprite = crate::projectile_visuals::load_sprite(&vfs, name).unwrap();
            assert!(sprite.image.width > 0 && sprite.image.height > 0);
            assert_eq!(
                sprite.kind,
                if name.contains("noisesphere") || name.contains("fleck_cement") {
                    2
                } else {
                    3
                }
            );
        }
    }
}
