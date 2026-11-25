//! Blend between a cached silhouette and the LOD blob as screen-area ranks change.
use super::{Caster, Look};
use hl2_simulation::shadows::receivers::Decal;

#[derive(Default)]
pub(super) struct Transition {
    /// Silhouette contribution. The blob receives the complement during a handoff.
    pub weight: f32,
    /// The unclipped blob projection while the main decal holds the clipped silhouette.
    pub blob: Decal,
}

impl Transition {
    pub fn blending(&self) -> bool {
        self.weight > 0. && self.weight < 1.
    }

    fn advance(&mut self, detailed: bool, step: f32) {
        self.weight = if detailed {
            (self.weight + step).min(1.)
        } else {
            (self.weight - step).max(0.)
        };
    }
}

pub(super) fn update(caster: &mut Caster, step: f32, texture_mode: bool) -> bool {
    let before = caster.transition.weight;
    let detailed = matches!(caster.wanted, Look::Cell(..));
    if !texture_mode || !caster.shown || caster.area.is_none() {
        caster.transition.weight = f32::from(detailed);
    } else if detailed || caster.retained {
        caster.transition.advance(detailed, step);
        if caster.retained && caster.transition.weight > 0. {
            caster.wanted = caster.look.expect("retained silhouette");
        }
    } else {
        caster.transition.weight = 0.;
    }
    if detailed && caster.transition.weight == 0. {
        caster.wanted = Look::Lod;
    }
    before != caster.transition.weight
}

#[cfg(test)]
mod tests {
    use super::Transition;

    #[test]
    fn a_reversed_handoff_continues_from_the_current_weight() {
        let mut fade = Transition::default();
        fade.advance(true, 1.);
        fade.advance(false, 0.25);
        assert!(fade.blending());
        assert_eq!(fade.weight, 0.75);
        fade.advance(true, 0.125);
        assert_eq!(fade.weight, 0.875);
        fade.advance(true, 1.);
        assert!(!fade.blending());
        fade.advance(false, 2.);
        assert_eq!(fade.weight, 0.);
    }
}
