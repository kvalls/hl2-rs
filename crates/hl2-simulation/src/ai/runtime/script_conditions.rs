//! Player proximity, view and LOS to a named actor (SDK ai_scriptconditions.cpp).
use super::{eye_z, AiRuntime, NpcState, PhysicsSight, PlayerState};
use crate::entities::{number, Scene};
use crate::physics::Physics;
use glam::Vec3;
use modkit_core::{Entity, World};

const INTERVAL: f64 = 0.25;

pub(super) struct ActorCondition {
    owner: usize,
    actor: usize,
    range: f32,
    fov: f32,
    cone_dot: f32,
    true_cone: bool,
    los: i32,
    min_state: i32,
    max_state: i32,
    allow_script: bool,
    hold: f64,
    next: Option<f64>,
    since: Option<f64>,
}

pub(super) fn for_world(world: &World) -> Vec<ActorCondition> {
    world
        .entities
        .iter()
        .enumerate()
        .filter_map(|(id, e)| {
            if e.class() != "ai_script_conditions" || !actor_only(e) {
                return None;
            }
            let name = e.get("Actor")?;
            let mut matches = world.entities.iter().enumerate().filter(|(_, actor)| {
                actor.class().starts_with("npc_")
                    && actor
                        .get("targetname")
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
            });
            let actor = matches.next()?.0;
            if matches.next().is_some() {
                return None;
            }
            let fov = number(e, "PlayerActorFOV", -1.);
            Some(ActorCondition {
                owner: id,
                actor,
                range: number(e, "PlayerActorProximity", 0.),
                fov,
                cone_dot: (fov.abs() * 0.5).to_radians().cos(),
                true_cone: number(e, "PlayerActorFOVTrueCone", 0.) != 0.,
                los: number(e, "PlayerActorLOS", 2.) as i32,
                min_state: number(e, "MinimumState", 1.) as i32,
                max_state: number(e, "MaximumState", 1.) as i32,
                allow_script: number(e, "ScriptStatus", 2.) >= 1.,
                hold: f64::from(number(e, "RequiredTime", 0.)),
                next: None,
                since: None,
            })
        })
        .collect()
}

/// Leave unsupported compound conditions dormant instead of firing a partial match.
fn actor_only(e: &Entity) -> bool {
    e.get("target").is_none_or(str::is_empty)
        && number(e, "MinTimeout", 0.) == 0.
        && number(e, "spawnflags", 0.) == 0.
        && [
            "ActorSeePlayer",
            "PlayerBlockingActor",
            "ActorInPVS",
            "PlayerInVehicle",
            "ActorInVehicle",
        ]
        .iter()
        .all(|key| number(e, key, 2.) == 2.)
}

impl ActorCondition {
    fn in_view(&self, player: PlayerState, actor_feet: Vec3, eye: Vec3) -> bool {
        let inside = actor_feet.distance_squared(player.feet) < self.range * self.range;
        if self.range != 0. && inside != (self.range > 0.) {
            return false;
        }
        if self.fov >= 360. {
            return true;
        }
        let dir = eye - player.eye();
        let dot = if self.true_cone {
            dir.normalize_or_zero()
                .dot(player.facing.normalize_or_zero())
        } else {
            dir.truncate()
                .normalize_or_zero()
                .dot(player.facing.truncate().normalize_or_zero())
        };
        (dot >= self.cone_dot) == (self.fov >= 0.)
    }

    fn state_matches(&self, state: NpcState) -> bool {
        let value = match state {
            NpcState::Idle => 1,
            NpcState::Alert => 2,
            NpcState::Combat => 3,
            NpcState::Script => return self.allow_script,
            _ => return false,
        };
        (self.min_state..=self.max_state).contains(&value)
    }
}

impl AiRuntime {
    pub(super) fn evaluate_script_conditions(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        player: PlayerState,
    ) {
        let now = scene.time;
        for gate in &mut self.script_conditions {
            if !scene.states[gate.owner].enabled || scene.states[gate.owner].killed {
                gate.next = None;
                gate.since = None;
                continue;
            }
            if now < *gate.next.get_or_insert(now + INTERVAL) {
                continue;
            }
            gate.next = Some(now + INTERVAL);
            let Some(actor) = self.actors.get(&gate.actor) else {
                continue;
            };
            let state = &scene.states[gate.actor];
            if state.killed {
                scene.fire(gate.owner, "NoValidActors", gate.owner);
                scene.states[gate.owner].enabled = false;
                continue;
            }
            let feet = state.origin;
            let eye = feet + Vec3::Z * eye_z(world, gate.actor);
            let state = if scene.ai_scripted(gate.actor) {
                NpcState::Script
            } else {
                actor.npc.state
            };
            if gate.state_matches(state)
                && gate.in_view(player, feet, eye)
                && (gate.los == 2
                    || ((!self.ignore_player
                        && PhysicsSight { world, physics }
                            .first_solid(eye, player.eye(), &[gate.actor])
                            .is_none())
                        == (gate.los == 1)))
            {
                if now - *gate.since.get_or_insert(now) >= gate.hold {
                    scene.fire(gate.owner, "OnConditionsSatisfied", gate.owner);
                    scene.states[gate.owner].enabled = false;
                    gate.since = None;
                    gate.next = None;
                }
            } else {
                gate.since = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{for_world, AiRuntime, NpcState, Physics, PlayerState, Scene};
    use glam::Vec3;
    use modkit_core::{Entity, World};

    fn entity(properties: &[(&str, &str)]) -> Entity {
        Entity {
            properties: properties
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn world() -> World {
        World {
            entities: vec![
                entity(&[
                    ("classname", "npc_citizen"),
                    ("targetname", "witness"),
                    ("origin", "100 0 0"),
                ]),
                entity(&[
                    ("classname", "ai_script_conditions"),
                    ("Actor", "witness"),
                    ("PlayerActorFOV", "50"),
                    ("PlayerActorFOVTrueCone", "1"),
                    ("PlayerActorLOS", "1"),
                    ("PlayerActorProximity", "500"),
                    ("RequiredTime", "0.4"),
                ]),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn actor_condition_requires_continuous_attention_and_rearms() {
        let world = world();
        let mut scene = Scene::new(&world);
        let mut ai = AiRuntime::new(&world, None);
        ai.actors.get_mut(&0).unwrap().npc.state = NpcState::Idle;
        let physics = Physics::new(&world);
        for (now, facing, enabled) in [
            (0., Vec3::X, true),
            (0.25, Vec3::X, true),
            (0.5, -Vec3::X, true),
            (0.75, Vec3::X, true),
            (1., Vec3::X, true),
            (1.25, Vec3::X, false),
        ] {
            scene.time = now;
            ai.evaluate_script_conditions(
                &world,
                &mut scene,
                &physics,
                PlayerState {
                    facing,
                    ..Default::default()
                },
            );
            assert_eq!(scene.states[1].enabled, enabled, "at {now}");
        }
        scene.states[1].enabled = true;
        ai.evaluate_script_conditions(&world, &mut scene, &physics, PlayerState::default());
        assert!(scene.states[1].enabled);
    }

    #[test]
    fn out_of_range_or_notarget_player_does_not_satisfy_condition() {
        let world = world();
        let mut scene = Scene::new(&world);
        let mut ai = AiRuntime::new(&world, None);
        ai.actors.get_mut(&0).unwrap().npc.state = NpcState::Idle;
        let physics = Physics::new(&world);
        for (x, notarget) in [(-550., false), (0., true)] {
            ai.ignore_player = notarget;
            for _ in 0..8 {
                scene.time += 0.25;
                ai.evaluate_script_conditions(
                    &world,
                    &mut scene,
                    &physics,
                    PlayerState {
                        feet: Vec3::X * x,
                        facing: Vec3::X,
                        ..Default::default()
                    },
                );
            }
            assert!(scene.states[1].enabled);
        }
    }

    #[test]
    fn unsupported_compound_condition_stays_dormant() {
        let mut world = world();
        world.entities[1]
            .properties
            .push(("Target".into(), "another".into()));
        assert!(for_world(&world).is_empty());
    }
}
