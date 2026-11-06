//! What damage shows on a scanner (DESIGN 14o session 48): the smoke it trails once hurt, and the
//! sparks of its end.
use crate::projectiles::{Effect, EffectKind};
use glam::Vec3;

/// How many sparks cover its end, and how far from its middle along each axis.
pub const GIB_SPARKS: usize = 4;
pub const GIB_SPARK_REACH: f32 = 12.;

/// Whether a hit starts the smoke: the health it had before the hit is under three quarters, so
/// the first hit on a healthy scanner never does.
pub fn starts_smoking(before: f32, max_health: f32) -> bool {
    before < max_health - (max_health / 4.).floor()
}

/// An effect of a scanner's at `at`, with `normal` carrying what the kind reads there.
pub fn effect(kind: EffectKind, at: Vec3, normal: Vec3, time: f64) -> Effect {
    Effect {
        id: 0,
        magnitude: 1.,
        kind,
        position: at,
        normal,
        at: time,
        radius: 0.,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_hit_on_a_healthy_scanner_starts_no_smoke() {
        assert!(!starts_smoking(30., 30.));
    }

    #[test]
    fn smoke_starts_once_it_was_already_under_three_quarters() {
        assert!(!starts_smoking(23., 30.));
        assert!(starts_smoking(22., 30.));
    }
}
