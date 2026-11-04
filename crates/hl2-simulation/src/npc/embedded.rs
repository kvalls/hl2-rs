//! CAI_MoveProbe::GroundMoveLimit's bTryNavIgnore: an actor a mover starts embedded in (two fast
//! zombies landing together, two spawned on one spot) does not block its moves, or neither could
//! ever walk out of the other.
use super::{Actor, GoalBlockReason, GoalKey, AI_SCENE};
use crate::npc_probe::{self, Hull, NpcCollisionWorld, Query};
use glam::Vec3;
use std::borrow::Cow;

/// GroundMoveLimit's retries of one step.
const MAX_IGNORED: usize = 16;

/// What a move from `feet` leaves out of its probes: the query's exclusions and every NPC the
/// hull starts inside, by its box or by its wider model collider (`is_npc` asks the controller:
/// `query.transients` need not list it). Starting inside anything else (the world, a prop, the
/// player) fails. This is CanFitAtPosition's 0.01 upward check, so a clear start costs one trace.
pub(super) fn ignored<'a, W: NpcCollisionWorld>(
    world: &W,
    feet: Vec3,
    hull: Hull,
    query: Query<'a>,
    is_npc: impl Fn(usize) -> bool,
) -> Result<Cow<'a, [usize]>, GoalBlockReason> {
    let mut excluded = Cow::Borrowed(query.excluded_entities);
    let mut inside = None;
    for _ in 0..MAX_IGNORED {
        let probe = Query {
            excluded_entities: &excluded,
            ..query
        };
        let start = world.npc_trace_hull(feet, feet + Vec3::Z * 0.01, hull, probe);
        if !start.trace.start_solid {
            return Ok(excluded);
        }
        inside = start.entity;
        match start.entity.filter(|id| is_npc(*id)) {
            Some(npc) => excluded.to_mut().push(npc),
            None => break,
        }
    }
    Err(GoalBlockReason::Collision {
        blocker: None,
        reason: format!("actor starts inside {inside:?}"),
    })
}

/// CheckStandPosition at the start of a move: Ok(true) when the start is shaky (hanging over an
/// edge, with a floor in reach), which the move tolerates. An AI mover with nothing under it
/// falls, as any MOVETYPE_STEP entity off the ground does (created in the air, or what held it up
/// walked away); a scene's mover stays where its script put it.
pub(super) fn stand<W: NpcCollisionWorld>(
    world: &W,
    key: GoalKey,
    feet: Vec3,
    actor: &Actor,
    query: Query<'_>,
    drops: &mut Vec<usize>,
) -> Result<bool, GoalBlockReason> {
    let ground = actor.ground;
    if npc_probe::stand(world, feet, ground, query) {
        return Ok(false);
    }
    let below = feet - Vec3::Z * ground.step_height * ground.step_down_multiplier;
    if world
        .npc_trace_hull(feet, below, ground.hull, query)
        .trace
        .fraction
        < 1.
    {
        return Ok(true);
    }
    if key.scene == AI_SCENE {
        drops.push(key.actor);
    }
    Err(cannot_stand())
}

/// The mover's start is no place to stand.
pub(super) fn cannot_stand() -> GoalBlockReason {
    GoalBlockReason::Collision {
        blocker: None,
        reason: "actor cannot stand".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{cube, key, locomotion, make_controller, pose, request, world};
    use super::super::{normal_human, GoalState, AI_SCENE};
    use super::ignored;
    use crate::npc_probe::{ground_move, ActorHull, Query};
    use crate::physics::Physics;
    use glam::Vec3;

    fn neighbour(entity: Option<usize>, x: f32) -> [ActorHull; 1] {
        [ActorHull {
            entity,
            feet: Vec3::X * x,
            hull: normal_human(1., 18., 1.).unwrap().hull,
        }]
    }

    #[test]
    fn an_actor_embedded_in_another_walks_out_of_it() {
        let physics = Physics::new(&world(vec![]));
        let mut controller = make_controller(None);
        let ground = normal_human(1., 18., 1.).unwrap();
        controller
            .register_actor(8, 1., ground, locomotion())
            .unwrap();
        let blocker = neighbour(Some(8), 10.);
        let status = controller.request(
            key(1),
            request(Vec3::X * 200.),
            pose(Vec3::ZERO),
            &physics,
            &blocker,
        );
        assert!(!matches!(status, GoalState::Blocked(_)), "{status:?}");
        let updates = controller.tick(&physics, &[pose(Vec3::ZERO)], &blocker, 0.5, false);
        let last = updates.last().unwrap();
        assert!(!matches!(last.state, GoalState::Blocked(_)), "{last:?}");
        assert!(last.feet.x > 20., "it passed the actor it started in");
    }

    #[test]
    fn only_npcs_are_walked_out_of_even_when_the_transients_do_not_list_them() {
        let physics = Physics::new(&world(vec![]));
        let hull = normal_human(1., 18., 1.).unwrap().hull;
        let excluded = [1];
        let query = Query {
            excluded_entities: &excluded,
            transients: &neighbour(Some(8), 10.),
            ..Default::default()
        };
        let free = ignored(&physics, Vec3::ZERO, hull, query, |id| id == 8).unwrap();
        assert_eq!(&*free, &[1, 8]);
        assert!(ignored(&physics, Vec3::ZERO, hull, query, |_| false).is_err());
    }

    #[test]
    fn an_ai_mover_with_nothing_under_it_falls_and_a_scene_mover_stays() {
        let physics = Physics::new(&world(vec![]));
        let drops = |scene: usize| {
            let mut controller = make_controller(None);
            let air = Vec3::Z * 200.;
            controller.request(
                key(scene),
                request(Vec3::X * 200.),
                pose(air),
                &physics,
                &[],
            );
            controller.take_drops()
        };
        assert_eq!(drops(AI_SCENE), [7]);
        assert!(drops(1).is_empty());
    }

    /// A floor ending at x = 0 above a pit.
    fn ledge() -> Physics {
        Physics::new(&modkit_core::World {
            brushes: vec![
                cube(Vec3::new(-1000., -1000., -50.), Vec3::new(0., 1000., 0.)),
                cube(
                    Vec3::new(-1000., -1000., -900.),
                    Vec3::new(1000., 1000., -800.),
                ),
            ],
            ..Default::default()
        })
    }

    #[test]
    fn a_start_hanging_over_an_edge_walks_back_and_valid_ground_stays_strict() {
        let physics = ledge();
        let ground = normal_human(1., 18., 1.).unwrap();
        let edge = Vec3::X * 8.;
        assert!(!crate::npc_probe::stand(
            &physics,
            edge,
            ground,
            Query::default()
        ));
        let back = ground_move(&physics, edge, Vec3::X * -40., ground, Query::default());
        assert!(back.completed, "{back:?}");
        let out = ground_move(&physics, Vec3::X * -40., edge, ground, Query::default());
        assert!(
            !out.completed,
            "valid ground may not walk onto the edge: {out:?}"
        );

        let mut controller = make_controller(None);
        let mut feet = edge;
        let status =
            controller.request(key(1), request(Vec3::X * -200.), pose(feet), &physics, &[]);
        assert!(!matches!(status, GoalState::Blocked(_)), "{status:?}");
        for _ in 0..10 {
            if let Some(last) = controller
                .tick(&physics, &[pose(feet)], &[], 0.1, false)
                .last()
            {
                feet = last.feet;
            }
        }
        assert!(feet.x < -20., "it walked off the edge: {feet:?}");
    }

    #[test]
    fn a_start_inside_the_player_stays_blocked() {
        let physics = Physics::new(&world(vec![]));
        let mut controller = make_controller(None);
        let status = controller.request(
            key(1),
            request(Vec3::X * 200.),
            pose(Vec3::ZERO),
            &physics,
            &neighbour(None, 10.),
        );
        assert!(matches!(status, GoalState::Blocked(_)), "{status:?}");
    }
}
