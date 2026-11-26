//! The plane solver's vote (Source SDK `game/server/ai_movesolver.cpp`: CAI_MoveSolver::Solve
//! and the suggestion weights of NormalizeSuggestions). Arcs of yaw add their weight to
//! 3-degree bins; the bin with the largest positive sum is the direction to walk.

/// What an arc says about the directions it covers, and how its weight scales.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    /// The wanted direction (AIMST_MOVE): 0.2 at weight 0 up to 1.0.
    Move,
    /// An obstruction (AIMST_AVOID_WORLD, _OBJECT, _NPC): down to -0.25.
    Avoid,
    /// Directions no probe looked at (AIMST_NO_KNOWLEDGE): -1.
    Unknown,
    /// The reverse of the previous solution (AIMST_OSCILLATION_DETERRANCE): -0.6.
    Deterrent,
}

/// The side of a positive arc that keeps more of its weight away from the centre
/// (AIMS_FAVOR_LEFT and AIMS_FAVOR_RIGHT); left is the larger yaw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Favor {
    Neither,
    Left,
}

/// One weighted arc of yaw, in degrees.
#[derive(Clone, Copy, Debug)]
pub(super) struct Arc {
    pub kind: Kind,
    /// 0 to 1, before the kind's scale.
    pub weight: f32,
    pub center: f32,
    pub span: f32,
    pub favor: Favor,
}

impl Arc {
    pub fn new(kind: Kind, weight: f32, center: f32, span: f32) -> Self {
        Self {
            kind,
            weight,
            center,
            span,
            favor: Favor::Neither,
        }
    }

    /// The weight on the kind's scale; anything under 0.01 counts as nothing.
    fn scaled(&self) -> f32 {
        if self.weight < 0.01 {
            return 0.;
        }
        let (at_zero, at_one) = match self.kind {
            Kind::Move => (0.2, 1.),
            Kind::Avoid => (0., -0.25),
            Kind::Unknown => (-1., -1.),
            Kind::Deterrent => (-0.6, -0.6),
        };
        at_zero + (at_one - at_zero) * self.weight
    }
}

/// Degrees per bin (NUM_SOLUTIONS 120).
const BIN: f32 = 3.;
const BINS: i32 = 120;
/// A positive arc keeps 95% of its weight 180 degrees from its centre.
const LOSS_AT_HALF_TURN: f32 = 0.05;
/// The favoured side loses this share of the usual fall-off.
const FAVORED_LOSS: f32 = 0.9;

/// The yaw (0 to 360) with the largest positive vote, None when no bin is positive. A bin
/// that contains the centre of its strongest positive arc answers with that exact centre.
pub(super) fn solve(arcs: &[Arc]) -> Option<f32> {
    let mut sums = [0f32; BINS as usize];
    let mut strongest = [(0f32, None::<usize>); BINS as usize];
    for (index, arc) in arcs.iter().enumerate() {
        let weight = arc.scaled();
        let half = arc.span * 0.5;
        let middle = (half / BIN).round() as i32;
        let first = ((arc.center.rem_euclid(360.) - half).rem_euclid(360.) / BIN) as i32;
        for offset in 0..=(arc.span / BIN) as i32 {
            let vote = if weight > 0. {
                let mut loss =
                    (middle - offset).abs() as f32 * LOSS_AT_HALF_TURN / (BINS / 2) as f32;
                if arc.favor == Favor::Left && offset > middle {
                    loss *= FAVORED_LOSS;
                }
                weight * (1. - loss)
            } else {
                weight
            };
            let bin = (first + offset).rem_euclid(BINS) as usize;
            sums[bin] += vote;
            if vote > strongest[bin].0 {
                strongest[bin] = (vote, Some(index));
            }
        }
    }
    let mut best = None;
    let mut best_sum = 0.;
    for (bin, &sum) in sums.iter().enumerate() {
        if sum > best_sum {
            (best, best_sum) = (Some(bin), sum);
        }
    }
    let best = best?;
    let yaw = best as f32 * BIN;
    let center = strongest[best]
        .1
        .map(|index| arcs[index].center.rem_euclid(360.));
    Some(match center {
        Some(center) if center > yaw && center <= yaw + BIN => center,
        _ => yaw,
    })
}

#[cfg(test)]
mod tests {
    use super::{solve, Arc, Favor, Kind};

    fn mov(center: f32, span: f32) -> Arc {
        Arc::new(Kind::Move, 1., center, span)
    }

    #[test]
    fn an_unopposed_move_keeps_its_exact_direction() {
        assert_eq!(solve(&[mov(271.5, 270.)]), Some(271.5));
        assert_eq!(solve(&[mov(-90., 270.)]), Some(270.));
    }

    #[test]
    fn an_obstacle_ahead_pushes_the_answer_past_its_arc() {
        // Heading 0, an NPC arc of 60 degrees straight ahead and "unknown" behind.
        let arcs = [
            mov(0., 270.),
            Arc::new(Kind::Avoid, 1., 0., 60.),
            Arc::new(Kind::Unknown, 1., 180., 90.),
        ];
        let yaw = solve(&arcs).unwrap();
        let off = yaw.min(360. - yaw);
        assert!((30. ..=36.).contains(&off), "{yaw}");
    }

    #[test]
    fn the_favoured_side_wins_a_symmetric_choice() {
        let mut ahead = mov(0., 270.);
        ahead.favor = Favor::Left;
        let arcs = [ahead, Arc::new(Kind::Avoid, 1., 0., 60.)];
        let yaw = solve(&arcs).unwrap();
        assert!(
            yaw > 0. && yaw < 90.,
            "left (positive yaw) expected, got {yaw}"
        );
    }

    #[test]
    fn nothing_positive_has_no_answer() {
        assert_eq!(solve(&[Arc::new(Kind::Unknown, 1., 0., 360.)]), None);
        assert_eq!(solve(&[Arc::new(Kind::Move, 0.005, 0., 90.)]), None);
    }
}
