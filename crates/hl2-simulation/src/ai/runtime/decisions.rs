//! Decision cadence for quiet actors. Movement and animation events have their own clocks.
use super::{AiActor, AiClass, AiRuntime, NpcState, PlayerState, THINK_INTERVAL};
use crate::ai::{efficiency::efficiency, senses::sound};
use crate::entities::Scene;
use modkit_core::{Entity, World};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct DecisionClock {
    pub count: u64,
    pub interval: f64,
    #[serde(skip)]
    eligible: bool,
}

impl DecisionClock {
    pub(super) fn new(entity: &Entity, spawnflags: u32) -> Self {
        // Townsfolk and story allies service civilian interactions on their usual clock.
        let civilian = entity.class() == "npc_citizen"
            && crate::citizens::citizen_type(entity.get("citizentype"), "")
                != crate::citizens::CT_REBEL;
        Self {
            count: 0,
            interval: THINK_INTERVAL,
            eligible: !civilian && spawnflags & (1 << 10) == 0, // SF_NPC_ALWAYSTHINK
        }
    }
}

impl AiRuntime {
    pub(super) fn take_damage_reports(
        &mut self,
        world: &World,
        scene: &mut Scene,
        player: PlayerState,
    ) {
        for report in std::mem::take(&mut scene.npc_damage) {
            if let Some(actor) = self.actors.get_mut(&report.victim) {
                actor.damage_taken = actor.damage_taken.max(report.damage);
                actor.damaged_by_player |= report.by_player;
                actor.class.hurt(&report, scene);
            }
            self.barnacle_hurt(report.victim);
            self.pain_or_death_sound(report.victim, world, scene, player);
        }
    }

    pub(super) fn decision_due(&self, id: usize, scene: &Scene, player: PlayerState) -> bool {
        let actor = &self.actors[&id];
        let now = scene.time + 1e-9;
        now >= actor.next_think
            || (actor.decisions.interval > THINK_INTERVAL
                && (actor.damage_taken > 0.
                    || actor.squad_new_enemy
                    || (now >= actor.last_think + THINK_INTERVAL
                        && now >= actor.last_think + self.decision_interval(id, scene, player))))
    }

    pub(super) fn record_decision(&mut self, id: usize, scene: &Scene, player: PlayerState) {
        let interval = self.decision_interval(id, scene, player);
        let actor = self
            .actors
            .get_mut(&id)
            .expect("actor completed a decision");
        actor.decisions.count += 1;
        actor.decisions.interval = interval;
        actor.next_think = scene.time + interval;
    }

    fn decision_interval(&self, id: usize, scene: &Scene, player: PlayerState) -> f64 {
        let actor = &self.actors[&id];
        if !actor.can_defer_decision() {
            return THINK_INTERVAL;
        }
        let Some(in_pvs) = scene.look_targets.actor_pvs(id) else {
            return THINK_INTERVAL;
        };
        let origin = scene.states[id].origin;
        let to_actor = origin - player.eye();
        let distance_squared = to_actor.length_squared();
        // Proximity also promotes an actor during the interval between host PVS samples.
        if distance_squared < 300. * 300. {
            return THINK_INTERVAL;
        }
        let interval = efficiency(
            actor.npc.state,
            in_pvs,
            in_pvs && player.facing.dot(to_actor) > 0.,
            distance_squared.sqrt(),
        )
        .decision_interval();
        let danger = actor.class.sound_interests() & (sound::DANGER | sound::PHYSICS_DANGER);
        if interval > THINK_INTERVAL
            && (scene.fires.burning(id)
                || self.sounds.sounds.iter().any(|s| {
                    s.kind & danger != 0
                        && s.owner != Some(id)
                        && origin.distance_squared(s.origin) <= s.volume * s.volume
                }))
        {
            return THINK_INTERVAL;
        }
        interval
    }
}

impl AiActor {
    fn can_defer_decision(&self) -> bool {
        self.decisions.eligible
            && self.npc.enemy.is_none()
            && self.goal.is_none()
            && self.class.assault().is_none()
            && matches!(self.npc.state, NpcState::Idle | NpcState::Alert)
            && matches!(
                self.npc.schedule.as_deref(),
                Some("SCHED_IDLE_STAND" | "SCHED_ALERT_STAND")
            )
            && match &self.class {
                AiClass::Humanoid(h) => {
                    !h.actbusy.enabled
                        && h.train.is_none()
                        && matches!(
                            h.kind,
                            super::humanoid::Kind::Combine
                                | super::humanoid::Kind::Companion { citizen: true }
                        )
                }
                AiClass::Zombie(_) | AiClass::Headcrab(_) => true,
                _ => false,
            }
    }
}
