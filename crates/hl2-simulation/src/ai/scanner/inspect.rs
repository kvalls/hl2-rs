//! What a scanner goes looking for with nothing else to do (DESIGN 14o session 47): the SDK's
//! BestInspectTarget and the part of GatherConditions that calls it. `runtime::scanners` lists who
//! there is and traces the lines of sight.

use glam::Vec3;

/// SCANNER_CIT_INSPECT_DELAY: how long after spawning or an inspection it looks again.
pub const SEARCH_DELAY: f64 = 10.;
/// SCANNER_CIT_INSPECT_GROUND_DIST, from where its spotlight lands, and
/// SCANNER_CIT_INSPECT_FLY_DIST, from its body when it has no spotlight.
const GROUND_REACH: f32 = 500.;
const FLY_REACH: f32 = 1500.;
/// The share of inspections made with the spotlight; the others are photographs.
pub const SPOT_SHARE: f32 = 0.25;

/// Someone it could inspect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub subject: usize,
    pub feet: Vec3,
    pub eyes: Vec3,
}

/// Where a search starts from and how far it reaches.
pub fn search_from(body: Vec3, spotlight_end: Option<Vec3>) -> (Vec3, f32) {
    match spotlight_end {
        Some(end) => (end, GROUND_REACH),
        None => (body, FLY_REACH),
    }
}

/// The nearest of `candidates` to `body` that is within reach of `from` and that it can see.
/// Reach counts to the feet and nearness to the eyes, as the SDK's do.
pub fn nearest(
    candidates: &[Candidate],
    body: Vec3,
    (from, reach): (Vec3, f32),
    mut sees: impl FnMut(&Candidate) -> bool,
) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for candidate in candidates.iter().filter(|c| from.distance(c.feet) < reach) {
        let distance = body.distance(candidate.eyes);
        if best.is_none_or(|(nearest, _)| distance < nearest) && sees(candidate) {
            best = Some((distance, candidate.subject));
        }
    }
    best.map(|(_, subject)| subject)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(subject: usize, x: f32) -> Candidate {
        let feet = Vec3::X * x;
        Candidate {
            subject,
            feet,
            eyes: feet + Vec3::Z * 64.,
        }
    }

    #[test]
    fn with_a_spotlight_it_searches_a_short_way_from_where_it_lands() {
        let end = Vec3::X * 900.;
        assert_eq!(search_from(Vec3::ZERO, Some(end)), (end, 500.));
        assert_eq!(search_from(Vec3::Z, None), (Vec3::Z, 1500.));
    }

    #[test]
    fn the_nearest_one_it_can_see_is_chosen() {
        let list = [at(1, 800.), at(2, 300.), at(3, 500.)];
        let reach = (Vec3::ZERO, 1500.);
        assert_eq!(nearest(&list, Vec3::ZERO, reach, |_| true), Some(2));
        let blocked = |c: &Candidate| c.subject != 2;
        assert_eq!(nearest(&list, Vec3::ZERO, reach, blocked), Some(3));
    }

    #[test]
    fn past_its_reach_or_out_of_sight_there_is_nobody() {
        let list = [at(1, 1600.)];
        assert_eq!(
            nearest(&list, Vec3::ZERO, (Vec3::ZERO, 1500.), |_| true),
            None
        );
        let near = [at(1, 100.)];
        assert_eq!(
            nearest(&near, Vec3::ZERO, (Vec3::ZERO, 1500.), |_| false),
            None
        );
    }

    #[test]
    fn reach_is_measured_from_the_search_and_nearness_from_the_body() {
        // Both are in reach of where the beam lands; the one nearer the body wins.
        let list = [at(1, 950.), at(2, 700.)];
        let reach = (Vec3::X * 900., 500.);
        assert_eq!(nearest(&list, Vec3::ZERO, reach, |_| true), Some(2));
    }

    #[test]
    fn a_target_further_than_the_best_is_not_even_traced() {
        let list = [at(1, 100.), at(2, 200.)];
        let mut traces = 0;
        nearest(&list, Vec3::ZERO, (Vec3::ZERO, 1500.), |_| {
            traces += 1;
            true
        });
        assert_eq!(traces, 1);
    }
}
