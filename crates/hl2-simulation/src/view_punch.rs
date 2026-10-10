//! CBasePlayer::ViewPunch and CGameMovement::DecayPunchAngle (SDK baseplayer_shared.cpp,
//! gamemovement.cpp): a damped torsional spring on the view's punch angle, in Source
//! degrees (pitch down positive, yaw left positive, roll).
use glam::Vec3;
use serde::Serialize;

/// PUNCH_DAMPING and PUNCH_SPRING_CONSTANT (gamemovement.cpp).
const DAMPING: f32 = 9.;
const SPRING: f32 = 65.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct ViewPunch {
    pub angle: Vec3,
    pub velocity: Vec3,
}
impl ViewPunch {
    /// ViewPunch(angleOffset): m_vecPunchAngleVel += angleOffset * 20.
    pub fn punch(&mut self, offset: Vec3) {
        self.velocity += offset * 20.;
    }
    /// DecayPunchAngle, once per player movement tick.
    pub fn decay(&mut self, dt: f32) {
        if self.angle.length_squared() > 0.001 || self.velocity.length_squared() > 0.001 {
            self.angle += self.velocity * dt;
            self.velocity *= (1. - DAMPING * dt).max(0.);
            self.velocity -= self.angle * (SPRING * dt).clamp(0., 2.);
            self.angle = Vec3::new(
                self.angle.x.clamp(-89., 89.),
                self.angle.y.clamp(-179., 179.),
                self.angle.z.clamp(-89., 89.),
            );
        } else {
            *self = Self::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn punch_kicks_then_springs_back_to_rest() {
        let mut punch = ViewPunch::default();
        punch.punch(Vec3::new(-2., 0., 0.));
        assert_eq!(punch.velocity, Vec3::new(-40., 0., 0.));
        punch.decay(0.015);
        assert!((punch.angle.x + 0.6).abs() < 1e-6);
        let mut peak = 0f32;
        for _ in 0..200 {
            punch.decay(0.015);
            peak = peak.min(punch.angle.x);
        }
        assert!(peak < -1. && peak > -3., "{peak}");
        assert_eq!(punch, ViewPunch::default());
    }
}
