//! Scripted sequences and the AI's animation bridge: activities, sequence events, script moves
//! and scripted_sequence playback.

use super::{metropolice_idle_activity, number, sequence_finish, Pending, Scene, SceneMoveCommand};
use crate::physics;
use glam::Vec3;
use modkit_core::World;

impl Scene {
    /// The actor belongs to a script: a scripted_sequence, a choreographed scene (as an
    /// actor of a running playback or through its body animation) or a scene move.
    /// The AI yields to it (NPC_STATE_SCRIPT / SCHED_AISCRIPT semantics).
    pub fn ai_scripted(&self, id: usize) -> bool {
        let Some(state) = self.states.get(id) else {
            return true;
        };
        state.scripted_by.is_some()
            || state.scene_animation.is_some()
            || state
                .movement_animation
                .as_ref()
                .is_some_and(|m| m.key.scene != crate::npc::AI_SCENE)
            // A scene holds an NPC's AI only when it has events that enter
            // SCHED_SCENE_GENERIC for it; speech-only scenes (the can cop's nags and
            // "carry on" lines) leave the AI running, like native.
            || self.choreography.values().any(|c| {
                c.playback.as_ref().is_some_and(|p| {
                    p.actors.iter().enumerate().any(|(slot, a)| {
                        *a == Some(id)
                            && c.data
                                .events
                                .iter()
                                .any(|e| e.actor == Some(slot) && e.active() && e.takes_npc_ai())
                    })
                })
            })
            || self
                .movement_ready
                .keys()
                .any(|k| k.actor == id && k.scene != crate::npc::AI_SCENE)
    }

    /// Play an activity or sequence on an actor for the AI (CAI_BaseNPC::SetActivity:
    /// the model's weighted sequence for the activity). False when the rig has none.
    pub fn ai_set_activity(&mut self, world: &World, id: usize, activity: &str) -> bool {
        if !self.animate(world, id, activity, false) {
            return false;
        }
        let finish = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))
            .and_then(|i| world.rigs.get(&i.asset_key()))
            .and_then(|rig| rig.clips.get(&self.states[id].animation))
            .map_or(0., sequence_finish);
        self.ai_sequence_end.insert(id, self.time + finish);
        true
    }

    /// Animation events (name, id) of the actor's current sequence whose time falls in
    /// (from, to] (studio events, played once per sequence start).
    pub fn ai_events(&self, world: &World, id: usize, from: f64, to: f64) -> Vec<(String, i32)> {
        let Some(state) = self.states.get(id) else {
            return Vec::new();
        };
        let Some(clip) = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))
            .and_then(|i| world.rigs.get(&i.asset_key()))
            .and_then(|rig| rig.clips.get(&state.animation))
        else {
            return Vec::new();
        };
        let duration = f64::from(clip.duration().max(0.015));
        // DispatchAnimEvents: once a non-looping sequence is finished (flEnd = 1.01),
        // its remaining events all fire.
        let finished = state.animation_started + sequence_finish(clip);
        clip.events
            .iter()
            .filter(|e| {
                let mut at = state.animation_started + f64::from(e.cycle) * duration;
                if !clip.looping {
                    at = at.min(finished);
                }
                at > from && at <= to
            })
            .map(|e| (e.name.clone(), e.id))
            .collect()
    }

    /// SequenceDuration of the actor's current sequence (0 without a clip).
    pub fn ai_sequence_duration(&self, world: &World, id: usize) -> f32 {
        world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))
            .and_then(|i| world.rigs.get(&i.asset_key()))
            .and_then(|rig| rig.clips.get(&self.states.get(id)?.animation))
            .map_or(0., |clip| clip.duration().max(0.015))
    }

    /// The AI's current sequence is finished (m_bSequenceFinished, `sequence_finish`).
    pub fn ai_activity_finished(&self, id: usize) -> bool {
        self.ai_sequence_end
            .get(&id)
            .is_none_or(|end| self.time >= *end)
    }

    /// Accept movement updates for an AI goal (apply_movement only follows known keys).
    pub fn ai_register_move(&mut self, key: crate::npc::GoalKey) {
        self.movement_ready.insert(key, false);
    }

    pub fn ai_forget_move(&mut self, key: crate::npc::GoalKey) {
        self.movement_ready.remove(&key);
        if self
            .states
            .get(key.actor)
            .and_then(|s| s.movement_animation.as_ref())
            .is_some_and(|m| m.key == key)
        {
            self.restore_movement_animation(key.actor);
        }
    }

    /// The AI turned the actor (CAI_Motor::UpdateYaw) while standing.
    pub fn ai_set_yaw(&mut self, id: usize, yaw_degrees: f32) {
        if let Some(state) = self.states.get_mut(id) {
            state.rotation = physics::angles(Vec3::new(0., yaw_degrees, 0.));
        }
    }

    pub fn body_sequence_owner(&self, id: usize) -> Option<usize> {
        self.states
            .get(id)?
            .scene_animation
            .as_ref()
            .map(|s| s.owner)
    }

    /// HaveSequenceForActivity on the actor's loaded rig.
    pub fn actor_has_activity(&self, world: &World, id: usize, activity: &str) -> bool {
        world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))
            .and_then(|i| world.rigs.get(&i.asset_key()))
            .is_some_and(|r| {
                r.sequences
                    .iter()
                    .any(|s| s.activity.eq_ignore_ascii_case(activity))
            })
    }

    /// `animate` with an activity only when the actor's rig has a sequence for it.
    pub(super) fn animate_activity_if_present(&mut self, world: &World, id: usize, activity: &str) {
        if self.actor_has_activity(world, id, activity) {
            self.animate(world, id, activity, false);
        }
    }

    pub(super) fn animate(&mut self, world: &World, id: usize, name: &str, finish: bool) -> bool {
        let key = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))
            .map(|i| i.asset_key());
        let rig = key.as_ref().and_then(|key| world.rigs.get(key));
        let random = &mut self.random;
        let resolved = rig.and_then(|r| {
            r.lookup_sequence(name, None, |upper| {
                *random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                (*random >> 32) as u32 % upper
            })
        });
        let clip = rig.zip(resolved).and_then(|(r, label)| r.clips.get(label));
        let Some((clip, resolved)) = clip.zip(resolved) else {
            self.unsupported_input(self.ent(world, id).class(), &format!("animation:{name}"));
            return false;
        };
        self.states[id].animation = resolved.to_owned();
        self.states[id].scene_animation = None;
        self.states[id].animation_started = self.time;
        self.states[id].animation_done =
            finish.then_some(self.time + clip.duration().max(0.015) as f64);
        true
    }

    /// Combine camera activity for its enabled state (MaintainActivity toward the ideal
    /// activity): open idle when enabled; when disabled the close transition plays first
    /// (if `transition`), then closed idle. The reversed opening transition is not modeled.
    /// CNPC_CombineCamera::Spawn: deployed (open idle) unless spawnflag 0x80 (StartInactive),
    /// which keeps it closed and disabled.
    pub(super) fn spawn_combine_cameras(&mut self, world: &World) {
        for id in 0..world.entities.len() {
            if world.entities[id].class() == "npc_combine_camera" {
                let enabled =
                    super::number(&world.entities[id], "spawnflags", 0.) as u32 & 0x80 == 0;
                self.states[id].enabled = enabled;
                self.camera_activity(world, id, false);
            }
        }
    }

    pub(super) fn camera_activity(&mut self, world: &World, id: usize, transition: bool) {
        let open = self.states[id].animation.eq_ignore_ascii_case("idlealert");
        if self.states[id].enabled {
            self.animate(world, id, "ACT_COMBINE_CAMERA_OPEN_IDLE", false);
        } else if transition && open {
            self.animate(world, id, "ACT_COMBINE_CAMERA_CLOSE", true);
        } else {
            self.animate(world, id, "ACT_COMBINE_CAMERA_CLOSED_IDLE", false);
        }
    }

    pub(super) fn begin_sequence(&mut self, world: &World, id: usize) {
        let script = self.ent(world, id);
        let name = script.get("m_iszEntity").unwrap_or("");
        let actor = world
            .entities
            .iter()
            .enumerate()
            .filter(|(i, e)| {
                !self.states[*i].killed
                    && e.class().starts_with("npc_")
                    && (e.get("targetname") == Some(name) || e.class() == name)
            })
            .min_by(|(a, _), (b, _)| {
                self.states[*a]
                    .origin
                    .distance_squared(script.origin())
                    .total_cmp(&self.states[*b].origin.distance_squared(script.origin()))
            })
            .map(|(i, _)| i);
        let Some(actor) = actor else {
            self.unsupported_input("scripted_sequence", &format!("actor:{name}"));
            return;
        };
        if self.states[actor]
            .scripted_by
            .is_some_and(|owner| owner != id)
        {
            self.sequence += 1;
            self.queue.push(Pending {
                due: self.time + 0.25,
                sequence: self.sequence,
                target: format!("#{id}"),
                input: "BeginSequence".into(),
                parameter: String::new(),
                caller: id,
                activator: usize::MAX,
                opener: None,
            });
            return;
        }
        let movement = number(script, "m_fMoveTo", 0.) as i32;
        match movement {
            // CINE_MOVETO_TELEPORT.
            4 => {
                self.states[actor].origin = script.origin();
                self.states[actor].rotation = self.states[id].rotation;
            }
            // CINE_MOVETO_WAIT_FACING: face the script's angles, then play (the turn is
            // instant here).
            5 => self.states[actor].rotation = self.states[id].rotation,
            // CINE_MOVETO_WALK / RUN (SCHED_SCRIPTED_WALK/RUN): the actor goes to the mark
            // through the NPC movement controller; arrival (tick) plants him on the script
            // origin, faces its angles and plays.
            1 | 2 => {
                let key = crate::npc::GoalKey {
                    scene: id,
                    event: 0,
                    actor,
                };
                self.states[actor].script_post_idle = None;
                self.states[id].script_actor = Some(actor);
                self.states[id].script_move = Some(key);
                self.movement_ready.insert(key, false);
                self.movement_commands.push(SceneMoveCommand::Start {
                    key,
                    request: crate::npc::MoveRequest {
                        target_entity: id,
                        target_feet: script.origin(),
                        style: if movement == 1 {
                            crate::npc::MoveStyle::Walk
                        } else {
                            crate::npc::MoveStyle::Run
                        },
                        event_distance: 0.,
                        // StartScriptMoveToTargetTask walks unless within 1 unit.
                        force_short: true,
                    },
                });
                return;
            }
            0 => {}
            _ => {
                self.unsupported_input("scripted_sequence", &format!("movement:{movement}"));
                return;
            }
        }
        self.start_script_play(world, id, actor);
    }

    /// The script's arrival (CINE_MOVETO_WALK/RUN): TASK_PLANT_ON_SCRIPT and
    /// TASK_FACE_SCRIPT, then the sequence.
    pub(super) fn finish_script_move(&mut self, world: &World, id: usize) {
        let Some(key) = self.states[id].script_move else {
            return;
        };
        if !self.movement_ready.get(&key).copied().unwrap_or(false) {
            return;
        }
        self.states[id].script_move = None;
        self.movement_ready.remove(&key);
        self.restore_movement_animation(key.actor);
        self.states[key.actor].origin = self.states[id].origin;
        self.states[key.actor].rotation = self.states[id].rotation;
        self.start_script_play(world, id, key.actor);
    }

    /// StartSequence: the actor plays m_iszPlay (or ends at once without one).
    fn start_script_play(&mut self, world: &World, id: usize, actor: usize) {
        let script = self.ent(world, id);
        // Another script takes the actor: the old post idle ends (m_hNextCine).
        self.states[actor].script_post_idle = None;
        let play = script.get("m_iszPlay").unwrap_or("");
        if play.is_empty() {
            // Retail StartSequence completes a null action immediately, allowing the authored post idle.
            self.states[actor].scripted_by = Some(id);
            self.states[id].script_actor = Some(actor);
            self.fire(id, "OnBeginSequence", actor);
            self.end_sequence(world, id, false);
            return;
        }
        if !self.animate(world, actor, play, false) {
            return;
        }
        let duration = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(actor))
            .and_then(|i| world.rigs.get(&i.asset_key()))
            .and_then(|r| r.clips.get(&self.states[actor].animation))
            .map(|c| c.duration())
            .unwrap_or(0.);
        self.states[actor].scripted_by = Some(id);
        self.states[id].script_actor = Some(actor);
        self.states[id].script_finish = Some(self.time + duration.max(0.015) as f64);
        self.fire(id, "OnBeginSequence", actor);
    }

    /// SCRIPT_EVENT_FIREEVENT (1003) animation events crossed this tick by the actor's
    /// scripted clip fire the script's OnScriptEventNN output (options = NN).
    pub(super) fn script_events(&mut self, world: &World, id: usize) {
        let Some(actor) = self.states[id].script_actor else {
            return;
        };
        let state = &self.states[actor];
        let Some(clip) = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(actor))
            .and_then(|i| world.rigs.get(&i.asset_key()))
            .and_then(|r| r.clips.get(&state.animation))
        else {
            return;
        };
        let duration = clip.duration().max(0.015);
        let now = ((self.time - state.animation_started) as f32 / duration).min(1.);
        let before = now - self.tick_dt / duration;
        let fired: Vec<String> = clip
            .events
            .iter()
            .filter(|e| e.id == 1003 && e.cycle > before && e.cycle <= now)
            .filter_map(|e| e.options.trim().parse::<u32>().ok())
            .map(|n| format!("OnScriptEvent{n:02}"))
            .collect();
        for output in fired {
            self.fire(id, &output, actor);
        }
    }

    pub(super) fn end_sequence(&mut self, world: &World, id: usize, cancel: bool) {
        if cancel {
            for state in &mut self.states {
                if state.script_post_idle == Some(id) {
                    state.script_post_idle = None;
                }
            }
        }
        let actor = self.states[id].script_actor.take();
        self.states[id].script_finish = None;
        self.states[id].post_idle_done = None;
        if let Some(key) = self.states[id].script_move.take() {
            self.movement_ready.remove(&key);
            self.movement_commands
                .push(SceneMoveCommand::CancelScene(id));
            self.restore_movement_animation(key.actor);
        }
        if let Some(actor) = actor {
            self.states[actor].scripted_by = None;
            let name = self
                .ent(world, id)
                .get("m_iszPostIdle")
                .filter(|s| !s.is_empty())
                .or(self.ent(world, actor).get("DefaultAnim"))
                .unwrap_or_else(|| {
                    let preferred = if self.ent(world, actor).class() == "npc_metropolice" {
                        metropolice_idle_activity(self.ent(world, actor))
                    } else {
                        "idle_subtle"
                    };
                    let rig = world
                        .model_instances
                        .iter()
                        .find(|i| i.entity == Some(actor))
                        .and_then(|i| world.rigs.get(&i.asset_key()));
                    if preferred.starts_with("ACT_") {
                        preferred
                    } else if rig.is_some_and(|r| {
                        !r.clips.contains_key(preferred)
                            && r.sequences
                                .iter()
                                .any(|s| s.activity.eq_ignore_ascii_case("ACT_IDLE"))
                    }) {
                        "ACT_IDLE"
                    } else {
                        preferred
                    }
                });
            let animated = self.animate(world, actor, name, false);
            self.fire(
                id,
                if cancel {
                    "OnCancelSequence"
                } else {
                    "OnEndSequence"
                },
                actor,
            );
            if !cancel {
                let duration = if animated
                    && self
                        .ent(world, id)
                        .get("m_iszPostIdle")
                        .is_some_and(|n| !n.is_empty())
                {
                    world
                        .model_instances
                        .iter()
                        .find(|i| i.entity == Some(actor))
                        .and_then(|i| world.rigs.get(&i.asset_key()))
                        .and_then(|r| r.clips.get(&self.states[actor].animation))
                        .map_or(0., |c| c.duration())
                } else {
                    0.
                };
                self.states[id].post_idle_done = Some((self.time + duration as f64, actor));
                if duration > 0. {
                    self.states[actor].script_post_idle = Some(id);
                }
            }
        }
    }
}
