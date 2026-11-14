//! env_starfield's client effect (SDK client/hl2/c_env_starfield.cpp `C_EnvStarfield`, drawn
//! as fx_sparks.cpp `CTrailParticles`). While the starfield is on, every client frame adds
//! `density` particles (the fraction carries to the next frame) on the line from the point
//! `cl_starfield_distance` (256) ahead of the view toward a point up to
//! `cl_starfield_diameter` (128) off the view axis around the eye, 10-30 % of the way along;
//! each flies the rest in 2 s. They draw as effects/spark_noz tracers of their velocity times
//! `length x (1 - life share)`, fading in over their first 0.3 s, then out with their life
//! (FADE_IN before FADE). The engine adds them per rendered frame, so the count depends on
//! the frame rate, as natively.
use crate::projectile_visuals::Quad;
use glam::Vec3;

const DISTANCE: f32 = 256.;
const DIAMETER: f32 = 128.;
const TRAVEL: f32 = 2.;

#[derive(Clone, Debug)]
struct Star {
    position: Vec3,
    velocity: Vec3,
    life: f32,
    width: f32,
    length: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Starfield {
    stars: Vec<Star>,
    carry: f32,
    seed: u64,
}

impl Starfield {
    fn random(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.random()
    }

    /// One frame: move the stars by `dt`, add this frame's while `density` is Some (on), and
    /// push their tracers. `view` is the eye and its forward, right and up axes.
    pub fn draw(&mut self, density: Option<f32>, view: [Vec3; 4], dt: f32, out: &mut Vec<Quad>) {
        let [eye, forward, right, up] = view;
        for star in &mut self.stars {
            star.position += star.velocity * dt;
            star.life += dt;
        }
        self.stars.retain(|s| s.life < TRAVEL);
        if let Some(density) = density.filter(|d| *d > 0.) {
            self.carry += density;
            let count = self.carry.floor();
            self.carry -= count;
            for _ in 0..count as usize {
                let start = eye + forward * DISTANCE;
                let end = eye
                    + right * self.range(-DIAMETER, DIAMETER)
                    + up * self.range(-DIAMETER, DIAMETER);
                let distance = (end - start).length();
                let direction = (end - start).normalize_or_zero();
                let position = start + direction * self.range(0.1, 0.3) * distance;
                let star = Star {
                    position,
                    velocity: direction * (distance / TRAVEL),
                    life: 0.,
                    width: self.range(1., 3.),
                    length: self.range(0.05, 0.4),
                };
                self.stars.push(star);
            }
        }
        for star in &self.stars {
            let share = 1. - star.life / TRAVEL;
            let ramp = if star.life <= 0.3 { star.life } else { share };
            let delta = star.velocity * (star.length * share).max(0.01);
            let width = delta.length().min(star.width);
            let side = delta.cross(star.position - eye).normalize_or_zero() * width * 0.5;
            if side == Vec3::ZERO {
                continue;
            }
            let (a, b) = (star.position, star.position + delta);
            let shade = (255. * ramp.clamp(0., 1.)) as u8;
            out.push(Quad {
                material: "effects/spark_noz".into(),
                positions: [a - side, b - side, b + side, a + side],
                color: [shade; 4],
                uv: [[0., 0.], [0., 1.], [1., 1.], [1., 0.]],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_spawn_by_density_fly_outward_and_die_after_two_seconds() {
        let mut field = Starfield::default();
        let view = [Vec3::ZERO, Vec3::X, Vec3::NEG_Y, Vec3::Z];
        let mut quads = Vec::new();
        // Density 0.5 a frame: one star every second frame.
        for _ in 0..4 {
            field.draw(Some(0.5), view, 0.01, &mut quads);
        }
        assert_eq!(field.stars.len(), 2);
        let star = &field.stars[0];
        assert!(
            star.position.x > 150. && star.position.x < 240.,
            "{}",
            star.position
        );
        assert!(star.velocity.x < 0., "toward the eye plane");
        // Off: no new stars; the old ones die after 2 s.
        field.draw(None, view, 2.5, &mut quads);
        assert!(field.stars.is_empty());
    }
}
