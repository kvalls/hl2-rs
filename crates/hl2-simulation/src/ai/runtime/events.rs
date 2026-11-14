//! Stages of `AiRuntime::tick`: yielding to a script and the weapon's events and sounds.
use super::{AiRuntime, NpcState};
use crate::entities::Scene;
use crate::npc::Controller;
use crate::npc_weapons::{self, NpcWeaponDef};
use glam::Vec3;
use modkit_core::World;

impl AiRuntime {
    /// NPC_STATE_SCRIPT: scripts own the actor; its goal stops and its activity is the script's.
    pub(super) fn yield_to_script(
        &mut self,
        id: usize,
        world: &World,
        scene: &mut Scene,
        controller: &mut Controller,
    ) {
        let actor = self.actors.get_mut(&id).expect("actor");
        if actor.npc.state != NpcState::Script {
            actor.npc.state = NpcState::Script;
            actor.npc.schedule = None;
        }
        if let Some(key) = actor.goal.take() {
            controller.cancel(key);
            controller.forget(key);
            scene.ai_forget_move(key);
            // SCHED_SCENE_GENERIC's TASK_STOP_MOVING and TASK_SET_ACTIVITY ACT_IDLE: a scene
            // that takes a walking NPC without animating it itself leaves it idle.
            if !scene.script_animates(id) {
                scene.ai_set_activity(world, id, "ACT_IDLE");
            }
        }
        if let Some(cop) = actor.class.cop_mut() {
            cop.scripted = true;
        }
        // The script's sequence sets m_Activity: when the AI takes over, its
        // TASK_SET_ACTIVITY of that same activity keeps the sequence running.
        if let Some(activity) = scene.sequence_activity(world, id) {
            actor.npc.activity = activity;
        }
    }

    /// The weapon's animation events due this tick, after the NPC's own.
    pub(super) fn weapon_events(
        &mut self,
        id: usize,
        world: &World,
        now: f64,
        events: &mut Vec<(String, i32)>,
    ) {
        // Move and shoot: the restarted ACT_GESTURE_RANGE_ATTACK1's fire event.
        let queued = std::mem::take(&mut self.actors.get_mut(&id).expect("actor").move_fire);
        if queued > 0 {
            if let Some(event) = self.actors[&id]
                .weapon
                .as_ref()
                .and_then(|w| self.weapon_def(&w.class))
                .and_then(|d| d.fire)
                .map(|f| f.event)
            {
                events.extend((0..queued).map(|_| (String::new(), event)));
            }
        }
        // Operator_FrameUpdate: the weapon world model's events go to its operator.
        if let Some(weapon) = self.actors.get_mut(&id).and_then(|a| a.weapon.as_mut()) {
            if let Some(rig) = self
                .weapon_defs
                .get(&weapon.class)
                .and_then(|d| world.rigs.get(&npc_weapons::rig_key(&d.world_model)))
            {
                for (name, event, options) in weapon.due_events(rig, now) {
                    // AE_NPC_MUZZLEFLASH is a client event (DispatchMuzzleEffect).
                    if name.eq_ignore_ascii_case("AE_NPC_MUZZLEFLASH") {
                        self.muzzle_flashes.push((id, options));
                    } else {
                        events.push((name, event));
                    }
                }
            }
        }
    }

    /// The sounds of this tick's shots and reload: `shots` is whether it fired, the times of
    /// its shot sounds and whether it reloaded.
    pub(super) fn weapon_sounds(
        &mut self,
        id: usize,
        def: Option<&NpcWeaponDef>,
        (fired, mut shot_sound_times, reloaded): (bool, Vec<f64>, bool),
        scene: &mut Scene,
        now: f64,
    ) {
        // WeaponSound(SINGLE_NPC / RELOAD_NPC) at the operator; a machine gun's
        // later realtime sounds wait in `delayed_sounds`.
        if fired && shot_sound_times.is_empty() {
            shot_sound_times.push(now);
        }
        let origin = scene.states[id].origin + Vec3::Z * 55.;
        for (times, key) in [
            (shot_sound_times, "single_shot_npc"),
            (if reloaded { vec![now] } else { Vec::new() }, "reload_npc"),
        ] {
            let Some(cue) = def.and_then(|d| d.sounds.get(key)) else {
                continue;
            };
            for at in times {
                let mut requests = vec![crate::sounds::SoundRequest {
                    origin: Some(origin),
                    ..cue.clone().into()
                }];
                // SINGLE_NPC plays on CHAN_WEAPON of the operator: a new shot
                // sound replaces the previous one (native recording: run levels
                // match a single attack, not stacked tails).
                if key == "single_shot_npc" {
                    let emitter = crate::sounds::entity_emitter(id);
                    requests[0].emitter = Some(emitter);
                    requests.insert(0, crate::sounds::SoundRequest::stop(cue, emitter));
                }
                for request in requests {
                    if at <= now + 1e-9 {
                        scene.sounds.push(request);
                    } else {
                        self.delayed_sounds.push((at, request));
                    }
                }
            }
        }
    }
}
