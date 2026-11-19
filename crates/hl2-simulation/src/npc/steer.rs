//! Steering a ground step around another NPC or the player that blocks it (Source SDK
//! `game/server/ai_navigator.cpp` OnObstructionPreSteer, `ai_planesolver.cpp`). The deflection
//! along the obstruction's surface comes first when it keeps close to the heading; then
//! directions either side of the heading, the side away from the obstruction first. The route
//! is unchanged: the next step heads for its waypoint again.
use super::{Actor, Goal};
use crate::npc_probe::{self, GroundMove, NpcCollisionWorld, Query};
use glam::{Quat, Vec3};

/// OnObstructionPreSteer takes the deflection only this close to the heading (cosine).
const DEFLECTION_MIN_DOT: f32 = 0.7;
/// Turns tried either side of the heading, degrees. The SDK's plane solver spreads five probes
/// over a wider arc with weighted suggestions; these are ours.
const TURNS: [f32; 3] = [30., 60., 90.];
/// A steered step is taken only if it covers this share of the wanted distance (ours).
const MIN_PROGRESS: f32 = 0.5;

/// One ground step of `amount` from `from` along `direction`; when an actor blocks it, the
/// first steered step that makes progress, with its direction.
pub(super) fn step<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    goal: &Goal,
    (direction, amount): (Vec3, f32),
    query: Query<'_>,
    start_validated: bool,
) -> (GroundMove, Vec3) {
    let from = goal.feet;
    let walk = |direction: Vec3| {
        let desired = from + direction * amount;
        if start_validated {
            npc_probe::ground_move_from_valid(world, from, desired, actor.ground, query)
        } else {
            npc_probe::ground_move(world, from, desired, actor.ground, query)
        }
    };
    let blocked = walk(direction);
    if blocked.completed || !blocked.by_actor {
        return (blocked, direction);
    }
    // A detour around the actor standing on the destination would send the mover back
    // into the same hull. Let its schedule handle the obstruction (e.g. an ActBusy exit).
    if query.transients.iter().any(|other| {
        let local = goal.request.target_feet - other.feet;
        other.entity == blocked.blocker
            && local.cmpge(other.hull.mins).all()
            && local.cmple(other.hull.maxs).all()
    }) {
        return (blocked, direction);
    }
    candidates(direction, blocked.normal)
        .map(|turned| (walk(turned), turned))
        .find(|(moved, _)| {
            let travel = (moved.end - from).truncate().length();
            moved.completed || travel >= amount * MIN_PROGRESS
        })
        .unwrap_or((blocked, direction))
}

/// The directions to try, in order: the deflection along the obstruction's surface when it
/// stays near the heading, then the turns, the side the obstruction's surface faces first
/// (head-on, the right: our tie-break, so two actors meeting head-on pass each other).
fn candidates(direction: Vec3, normal: Vec3) -> impl Iterator<Item = Vec3> {
    let flat = |v: Vec3| (v * Vec3::new(1., 1., 0.)).normalize_or_zero();
    let normal = flat(normal);
    // CalculateDeflection: the heading projected onto the obstruction's plane.
    let deflected = flat(normal.cross(direction.cross(normal)));
    let deflection = (deflected.dot(direction) > DEFLECTION_MIN_DOT).then_some(deflected);
    // Positive turns are to the left (counter-clockwise about +Z, Source's yaw).
    let left_is_free = direction.cross(normal).z < 0.;
    let first = if left_is_free { 1. } else { -1. };
    deflection
        .into_iter()
        .chain(TURNS.into_iter().flat_map(move |turn| {
            [first, -first].into_iter().map(move |side| {
                flat(Quat::from_rotation_z((turn * side).to_radians()) * direction)
            })
        }))
}

#[cfg(test)]
mod tests {
    use super::candidates;
    use glam::Vec3;

    #[test]
    fn a_slanted_obstruction_deflects_along_its_surface_first() {
        let heading = Vec3::X;
        // The blocker's surface faces back and to the left of the heading.
        let normal = Vec3::new(-0.5, 0.866, 0.);
        let first = candidates(heading, normal).next().unwrap();
        assert!(first.dot(heading) > 0.7 && first.y > 0., "{first}");
        assert!(first.dot(normal).abs() < 1e-4);
    }

    #[test]
    fn head_on_turns_right_first_then_alternates() {
        let order: Vec<_> = candidates(Vec3::X, -Vec3::X).collect();
        // No deflection head-on: the first candidate is the 30 degree turn to the right.
        assert!(order[0].y < 0. && (order[0].x - 30f32.to_radians().cos()).abs() < 1e-4);
        assert!(order[1].y > 0.);
        assert_eq!(order.len(), 6);
    }
}

#[cfg(test)]
mod passing {
    use super::super::tests::{key, locomotion, make_controller, pose, request, world};
    use super::super::{normal_human, ActorPose, GoalKey, GoalState};
    use crate::npc_probe::ActorHull;
    use crate::physics::Physics;
    use glam::Vec3;

    /// Two humans walk head-on along one line; each is the other's obstruction.
    fn meet(wall: bool) -> [GoalState; 2] {
        let walls = if wall {
            vec![super::super::tests::cube(
                Vec3::new(140., -200., 0.),
                Vec3::new(160., 200., 100.),
            )]
        } else {
            vec![]
        };
        let physics = Physics::new(&world(walls));
        let mut controller = make_controller(None);
        let human = normal_human(1., 18., 1.).unwrap();
        controller
            .register_actor(8, 1., human, locomotion())
            .unwrap();
        controller.set_view_offset(8, Vec3::Z * 70.).unwrap();
        let other = GoalKey { actor: 8, ..key(1) };
        let mut feet = [Vec3::ZERO, Vec3::X * 300.];
        let pose = |entity: usize, feet: Vec3| ActorPose {
            entity,
            feet,
            yaw_degrees: 0.,
            scripted_by: None,
        };
        controller.request(
            key(1),
            request(Vec3::X * 300.),
            pose(7, feet[0]),
            &physics,
            &[],
        );
        controller.request(other, request(Vec3::ZERO), pose(8, feet[1]), &physics, &[]);
        let mut states = [GoalState::Moving, GoalState::Moving];
        for _ in 0..200 {
            let poses = [pose(7, feet[0]), pose(8, feet[1])];
            let hulls = [7, 8].map(|entity| ActorHull {
                entity: Some(entity),
                feet: feet[entity - 7],
                hull: human.hull,
            });
            for update in controller.tick(&physics, &poses, &hulls, 0.05, false) {
                let index = update.key.actor - 7;
                feet[index] = update.feet;
                states[index] = update.state.clone();
            }
        }
        states
    }

    #[test]
    fn two_humans_meeting_head_on_pass_each_other_and_arrive() {
        let states = meet(false);
        assert!(states.iter().all(GoalState::is_arrived), "{states:?}");
    }

    #[test]
    fn occupied_destination_reports_contact_then_allows_arrival_when_clear() {
        let physics = Physics::new(&world(vec![]));
        let mut controller = make_controller(None);
        let target = Vec3::X * 50.;
        let blocker = ActorHull {
            entity: Some(8),
            feet: target,
            hull: normal_human(1., 18., 1.).unwrap().hull,
        };
        controller.request(key(1), request(target), pose(Vec3::ZERO), &physics, &[]);
        let updates = controller.tick(&physics, &[pose(Vec3::ZERO)], &[blocker], 0.5, false);
        let contact = updates.last().unwrap();
        assert!(matches!(contact.state, GoalState::Blocked(_)));
        let mut feet = contact.feet;
        for _ in 0..40 {
            for update in controller.tick(&physics, &[pose(feet)], &[], 0.1, false) {
                feet = update.feet;
            }
        }
        assert!(controller.status(key(1)).unwrap().is_arrived());
    }

    #[test]
    fn a_wall_is_not_steered_around() {
        let states = meet(true);
        assert!(!states.iter().any(GoalState::is_arrived), "{states:?}");
    }
}
