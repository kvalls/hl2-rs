//! ai_script_conditions (SDK game/server/ai_scriptconditions.cpp; retail server.dll also keeps
//! the episodic keyfields ActorInPVS, PlayerInVehicle and ActorInVehicle). While enabled the
//! entity thinks every 0.25 s and tests one element per named actor (or one without an actor):
//! every condition must hold for RequiredTime, then OnConditionsSatisfied fires; past a
//! timeout (MinTimeout > 0) OnConditionsTimeout fires instead; a vanished actor drops its
//! element (NoValidActors when it was the last). When every element is done the entity
//! disables itself. Three-state keys: 0 false, 1 true, 2 (TRS_NONE) not tested.

use super::{AiRuntime, PlayerState};
use crate::ai::conditions::COND_SEE_PLAYER;
use crate::ai::host::PhysicsSight;
use crate::ai::senses::SightWorld;
use crate::ai::state::NpcState;
use crate::entities::Scene;
use crate::physics::Physics;
use glam::Vec3;
use modkit_core::{Entity, World};
use std::collections::BTreeMap;

/// SetThinkTime: every 0.25 s while enabled.
const THINK: f64 = 0.25;
const TRS_TRUE: i32 = 1;
const TRS_NONE: i32 = 2;
/// CSimTimer::Expired's ST_EPS (SDK game/shared/simtimer.h).
const EPS: f64 = 0.001;
/// SF_ACTOR_AS_ACTIVATOR (episodic): the actor, not the entity, activates the outputs.
const ACTOR_AS_ACTIVATOR: u32 = 1;

/// The keyvalues, read once at Enable (the SDK's constructor defaults where absent).
#[derive(Clone, Debug)]
struct Settings {
    actor: String,
    states: (i32, i32),
    script_status: i32,
    actor_see_player: i32,
    proximity: [f32; 3],
    actor_fov: (f32, bool),
    target_fov: (f32, bool),
    actor_los: i32,
    target_los: i32,
    actor_see_target: i32,
    blocking: i32,
    actor_in_pvs: i32,
    player_in_vehicle: i32,
    actor_in_vehicle: i32,
    required: f64,
    timeouts: (f64, f64),
    actor_as_activator: bool,
}
impl Settings {
    fn read(e: &Entity) -> Self {
        let f = |k: &str, d: f32| e.get(k).and_then(|v| v.trim().parse().ok()).unwrap_or(d);
        let i = |k: &str| f(k, TRS_NONE as f32) as i32;
        let cone = |k: &str| f(k, 0.) != 0.;
        Self {
            actor: e.get("Actor").unwrap_or("").trim().to_owned(),
            states: (f("MinimumState", 1.) as i32, f("MaximumState", 1.) as i32),
            script_status: i("ScriptStatus"),
            actor_see_player: i("ActorSeePlayer"),
            proximity: [
                f("PlayerActorProximity", 0.),
                f("PlayerTargetProximity", 0.),
                f("ActorTargetProximity", 0.),
            ],
            actor_fov: (f("PlayerActorFOV", -1.), cone("PlayerActorFOVTrueCone")),
            target_fov: (f("PlayerTargetFOV", 0.), cone("PlayerTargetFOVTrueCone")),
            actor_los: i("PlayerActorLOS"),
            target_los: i("PlayerTargetLOS"),
            actor_see_target: i("ActorSeeTarget"),
            blocking: i("PlayerBlockingActor"),
            actor_in_pvs: i("ActorInPVS"),
            player_in_vehicle: i("PlayerInVehicle"),
            actor_in_vehicle: i("ActorInVehicle"),
            required: f("RequiredTime", 0.) as f64,
            timeouts: (f("MinTimeout", 0.) as f64, f("MaxTimeout", 0.) as f64),
            actor_as_activator: f("spawnflags", 0.) as u32 & ACTOR_AS_ACTIVATOR != 0,
        }
    }
}

/// One actor's element: when its required time has run (a CSimTimer that starts expired)
/// and when it times out.
#[derive(Clone, Debug)]
struct Element {
    actor: Option<usize>,
    ready_at: f64,
    timeout_at: f64,
}

/// One enabled ai_script_conditions entity.
#[derive(Clone, Debug)]
struct Conditions {
    settings: Settings,
    target: Option<usize>,
    elements: Vec<Element>,
    next_think: f64,
}

/// Every ai_script_conditions entity that is enabled, and whether the map's Activate ran.
#[derive(Clone, Debug, Default)]
pub struct ScriptConditions {
    activated: bool,
    enabled: BTreeMap<usize, Conditions>,
}

/// What one evaluation reads.
struct Eval<'a> {
    world: &'a World,
    scene: &'a Scene,
    sight: PhysicsSight<'a>,
    player: PlayerState,
}

impl AiRuntime {
    /// Enable and Disable (Scene::ai_inputs).
    pub(super) fn script_conditions_input(
        &mut self,
        world: &World,
        scene: &mut Scene,
        id: usize,
        input: &str,
    ) {
        if input == "enable" {
            self.enable_conditions(world, scene, id);
        } else {
            self.script_conditions.enabled.remove(&id);
        }
    }

    /// CAI_ScriptConditions::Enable: an element for every NPC named `Actor` not in the list
    /// (one without an actor when the name is empty); a named actor that does not exist
    /// fires NoValidActors and leaves the entity disabled.
    fn enable_conditions(&mut self, world: &World, scene: &mut Scene, id: usize) {
        let settings = Settings::read(&world.entities[id]);
        let actors: Vec<usize> = if settings.actor.is_empty() {
            Vec::new()
        } else {
            living_named(world, scene, &settings.actor)
        };
        let mut conditions = self
            .script_conditions
            .enabled
            .remove(&id)
            .unwrap_or(Conditions {
                settings: settings.clone(),
                target: world.entities[id]
                    .get("target")
                    .and_then(|t| crate::entities::named(world, t.trim())),
                elements: Vec::new(),
                next_think: 0.,
            });
        if conditions.elements.is_empty() && !settings.actor.is_empty() {
            let npc = actors
                .iter()
                .all(|&a| world.entities[a].class().starts_with("npc_"));
            if actors.is_empty() {
                scene.fire(id, "NoValidActors", id);
            }
            if actors.is_empty() || !npc {
                return;
            }
        }
        let now = scene.time;
        let wanted: Vec<Option<usize>> = if settings.actor.is_empty() {
            vec![None]
        } else {
            actors.into_iter().map(Some).collect()
        };
        for actor in wanted {
            if conditions.elements.iter().all(|e| e.actor != actor) {
                let (min, max) = settings.timeouts;
                let timeout = if max > 0. {
                    min + self.uniform() as f64 * (max - min)
                } else {
                    min
                };
                let ready_at = if settings.required > 0. {
                    now + settings.required
                } else {
                    -1.
                };
                let timeout_at = now + timeout;
                conditions.elements.push(Element {
                    actor,
                    ready_at,
                    timeout_at,
                });
            }
        }
        conditions.next_think = now + THINK;
        self.script_conditions.enabled.insert(id, conditions);
    }

    /// Activate (map start) enables those not StartDisabled; then each due entity thinks.
    pub(super) fn think_script_conditions(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        player: PlayerState,
    ) {
        if !std::mem::replace(&mut self.script_conditions.activated, true) {
            for (id, e) in world.entities.iter().enumerate() {
                if e.class() == "ai_script_conditions"
                    && e.get("StartDisabled").map(str::trim) != Some("1")
                {
                    self.enable_conditions(world, scene, id);
                }
            }
        }
        let due: Vec<usize> = (self.script_conditions.enabled.iter())
            .filter(|(_, c)| c.next_think <= scene.time + 1e-9)
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            let Some(mut conditions) = self.script_conditions.enabled.remove(&id) else {
                continue;
            };
            let sight = PhysicsSight { physics, world };
            let fired = {
                let eval = Eval {
                    world,
                    scene: &*scene,
                    sight,
                    player,
                };
                self.evaluate(&eval, id, &mut conditions)
            };
            for (output, activator) in &fired.outputs {
                scene.fire(id, output, *activator);
            }
            if !fired.disable {
                conditions.next_think = scene.time + THINK;
                self.script_conditions.enabled.insert(id, conditions);
            }
        }
    }

    /// EvaluationThink over the elements; the outputs to fire and whether all are done.
    fn evaluate(&self, eval: &Eval, id: usize, c: &mut Conditions) -> Fired {
        let now = eval.scene.time;
        let s = c.settings.clone();
        let mut fired = Fired::default();
        let mut done = 0;
        let mut i = 0;
        while i < c.elements.len() {
            let element = c.elements[i].clone();
            let activator = element.actor.filter(|_| s.actor_as_activator).unwrap_or(id);
            let gone = element.actor.is_none_or(|a| eval.scene.states[a].killed);
            if !s.actor.is_empty() && gone {
                if c.elements.len() == 1 {
                    fired.outputs.push(("NoValidActors", id));
                }
                done += 1;
                c.elements.remove(i);
                continue;
            }
            i += 1;
            if s.timeouts.0 > 0. && now - element.timeout_at > -EPS {
                done += 1;
                fired.outputs.push(("OnConditionsTimeout", activator));
                continue;
            }
            if !self.all_hold(eval, &s, element.actor, c.target) {
                c.elements[i - 1].ready_at = now + s.required;
                continue;
            }
            if now - element.ready_at > -EPS {
                done += 1;
                fired.outputs.push(("OnConditionsSatisfied", activator));
            }
        }
        fired.disable = done == c.elements.len() || s.states.0 > s.states.1;
        fired
    }

    /// The evaluators in the retail order; the first that fails resets the element's timer.
    fn all_hold(
        &self,
        eval: &Eval,
        s: &Settings,
        actor: Option<usize>,
        target: Option<usize>,
    ) -> bool {
        let npc = actor.and_then(|a| self.actors.get(&a));
        let feet = |e: usize| eval.scene.states[e].origin;
        let eye = |e: usize| {
            feet(e) + Vec3::Z * crate::ai_hull::eye_height(eval.world.entities[e].class())
        };
        let player_eye = eval.player.eye();
        let near = |dist: f32, a: Vec3, b: Vec3| {
            dist == 0. || (a.distance_squared(b) < dist * dist) == (dist > 0.)
        };
        let see_player = || npc.is_some_and(|n| n.npc.conditions.has(COND_SEE_PLAYER));
        let state = || match npc.map_or(NpcState::Idle, |n| n.npc.state) {
            NpcState::Idle => Some(0),
            NpcState::Alert => Some(1),
            NpcState::Combat => Some(2),
            _ => None,
        };
        let state_ok = || match state() {
            Some(v) => (s.states.0 - 1..=s.states.1 - 1).contains(&v),
            None => {
                npc.is_some_and(|n| n.npc.state == NpcState::Script) && s.script_status >= TRS_TRUE
            }
        };
        let look = |viewed: usize, (fov, cone): (f32, bool)| {
            in_player_fov(&eval.player, eye(viewed), fov, cone)
        };
        // PlayerHasLineOfSight asks the viewed entity's FVisible(player), false for a player
        // with FL_NOTARGET (SDK baseentity.cpp): `notarget` is the runtime's ignore_player.
        let seen = |viewed: usize| {
            !self.ignore_player && eval.sight.visible(eye(viewed), player_eye, &[viewed])
        };
        let los = |viewed: usize, want: i32| want == TRS_NONE || seen(viewed) == (want == TRS_TRUE);
        let target_seen = |a: usize, t: usize| {
            let fov = npc.map_or(0., |n| n.class.field_of_view());
            let forward = eval.scene.states[a].rotation * Vec3::X;
            let to = (feet(t) - feet(a)).truncate().normalize_or_zero();
            to.dot(forward.truncate().normalize_or_zero()) > fov
                && eval.sight.visible(eye(a), eye(t), &[a, t])
        };
        let tri = |want: i32, value: bool| want == TRS_NONE || value == (want == TRS_TRUE);
        let player_feet = eval.player.feet;
        (s.actor_see_player == TRS_NONE || actor.is_none() || tri(s.actor_see_player, see_player()))
            && (actor.is_none() || state_ok())
            && actor.is_none_or(|a| near(s.proximity[0], player_feet, feet(a)))
            && target.is_none_or(|t| near(s.proximity[1], player_feet, feet(t)))
            && match (actor, target) {
                (Some(a), Some(t)) => near(s.proximity[2], feet(a), feet(t)),
                _ => true,
            }
            && s.blocking != TRS_TRUE
            && actor.is_none_or(|a| look(a, s.actor_fov))
            && target.is_none_or(|t| look(t, s.target_fov))
            && (s.actor_see_target == TRS_NONE
                || target.is_none_or(|t| {
                    actor.is_none_or(|a| tri(s.actor_see_target, target_seen(a, t)))
                }))
            && actor.is_none_or(|a| los(a, s.actor_los))
            && target.is_none_or(|t| los(t, s.target_los))
            && actor.is_none_or(|a| tri(s.actor_in_pvs, eval.scene.look_targets.in_pvs(a)))
            && tri(s.player_in_vehicle, false)
            && (actor.is_none() || tri(s.actor_in_vehicle, false))
    }
}

/// What an evaluation fires, and whether the entity disables itself.
#[derive(Default)]
struct Fired {
    outputs: Vec<(&'static str, usize)>,
    disable: bool,
}

/// IsInFOV with the player as the viewer: within half of `fov` degrees of the view (3D for a
/// true cone, else flat); a negative `fov` inverts the test, 360 or more always passes.
fn in_player_fov(player: &PlayerState, viewed_eye: Vec3, fov: f32, true_cone: bool) -> bool {
    if fov < 360. {
        let limit = (fov / 2.).to_radians().cos();
        let to = viewed_eye - player.eye();
        let dot = if true_cone {
            player
                .facing
                .normalize_or_zero()
                .dot(to.normalize_or_zero())
        } else {
            let look = player.facing.truncate().normalize_or_zero();
            look.dot(to.truncate().normalize_or_zero())
        };
        if dot < limit {
            return fov < 0.;
        }
    }
    fov >= 0.
}

/// The living entities a name finds (FindEntityByName: case-insensitive, a trailing `*`).
fn living_named(world: &World, scene: &Scene, name: &str) -> Vec<usize> {
    let lower = name.to_ascii_lowercase();
    let matches = |t: &str| match lower.strip_suffix('*') {
        Some(prefix) => t.to_ascii_lowercase().starts_with(prefix),
        None => t.eq_ignore_ascii_case(&lower),
    };
    (world.entities.iter().enumerate())
        .filter(|(i, e)| !scene.states[*i].killed && e.get("targetname").is_some_and(matches))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(facing: Vec3) -> PlayerState {
        PlayerState {
            facing,
            ..Default::default()
        }
    }

    #[test]
    fn player_fov_tests_the_view_cone_and_a_negative_fov_inverts_it() {
        let ahead = Vec3::new(100., 0., 64.);
        let behind = Vec3::new(-100., 0., 64.);
        let p = player(Vec3::X);
        assert!(in_player_fov(&p, ahead, 50., true));
        assert!(!in_player_fov(&p, behind, 50., true));
        // The SDK default (-1): true unless the viewed eye is dead ahead.
        assert!(in_player_fov(&p, behind, -1., false));
        assert!(!in_player_fov(&p, ahead, -1., false));
        assert!(in_player_fov(&p, behind, 360., true));
    }
}
