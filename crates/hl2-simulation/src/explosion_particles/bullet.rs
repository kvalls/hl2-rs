//! PerformCustomEffects' bullet impacts: FX_DebrisFlecks, FX_DustImpact and FX_MetalSpark at
//! iScale 1. The effect's `normal` is the surface normal, or the shot's reflection for a spark.
use super::{Kind, Motion, Particle, Particles, Random};
use crate::projectiles::Effect;
use glam::Vec3;

const DUST: &str = "particle/particle_smokegrenade";
const NOISE: &str = "particle/particle_noisesphere";
/// StartSmokeTrail's spawn radius, and C_SmokeTrail's opacity.
const TRAIL_RADIUS: f32 = 10.;
const TRAIL_OPACITY: f32 = 0.5;
const CHUNKS: &str = "effects/blood";
const FLECK_ANGULAR_SPRAY: f32 = 0.6;
const FLECK_MIN_SPEED: f32 = 64.;
const FLECK_MAX_SPEED: f32 = 128.;
const METAL_SPARK_SPREAD: f32 = 0.5;

/// GetColorForSurface: the surface's color (here one per material, not its texture's
/// reflectivity) in the light at the hit, which the effect's `magnitude` carries.
fn surface_color(e: &Effect, wood: bool) -> Vec3 {
    let base = if wood {
        Vec3::new(0.7, 0.52, 0.34)
    } else {
        Vec3::splat(0.7)
    };
    base * e.magnitude
}

/// The surface color with SDK colorRamp's random brightness.
fn ramp(color: Vec3, rng: &mut Random, low: f32, high: f32) -> Vec3 {
    (color * rng.float(low, high)).min(Vec3::ONE)
}

/// The parts of a SimpleParticle that differ between the puffs below.
struct Puff {
    material: &'static str,
    life: f32,
    sizes: [f32; 2],
    alpha: f32,
    velocity: Vec3,
    roll_rate: f32,
}

impl Particles {
    fn puff(&mut self, e: &Effect, at: Vec3, puff: Puff, color: Vec3, rng: &mut Random) {
        let mut p = Particle::new(Kind::Smoke, puff.material, e, puff.life);
        p.position = at;
        p.velocity = puff.velocity;
        p.sizes = puff.sizes;
        p.alpha = [puff.alpha, 0.];
        p.color = color;
        p.roll = rng.float(0., 360.);
        p.roll_rate = puff.roll_rate.to_degrees();
        p.motion = Motion::Free;
        self.add(p);
    }

    /// FX_DebrisFlecks: flecks off concrete, tile or wood and the dust around the hole.
    pub(super) fn debris_flecks(&mut self, e: &Effect, wood: bool, rng: &mut Random) {
        let normal = e.normal.normalize_or_zero();
        let color = surface_color(e, wood);
        let materials = if wood {
            ["effects/fleck_wood1", "effects/fleck_wood2"]
        } else {
            ["effects/fleck_cement1", "effects/fleck_cement2"]
        };
        let spray = (FLECK_ANGULAR_SPRAY - 0.2).max(0.2);
        for _ in 0..rng.int(4, 16) {
            let material = materials[rng.int(0, 1) as usize];
            let mut p = Particle::new(Kind::Debris, material, e, 3.);
            p.position += normal;
            let size = rng.int(1, 2) as f32;
            let direction = normal + rng.vector(-spray, spray);
            p.velocity = direction * rng.float(FLECK_MIN_SPEED, FLECK_MAX_SPEED) * (3. - size);
            p.sizes = [size; 2];
            p.color = ramp(color, rng, 0.75, 1.25);
            p.roll = rng.float(0., 360.);
            p.roll_rate = rng.float(0., 360.);
            p.motion = Motion::Trail {
                gravity: 800.,
                drag: 0.,
                bounce: 0.3,
                collide: true,
            };
            self.add(p);
        }
        self.flecks_dust(e, normal, color, rng);
    }

    /// FX_DebrisFlecks' dust trail: two puffs, four chunks and the bullet hole capper.
    fn flecks_dust(&mut self, e: &Effect, normal: Vec3, color: Vec3, rng: &mut Random) {
        let at = e.position + normal * 2.;
        for i in 0..2 {
            let scale = (i + 1) as f32;
            let start = rng.int(2, 4) as f32;
            let mut velocity = (normal + rng.vector(-0.8, 0.8)) * rng.float(2., 24.) * scale;
            velocity.z -= rng.float(8., 32.) * scale;
            let puff = Puff {
                material: DUST,
                life: 1.,
                sizes: [start, start * 8.],
                alpha: rng.int(100, 200) as f32 / 255.,
                velocity,
                roll_rate: rng.float(-1., 1.),
            };
            let tint = ramp(color, rng, 0.5, 1.25);
            self.puff(e, at, puff, tint, rng);
        }
        for _ in 0..4 {
            let start = rng.int(1, 4) as f32;
            let mut velocity = (normal + rng.vector(-0.8, 0.8)) * rng.float(8., 32.);
            velocity.z -= rng.float(8., 64.);
            let puff = Puff {
                material: CHUNKS,
                life: rng.float(0.25, 0.5),
                sizes: [start, start * 4.],
                alpha: 1.,
                velocity,
                roll_rate: rng.float(-2., 2.),
            };
            let tint = ramp(color, rng, 0.5, 1.25);
            self.puff(e, at, puff, tint, rng);
        }
        let start = rng.int(4, 8) as f32;
        let mut velocity = (normal + rng.vector(-0.8, 0.8)) * rng.float(2., 24.);
        velocity.z = rng.float(-2., 2.);
        let puff = Puff {
            material: DUST,
            life: rng.float(1., 1.5),
            sizes: [start, start * 4.],
            alpha: rng.int(100, 200) as f32 / 255.,
            velocity,
            roll_rate: rng.float(-2., 2.),
        };
        let tint = ramp(color, rng, 0.5, 1.25);
        self.puff(e, at, puff, tint, rng);
    }

    /// FX_DustImpact on dirt and sand: three kinds of puff, four of each.
    pub(super) fn dust_impact(&mut self, e: &Effect, rng: &mut Random) {
        let normal = e.normal.normalize_or_zero();
        let color = surface_color(e, false);
        for id in [3., 1., 2., 0.] {
            let direction = rng.vector(-0.2, 0.2) + normal * rng.float(1., 6.);
            let start = rng.int(3, 4) as f32 * (id + 1.);
            let puff = Puff {
                material: DUST,
                life: rng.float(0.5, 1.),
                sizes: [start, start * 4.],
                alpha: rng.int(32, 255) as f32 / 255.,
                velocity: direction.normalize_or_zero() * rng.float(250., 500.) * id,
                roll_rate: rng.float(-8., 8.),
            };
            self.dust_puff(e, e.position, puff, ramp(color, rng, 0.75, 1.25), rng);
            let direction = rng.vector(-0.2, 0.2) + normal * rng.float(1., 6.);
            let start = rng.int(2, 4) as f32 * (id + 1.);
            let puff = Puff {
                material: CHUNKS,
                life: rng.float(0.25, 0.75),
                sizes: [start, start * 2.],
                alpha: 1.,
                velocity: direction.normalize_or_zero() * rng.float(250., 500.) * id,
                roll_rate: rng.float(-2., 2.),
            };
            self.dust_puff(e, e.position, puff, ramp(color, rng, 0.75, 1.25), rng);
            let direction = rng.vector(-1., 1.) + normal;
            let start = rng.int(1, 4) as f32;
            let puff = Puff {
                material: DUST,
                life: rng.float(0.5, 1.),
                sizes: [start, start * 4.],
                alpha: rng.int(32, 64) as f32 / 255.,
                velocity: direction.normalize_or_zero() * rng.float(0., 50.),
                roll_rate: rng.float(-16., 16.),
            };
            let at = e.position + rng.vector(-8., 8.) * Vec3::new(1., 1., 0.);
            self.dust_puff(e, at, puff, ramp(color, rng, 0.75, 1.25), rng);
        }
    }

    /// A CDustParticle: `puff` with its decaying velocity.
    fn dust_puff(&mut self, e: &Effect, at: Vec3, puff: Puff, color: Vec3, rng: &mut Random) {
        self.puff(e, at, puff, color, rng);
        if let Some(p) = self.particles.last_mut() {
            p.motion = Motion::Dust;
        }
    }

    /// FX_MetalSpark: short sparks along the reflected shot and a flash at the hit. Source draws
    /// the flash as a quad on the surface; here it is a sprite.
    pub(super) fn metal_spark(&mut self, e: &Effect, rng: &mut Random) {
        let reflect = e.normal + rng.vector(-0.2, 0.2);
        for _ in 0..rng.int(4, 8) * 2 {
            let mut p = Particle::new(Kind::Spark, "effects/spark", e, rng.float(0.05, 0.1));
            let spread = rng.float(0., 2.);
            let reach = METAL_SPARK_SPREAD * spread;
            let direction = (reflect + rng.vector(-reach, reach)).normalize_or_zero();
            let speed = rng.float(128. * (2. - spread), 512. * (2. - spread));
            p.velocity = direction * speed;
            p.sizes = [rng.float(1., 4.); 2];
            p.alpha = [1.; 2];
            p.trail = rng.float(0.025, 0.1);
            p.motion = Motion::Trail {
                gravity: 400.,
                drag: 8.,
                bounce: 0.25,
                collide: true,
            };
            self.add(p);
        }
        let mut flash = Particle::new(Kind::Glow, "effects/yellowflare_noz", e, 0.1);
        flash.sizes = [rng.int(24, 28) as f32, 0.];
        flash.roll = rng.float(0., 360.);
        self.add(flash);
    }
    /// One puff of the trail StartSmokeTrail sets up: a second's life, growing from 8 to 50 and
    /// fading out, with the body's speed and a little of its own. C_SmokeTrail spawns ten a second,
    /// one per think here.
    pub(super) fn trail_smoke(&mut self, e: &Effect, rng: &mut Random) {
        let material = if rng.unit() < 0.5 { DUST } else { NOISE };
        let mut puff = Particle::new(Kind::Smoke, material, e, 1.);
        puff.position += rng.vector(-TRAIL_RADIUS, TRAIL_RADIUS);
        puff.velocity = e.normal + rng.vector(-1., 1.) * rng.float(15., 25.);
        puff.sizes = [8., 50.];
        puff.alpha = [TRAIL_OPACITY * rng.float(0.75, 1.25), 0.];
        puff.color = Vec3::splat(0.5 + rng.float(-0.2, 0.2));
        puff.roll = rng.float(0., 360.);
        puff.roll_rate = rng.float(-1., 1.);
        puff.near_clip = true;
        self.add(puff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projectiles::EffectKind;

    fn effect(kind: EffectKind, normal: Vec3) -> Effect {
        Effect {
            id: 1,
            magnitude: 0.,
            kind,
            position: Vec3::ZERO,
            normal,
            at: 0.,
            radius: 0.,
        }
    }

    #[test]
    fn a_trail_puff_carries_the_bodys_speed_and_fades_as_it_grows() {
        let mut particles = Particles::default();
        particles.trail_smoke(
            &effect(EffectKind::TrailSmoke, Vec3::X * 200.),
            &mut Random(7),
        );
        let puff = &particles.particles[0];
        assert_eq!(puff.kind, Kind::Smoke);
        assert_eq!(puff.sizes, [8., 50.]);
        assert_eq!(puff.alpha[1], 0.);
        assert!(
            (0.375..=0.625).contains(&puff.alpha[0]),
            "{}",
            puff.alpha[0]
        );
        assert!(puff.velocity.x > 170., "{}", puff.velocity.x);
    }

    #[test]
    fn flecks_and_dust_leave_the_surface_and_wood_uses_its_own_flecks() {
        let mut particles = Particles::default();
        let e = effect(EffectKind::WoodFlecks, Vec3::Z);
        particles.debris_flecks(&e, true, &mut Random(7));
        let flecks: Vec<_> = particles
            .particles
            .iter()
            .filter(|p| p.kind == Kind::Debris)
            .collect();
        assert!((4..=16).contains(&flecks.len()), "{}", flecks.len());
        assert!(flecks.iter().all(|p| p.material.contains("fleck_wood")));
        assert!(flecks.iter().all(|p| p.velocity.z > 0.));
        let puffs = particles.particles.len() - flecks.len();
        assert_eq!(puffs, 7, "two puffs, four chunks and the capper");
    }

    #[test]
    fn dust_makes_twelve_puffs_that_slow_down_and_sparks_follow_the_reflection() {
        let mut particles = Particles::default();
        particles.dust_impact(&effect(EffectKind::Dust, Vec3::Z), &mut Random(7));
        assert_eq!(particles.particles.len(), 12);
        assert!(particles
            .particles
            .iter()
            .all(|p| matches!(p.motion, Motion::Dust)));

        let mut particles = Particles::default();
        particles.metal_spark(&effect(EffectKind::MetalSpark, Vec3::X), &mut Random(7));
        let sparks: Vec<_> = particles
            .particles
            .iter()
            .filter(|p| p.kind == Kind::Spark)
            .collect();
        assert!((8..=16).contains(&sparks.len()), "{}", sparks.len());
        let forward = sparks.iter().filter(|p| p.velocity.x > 0.).count();
        assert!(forward * 2 > sparks.len(), "most fly along the reflection");
        assert_eq!(particles.particles.len(), sparks.len() + 1, "and one flash");
    }
}
