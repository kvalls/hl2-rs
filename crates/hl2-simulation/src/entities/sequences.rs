//! Scripted sequences and the AI's animation bridge: activities, sequence events, script moves
//! and scripted_sequence playback.

use super::{metropolice_idle_activity, number, sequence_finish, Pending, Scene, SceneMoveCommand};
use crate::physics;
use glam::Vec3;
use modkit_core::World;

/// The internal input name of a scripted_sequence's search think.
pub(super) const SCRIPT_THINK: &str = "ScriptThink";
/// A scripted move's arrival to TASK_PLANT_ON_SCRIPT: two thinks of the AI (0.1 s each, seven
/// 15 ms ticks). Native (owned game, npc_task_text with point_proximity_sensor positions,
/// d1_trainstation_01's vortigaunt): on the mark at 105.96 and 119.17 s, planted (and
/// OnBeginSequence) at 106.17 and 119.40 s.
const SCRIPT_ARRIVAL_THINKS: f64 = 2. * 0.105;

impl Scene {
    /// The actor belongs to a script: a scripted_sequence, a choreographed scene (as an
    /// actor of a running playback or through its body animation) or a scene move.
    /// The AI yields to it (NPC_STATE_SCRIPT / SCHED_AISCRIPT semantics).
    pub fn ai_scripted(&self, id: usize) -> bool {
        self.script_holds(id) || self.scene_events_hold(id)
    }
    /// A scripted_sequence, a scene animation or a scene or script move owns the actor
    /// (an unknown actor counts as held).
    pub fn script_holds(&self, id: usize) -> bool {
        let Some(state) = self.states.get(id) else {
            return true;
        };
        state.scripted_by.is_some()
            || state.scene_animation.is_some()
            || state
                .movement_animation
                .as_ref()
                .is_some_and(|m| m.key.scene != crate::npc::AI_SCENE)
            || self
                .movement_ready
                .keys()
                .any(|k| k.actor == id && k.scene != crate::npc::AI_SCENE)
    }
    /// A scene holds an NPC's AI only when it has events that enter SCHED_SCENE_GENERIC
    /// for it; speech-only scenes (the can cop's nags and "carry on" lines) leave the AI
    /// running, like native.
    pub fn scene_events_hold(&self, id: usize) -> bool {
        self.choreography.values().any(|c| {
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
    /// SetIdealActivity + MaintainActivity for the activity already playing:
    /// SelectWeightedSequence(activity, current) and a switch only to another sequence
    /// (true). The same pick, an unknown activity or a missing rig keep the sequence.
    pub fn ai_reselect_activity(&mut self, world: &World, id: usize, activity: &str) -> bool {
        let current = self.states[id].animation.clone();
        match self.select_weighted_sequence(world, id, activity) {
            Some(picked) if !picked.eq_ignore_ascii_case(&current) => {
                self.ai_set_activity(world, id, &picked)
            }
            _ => false,
        }
    }
    /// SelectWeightedSequence(activity, current sequence) on the actor's loaded rig, with
    /// the scene's random stream; None for an unknown activity or a missing rig.
    pub fn select_weighted_sequence(
        &mut self,
        world: &World,
        id: usize,
        activity: &str,
    ) -> Option<String> {
        let rig = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))
            .and_then(|i| world.rigs.get(&i.asset_key()))?;
        let current = self.states.get(id)?.animation.clone();
        let random = &mut self.random;
        rig.lookup_sequence(activity, Some(&current), |upper| {
            *random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            (*random >> 32) as u32 % upper
        })
        .map(str::to_owned)
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

    /// m_flPlaybackRate over one tick of `dt`: the entity's sequence runs at `rate` of the game's
    /// clock. A sequence's time is the time since `animation_started`, so the start (and the AI's
    /// sequence end with it) moves on by the share it does not play.
    pub fn play_at_rate(&mut self, id: usize, rate: f32, dt: f32) {
        let held = f64::from((1. - rate) * dt);
        if let Some(state) = self.states.get_mut(id) {
            state.animation_started += held;
        }
        if let Some(end) = self.ai_sequence_end.get_mut(&id) {
            *end += held;
        }
    }

    pub fn body_sequence_owner(&self, id: usize) -> Option<usize> {
        self.states
            .get(id)?
            .scene_animation
            .as_ref()
            .map(|s| s.owner)
    }

    /// The activity of the actor's current sequence, ACT_IDLE when it has none
    /// (CAI_ScriptedSequence::StartSequence: SetActivityAndSequence(act, ...)).
    /// Whether a scripted_sequence or a scene plays its own animation on the actor.
    pub fn script_animates(&self, id: usize) -> bool {
        self.states
            .get(id)
            .is_some_and(|s| s.scripted_by.is_some() || s.scene_animation.is_some())
    }
    pub fn sequence_activity(&self, world: &World, id: usize) -> Option<String> {
        let animation = &self.states.get(id)?.animation;
        let rig = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))
            .and_then(|i| world.rigs.get(&i.asset_key()))?;
        let activity = rig
            .sequences
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(animation))
            .map(|s| s.activity.as_str())
            .filter(|a| !a.is_empty())
            .unwrap_or("ACT_IDLE");
        Some(activity.to_owned())
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
        self.ai_auto_motion_time.remove(&id);
        true
    }

    /// Combine camera activity for its enabled state (MaintainActivity toward the ideal
    /// activity): open idle when enabled; when disabled the close transition plays first
    /// (if `transition`), then closed idle. The reversed opening transition is not modeled.
    /// CNPC_CombineCamera::Spawn: deployed (open idle) unless spawnflag 0x80 (StartInactive),
    /// which keeps it closed and disabled.
    pub(super) fn spawn_combine_cameras(&mut self, world: &World) {
        for id in 0..world.entities.len() {
            if world.entities[id].class() == "npc_combine_camera"
                && world.entities[id].get(crate::ai::spawn::POOL_KEY).is_none()
            {
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

    /// CAI_ScriptedSequence::Spawn: a script without a name, or with SF_SCRIPT_START_ON_SPAWN
    /// (16), starts thinking 1 s after spawn; a named one then waits for BeginSequence.
    pub(super) fn start_scripts_on_spawn(&mut self, world: &World) {
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() != "scripted_sequence" || self.template_pending(id) {
                continue;
            }
            let named = e.get("targetname").is_some_and(|n| !n.is_empty());
            if named && number(e, "spawnflags", 0.) as u32 & 16 == 0 {
                continue;
            }
            let input = if named {
                "MoveToPosition"
            } else {
                "BeginSequence"
            };
            self.queue_script_input(id, input, 1.);
        }
    }

    /// Queues a script's own BeginSequence or MoveToPosition `delay` seconds from now.
    fn queue_script_input(&mut self, id: usize, input: &str, delay: f64) {
        self.sequence += 1;
        self.queue.push(Pending {
            due: self.time + delay,
            sequence: self.sequence,
            target: format!("#{id}"),
            input: input.into(),
            parameter: String::new(),
            caller: id,
            activator: usize::MAX,
            opener: None,
        });
    }

    /// MoveToPosition: the actor goes to the mark and idles there (m_iszIdle) until
    /// BeginSequence plays the action.
    pub(super) fn move_to_position(&mut self, world: &World, id: usize) {
        self.states[id].script_wait = true;
        self.start_script(world, id);
    }

    /// BeginSequence: a waiting script's actor plays the action on its mark (or on arrival).
    pub(super) fn begin_sequence(&mut self, world: &World, id: usize) {
        if std::mem::take(&mut self.states[id].script_wait) {
            if self.states[id].script_move.is_some() {
                return;
            }
            if let Some(actor) = self.states[id].script_actor {
                return self.start_script_play(world, id, actor);
            }
        }
        self.start_script(world, id);
    }

    fn start_script(&mut self, world: &World, id: usize) {
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
            return self.retry_script_search(id);
        };
        if self.states[actor]
            .scripted_by
            .is_some_and(|owner| owner != id)
        {
            let input = if self.states[id].script_wait {
                "MoveToPosition"
            } else {
                "BeginSequence"
            };
            self.queue_script_input(id, input, 0.25);
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
            // CINE_MOVETO_WALK / RUN / CUSTOM (SCHED_SCRIPTED_WALK/RUN/CUSTOM_MOVE): the actor
            // goes to the mark through the NPC movement controller; arrival (tick) plants him
            // on the script origin, faces its angles and plays. A custom move without
            // m_iszCustomMove walks (GetScriptCustomMoveActivity's ACT_WALK); one naming a
            // sequence moves with it (ss_VortSweep*: Walk_all_HoldBroom), loaded at actor
            // setup (actors.rs), walking when the actor has no such track.
            1..=3 => {
                let key = crate::npc::GoalKey {
                    scene: id,
                    event: 0,
                    actor,
                };
                self.states[actor].script_post_idle = None;
                self.states[id].script_actor = Some(actor);
                self.states[id].script_move = Some(key);
                self.states[id].script_plant_at = None;
                self.movement_ready.insert(key, false);
                self.movement_commands.push(SceneMoveCommand::Start {
                    key,
                    request: crate::npc::MoveRequest {
                        target_entity: id,
                        target_feet: script.origin(),
                        style: match movement {
                            2 => crate::npc::MoveStyle::Run,
                            3 if !custom_move_walks(script) => crate::npc::MoveStyle::Custom,
                            _ => crate::npc::MoveStyle::Walk,
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

    /// CAI_ScriptedSequence::ScriptThink without an NPC: CancelScript (OnCancelSequence, and
    /// OnCancelFailedSequence for a script that never started) and another search in 1 s, so
    /// an actor a point_template spawns later is found (d1_trainstation_01's ss_luggageShove).
    /// The think rides the input queue under SCRIPT_THINK, which is not an entity input: not
    /// counted, not traced. The cancel of other scripts of the same name is not modeled.
    fn retry_script_search(&mut self, id: usize) {
        self.fire(id, "OnCancelSequence", usize::MAX);
        self.fire(id, "OnCancelFailedSequence", usize::MAX);
        self.queue_script_input(id, SCRIPT_THINK, 1.);
    }

    /// The retry's think: the same search, still waiting for BeginSequence if it was.
    pub(super) fn script_think(&mut self, world: &World, id: usize) {
        self.start_script(world, id);
    }

    /// The script's arrival (CINE_MOVETO_WALK/RUN): TASK_PLANT_ON_SCRIPT and
    /// TASK_FACE_SCRIPT, then the sequence, `SCRIPT_ARRIVAL_THINKS` after the move ends
    /// (TASK_WAIT_FOR_MOVEMENT sees the navigator's goal cleared a think later and the plant
    /// starts on the next).
    pub(super) fn finish_script_move(&mut self, world: &World, id: usize) {
        let Some(key) = self.states[id].script_move else {
            return;
        };
        if !self.movement_ready.get(&key).copied().unwrap_or(false) {
            return;
        }
        let now = self.time;
        let plant_at = match self.states[id].script_plant_at {
            Some(at) => at,
            None => {
                // The walk ends on arrival; the plant waits for the AI's thinks.
                self.restore_movement_animation(key.actor);
                *self.states[id]
                    .script_plant_at
                    .insert(now + SCRIPT_ARRIVAL_THINKS)
            }
        };
        if plant_at > now + 1e-9 {
            return;
        }
        self.states[id].script_plant_at = None;
        self.states[id].script_move = None;
        self.movement_ready.remove(&key);
        self.restore_movement_animation(key.actor);
        self.states[key.actor].origin = self.states[id].origin;
        self.states[key.actor].rotation = self.states[id].rotation;
        self.start_script_play(world, id, key.actor);
    }

    /// TASK_WAIT_FOR_SCRIPT: the actor owned by a waiting script plays its m_iszIdle.
    fn start_script_idle(&mut self, world: &World, id: usize, actor: usize) {
        self.states[actor].script_post_idle = None;
        self.states[actor].scripted_by = Some(id);
        self.states[id].script_actor = Some(actor);
        if let Some(idle) = self
            .ent(world, id)
            .get("m_iszIdle")
            .filter(|s| !s.is_empty())
        {
            self.animate(world, actor, idle, false);
        }
    }

    /// StartSequence: the actor plays m_iszPlay (or ends at once without one); a waiting
    /// script idles instead.
    fn start_script_play(&mut self, world: &World, id: usize, actor: usize) {
        if self.states[id].script_wait {
            return self.start_script_idle(world, id, actor);
        }
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
            if !cancel && self.start_next_script(world, id, actor) {
                return;
            }
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
    /// Advance an actor's origin and yaw by the active sequence's authored root motion since
    /// the previous call (or sequence start), clamping to `cycle = 1.0` when `clamp_to_end` is
    /// set on a non-looping clip (`CBaseAnimating::AutoMovement`).
    pub fn ai_auto_movement(&mut self, world: &World, id: usize, clamp_to_end: bool) -> bool {
        let Some(state) = self.states.get(id) else {
            return false;
        };
        let seq_name = state.animation.clone();
        let started = state.animation_started;
        let forward = state.rotation * Vec3::X;
        let yaw_rad = forward.y.atan2(forward.x);
        let Some(clip) = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))
            .and_then(|i| world.rigs.get(&i.asset_key()))
            .and_then(|r| {
                r.clips
                    .get(&seq_name)
                    .or_else(|| r.clips.get(&seq_name.to_lowercase()))
            })
        else {
            return false;
        };
        let duration = f64::from(clip.duration());
        if duration <= 1e-6 {
            return false;
        }
        let prev_time = self
            .ai_auto_motion_time
            .get(&id)
            .and_then(|(prev_seq, t)| prev_seq.eq_ignore_ascii_case(&seq_name).then_some(*t))
            .unwrap_or(started);
        let next_time = if clamp_to_end && !clip.looping {
            (started + duration).max(self.time)
        } else {
            self.time
        };
        self.ai_auto_motion_time.insert(id, (seq_name, next_time));
        let mut from_cycle = ((prev_time - started) / duration) as f32;
        let mut to_cycle = ((next_time - started) / duration) as f32;
        if !clip.looping {
            from_cycle = from_cycle.clamp(0.0, 1.0);
            to_cycle = to_cycle.clamp(0.0, 1.0);
        }
        if to_cycle <= from_cycle {
            return false;
        }
        let Some(sample) = clip.movement(from_cycle, to_cycle) else {
            return false;
        };
        let (sin, cos) = yaw_rad.sin_cos();
        let world_delta = Vec3::new(
            sample.position.x * cos - sample.position.y * sin,
            sample.position.x * sin + sample.position.y * cos,
            sample.position.z,
        );
        self.states[id].origin += world_delta;
        if sample.yaw_degrees.abs() > 1e-4 {
            let new_yaw = yaw_rad.to_degrees() + sample.yaw_degrees;
            self.ai_set_yaw(id, new_yaw);
        }
        true
    }
}

/// A custom scripted move that walks: no m_iszCustomMove, or ACT_WALK itself.
pub(crate) fn custom_move_walks(script: &modkit_core::Entity) -> bool {
    script
        .get("m_iszCustomMove")
        .map(str::trim)
        .is_none_or(|m| m.is_empty() || m.eq_ignore_ascii_case("ACT_WALK"))
}
