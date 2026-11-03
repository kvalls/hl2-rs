//! Developer commentary system (`maps/<map>_commentary.txt`, `point_commentary_node`,
//! `point_commentary_viewpoint`, `commentary_auto`, and `commentary_semaphore`).

use crate::{
    captions::CaptionRequest,
    entities::Scene,
    player_damage::{ScreenFade, FFADE_IN},
    sounds::SoundRequest,
};
use glam::Vec3;
use modkit_core::{parse_vec3, Entity, ModelInstance, World};
use source_assets::{
    keyvalues::{self, decode_text, Entry},
    vpk::Vfs,
};

pub const SPEECH_MODEL: &str = "models/extras/info_speech.mdl";
pub const SPEECH_MAT_UNUSED: &str = "models/extras/speech_info";
pub const SPEECH_MAT_USED: &str = "models/extras/speech_info_used";
pub const VIEWPOINT_SPRITE: &str = "sprites/redglow1";
const NODE_HALF_EXTENT: f32 = 16.0;
const NODE_MAX_RANGE: f32 = 1024.0;
const DEFAULT_DURATION: f32 = 10.0;
const CONTENTS_SOLID_WINDOW: u32 = 0x1 | 0x2;

/// Active commentary HUD state for `HudCommentary`.
#[derive(Clone, Debug, PartialEq)]
pub struct CommentaryOverlay {
    pub start_time: f32,
    pub end_time: f32,
    pub node_number: usize,
    pub node_number_max: usize,
    pub speakers: String,
}

/// Active camera/movement override while a commentary node directs the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommentaryViewOverride {
    pub origin: Vec3,
    pub angles: [f32; 3],
    pub lock_movement: bool,
}

#[derive(Clone, Debug)]
pub struct CommentaryNode {
    pub entity_id: usize,
    pub node_number: usize,
    pub origin: Vec3,
    pub angles: [f32; 3],
    pub teleport_origin: Option<Vec3>,
    pub speakers: String,
    pub commentaryfile: String,
    pub commentaryfile_nohdr: String,
    pub duration: f32,
    pub duration_nohdr: f32,
    pub precommands: String,
    pub postcommands: String,
    pub viewposition: String,
    pub viewtarget: String,
    pub prevent_movement: bool,
    pub start_disabled: bool,
    pub enabled: bool,
    pub listened: bool,
    pub active: bool,
    pub unstoppable: bool,
    pub start_time: f32,
    pub active_duration: f32,
    pub active_cue: String,
}

#[derive(Clone, Debug, Default)]
pub struct CommentaryState {
    pub enabled: bool,
    pub hdr: bool,
    pub nodes: Vec<CommentaryNode>,
    pub active_node: Option<usize>,
    pub hovered_node: Option<usize>,
    pub current_jump_index: Option<usize>,
    pub view_override: Option<CommentaryViewOverride>,
    pub active_viewpoint_entity: Option<usize>,
    pub blend_start_origin: Vec3,
    pub blend_start_angles: [f32; 3],
    pub blend_initialized: bool,
    pub pending_commands: Vec<String>,
    pub pending_teleport: Option<(Vec3, [f32; 3])>,
}

/// Strips leading Source sound characters (`*?!#@><^)}`) and normalizes path separators.
pub fn clean_sound_cue(raw: &str) -> String {
    raw.trim()
        .trim_start_matches(['*', '?', '!', '#', '>', '<', '^', '@', ')', '}'])
        .replace('\\', "/")
}

fn unescape_caret(text: &str) -> String {
    text.replace('^', "\"")
}

fn entity_from_entry(entry: &Entry) -> Option<Entity> {
    let mut properties = Vec::new();
    for child in entry.children() {
        if child.key.eq_ignore_ascii_case("connections") {
            for conn in child.children() {
                if let Some(val) = conn.text() {
                    properties.push((conn.key.clone(), unescape_caret(val)));
                }
            }
        } else if let Some(val) = child.text() {
            properties.push((child.key.clone(), unescape_caret(val)));
        }
    }
    let class = properties
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("classname"))
        .map(|(_, v)| v.as_str())?;
    if class.eq_ignore_ascii_case("point_commentary_node") {
        if !properties
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("model"))
        {
            properties.push(("model".into(), SPEECH_MODEL.into()));
        }
        properties.push(("solid".into(), "0".into()));
    }
    Some(Entity { properties })
}

/// Parses `maps/<map>_commentary.txt` into map entities plus the `commentary_semaphore` target.
pub fn parse_commentary_entities(text: &str) -> Vec<Entity> {
    let Ok(roots) = keyvalues::parse(text) else {
        return Vec::new();
    };
    let mut entities = Vec::new();
    for root in &roots {
        for child in root.children() {
            if child.key.eq_ignore_ascii_case("entity") {
                if let Some(entity) = entity_from_entry(child) {
                    entities.push(entity);
                }
            }
        }
    }
    if !entities.is_empty()
        && !entities.iter().any(|e| {
            e.get("targetname")
                .is_some_and(|n| n.eq_ignore_ascii_case("commentary_semaphore"))
        })
    {
        entities.push(Entity {
            properties: vec![
                ("classname".into(), "info_target".into()),
                ("targetname".into(), "commentary_semaphore".into()),
            ],
        });
    }
    entities
}

fn wav_duration_from_vfs(vfs: &Vfs, cue: &str) -> Option<f32> {
    let clean = clean_sound_cue(cue);
    if clean.is_empty() {
        return None;
    }
    let path = format!("sound/{clean}");
    let bytes = vfs.read(&path).ok()??;
    let (_, summary) = crate::sounds::decode(&path, bytes).ok()?;
    let secs = summary.seconds();
    (secs > 0.0).then_some(secs as f32)
}

/// Appends `maps/<map>_commentary.txt` entities and balloon models to `world` on the map worker.
pub fn append_world_commentary(world: &mut World, vfs: &Vfs, map: &str) -> usize {
    let path = format!("maps/{map}_commentary.txt");
    let Some(bytes) = vfs.read(&path).ok().flatten() else {
        return 0;
    };
    let Ok(text) = decode_text(&bytes) else {
        return 0;
    };
    let mut entities = parse_commentary_entities(&text);
    if entities.is_empty() {
        return 0;
    }
    let mut added_nodes = 0;
    for mut entity in entities.drain(..) {
        if entity.class() == "point_commentary_node" {
            if let Some(cue) = entity.get("commentaryfile").map(str::to_owned) {
                if let Some(dur) = wav_duration_from_vfs(vfs, &cue) {
                    entity
                        .properties
                        .push(("_commentary_duration".into(), format!("{dur:.3}")));
                }
            }
            if let Some(cue) = entity.get("commentaryfile_nohdr").map(str::to_owned) {
                if let Some(dur) = wav_duration_from_vfs(vfs, &cue) {
                    entity
                        .properties
                        .push(("_commentary_duration_nohdr".into(), format!("{dur:.3}")));
                }
            }
        }
        let entity_id = world.entities.len();
        if entity.class() == "point_commentary_node" {
            added_nodes += 1;
            world.model_instances.push(ModelInstance {
                background: false,
                model: SPEECH_MODEL.into(),
                origin: entity.origin(),
                angles: parse_vec3(entity.get("angles").unwrap_or("0 0 0")).unwrap_or(Vec3::ZERO),
                skin: 0,
                body: 0,
                scale: 1.0,
                kind: "point_commentary_node".into(),
                solid: false,
                solid_mode: None,
                entity: Some(entity_id),
            });
        }
        world.entities.push(entity);
    }
    added_nodes
}

fn find_entity_by_name(world: &World, name: &str) -> Option<usize> {
    (!name.is_empty()).then(|| {
        world.entities.iter().position(|e| {
            e.get("targetname")
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
    })?
}

impl CommentaryState {
    pub fn spawn_init(scene: &mut Scene, world: &World, enabled: bool, hdr: bool, new_game: bool) {
        scene.commentary = Self {
            enabled,
            hdr,
            ..Default::default()
        };
        let mut node_number = 0;
        let mut auto_entities = Vec::new();
        for (id, entity) in world.entities.iter().enumerate() {
            match entity.class() {
                "point_commentary_viewpoint" => {
                    if let Some(state) = scene.states.get_mut(id) {
                        state.visible = false;
                    }
                }
                "commentary_auto" if enabled => auto_entities.push(id),
                "point_commentary_node" => {
                    let start_disabled = entity.get("start_disabled").unwrap_or("0") != "0";
                    if let Some(state) = scene.states.get_mut(id) {
                        state.visible = enabled && !start_disabled;
                        state.animation = "idle".into();
                        state.animation_started = scene.time;
                    }
                    if !enabled {
                        continue;
                    }
                    node_number += 1;
                    scene.commentary.nodes.push(Self::node_from_entity(
                        id,
                        node_number,
                        entity,
                        start_disabled,
                    ));
                }
                _ => {}
            }
        }
        if enabled {
            for id in auto_entities {
                if new_game {
                    scene.fire(id, "OnCommentaryNewGame", usize::MAX);
                }
                scene.fire(id, "OnCommentaryMultiplayerSpawn", usize::MAX);
            }
        }
    }

    fn node_from_entity(
        entity_id: usize,
        node_number: usize,
        entity: &Entity,
        start_disabled: bool,
    ) -> CommentaryNode {
        let angles_vec = parse_vec3(entity.get("angles").unwrap_or("0 0 0")).unwrap_or(Vec3::ZERO);
        let duration = entity
            .get("_commentary_duration")
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(DEFAULT_DURATION);
        let duration_nohdr = entity
            .get("_commentary_duration_nohdr")
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(duration);
        CommentaryNode {
            entity_id,
            node_number,
            origin: entity.origin(),
            angles: [angles_vec.x, angles_vec.y, angles_vec.z],
            teleport_origin: entity
                .get("teleport_origin")
                .and_then(parse_vec3)
                .filter(|v| *v != Vec3::ZERO),
            speakers: entity.get("speakers").unwrap_or_default().to_owned(),
            commentaryfile: entity.get("commentaryfile").unwrap_or_default().to_owned(),
            commentaryfile_nohdr: entity
                .get("commentaryfile_nohdr")
                .unwrap_or_default()
                .to_owned(),
            duration,
            duration_nohdr,
            precommands: entity.get("precommands").unwrap_or_default().to_owned(),
            postcommands: entity.get("postcommands").unwrap_or_default().to_owned(),
            viewposition: entity.get("viewposition").unwrap_or_default().to_owned(),
            viewtarget: entity.get("viewtarget").unwrap_or_default().to_owned(),
            prevent_movement: entity.get("prevent_movement").unwrap_or("0") != "0",
            start_disabled,
            enabled: !start_disabled,
            listened: false,
            active: false,
            unstoppable: false,
            start_time: 0.0,
            active_duration: duration,
            active_cue: String::new(),
        }
    }

    /// Skin index for a `point_commentary_node` (`0` unused/active, `1` already listened).
    pub fn node_skin(&self, entity_id: usize) -> i32 {
        self.nodes
            .iter()
            .find(|n| n.entity_id == entity_id)
            .map_or(0, |n| i32::from(n.listened && !n.active))
    }

    /// Sound requests to preload when commentary mode is active.
    pub fn required_sound_requests(&self) -> Vec<SoundRequest> {
        if !self.enabled {
            return Vec::new();
        }
        let mut out = Vec::new();
        for node in &self.nodes {
            for raw in [&node.commentaryfile, &node.commentaryfile_nohdr] {
                let clean = clean_sound_cue(raw);
                if !clean.is_empty() {
                    out.push(clean.as_str().into());
                }
            }
        }
        out
    }

    pub fn active_overlay(&self) -> Option<CommentaryOverlay> {
        let idx = self.active_node?;
        let node = self.nodes.get(idx)?;
        Some(CommentaryOverlay {
            start_time: node.start_time,
            end_time: node.start_time + node.active_duration.max(0.1),
            node_number: node.node_number,
            node_number_max: self.nodes.len(),
            speakers: node.speakers.clone(),
        })
    }

    /// Whether the active commentary node locks player walking/jumping (`prevent_movement`,
    /// `viewposition`, or `viewtarget`).
    pub fn movement_locked(&self) -> bool {
        self.view_override.is_some()
            || self
                .active_node
                .and_then(|idx| self.nodes.get(idx))
                .is_some_and(|n| n.prevent_movement)
    }

    /// Camera override when the active commentary node has a `viewposition` or `viewtarget`.
    pub fn view_control_override(&self) -> Option<crate::view_control::View> {
        let v = self.view_override?;
        Some(crate::view_control::View {
            origin: v.origin,
            pitch: v.angles[0],
            yaw: v.angles[1],
            freeze_player: v.lock_movement,
        })
    }

    pub fn handle_input(scene: &mut Scene, world: &World, target: usize, input: &str) -> bool {
        let Some(idx) = scene
            .commentary
            .nodes
            .iter()
            .position(|n| n.entity_id == target)
        else {
            return false;
        };
        match input {
            "startcommentary" => Self::start_node(scene, world, idx, false),
            "startunstoppablecommentary" => Self::start_node(scene, world, idx, true),
            "stopcommentary" => {
                if scene.commentary.active_node == Some(idx) {
                    Self::stop_node(scene, world, idx);
                }
            }
            "enable" => {
                scene.commentary.nodes[idx].enabled = true;
                if let Some(state) = scene.states.get_mut(target) {
                    state.visible = true;
                }
            }
            "disable" => {
                scene.commentary.nodes[idx].enabled = false;
                if let Some(state) = scene.states.get_mut(target) {
                    state.visible = false;
                }
                if scene.commentary.active_node == Some(idx) {
                    Self::stop_node(scene, world, idx);
                }
            }
            "toggle" => {
                let enabled = !scene.commentary.nodes[idx].enabled;
                scene.commentary.nodes[idx].enabled = enabled;
                if let Some(state) = scene.states.get_mut(target) {
                    state.visible = enabled;
                }
            }
            _ => return false,
        }
        true
    }

    fn queue_commands(scene: &mut Scene, commands: &str) {
        for raw in commands.split(';') {
            let cmd = raw.trim();
            if !cmd.is_empty() {
                scene.commentary.pending_commands.push(cmd.to_owned());
            }
        }
    }

    pub fn start_node(scene: &mut Scene, world: &World, idx: usize, unstoppable: bool) {
        if let Some(prev) = scene.commentary.active_node {
            Self::stop_node(scene, world, prev);
        }
        let Some(node) = scene.commentary.nodes.get(idx).cloned() else {
            return;
        };
        if !node.enabled {
            return;
        }
        let use_nohdr = !scene.commentary.hdr && !node.commentaryfile_nohdr.is_empty();
        let raw_cue = if use_nohdr {
            node.commentaryfile_nohdr.clone()
        } else {
            node.commentaryfile.clone()
        };
        let duration = if use_nohdr {
            node.duration_nohdr
        } else {
            node.duration
        };
        let clean_cue = clean_sound_cue(&raw_cue);
        let now = scene.time as f32;
        {
            let n = &mut scene.commentary.nodes[idx];
            n.active = true;
            n.listened = true;
            n.unstoppable = unstoppable;
            n.start_time = now;
            n.active_duration = duration;
            n.active_cue = clean_cue.clone();
        }
        scene.commentary.active_node = Some(idx);
        scene.commentary.blend_initialized = false;
        Self::queue_commands(scene, &node.precommands);
        scene.fire(node.entity_id, "OnCommentaryStarted", usize::MAX);
        if !clean_cue.is_empty() {
            scene.sounds.push(SoundRequest {
                origin: None,
                emitter: Some(node.entity_id as u64),
                caption: false,
                ..clean_cue.as_str().into()
            });
            scene.captions.push(CaptionRequest {
                token: raw_cue.to_ascii_lowercase(),
                duration,
                from_player: false,
                actor: None,
            });
        }
        Self::activate_viewpoint(scene, world, &node);
    }

    fn activate_viewpoint(scene: &mut Scene, world: &World, node: &CommentaryNode) {
        scene.commentary.active_viewpoint_entity = None;
        if let Some(vp_id) = find_entity_by_name(world, &node.viewposition) {
            scene.commentary.active_viewpoint_entity = Some(vp_id);
            if world
                .entities
                .get(vp_id)
                .is_some_and(|e| e.class() == "point_commentary_viewpoint")
            {
                if let Some(state) = scene.states.get_mut(vp_id) {
                    state.visible = true;
                }
            }
        }
    }

    pub fn stop_node(scene: &mut Scene, world: &World, idx: usize) {
        let Some(node) = scene.commentary.nodes.get(idx).cloned() else {
            return;
        };
        if !node.active {
            return;
        }
        scene.commentary.nodes[idx].active = false;
        scene.commentary.nodes[idx].unstoppable = false;
        if scene.commentary.active_node == Some(idx) {
            scene.commentary.active_node = None;
        }
        if let Some(vp_id) = scene.commentary.active_viewpoint_entity.take() {
            if world
                .entities
                .get(vp_id)
                .is_some_and(|e| e.class() == "point_commentary_viewpoint")
            {
                if let Some(state) = scene.states.get_mut(vp_id) {
                    state.visible = false;
                }
            }
        }
        scene.commentary.view_override = None;
        if !node.active_cue.is_empty() {
            scene.sounds.push(SoundRequest {
                stop: true,
                emitter: Some(node.entity_id as u64),
                caption: false,
                ..node.active_cue.as_str().into()
            });
        }
        scene.fire(node.entity_id, "OnCommentaryStopped", usize::MAX);
        Self::queue_commands(scene, &node.postcommands);
    }

    pub fn tick(
        scene: &mut Scene,
        world: &World,
        dt: f32,
        eye: Vec3,
        view_angles: [f32; 3],
        use_pressed: bool,
        secondary_pressed: bool,
    ) {
        if !scene.commentary.enabled || scene.commentary.nodes.is_empty() {
            return;
        }
        if let Some(active_idx) = scene.commentary.active_node {
            let finished =
                scene.commentary.nodes.get(active_idx).is_some_and(|n| {
                    (scene.time as f32) >= n.start_time + n.active_duration.max(0.1)
                });
            if finished {
                Self::stop_node(scene, world, active_idx);
            }
        }
        let hovered = Self::raycast_nodes(scene, world, eye, view_angles);
        scene.commentary.hovered_node = hovered;
        for (idx, node) in scene.commentary.nodes.iter().enumerate() {
            let rate = if node.active {
                3.0
            } else if hovered == Some(idx) && !node.listened {
                1.0
            } else {
                0.0
            };
            if let Some(state) = scene.states.get_mut(node.entity_id) {
                state.animation_started += f64::from((1.0 - rate) * dt);
            }
        }
        if use_pressed {
            Self::handle_use(scene, world, eye, view_angles);
        }
        if secondary_pressed {
            Self::jump_to_next_node(scene, view_angles);
        }
        Self::update_view_override(scene, world, eye, view_angles);
    }

    fn handle_use(scene: &mut Scene, world: &World, eye: Vec3, view_angles: [f32; 3]) {
        if let Some(active_idx) = scene.commentary.active_node {
            if scene.commentary.nodes[active_idx].unstoppable {
                return;
            }
            if scene.commentary.hovered_node == Some(active_idx)
                || scene.commentary.view_override.is_some()
            {
                Self::stop_node(scene, world, active_idx);
                return;
            }
        }
        if let Some(target_idx) = scene.commentary.hovered_node {
            Self::start_node(scene, world, target_idx, false);
            scene.commentary.blend_start_origin = eye;
            scene.commentary.blend_start_angles = view_angles;
            scene.commentary.blend_initialized = true;
        }
    }

    fn jump_to_next_node(scene: &mut Scene, view_angles: [f32; 3]) {
        if scene
            .commentary
            .active_node
            .is_some_and(|i| scene.commentary.nodes[i].unstoppable)
        {
            return;
        }
        let count = scene.commentary.nodes.len();
        let start = scene
            .commentary
            .current_jump_index
            .map_or(0, |i| (i + 1) % count);
        let Some(next_idx) = (0..count)
            .map(|offset| (start + offset) % count)
            .find(|&i| scene.commentary.nodes[i].enabled)
        else {
            return;
        };
        scene.commentary.current_jump_index = Some(next_idx);
        let node = &scene.commentary.nodes[next_idx];
        let dest_origin = node.teleport_origin.unwrap_or_else(|| {
            node.origin - forward_from_angles(node.angles) * 64.0 - Vec3::new(0.0, 0.0, 36.0)
        });
        let dest_eye = dest_origin + Vec3::new(0.0, 0.0, 64.0);
        let look_angles = angles_toward(dest_eye, node.origin, view_angles[2]);
        scene.commentary.pending_teleport = Some((dest_origin, look_angles));
        scene
            .screen_fades
            .push(ScreenFade::new([0, 0, 0, 255], 0.5, 0.1, FFADE_IN));
    }

    fn update_view_override(scene: &mut Scene, world: &World, eye: Vec3, view_angles: [f32; 3]) {
        let Some(active_idx) = scene.commentary.active_node else {
            scene.commentary.view_override = None;
            return;
        };
        let node = &scene.commentary.nodes[active_idx];
        let vp_origin = find_entity_by_name(world, &node.viewposition)
            .and_then(|id| scene.states.get(id))
            .map(|s| s.origin);
        let vt_origin = find_entity_by_name(world, &node.viewtarget)
            .and_then(|id| scene.states.get(id))
            .map(|s| s.origin);
        if vp_origin.is_none() && vt_origin.is_none() {
            scene.commentary.view_override = None;
            return;
        }
        if !scene.commentary.blend_initialized {
            scene.commentary.blend_start_origin = eye;
            scene.commentary.blend_start_angles = view_angles;
            scene.commentary.blend_initialized = true;
        }
        let elapsed = ((scene.time as f32) - node.start_time).max(0.0);
        let blend = (elapsed / 1.5).clamp(0.0, 1.0);
        let smooth = blend * blend * (3.0 - 2.0 * blend);
        let start_o = scene.commentary.blend_start_origin;
        let target_o = vp_origin.unwrap_or(eye);
        let origin = if vp_origin.is_some() {
            start_o.lerp(target_o, smooth)
        } else {
            eye
        };
        let angles = if let Some(target) = vt_origin {
            let desired = angles_toward(origin, target, 0.0);
            lerp_angles(scene.commentary.blend_start_angles, desired, smooth)
        } else {
            view_angles
        };
        scene.commentary.view_override = Some(CommentaryViewOverride {
            origin,
            angles,
            lock_movement: node.prevent_movement || vp_origin.is_some() || vt_origin.is_some(),
        });
    }

    fn raycast_nodes(
        scene: &Scene,
        world: &World,
        eye: Vec3,
        view_angles: [f32; 3],
    ) -> Option<usize> {
        let dir = forward_from_angles(view_angles).normalize_or_zero();
        if dir == Vec3::ZERO {
            return None;
        }
        let world_trace = modkit_core::trace_brushes(
            &world.brushes,
            eye,
            eye + dir * NODE_MAX_RANGE,
            Vec3::ZERO,
            Vec3::ZERO,
            CONTENTS_SOLID_WINDOW,
        );
        let max_dist = NODE_MAX_RANGE * world_trace.fraction;
        let mut best: Option<(usize, f32)> = None;
        for (idx, node) in scene.commentary.nodes.iter().enumerate() {
            if !node.enabled {
                continue;
            }
            if scene.states.get(node.entity_id).is_some_and(|s| !s.visible) {
                continue;
            }
            let min = node.origin - Vec3::splat(NODE_HALF_EXTENT);
            let max = node.origin + Vec3::splat(NODE_HALF_EXTENT);
            if let Some(dist) = ray_aabb(eye, dir, min, max) {
                if dist <= max_dist && best.is_none_or(|(_, d)| dist < d) {
                    best = Some((idx, dist));
                }
            }
        }
        best.map(|(idx, _)| idx)
    }
}

fn forward_from_angles(angles: [f32; 3]) -> Vec3 {
    let pitch = angles[0].to_radians();
    let yaw = angles[1].to_radians();
    Vec3::new(
        pitch.cos() * yaw.cos(),
        pitch.cos() * yaw.sin(),
        -pitch.sin(),
    )
}

fn ray_aabb(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut tmin = 0.0_f32;
    let mut tmax = f32::INFINITY;
    for axis in 0..3 {
        let o = origin[axis];
        let d = dir[axis];
        let bmin = min[axis];
        let bmax = max[axis];
        if d.abs() < 1e-6 {
            if o < bmin || o > bmax {
                return None;
            }
        } else {
            let inv = 1.0 / d;
            let (t1, t2) = if inv >= 0.0 {
                ((bmin - o) * inv, (bmax - o) * inv)
            } else {
                ((bmax - o) * inv, (bmin - o) * inv)
            };
            tmin = tmin.max(t1);
            tmax = tmax.min(t2);
            if tmin > tmax {
                return None;
            }
        }
    }
    Some(tmin)
}

fn angles_toward(from: Vec3, to: Vec3, roll: f32) -> [f32; 3] {
    let delta = to - from;
    let horiz = delta.truncate().length();
    let yaw = delta.y.atan2(delta.x).to_degrees();
    let pitch = (-delta.z).atan2(horiz).to_degrees();
    [pitch, yaw, roll]
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let diff = (b - a + 180.0).rem_euclid(360.0) - 180.0;
    a + diff * t
}

fn lerp_angles(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        lerp_angle(a[0], b[0], t),
        lerp_angle(a[1], b[1], t),
        lerp_angle(a[2], b[2], t),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commentary_entities_caret_quotes_connections_and_semaphore() {
        let text = r##"
"Entities"
{
    "entity"
    {
        "classname" "point_commentary_node"
        "origin" "10 20 30"
        "angles" "0 90 0"
        "targetname" "comm_node_1"
        "speakers" "^Gabe Newell^"
        "commentaryfile" "#commentary\comm_01.wav"
        "precommands" "ai_disable 1;ent_fire door open"
        "connections"
        {
            "OnCommentaryStarted" "relay_1,Trigger,,0,-1"
        }
    }
}
"##;
        let entities = parse_commentary_entities(text);
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].class(), "point_commentary_node");
        assert_eq!(entities[0].get("speakers"), Some("\"Gabe Newell\""));
        assert_eq!(entities[0].get("model"), Some(SPEECH_MODEL));
        assert_eq!(
            entities[0].get("OnCommentaryStarted"),
            Some("relay_1,Trigger,,0,-1")
        );
        assert_eq!(entities[1].class(), "info_target");
        assert_eq!(entities[1].get("targetname"), Some("commentary_semaphore"));
    }

    #[test]
    fn use_starts_and_stops_hovered_node_switches_skin_and_secondary_teleports() {
        let text = r##"
"Entities"
{
    "entity"
    {
        "classname" "point_commentary_node"
        "origin" "100 0 64"
        "angles" "0 0 0"
        "targetname" "node_1"
        "speakers" "Gabe Newell"
        "commentaryfile" "#commentary\comm_01.wav"
        "_commentary_duration" "5.0"
        "precommands" "ai_disable 1"
        "postcommands" "ai_disable 0"
        "teleport_origin" "40 0 0"
    }
}
"##;
        let mut world = World::default();
        world.entities.extend(parse_commentary_entities(text));
        let mut scene = Scene::new(&world);
        CommentaryState::spawn_init(&mut scene, &world, true, true, true);
        assert_eq!(scene.commentary.nodes.len(), 1);
        let id = scene.commentary.nodes[0].entity_id;
        assert_eq!(scene.commentary.node_skin(id), 0);

        // Aim at (100, 0, 64) from (0, 0, 64) and press +use.
        CommentaryState::tick(
            &mut scene,
            &world,
            0.015,
            Vec3::new(0.0, 0.0, 64.0),
            [0.0, 0.0, 0.0],
            true,
            false,
        );
        assert_eq!(scene.commentary.active_node, Some(0));
        assert_eq!(scene.commentary.pending_commands, vec!["ai_disable 1"]);
        assert_eq!(scene.commentary.node_skin(id), 0);
        assert_eq!(
            scene.commentary.active_overlay().unwrap().speakers,
            "Gabe Newell"
        );

        // Press +use again while looking at the active node to stop it; skin becomes 1 (used).
        scene.commentary.pending_commands.clear();
        CommentaryState::tick(
            &mut scene,
            &world,
            0.015,
            Vec3::new(0.0, 0.0, 64.0),
            [0.0, 0.0, 0.0],
            true,
            true,
        );
        assert_eq!(scene.commentary.active_node, None);
        assert_eq!(scene.commentary.node_skin(id), 1);
        assert_eq!(scene.commentary.pending_commands, vec!["ai_disable 0"]);
        let (dest, _) = scene
            .commentary
            .pending_teleport
            .expect("secondary attack teleports to next node");
        assert_eq!(dest, Vec3::new(40.0, 0.0, 0.0));
    }

    #[test]
    #[ignore = "requires an owned installation"]
    fn owned_lostcoast_commentary_file_parses_nodes_and_audio_durations() {
        let root = source_assets::install::discover().expect("owned installation");
        let vfs = Vfs::mount_game(&root, "lostcoast").unwrap();
        let mut world = World::default();
        let count = append_world_commentary(&mut world, &vfs, "d2_lostcoast");
        assert!(
            count >= 10,
            "expected Lost Coast commentary nodes, got {count}"
        );
        let node = world
            .entities
            .iter()
            .find(|e| e.class() == "point_commentary_node")
            .unwrap();

        for (k, v) in &node.properties {
            println!("  {k} = {v}");
        }
        let dur: f32 = node
            .get("_commentary_duration")
            .expect("commentary WAV duration")
            .parse()
            .unwrap();
        assert!(dur > 1.0, "expected positive WAV duration, got {dur}");
    }
}
