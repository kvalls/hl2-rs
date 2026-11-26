//! The plane solver's obstacle arcs (Source SDK `game/server/ai_planesolver.cpp`:
//! GenerateObstacleNpcs, GenerateCircleObstacleSuggestions, AdjustRegulationWeight,
//! GenerateSuggestionFromTrace, CalculateRegulationWeight and CalcYawsFromOffset).

use super::plane::{Arc, Kind};
use glam::Vec2;

/// Another actor as a circle on the ground plane.
pub(super) struct Circle {
    pub center: Vec2,
    pub radius: f32,
    /// An NPC's current velocity; None for the player.
    pub velocity: Option<Vec2>,
}

/// The mover: where it stands, its own radius, the wanted heading (unit), its velocity and
/// the probe distance.
pub(super) struct Mover {
    pub at: Vec2,
    pub radius: f32,
    pub heading: Vec2,
    pub velocity: Vec2,
    pub probe: f32,
}

/// Degrees of the planar direction `v` (UTIL_VecToYaw).
pub(super) fn yaw(v: Vec2) -> f32 {
    v.y.atan2(v.x).to_degrees()
}

/// The unit vector of `yaw` degrees.
pub(super) fn heading(yaw: f32) -> Vec2 {
    let radians = yaw.to_radians();
    Vec2::new(radians.cos(), radians.sin())
}

/// The arc to keep away from around `circle`: its tangent cone, widened when very near, or
/// 210 degrees when already inside it; None when out of reach or behind the mover. The weight
/// grows as the gap closes.
pub(super) fn circle_arc(mover: &Mover, circle: &Circle) -> Option<Arc> {
    let offset = circle.center - mover.at;
    let distance_sq = offset.length_squared();
    let radius = circle.radius + mover.radius;
    if distance_sq <= 0.001 || distance_sq >= (radius + mover.probe).powi(2) {
        return None;
    }
    let distance = distance_sq.sqrt();
    let toward = offset / distance;
    let outside = distance_sq > radius * radius;
    if outside && toward.dot(mover.heading) < 0. {
        return None;
    }
    let (span, mut weight) = if outside {
        let span = (radius / distance).asin().to_degrees() * 2.;
        let weight = 1. - (distance - radius) / mover.probe;
        // The last quarter of the approach widens the cone by up to half again.
        let widen = ((weight - 0.75) / 0.25).max(0.) * 0.5;
        (span * (1. + widen), weight)
    } else {
        (210., 1.)
    };
    if let Some(velocity) = circle.velocity {
        weight = same_way(weight, mover.velocity, velocity);
    }
    Some(Arc::new(Kind::Avoid, weight, yaw(toward), span))
}

/// An NPC walking the mover's way at 90% of its speed or more is less of an obstacle, and
/// none at all when nearly parallel (AdjustRegulationWeight).
fn same_way(weight: f32, mine: Vec2, theirs: Vec2) -> f32 {
    let their_speed = theirs.length();
    if their_speed <= 0. || their_speed < mine.length() * 0.9 {
        return weight;
    }
    let dot = mine.normalize_or_zero().dot(theirs / their_speed);
    match dot {
        d if d > 0.86 => 0.,
        d if d > 0.7 => weight.powi(3),
        d if d > 0. => weight * weight,
        _ => weight,
    }
}

/// A probe toward `center` (degrees) that stopped `obstructed` units short of `probe`: three
/// avoidance arcs over `span`, the side away from the hit surface lighter (`favor_left`:
/// Some(true) when the left is the way around). Nothing when less than a tenth is blocked.
pub(super) fn trace_arcs(
    center: f32,
    span: f32,
    (probe, obstructed): (f32, f32),
    favor_left: Option<bool>,
) -> Vec<Arc> {
    let blocked = (obstructed / probe).clamp(0., 1.);
    let weight = match blocked {
        b if b > 0.9 => 1.,
        b if b < 0.1 => 0.,
        b => ((b - 0.1) / 0.8).powi(2),
    };
    if weight < 0.001 {
        return Vec::new();
    }
    let span = if blocked < 0.5 {
        span * blocked * 2.
    } else {
        span
    };
    let third = span / 3.;
    let lighter = weight * blocked;
    let right = if favor_left == Some(false) {
        lighter
    } else {
        weight
    };
    let left = if favor_left == Some(true) {
        lighter
    } else {
        weight
    };
    vec![
        Arc::new(Kind::Avoid, weight, center, third),
        Arc::new(Kind::Avoid, right, center - third, third),
        Arc::new(Kind::Avoid, left, center + third, third),
    ]
}

/// Probe `offset` (0 ahead, positive left) of a scan around `heading`: the yaw it covers (its
/// arc's centre) and the yaw it traces, the arc's edge nearer the heading.
pub(super) fn probe_yaws(heading: f32, span: f32, offset: i32) -> (f32, f32) {
    let center = heading + offset as f32 * span;
    let test = center - offset.signum() as f32 * span * 0.5;
    (center, test)
}

#[cfg(test)]
mod tests {
    use super::{circle_arc, probe_yaws, trace_arcs, Circle, Mover};
    use glam::Vec2;

    fn mover() -> Mover {
        Mover {
            at: Vec2::ZERO,
            radius: 18.4,
            heading: Vec2::X,
            velocity: Vec2::X * 70.,
            probe: 70.,
        }
    }

    #[test]
    fn a_standing_npc_ahead_is_a_tangent_arc_heavier_when_near() {
        let far = Circle {
            center: Vec2::new(80., 0.),
            radius: 18.4,
            velocity: Some(Vec2::ZERO),
        };
        let near = Circle {
            center: Vec2::new(45., 0.),
            ..far
        };
        let (far, near) = (
            circle_arc(&mover(), &far).unwrap(),
            circle_arc(&mover(), &near).unwrap(),
        );
        assert!(far.center.abs() < 1e-3);
        assert!((far.span - 2. * (36.8f32 / 80.).asin().to_degrees()).abs() < 1e-3);
        assert!(near.weight > far.weight && near.span > far.span);
        let behind = Circle {
            center: Vec2::new(-60., 0.),
            radius: 18.4,
            velocity: None,
        };
        assert!(circle_arc(&mover(), &behind).is_none());
    }

    #[test]
    fn an_npc_walking_the_same_way_is_ignored() {
        let ahead = Circle {
            center: Vec2::new(60., 0.),
            radius: 18.4,
            velocity: Some(Vec2::X * 65.),
        };
        assert_eq!(circle_arc(&mover(), &ahead).unwrap().weight, 0.);
    }

    #[test]
    fn a_probe_blocked_near_its_end_adds_nothing_and_a_short_one_three_arcs() {
        assert!(trace_arcs(0., 54., (100., 5.), None).is_empty());
        let arcs = trace_arcs(10., 54., (100., 95.), Some(true));
        assert_eq!(arcs.len(), 3);
        assert_eq!(arcs[0].weight, 1.);
        assert!(arcs[2].weight < arcs[1].weight && arcs[2].center > arcs[1].center);
        assert_eq!(probe_yaws(0., 54., 2), (108., 81.));
        assert_eq!(probe_yaws(0., 54., -1), (-54., -27.));
    }
}
