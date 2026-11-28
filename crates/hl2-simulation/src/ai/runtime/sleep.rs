//! Authored CAI_BaseNPC sleep states. Runtime spawns default to awake.
use super::{AiRuntime, NpcState, PhysicsSight, PlayerState, SightWorld};
use crate::{ai::senses::sound, entities::Scene, npc::Controller};
use glam::Vec3;
use modkit_core::Entity;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Awake,
    Threat,
    Pvs,
    Input,
    AutoPvs,
    AutoPvsAfterSeen,
}

#[derive(Clone, Debug)]
pub(super) struct Sleep {
    mode: Mode,
    radius: f32,
    wait_till_seen: bool,
}

impl Sleep {
    pub(super) fn new(entity: &Entity, flags: u32) -> Self {
        let mode = match entity
            .get("sleepstate")
            .and_then(|v| v.trim().parse::<u8>().ok())
        {
            Some(1) => Mode::Threat,
            Some(2) => Mode::Pvs,
            Some(3) => Mode::Input,
            Some(4) => Mode::AutoPvs,
            Some(5) => Mode::AutoPvsAfterSeen,
            _ => Mode::Awake,
        };
        let radius = entity
            .get("wakeradius")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|r| r.is_finite() && *r > 0.1)
            .unwrap_or(0.);
        Self {
            mode,
            radius,
            wait_till_seen: flags & 1 != 0,
        }
    }
}

impl AiRuntime {
    pub(super) fn actor_runs_ai(
        &mut self,
        id: usize,
        scene: &mut Scene,
        controller: &mut Controller,
        sight: &PhysicsSight<'_>,
        player: PlayerState,
    ) -> bool {
        let state = &scene.states[id];
        if state.killed || !state.visible || self.held_npcs.contains(&id) {
            return false;
        }
        // NPC_STATE_SCRIPT: the scene owns the actor.
        if scene.ai_scripted(id) && !self.refuses_scene(id, scene) {
            self.wake_actor(id, scene);
            self.yield_to_script(id, sight.world, scene, controller);
            self.diagnostics.scripted_skips += 1;
            return false;
        }
        !self.update_actor_sleep(id, scene, controller, sight, player)
    }

    pub(super) fn wake_actor(&mut self, id: usize, scene: &mut Scene) {
        if let Some(actor) = self.actors.get_mut(&id) {
            if matches!(actor.sleep.mode, Mode::Threat | Mode::Pvs | Mode::Input) {
                actor.sleep.mode = Mode::Awake;
            }
            if std::mem::take(&mut scene.states[id].ai_sleeping) {
                actor.next_think = scene.time;
                scene.fire(id, "OnWake", id);
            }
        }
    }

    fn update_actor_sleep(
        &mut self,
        id: usize,
        scene: &mut Scene,
        controller: &mut Controller,
        sight: &PhysicsSight<'_>,
        player: PlayerState,
    ) -> bool {
        let actor = self.actors.get_mut(&id).expect("tracked actor");
        let sleep = &mut actor.sleep;
        if sleep.mode == Mode::Awake {
            return false;
        }
        let state = &scene.states[id];
        let origin = state.origin;
        let in_pvs = scene.look_targets.actor_pvs(id);
        let in_radius = !player.noclip
            && sleep.radius > 0.
            && origin.distance_squared(player.feet) <= sleep.radius * sleep.radius;
        if sleep.mode == Mode::AutoPvsAfterSeen && in_pvs == Some(true) {
            sleep.mode = Mode::AutoPvs;
        }
        let should_sleep = match sleep.mode {
            Mode::AutoPvs => {
                in_pvs == Some(false)
                    && actor.npc.state != NpcState::Script
                    && actor.damage_taken == 0.
            }
            Mode::Input => true,
            Mode::Pvs => in_pvs == Some(false) || scene.look_targets.pvs_time == 0.,
            Mode::Threat => {
                actor.damage_taken == 0.
                    && !visible_threat(sight, id, origin, player, in_pvs)
                    && (sleep.wait_till_seen
                        || !self.sounds.sounds.iter().any(|s| {
                            s.kind & actor.class.sound_interests() & sound::DANGER != 0
                                && s.origin.distance_squared(origin) <= s.volume * s.volume
                                && sight.visible(origin + Vec3::Z * 64., s.origin, &[id])
                        }))
            }
            _ => return false,
        };
        if in_radius || !should_sleep {
            self.wake_actor(id, scene);
            return false;
        }
        if !std::mem::replace(&mut scene.states[id].ai_sleeping, true) {
            if let Some(goal) = actor.goal.take() {
                controller.cancel(goal);
            }
            actor.npc.schedule = None;
            scene.fire(id, "OnSleep", id);
        }
        true
    }
}

fn visible_threat(
    sight: &PhysicsSight<'_>,
    id: usize,
    origin: Vec3,
    player: PlayerState,
    in_pvs: Option<bool>,
) -> bool {
    in_pvs != Some(false)
        && !player.dead
        && !player.noclip
        && sight.visible(player.eye(), origin + Vec3::Z * 36., &[id])
}
