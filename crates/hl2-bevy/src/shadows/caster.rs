//! One shadow caster: which entities cast, their render bounds, where their shadow frame is, how
//! big it is on screen, and their posed silhouette.
use super::{MOVE, TURN, texture::Textures};
use crate::{gameplay::Gameplay, simulation::Simulation};
use bevy::{
    asset::{Assets, Handle},
    ecs::entity::Entity,
    image::Image,
    mesh::Mesh,
};
use glam::{UVec2, Vec3};
use hl2_simulation::shadows::{
    ShadowControl,
    allocator::TextureId,
    frame::{Frame, Placement},
    receivers::Decal,
};
use modkit_core::World;

/// How a caster's decal is textured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Look {
    /// The blob picture, straight down (Low).
    Blob,
    /// The blob picture on the render-to-texture frame, without clip planes (`DrawRenderToTextureShadowLOD`).
    Lod,
    /// Its silhouette in a page fragment (corner, size).
    Cell(UVec2, u32),
}

pub(super) struct Caster {
    pub id: usize,
    pub key: String,
    scale: f32,
    /// An NPC: its shadow reaches static props.
    pub npc: bool,
    /// Pose parameters: the silhouette changes every frame (`SHADOWS_RENDER_TO_TEXTURE_DYNAMIC`).
    pub dynamic: bool,
    /// The sequence the bounds were taken for and those bounds (model space, scaled).
    sequence: Option<(String, (Vec3, Vec3))>,
    /// Where the frame was last built for (origin, rotation, straight down).
    placed: Option<(Vec3, glam::Quat, bool)>,
    pub at: Placement,
    pub frame: Option<Frame>,
    /// The frame moved since the decal was laid.
    pub moved: bool,
    /// The silhouette must be redrawn (moved, or animating).
    pub dirty: bool,
    pub texture: Option<TextureId>,
    /// The texture size its bounds ask for.
    pub size: u32,
    /// Screen area of its bounding sphere this frame (pixels), when in view.
    pub area: Option<f32>,
    /// The look this frame wants and the one its mesh has.
    pub wanted: Look,
    pub look: Option<Look>,
    pub decal: Decal,
    pub draw: Option<(Entity, Handle<Mesh>)>,
}

impl Caster {
    /// A caster for entity `id` with a studio model `key`, when it casts (`ShadowCastType`: a rig,
    /// drawn, no `disableshadows`).
    pub fn of(world: &World, id: usize, (key, scale): (String, f32)) -> Option<Self> {
        let rig = world.rigs.get(&key)?;
        let entity = &world.entities[id];
        let casts = crate::assets::visible_entity(world, id)
            && entity
                .get("disableshadows")
                .is_none_or(|v| v.trim().parse::<i32>().unwrap_or(0) == 0);
        casts.then(|| Self {
            id,
            dynamic: !rig.pose_parameters.is_empty(),
            npc: entity.class().starts_with("npc_"),
            key,
            scale,
            sequence: None,
            placed: None,
            at: Placement::new(Vec3::ZERO, glam::Quat::IDENTITY),
            frame: None,
            moved: true,
            dirty: true,
            texture: None,
            size: 0,
            area: None,
            wanted: Look::Lod,
            look: None,
            decal: Decal::default(),
            draw: None,
        })
    }

    /// Forgets where the frame was built, so the next update rebuilds it.
    pub fn invalidate(&mut self) {
        self.placed = None;
    }

    /// Render bounds for the current sequence (`GetRenderBounds`), cached per sequence.
    fn bounds(&mut self, world: &World, sequence: &str) -> Option<(Vec3, Vec3)> {
        if let Some((name, bounds)) = &self.sequence
            && name == sequence
        {
            return Some(*bounds);
        }
        let rig = world.rigs.get(&self.key)?;
        let (mut lo, mut hi) = rig.view_bounds;
        if let Some(s) = rig
            .sequences
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(sequence))
        {
            lo = lo.min(s.bounds.0);
            hi = hi.max(s.bounds.1);
        }
        let bounds = (lo * self.scale, hi * self.scale);
        self.sequence = Some((sequence.to_owned(), bounds));
        self.placed = None;
        if self.size == 0 {
            self.size = hl2_simulation::shadows::silhouette::cell_size(bounds);
        }
        Some(bounds)
    }

    /// Places the shadow frame: a blob straight down (`down`) or the render-to-texture frame along
    /// the map's direction; true when it moved, turned or changed bounds since the last build.
    pub fn place(
        &mut self,
        (game, sim): (&Gameplay, &Simulation),
        control: &ShadowControl,
        down: bool,
    ) -> Option<bool> {
        let state = game.scene.states.get(self.id)?;
        let (origin, rotation) = sim
            .physics
            .entity_pose(self.id)
            .unwrap_or((state.origin, state.rotation));
        let bounds = self.bounds(&game.world, &state.animation)?;
        if self.placed.is_some_and(|(o, r, d)| {
            d == down && o.distance(origin) <= MOVE && r.angle_between(rotation) <= TURN
        }) {
            return Some(false);
        }
        self.placed = Some((origin, rotation, down));
        self.at = Placement::new(origin, rotation);
        self.frame = Some(if down {
            Frame::blob(bounds, &self.at, Vec3::NEG_Z, control.distance)
        } else {
            Frame::render_to_texture(bounds, &self.at, control.direction, control.distance)
        });
        self.dirty = true;
        Some(true)
    }

    /// The bounding sphere of the current bounds (world centre, radius).
    pub fn sphere(&self) -> Option<(Vec3, f32)> {
        let (_, (lo, hi)) = self.sequence.as_ref()?;
        let centre = (*lo + *hi) * 0.5;
        let world = self.at.origin
            + self.at.axes[0] * centre.x
            + self.at.axes[1] * centre.y
            + self.at.axes[2] * centre.z;
        Some((world, (*hi - *lo).length() * 0.5))
    }

    /// Draws the silhouette from the model's coarsest level, posed by its rig, into `cell`.
    pub fn silhouette(
        &self,
        game: &Gameplay,
        (textures, images): (&mut Textures, &mut Assets<Image>),
        cell: (u32, UVec2),
    ) {
        let (world, scale, at) = (&game.world, self.scale, &self.at);
        let (Some(frame), Some(surfaces)) = (
            self.frame.as_ref(),
            world
                .model_lods
                .get(&self.key)
                .and_then(|levels| levels.last())
                .map(|level| level.surfaces.as_slice())
                .or_else(|| world.model_assets.get(&self.key).map(Vec::as_slice)),
        ) else {
            return;
        };
        let matrices = world
            .rigs
            .get(&self.key)
            .filter(|rig| !rig.clips.is_empty())
            .map(|rig| game.scene.actor_matrices(rig, self.id));
        let place = |v: &modkit_core::Vertex| {
            let p = match (&matrices, &v.skin) {
                (Some(m), Some(w)) => modkit_core::animation::skin(v.position, w, m),
                _ => v.position,
            } * scale;
            at.origin + at.axes[0] * p.x + at.axes[1] * p.y + at.axes[2] * p.z
        };
        textures.draw(images, cell, frame, (surfaces, &place));
    }
}

/// Whether a caster's parent chain holds another caster (`ShouldUseParentShadow` /
/// `WillParentRenderBlobbyShadow`: the child projects nothing of its own).
pub(super) fn under_caster(game: &Gameplay, casters: &[Caster], id: usize) -> bool {
    let mut at = game
        .scene
        .states
        .get(id)
        .and_then(|s| s.parent.as_ref().map(|p| p.entity));
    let mut depth = 0;
    while let Some(parent) = at
        && depth < 16
    {
        if casters.iter().any(|c| c.id == parent) {
            return true;
        }
        at = game
            .scene
            .states
            .get(parent)
            .and_then(|s| s.parent.as_ref().map(|p| p.entity));
        depth += 1;
    }
    false
}
