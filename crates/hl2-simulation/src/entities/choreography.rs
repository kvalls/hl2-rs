//! Choreographed scenes and actor animation: loading and installing scenes, their actors,
//! sounds and captions, poses, flexes, heads and look targets, scene movement, and the per-tick
//! scene playback.

use super::{
    angle_diff, clamp_yaw, glob, number, resolve_target_slot, scene_actor_class, Choreography,
    ChoreographyState, MovementAnimation, Playback, Scene, SceneAnimation, SceneAnimationState,
    SceneMoveCommand, ScenePause,
};
use crate::physics;
use glam::Vec3;
use modkit_core::World;
use source_assets::scenes::{Cache, ChoreoScene, EventType};
use source_assets::vpk::Vfs;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

impl Scene {
    /// Preload map choreography before simulation; no blocking asset reads in tick.
    pub fn load_choreography(&mut self, world: &World, vfs: &Vfs) -> anyhow::Result<usize> {
        use anyhow::Context;
        if !world
            .entities
            .iter()
            .any(|e| e.class() == "logic_choreographed_scene")
        {
            return Ok(0);
        }
        self.lipsync.load_classes(vfs);
        let bytes = vfs
            .read("scenes/scenes.image")?
            .context("installed scenes.image missing")?;
        let cache = Cache::parse(bytes)?;
        let mut parsed: BTreeMap<String, Arc<ChoreoScene>> = BTreeMap::new();
        for (id, entity) in world.entities.iter().enumerate() {
            if entity.class() != "logic_choreographed_scene" {
                continue;
            }
            let path = entity.get("SceneFile").unwrap_or("");
            let result = if let Some(data) = parsed.get(path) {
                Ok(Some(data.clone()))
            } else {
                cache.scene(path).map(|scene| {
                    scene.map(|data| {
                        let data = Arc::new(data);
                        parsed.insert(path.into(), data.clone());
                        data
                    })
                })
            };
            match result {
                Ok(Some(data)) => self.install_choreography(id, data),
                Ok(None) => {
                    self.unsupported_input(entity.class(), &format!("missing-scene:{path}"))
                }
                Err(error) => {
                    self.unsupported_input(entity.class(), &format!("scene:{path}:{error:#}"))
                }
            }
        }
        Ok(self.choreography.len())
    }

    pub(super) fn install_choreography(&mut self, id: usize, data: Arc<ChoreoScene>) {
        let mut order: Vec<_> = (0..data.events.len()).collect();
        order.sort_by(|a, b| {
            data.events[*a]
                .start
                .total_cmp(&data.events[*b].start)
                .then_with(|| data.events[*a].name.cmp(&data.events[*b].name))
        });
        self.choreography.insert(
            id,
            Choreography {
                data,
                order,
                playback: None,
            },
        );
        self.diagnostics.scenes_loaded = self.choreography.len();
    }

    /// Every actor-class entity a scene actor name can resolve to, including killed and
    /// template-pending ones (`!activator` is unknown before playback and resolves to none).
    fn scene_actor_candidates(&self, world: &World, scene: usize, name: &str) -> Vec<usize> {
        let name = resolve_target_slot(world, scene, name).to_lowercase();
        world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| {
                scene_actor_class(entity.class())
                    && entity
                        .get("targetname")
                        .is_some_and(|target| glob(&name, &target.to_lowercase()))
            })
            .map(|(id, _)| id)
            .collect()
    }

    pub(super) fn scene_actor(
        &self,
        world: &World,
        scene: usize,
        name: &str,
        activator: usize,
    ) -> Option<usize> {
        let name = resolve_target_slot(world, scene, name);
        if name.eq_ignore_ascii_case("!activator") {
            return (activator < self.states.len()).then_some(activator);
        }
        world
            .entities
            .iter()
            .enumerate()
            .find(|(id, entity)| {
                !self.states[*id].killed
                    && scene_actor_class(entity.class())
                    && entity
                        .get("targetname")
                        .is_some_and(|target| glob(&name.to_lowercase(), &target.to_lowercase()))
            })
            .map(|(id, _)| id)
    }

    /// Developer fixture setup only; does not reproduce campaign actor placement.
    pub fn fixture_actor_pose(
        &mut self,
        world: &World,
        target: &str,
        origin: Vec3,
        yaw: f32,
    ) -> bool {
        let Some(id) = world
            .entities
            .iter()
            .enumerate()
            .find(|(id, e)| {
                e.class().starts_with("npc_")
                    && !self.states[*id].killed
                    && e.get("targetname")
                        .is_some_and(|n| n.eq_ignore_ascii_case(target))
            })
            .map(|(id, _)| id)
        else {
            return false;
        };
        self.states[id].origin = origin;
        self.states[id].rotation = physics::angles(Vec3::new(0., yaw, 0.));
        true
    }

    fn scene_look_target(
        &self,
        world: &World,
        scene: usize,
        actor: usize,
        name: &str,
    ) -> Option<crate::attention::Target> {
        use crate::attention::Target;
        if name.eq_ignore_ascii_case("!self") {
            return Some(Target::Entity(actor));
        }
        let name = name
            .strip_prefix('!')
            .and_then(|s| s.to_lowercase().strip_prefix("target").map(str::to_owned))
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|slot| (1..=8).contains(slot))
            .and_then(|slot| self.ent(world, scene).get(&format!("target{slot}")))
            .unwrap_or(name);
        if name.eq_ignore_ascii_case("!player") || name.eq_ignore_ascii_case("player") {
            return Some(Target::Player);
        }
        world
            .entities
            .iter()
            .enumerate()
            .find(|(id, e)| {
                !self.states[*id].killed
                    && e.get("targetname")
                        .is_some_and(|n| glob(&name.to_lowercase(), &n.to_lowercase()))
            })
            .map(|(id, _)| Target::Entity(id))
    }

    pub fn scene_sound_request(
        &self,
        world: &World,
        scene: usize,
        actor_name: &str,
        activator: usize,
        cue: &str,
    ) -> Option<crate::sounds::SoundRequest> {
        let actor = self.scene_actor(world, scene, actor_name, activator)?;
        Some(self.actor_sound_request(world, actor, cue))
    }

    fn actor_sound_request(
        &self,
        world: &World,
        actor: usize,
        cue: &str,
    ) -> crate::sounds::SoundRequest {
        crate::sounds::SoundRequest {
            actor: Some(crate::sounds::SoundActor {
                name: self
                    .ent(world, actor)
                    .get("targetname")
                    .unwrap_or("")
                    .into(),
                model: world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(actor))
                    .map(|i| i.model.as_str())
                    .unwrap_or(self.ent(world, actor).get("model").unwrap_or(""))
                    .into(),
                entity: Some(actor),
            }),
            ..cue.into()
        }
    }

    /// The caption of SPEAK event `index` of `scene` spoken by `actor`: only its group's
    /// master captions, and only when the player or its view entity is within the caption
    /// radius of one of the sound's origins (the actor, and the speakers of env_microphones
    /// that hear it), unless the event suppresses that check.
    fn scene_caption(
        &mut self,
        world: &World,
        scene: &source_assets::scenes::ChoreoScene,
        index: usize,
        actor: usize,
        speaker: Option<crate::sounds::SoundActor>,
    ) {
        let Some((token, duration)) = crate::captions::scene_caption(scene, index) else {
            return;
        };
        if !crate::captions::suppresses_attenuation(scene, index) {
            let origins =
                crate::captions::sound_origins(self.states[actor].origin, &self.microphones(world));
            let view = self
                .view_control
                .active()
                .map(|camera| self.states[camera].origin);
            if crate::captions::attenuated(self.player_feet, &origins)
                && view.is_none_or(|view| crate::captions::attenuated(view, &origins))
            {
                self.diagnostics.attenuated_captions += 1;
                return;
            }
        }
        self.captions.push(crate::captions::CaptionRequest {
            token,
            duration,
            // Never "from player" for scenes.
            from_player: false,
            actor: speaker,
        });
    }

    /// Enabled env_microphones with a speaker, for caption sound origins.
    fn microphones(&self, world: &World) -> Vec<crate::captions::Microphone> {
        let named = |name: &str| {
            if name.eq_ignore_ascii_case("!player") {
                return Some(self.player_feet);
            }
            world
                .entities
                .iter()
                .position(|e| {
                    e.get("targetname")
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
                .filter(|&id| !self.states[id].killed)
                .map(|id| self.states[id].origin)
        };
        world
            .entities
            .iter()
            .enumerate()
            .filter(|(id, e)| {
                e.class() == "env_microphone"
                    && self.states[*id].enabled
                    && !self.states[*id].killed
            })
            .filter_map(|(id, e)| {
                let speaker = named(e.get("SpeakerName").filter(|s| !s.is_empty())?)?;
                let measure = e
                    .get("target")
                    .filter(|t| !t.is_empty())
                    .and_then(named)
                    .unwrap_or(self.states[id].origin);
                Some(crate::captions::Microphone {
                    measure,
                    max_range: number(e, "MaxRange", 0.),
                    speaker,
                })
            })
            .collect()
    }

    /// Precache SPEAK cues from every loaded scene without advancing playback.
    /// Unresolved dynamic actors retain a context-free request for broad preloading.
    pub fn required_sound_requests(&self, world: &World) -> Vec<crate::sounds::SoundRequest> {
        let mut result = Vec::new();
        for (id, scene) in &self.choreography {
            for event in &scene.data.events {
                if !event.active()
                    || event.kind != EventType::Speak
                    || event.parameters[0].is_empty()
                {
                    continue;
                }
                let request = event.actor.and_then(|actor| {
                    self.scene_sound_request(
                        world,
                        *id,
                        &scene.data.actors[actor].name,
                        usize::MAX,
                        &event.parameters[0],
                    )
                });
                result.push(request.unwrap_or_else(|| event.parameters[0].clone().into()));
            }
        }
        result.extend(self.commentary.required_sound_requests());
        result
    }

    /// Installed animation names required by loaded VCDs, grouped by model/skin.
    /// Gestures are requested for future layer support, but are not substituted for a body sequence.
    pub fn required_animation_clips(&self, world: &World) -> BTreeMap<String, BTreeSet<String>> {
        let mut result: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (id, scene) in &self.choreography {
            for event in &scene.data.events {
                if !event.active()
                    || !matches!(event.kind, EventType::Sequence | EventType::Gesture)
                    || event.name.eq_ignore_ascii_case("NULL")
                    || event.parameters[0].is_empty()
                {
                    continue;
                }
                let Some(actor) = event.actor else {
                    continue;
                };
                // Preparation runs at load, before point_template children (Barney, Kleiner)
                // are force-spawned, so include every entity the name can resolve to.
                for actor in self.scene_actor_candidates(world, *id, &scene.data.actors[actor].name)
                {
                    if let Some(instance) = world
                        .model_instances
                        .iter()
                        .find(|i| i.entity == Some(actor))
                    {
                        result
                            .entry(instance.asset_key())
                            .or_default()
                            .insert(event.parameters[0].to_lowercase());
                    }
                }
            }
        }
        result
    }

    /// Flex controller values a renderer applies: scene/expression controllers plus the
    /// visemes of any line the actor is speaking (client ProcessVisemes order).
    pub fn actor_flex_values(&self, id: usize) -> BTreeMap<String, f32> {
        let mut values = self.flex_controllers.get(&id).cloned().unwrap_or_default();
        for (name, add) in self.lipsync.visemes(id, self.time) {
            *values.entry(name).or_insert(0.) += add;
        }
        values
    }

    pub fn actor_matrices(&self, rig: &modkit_core::animation::Rig, id: usize) -> Vec<glam::Mat4> {
        if let Some(ragdoll) = self.ragdoll_matrices.get(&id) {
            return ragdoll.clone();
        }
        let Some(state) = self.states.get(id) else {
            return rig.matrices("", 0.);
        };
        let params = self.actor_pose_values(rig, id);
        self.gestures
            .compose(rig, id, &state.animation, self.animation_time(id), &params)
            .0
    }

    /// Normalized pose parameters: model defaults plus head control (head_pitch/yaw/roll)
    /// and the server-side body/head flex controllers (CAI_BaseActor UpdateHeadControl adds
    /// head_rightleft/updown/tilt to the look correction; UpdateBodyControl drives
    /// body_yaw, spine_yaw and neck_trans from body_rightleft, chest_rightleft and
    /// head_forwardback).
    pub fn actor_pose_values(&self, rig: &modkit_core::animation::Rig, id: usize) -> Vec<f32> {
        let mut params = rig.default_pose_values();
        let flexes = self.flex_controllers.get(&id);
        let flex = |name: &str| flexes.and_then(|v| v.get(name)).copied();
        let head = self.heads.get(&id).map(|h| h.goal);
        for (param, goal, controller) in [
            ("head_pitch", head.map(|g| g.x), "head_updown"),
            ("head_yaw", head.map(|g| g.y), "head_rightleft"),
            ("head_roll", head.map(|g| g.z), "head_tilt"),
            ("body_yaw", None, "body_rightleft"),
            ("spine_yaw", None, "chest_rightleft"),
            ("neck_trans", None, "head_forwardback"),
            // MaintainLookTargets: gesture blend position from the gesture flexes.
            ("gesture_height", None, "gesture_updown"),
            ("gesture_width", None, "gesture_rightleft"),
        ] {
            let offset = flex(controller);
            if goal.is_none() && offset.is_none() {
                continue;
            }
            if let Some(i) = rig.pose_parameter(param) {
                let value = goal.unwrap_or(0.) + offset.unwrap_or(0.);
                params[i] = rig.pose_parameters[i].normalize(value);
            }
        }
        let move_yaw = self
            .states
            .get(id)
            .and_then(|s| s.movement_animation.as_ref());
        if let (Some(m), Some(i)) = (move_yaw, rig.pose_parameter("move_yaw")) {
            params[i] = rig.pose_parameters[i].normalize(m.move_yaw);
        }
        params
    }

    /// Changes whenever an actor's composed pose inputs change; zero for plain base clips.
    pub fn pose_signature(&self, id: usize) -> u64 {
        if self.ragdoll_matrices.contains_key(&id) {
            return self.ragdoll_frame | 1 << 63;
        }
        let mut hash = self.gestures.signature(id);
        let mix = |hash: u64, v: f32| {
            (hash ^ v.to_bits() as u64)
                .wrapping_mul(0x100000001b3)
                .rotate_left(9)
        };
        if let Some(head) = self.heads.get(&id) {
            for v in head.goal.to_array() {
                hash = mix(hash, v);
            }
        }
        if let Some(m) = self
            .states
            .get(id)
            .and_then(|s| s.movement_animation.as_ref())
        {
            hash = mix(hash, m.move_yaw);
        }
        if let Some(values) = self.flex_controllers.get(&id) {
            for name in [
                "head_updown",
                "head_rightleft",
                "head_tilt",
                "body_rightleft",
                "chest_rightleft",
                "head_forwardback",
                "gesture_updown",
                "gesture_rightleft",
            ] {
                if let Some(v) = values.get(name) {
                    hash = mix(hash, *v);
                }
            }
        }
        hash
    }

    /// A look target's EyePosition: the player's eye, an NPC's animated "eyes"
    /// attachment (else origin + 64), another entity's origin, or the point itself.
    pub fn look_position(&self, world: &World, target: crate::attention::Target) -> Option<Vec3> {
        use crate::attention::Target;
        match target {
            Target::Player => Some(self.player_feet + Vec3::Z * 64.),
            Target::Position(p) => Some(p),
            Target::Entity(t) => {
                let state = self.states.get(t).filter(|s| !s.killed)?;
                if self.ent(world, t).class().starts_with("npc_") {
                    let eyes = self.attachment_frames(world, t, &["eyes"])[0];
                    Some(eyes.map_or(state.origin + Vec3::Z * 64., |m| m.w_axis.truncate()))
                } else {
                    Some(state.origin)
                }
            }
        }
    }

    /// MaintainLookTargets: blend this tick's interests into each NPC's head control.
    pub(super) fn update_heads(&mut self, world: &World, dt: f32) {
        let mut actors: BTreeSet<usize> = self.look_targets.actors().collect();
        actors.extend(self.heads.keys().copied());
        for actor in actors {
            let state = &self.states[actor];
            if state.killed || !self.ent(world, actor).class().starts_with("npc_") {
                self.heads.remove(&actor);
                continue;
            }
            // Outside the player's PVS the head control keeps its last values.
            if !self.look_targets.in_pvs(actor) {
                continue;
            }
            // CAI_BaseActor: eye position from the animated "eyes" attachment; self-interest
            // and the head frame follow the animated "forward" attachment.
            let frames = self.attachment_frames(world, actor, &["eyes", "forward"]);
            let state = &self.states[actor];
            let eye = frames[0].map_or(state.origin + Vec3::Z * 64., |m| m.w_axis.truncate());
            let frame = frames[1].map_or(glam::Mat3::from_quat(state.rotation), |m| {
                glam::Mat3::from_mat4(m)
            });
            let forward = frame.x_axis;
            // Active interests (random, then explicit) at their ramped Interest(), scaled by
            // HeadTargetValidity; targets are eye positions (CAI_InterestTarget_t::
            // GetPosition -> EyePosition).
            let now = self.time;
            let targets = self
                .look_targets
                .active(actor)
                .filter_map(|interest| {
                    let value = interest.interest(now);
                    let position = match interest.target {
                        crate::attention::Target::Entity(t) if t == actor => {
                            return Some((forward, value));
                        }
                        target => self.look_position(world, target)?,
                    };
                    let validity = crate::attention::head_target_validity(eye, forward, position);
                    Some((position - eye, value * validity))
                })
                .collect::<Vec<_>>();
            let head = self.heads.entry(actor).or_default();
            head.update(&targets, frame, dt);
            if targets.is_empty() && head.influence == 0. && head.goal.length() < 0.01 {
                self.heads.remove(&actor);
            }
        }
    }

    /// Sequence sampling uses the paused choreography clock, not wall/game time.
    pub fn animation_time(&self, id: usize) -> f32 {
        let Some(state) = self.states.get(id) else {
            return 0.;
        };
        if let Some(movement) = &state.movement_animation {
            return movement.elapsed;
        }
        let event = state.scene_animation.as_ref().and_then(|a| {
            let scene = self.choreography.get(&a.owner)?;
            let elapsed = scene.playback.as_ref()?.elapsed;
            Some((elapsed, scene.data.events[a.event].start))
        });
        if let Some((elapsed, start)) = event {
            return (elapsed - start as f64).max(0.) as f32;
        }
        (self.time - state.animation_started).max(0.) as f32
    }

    /// Read-only capture metadata; this neither advances nor resolves scene readiness.
    pub fn choreography_states(&self, world: &World) -> Vec<ChoreographyState> {
        self.choreography
            .iter()
            .map(|(id, scene)| {
                let play = scene.playback.as_ref();
                let pause_reason = play.and_then(|p| p.pause.as_ref()).map(|p| match p {
                    ScenePause::Input => "input",
                    ScenePause::Section { blocked: true, .. } => "actor_condition",
                    ScenePause::Section { blocked: false, .. } => "section",
                    ScenePause::UnsupportedControl => "unsupported_control",
                });
                ChoreographyState {
                    entity: *id,
                    targetname: self.ent(world, *id).get("targetname").unwrap_or("").into(),
                    file: self.ent(world, *id).get("SceneFile").unwrap_or("").into(),
                    elapsed: play.map(|p| p.elapsed),
                    pause: pause_reason.is_some(),
                    pause_reason,
                    completed_early: play.is_some_and(|p| p.completed_early),
                    active: play.is_some(),
                }
            })
            .collect()
    }

    /// Named supported actor samples, including whether an installed rig supplies the clip.
    pub fn animation_states(&self, world: &World) -> Vec<SceneAnimationState> {
        world
            .entities
            .iter()
            .enumerate()
            .filter_map(|(id, entity)| {
                if !scene_actor_class(entity.class()) {
                    return None;
                }
                let targetname = entity.get("targetname").filter(|n| !n.is_empty())?;
                let state = &self.states[id];
                if state.killed || state.animation.is_empty() {
                    return None;
                }
                let clip_loaded = world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(id))
                    .and_then(|i| world.rigs.get(&i.asset_key()))
                    .is_some_and(|rig| rig.clips.contains_key(&state.animation));
                Some(SceneAnimationState {
                    entity: id,
                    targetname: targetname.into(),
                    animation: state.animation.clone(),
                    sample_time: self.animation_time(id),
                    scene_owner: state.scene_animation.as_ref().map(|a| a.owner),
                    scripted_by: state.scripted_by,
                    clip_loaded,
                    origin: state.origin.to_array(),
                    yaw_degrees: {
                        let forward = state.rotation * Vec3::X;
                        forward.y.atan2(forward.x).to_degrees()
                    },
                })
            })
            .collect()
    }

    fn restore_scene_animation(&mut self, actor: usize) {
        let state = &mut self.states[actor];
        if let Some(animation) = state.scene_animation.take() {
            state.animation = animation.previous;
            state.animation_started = animation.previous_started;
            state.animation_done = animation.previous_done;
        }
    }

    fn clear_scene_animations(&mut self, owner: usize, canceled: bool) {
        self.gestures.release(owner, None, canceled);
        self.movement_commands
            .push(SceneMoveCommand::CancelScene(owner));
        self.movement_ready.retain(|key, _| key.scene != owner);
        for actor in 0..self.states.len() {
            if self.states[actor]
                .movement_animation
                .as_ref()
                .is_some_and(|m| m.key.scene == owner)
            {
                self.restore_movement_animation(actor);
            }
            if self.states[actor]
                .scene_animation
                .as_ref()
                .is_some_and(|s| s.owner == owner)
            {
                self.restore_scene_animation(actor);
            }
        }
    }

    pub(super) fn restore_movement_animation(&mut self, actor: usize) {
        let state = &mut self.states[actor];
        if let Some(previous) = state.movement_animation.take() {
            // A full-body sequence or scripted owner may have taken over since the last update.
            if state.animation == previous.sequence
                && state.scene_animation.is_none()
                && state.scripted_by.is_none()
            {
                state.animation = previous.previous;
                state.animation_started = previous.previous_started;
                state.animation_done = previous.previous_done;
            }
        }
    }

    pub fn apply_movement(
        &mut self,
        key: crate::npc::GoalKey,
        feet: Vec3,
        yaw_degrees: f32,
        animation: Option<(&str, f32)>,
        arrived: bool,
    ) {
        let Some(ready) = self.movement_ready.get_mut(&key) else {
            return;
        };
        let state = &self.states[key.actor];
        if state
            .movement_animation
            .as_ref()
            .is_some_and(|m| m.key != key)
        {
            *ready = arrived;
            return;
        }
        if state.killed
            || state.scripted_by.is_some()
            || state.scene_animation.is_some()
            || !feet.is_finite()
            || !yaw_degrees.is_finite()
        {
            *ready = false;
            self.restore_movement_animation(key.actor);
            return;
        }
        *ready = arrived;
        // CAI_Motor::MoveFacing/UpdateYaw: the body turns toward the path direction at
        // MaxYawSpeed x 10 degrees per second; move_yaw keeps the legs on the path.
        // AI goals belong to npc_metropolice (CNPC_MetroPolice::MaxYawSpeed: ACT_RUN 15,
        // ACT_WALK 25); scene moves keep the CAI_BaseNPC default 45.
        let yaw_speed = match animation.map(|(sequence, _)| sequence) {
            Some("run_all") if key.scene == crate::npc::AI_SCENE => 150.,
            Some("walk_all") if key.scene == crate::npc::AI_SCENE => 250.,
            _ => 450.,
        };
        let forward = self.states[key.actor].rotation * Vec3::X;
        let current = forward.y.atan2(forward.x).to_degrees();
        // CAI_BaseNPC::OverrideMoveFacing: an active facing target (move and shoot) turns
        // the body toward it instead of the path.
        let facing = self.states[key.actor]
            .ai_move_facing
            .filter(|(_, until)| self.time < *until && key.scene == crate::npc::AI_SCENE)
            .map(|(point, _)| {
                let d = point - feet;
                d.y.atan2(d.x).to_degrees()
            });
        let body = if animation.is_some() {
            clamp_yaw(
                yaw_speed * self.tick_dt,
                current,
                facing.unwrap_or(yaw_degrees),
            )
        } else {
            yaw_degrees
        };
        let move_yaw = angle_diff(yaw_degrees, body);
        self.states[key.actor].origin = feet;
        self.states[key.actor].rotation = physics::angles(Vec3::new(0., body, 0.));
        if let Some((sequence, elapsed)) = animation {
            if self.states[key.actor]
                .movement_animation
                .as_ref()
                .is_none_or(|m| m.key != key || m.sequence != sequence)
            {
                self.restore_movement_animation(key.actor);
                let state = &mut self.states[key.actor];
                state.movement_animation = Some(MovementAnimation {
                    key,
                    sequence: sequence.into(),
                    elapsed,
                    move_yaw,
                    previous: state.animation.clone(),
                    previous_started: state.animation_started,
                    previous_done: state.animation_done,
                });
                state.animation = sequence.into();
                state.animation_done = None;
            } else if let Some(m) = &mut self.states[key.actor].movement_animation {
                m.elapsed = elapsed;
                m.move_yaw = move_yaw;
            }
        } else {
            self.restore_movement_animation(key.actor);
        }
    }

    fn section_blocked(&self, id: usize, scene: &ChoreoScene, play: &Playback, at: f64) -> bool {
        scene.events.iter().enumerate().any(|(event, e)| {
            if !e.active() || !e.resume_condition() || e.start as f64 > at {
                return false;
            }
            let Some(actor) = e.actor.and_then(|a| play.actors[a]) else {
                return false;
            };
            e.kind != EventType::MoveTo
                || !self
                    .movement_ready
                    .get(&crate::npc::GoalKey {
                        scene: id,
                        event,
                        actor,
                    })
                    .copied()
                    .unwrap_or(false)
        })
    }

    pub(super) fn start_choreography(&mut self, world: &World, id: usize, activator: usize) {
        let Some(mut scene) = self.choreography.remove(&id) else {
            self.unsupported_input("logic_choreographed_scene", "Start:scene-not-loaded");
            return;
        };
        if scene.playback.is_none() {
            let actors = scene
                .data
                .actors
                .iter()
                .map(|a| self.scene_actor(world, id, &a.name, activator))
                .collect();
            scene.playback = Some(Playback {
                elapsed: 0.,
                next: 0,
                completed_early: false,
                pause: None,
                actors,
                look_targets: BTreeMap::new(),
                face_targets: BTreeMap::new(),
                face_initial: BTreeMap::new(),
            });
            self.fire(id, "OnStart", id);
        }
        self.choreography.insert(id, scene);
    }

    /// ProcessGestureSceneEvent for started events inside their window; RemoveLayer
    /// after the end. Absent or empty gesture names produce no layer (LookupSequence fails).
    fn process_gestures(&mut self, id: usize, scene: &Choreography, play: &Playback) {
        for event in self.gestures.active_events(id) {
            let e = &scene.data.events[event];
            if play.elapsed > e.end.unwrap_or(e.start) as f64 {
                self.gestures.release(id, Some(event), false);
            }
        }
        for &index in scene.order.iter().take(play.next) {
            let event = &scene.data.events[index];
            let Some(end) = event.end else {
                continue;
            };
            let duration = end - event.start;
            if event.kind != EventType::Gesture
                || !event.active()
                || event.parameters[0].is_empty()
                || duration <= 0.
                || play.elapsed < event.start as f64
                || play.elapsed > end as f64
            {
                continue;
            }
            let Some(actor) = event.actor.and_then(|a| play.actors[a]) else {
                continue;
            };
            if self.states[actor].killed {
                continue;
            }
            self.gestures.update(crate::gestures::GestureUpdate {
                actor,
                scene: id,
                event: index,
                data: &scene.data,
                playback: (play.elapsed as f32 - event.start) / duration,
                weight: event.intensity(&scene.data, play.elapsed as f32),
                priority: event.channel.unwrap_or(0),
                moving: self.states[actor].movement_animation.is_some(),
            });
        }
    }

    /// ProcessFacingSceneEvent for a standing NPC: ideal yaw blends from the yaw latched at
    /// the first process toward the target by event intensity, and the motor turns at most
    /// MaxYawSpeed (45 per 0.1 s) toward it. Facing while moving (AddFacingTarget) is not
    /// implemented; a moving actor only re-latches its initial yaw.
    fn process_faces(&mut self, _id: usize, scene: &Choreography, play: &mut Playback, dt: f32) {
        for &index in scene.order.iter().take(play.next) {
            let event = &scene.data.events[index];
            if event.kind != EventType::Face
                || !event.active()
                || play.elapsed < event.start as f64
                || play.elapsed > event.end.unwrap_or(event.start) as f64
            {
                continue;
            }
            let (Some(target), Some(actor)) = (
                play.face_targets.get(&index).copied(),
                event.actor.and_then(|a| play.actors[a]),
            ) else {
                continue;
            };
            if self.states[actor].killed {
                continue;
            }
            let target_position = match target {
                crate::attention::Target::Player => self.player_feet,
                crate::attention::Target::Entity(t) if t == actor => continue,
                crate::attention::Target::Entity(t) => self.states[t].origin,
                crate::attention::Target::Position(p) => p,
            };
            let forward = self.states[actor].rotation * Vec3::X;
            let current = forward.y.atan2(forward.x).to_degrees();
            let moving = self.states[actor].movement_animation.is_some();
            let latched = play.face_initial.entry(index).or_insert((current, moving));
            if latched.1 != moving {
                *latched = (current, moving);
            }
            if moving {
                continue;
            }
            let delta = target_position - self.states[actor].origin;
            if delta.truncate().length_squared() < 1e-6 {
                continue;
            }
            let goal = delta.y.atan2(delta.x).to_degrees();
            let initial = latched.0;
            let intensity = event.intensity(&scene.data, play.elapsed as f32);
            let ideal = initial + angle_diff(goal, initial) * intensity;
            let yaw = clamp_yaw(450. * dt.min(0.2), current, ideal);
            self.states[actor].rotation = physics::angles(Vec3::new(0., yaw, 0.));
        }
    }

    /// CBaseFlex::AddFlexAnimation: each active track blends its intensity into the actor's
    /// controller value by event intensity; stereo tracks drive right_/left_ controllers.
    fn process_flex_animation(&mut self, scene: &Choreography, play: &Playback) {
        let time = play.elapsed as f32;
        for &index in scene.order.iter().take(play.next) {
            let event = &scene.data.events[index];
            if event.kind != EventType::FlexAnimation
                || !event.active()
                || play.elapsed < event.start as f64
                || play.elapsed > event.end.unwrap_or(event.start) as f64
            {
                continue;
            }
            let Some(actor) = event.actor.and_then(|a| play.actors[a]) else {
                continue;
            };
            if self.states[actor].killed {
                continue;
            }
            let weight = event.intensity(&scene.data, time);
            let values = self.flex_controllers.entry(actor).or_default();
            for track in event.flex_tracks.iter().filter(|t| t.is_active()) {
                let name = track.controller.to_lowercase();
                let sides: &[(usize, String)] = &if track.is_combo() {
                    vec![(0, format!("right_{name}")), (1, format!("left_{name}"))]
                } else {
                    vec![(0, name)]
                };
                for (side, controller) in sides {
                    let value = track.intensity(time, event.start, event.end, *side);
                    let current = values.entry(controller.clone()).or_insert(0.);
                    *current = *current * (1. - weight) + value * weight;
                }
            }
        }
    }

    pub(super) fn cancel_choreography(&mut self, id: usize) {
        if self
            .choreography
            .get_mut(&id)
            .is_some_and(|s| s.playback.take().is_some())
        {
            self.clear_scene_animations(id, true);
            self.fire(id, "OnCanceled", id);
        }
    }

    pub(super) fn tick_choreography(&mut self, world: &World, dt: f32) {
        let ids: Vec<_> = self.choreography.keys().copied().collect();
        for id in ids {
            let mut scene = self.choreography.remove(&id).unwrap();
            if self.states[id].killed {
                scene.playback = None;
                self.clear_scene_animations(id, true);
            }
            if let Some(mut play) = scene.playback.take() {
                let mut canceled = false;
                let current_blocked = self.section_blocked(id, &scene.data, &play, play.elapsed);
                if let Some(ScenePause::Section { blocked, .. }) = &mut play.pause {
                    *blocked = current_blocked;
                }
                if let Some(ScenePause::Section { blocked, automated }) = &play.pause {
                    if let Some((resume, due)) = automated {
                        if self.time >= *due {
                            canceled = !resume;
                            play.pause = None;
                        }
                    } else if !blocked {
                        play.pause = None;
                    }
                }
                if canceled {
                    self.clear_scene_animations(id, true);
                    self.fire(id, "OnCanceled", id);
                } else {
                    if play.pause.is_none() {
                        let end = play.elapsed + dt as f64;
                        while let Some(index) = scene.order.get(play.next).copied() {
                            let event = &scene.data.events[index];
                            if event.start as f64 > end {
                                break;
                            }
                            play.next += 1;
                            if !event.active() || event.name.eq_ignore_ascii_case("NULL") {
                                continue;
                            }
                            let actor = event.actor.and_then(|actor| play.actors[actor]);
                            if event.actor.is_some() && actor.is_none() {
                                self.unsupported_input(
                                    "logic_choreographed_scene",
                                    "missing-actor",
                                );
                                continue;
                            }
                            self.diagnostics.scene_events_started += 1;
                            match event.kind {
                                EventType::FireTrigger => {
                                    if let Ok(trigger) = event.parameters[0].parse::<u8>() {
                                        if (1..=16).contains(&trigger) {
                                            self.fire(
                                                id,
                                                &format!("OnTrigger{trigger}"),
                                                actor.unwrap_or(id),
                                            );
                                        }
                                    }
                                }
                                EventType::StopPoint => {
                                    if !play.completed_early {
                                        play.completed_early = true;
                                        self.diagnostics.scene_completions += 1;
                                        self.fire(id, "OnCompletion", id);
                                    }
                                }
                                EventType::Speak => {
                                    if let Some(actor) =
                                        actor.filter(|_| !event.parameters[0].is_empty())
                                    {
                                        if let Some(mut request) = self.scene_sound_request(
                                            world,
                                            id,
                                            &scene.data.actors[event.actor.unwrap()].name,
                                            actor,
                                            &event.parameters[0],
                                        ) {
                                            // Scenes marked ignorePhonemes play without
                                            // lip sync (CVoiceData::ShouldIgnorePhonemes).
                                            if scene.data.ignore_phonemes {
                                                if let Some(a) = request.actor.as_mut() {
                                                    a.entity = None;
                                                }
                                            }
                                            // The scene captions its master token itself
                                            // (CSceneEntity::DispatchStartSpeak).
                                            request.caption = false;
                                            self.scene_caption(
                                                world,
                                                &scene.data,
                                                index,
                                                actor,
                                                request.actor.clone(),
                                            );
                                            self.sounds.push(request);
                                        }
                                    }
                                }
                                EventType::Gesture => {
                                    // Layers start and update in process_gestures each tick.
                                }
                                EventType::FlexAnimation => {
                                    // Tracks apply in process_flex_animation each tick.
                                }
                                EventType::Face => {
                                    // StartFacingSceneEvent needs a target; NPCs turn in process_faces.
                                    if let Some(actor) = actor
                                        .filter(|a| self.ent(world, *a).class().starts_with("npc_"))
                                    {
                                        if let Some(target) = self.scene_look_target(
                                            world,
                                            id,
                                            actor,
                                            &event.parameters[0],
                                        ) {
                                            play.face_targets.insert(index, target);
                                        } else {
                                            self.unsupported_input(
                                                "logic_choreographed_scene",
                                                "FACE:missing-target",
                                            );
                                        }
                                    }
                                }
                                EventType::LookAt => {
                                    // CBaseFlex LOOKAT refreshes NPC interests; non-NPC flex actors are a no-op.
                                    if let Some(actor) = actor
                                        .filter(|a| self.ent(world, *a).class().starts_with("npc_"))
                                    {
                                        if let Some(target) = self.scene_look_target(
                                            world,
                                            id,
                                            actor,
                                            &event.parameters[0],
                                        ) {
                                            play.look_targets.insert(index, target);
                                        } else {
                                            self.unsupported_input(
                                                "logic_choreographed_scene",
                                                "LOOKAT:missing-target",
                                            );
                                        }
                                    }
                                }
                                EventType::MoveTo => {
                                    if let Some(actor) = actor {
                                        let key = crate::npc::GoalKey {
                                            scene: id,
                                            event: index,
                                            actor,
                                        };
                                        let target = world
                                            .entities
                                            .iter()
                                            .enumerate()
                                            .find(|(target, e)| {
                                                !self.states[*target].killed
                                                    && e.get("targetname").is_some_and(|name| {
                                                        name.eq_ignore_ascii_case(
                                                            &event.parameters[0],
                                                        )
                                                    })
                                            })
                                            .map(|(id, _)| id);
                                        match (
                                            target,
                                            crate::npc::MoveStyle::parse(&event.parameters[1]),
                                        ) {
                                            (Some(target), Ok(style)) => {
                                                self.movement_ready.insert(key, false);
                                                self.movement_commands.push(
                                                    SceneMoveCommand::Start {
                                                        key,
                                                        request: crate::npc::MoveRequest {
                                                            target_entity: target,
                                                            target_feet: self.states[target].origin,
                                                            style,
                                                            event_distance: event.distance,
                                                            force_short: event.flags & 16 != 0,
                                                        },
                                                    },
                                                );
                                            }
                                            _ => self.unsupported_input(
                                                "logic_choreographed_scene",
                                                "MOVETO:target-or-style",
                                            ),
                                        }
                                    }
                                }
                                EventType::Sequence => {
                                    if let Some(actor) = actor {
                                        if self.states[actor].scripted_by.is_some()
                                            && event.flags & 32 == 0
                                        {
                                            self.unsupported_input(
                                                "logic_choreographed_scene",
                                                "SEQUENCE:scripted-actor-busy",
                                            );
                                        } else {
                                            self.restore_movement_animation(actor);
                                            self.restore_scene_animation(actor);
                                            let previous = SceneAnimation {
                                                owner: id,
                                                event: index,
                                                previous: self.states[actor].animation.clone(),
                                                previous_started: self.states[actor]
                                                    .animation_started,
                                                previous_done: self.states[actor].animation_done,
                                            };
                                            if self.animate(
                                                world,
                                                actor,
                                                &event.parameters[0],
                                                false,
                                            ) {
                                                self.states[actor].scene_animation = Some(previous);
                                            }
                                        }
                                    }
                                }
                                EventType::Section => {
                                    let blocked = self.section_blocked(
                                        id,
                                        &scene.data,
                                        &play,
                                        event.start as f64,
                                    );
                                    let tokens: Vec<_> =
                                        event.parameters[0].split_whitespace().collect();
                                    let automated = if tokens.len() == 3
                                        && tokens[0].eq_ignore_ascii_case("automate")
                                    {
                                        tokens[2]
                                            .parse::<f64>()
                                            .ok()
                                            .filter(|d| d.is_finite() && *d > 0.)
                                            .and_then(|d| {
                                                if tokens[1].eq_ignore_ascii_case("resume") {
                                                    Some((true, self.time + d))
                                                } else if tokens[1].eq_ignore_ascii_case("cancel") {
                                                    Some((false, self.time + d))
                                                } else {
                                                    None
                                                }
                                            })
                                    } else {
                                        None
                                    };
                                    let unknown_condition = scene
                                        .data
                                        .events
                                        .iter()
                                        .enumerate()
                                        .any(|(event_index, e)| {
                                            e.active()
                                                && e.resume_condition()
                                                && e.start <= event.start
                                                && e.actor.and_then(|a| play.actors[a]).is_some_and(
                                                    |actor| {
                                                        e.kind != EventType::MoveTo
                                                            || !self.movement_ready.contains_key(
                                                                &crate::npc::GoalKey {
                                                                    scene: id,
                                                                    event: event_index,
                                                                    actor,
                                                                },
                                                            )
                                                    },
                                                )
                                        });
                                    if blocked && automated.is_none() && unknown_condition {
                                        self.unsupported_input(
                                            "logic_choreographed_scene",
                                            "SECTION:actor-resume-condition",
                                        );
                                    }
                                    play.pause = Some(ScenePause::Section { blocked, automated });
                                }
                                EventType::Loop | EventType::SubScene => {
                                    self.unsupported_input(
                                        "logic_choreographed_scene",
                                        &format!("control:{:?}", event.kind),
                                    );
                                    play.pause = Some(ScenePause::UnsupportedControl);
                                }
                                EventType::Interrupt
                                | EventType::PermitResponses
                                | EventType::Unspecified => {
                                    self.unsupported_input(
                                        "logic_choreographed_scene",
                                        &format!("event:{:?}", event.kind),
                                    );
                                }
                                _ => self.unsupported_input(
                                    "logic_choreographed_scene",
                                    &format!("actor-event:{:?}", event.kind),
                                ),
                            }
                            if play.pause.is_some() {
                                play.elapsed = event.start as f64;
                                break;
                            }
                        }
                        if play.pause.is_none() {
                            play.elapsed = end;
                        }
                    }
                    // Preserve start-dispatch order for overlapping events. Pause freezes scene
                    // elapsed time while the global actor interest lifetime continues to refresh.
                    for &index in scene.order.iter().take(play.next) {
                        let event = &scene.data.events[index];
                        if event.kind != EventType::LookAt
                            || !event.active()
                            || play.elapsed < event.start as f64
                            || play.elapsed > event.end.unwrap_or(event.start) as f64
                        {
                            continue;
                        }
                        let Some(target) = play.look_targets.get(&index).copied() else {
                            continue;
                        };
                        let Some(actor) = event.actor.and_then(|a| play.actors[a]) else {
                            continue;
                        };
                        if self.states[actor].killed
                            || matches!(target, crate::attention::Target::Entity(t) if self.states[t].killed)
                        {
                            continue;
                        }
                        let importance = crate::attention::scene_importance(
                            event.intensity(&scene.data, play.elapsed as f32),
                            play.elapsed as f32 - event.start,
                        );
                        self.look_targets
                            .refresh(actor, target, importance, self.time, id, index);
                    }
                    if play.pause.is_none() {
                        self.process_gestures(id, &scene, &play);
                        self.process_faces(id, &scene, &mut play, dt);
                        self.process_flex_animation(&scene, &play);
                    }
                    if play.pause.is_none()
                        && play.next == scene.order.len()
                        && play.elapsed > scene.data.stop_time() as f64
                    {
                        if !play.completed_early {
                            self.diagnostics.scene_completions += 1;
                            self.fire(id, "OnCompletion", id);
                        }
                        self.clear_scene_animations(id, false);
                    } else {
                        for actor in 0..self.states.len() {
                            if self.states[actor]
                                .scene_animation
                                .as_ref()
                                .is_some_and(|a| {
                                    a.owner == id
                                        && play.elapsed
                                            > scene.data.events[a.event]
                                                .end
                                                .unwrap_or(scene.data.events[a.event].start)
                                                as f64
                                })
                            {
                                self.restore_scene_animation(actor);
                            }
                        }
                        scene.playback = Some(play);
                    }
                }
            }
            self.choreography.insert(id, scene);
        }
    }
}
