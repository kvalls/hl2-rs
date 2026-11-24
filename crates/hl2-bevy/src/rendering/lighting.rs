//! Model ambient/local light presentation and the viewmodel's draw-to-world lighting basis.
use super::{ModelLighting, SourceMaterial};
use crate::source_to_bevy;
use bevy::{
    asset::{Assets, Handle},
    camera::visibility::InheritedVisibility,
    ecs::{
        component::Component,
        entity::Entity,
        system::{Local, Query, Res, ResMut},
    },
    image::Image,
    math::{Mat4, Vec3},
    mesh::MeshTag,
    pbr::MeshMaterial3d,
    transform::components::Transform,
};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

/// A model lit by its own light state.
#[derive(Component)]
pub struct ModelLit {
    pub(super) entity: Option<usize>,
    pub(super) key: String,
    pub(super) scale: f32,
    pub(super) half_lambert: bool,
    last: Option<(glam::Vec3, bool)>,
    /// Last unobstructed light-cache cell `(cache_cell(origin), half_lambert)`.
    last_cell: Option<([i32; 3], bool)>,
    /// Entity lights (muzzle elights) lit this draw last time: relight once they end.
    dynamic: bool,
    /// The cubemap sample it reflects, for a material that reflects one; usize::MAX before any.
    cubemap: Option<usize>,
    /// The light cache cell that sample was chosen in: the choice holds until it leaves the cell.
    cell: Option<[i32; 3]>,
    /// Cached `ModelIllumination` looked up once from `world.illumination`.
    illumination: Option<modkit_core::lighting::ModelIllumination>,
    /// Its entity's LightingOrigin/LightingOriginHack, resolved at its first lighting.
    origins: Option<Option<hl2_simulation::lighting_origins::LightingOrigin>>,
    /// The scene's lighting_origin_changes when `origins` was resolved.
    origins_changes: u32,
}

impl ModelLit {
    pub(super) fn new(entity: Option<usize>, key: String, scale: f32, half_lambert: bool) -> Self {
        Self {
            entity,
            key,
            scale,
            half_lambert,
            last: None,
            last_cell: None,
            dynamic: false,
            cubemap: None,
            cell: None,
            illumination: None,
            origins: None,
            origins_changes: 0,
        }
    }
    /// The point its entity is lit at: its own illumination point unless the entity names a
    /// lighting origin (hl2_simulation::lighting_origins).
    fn lighting_point(
        &mut self,
        game: &crate::gameplay::Gameplay,
        id: usize,
        point: glam::Vec3,
    ) -> glam::Vec3 {
        let scene = &game.scene;
        if self.origins_changes != scene.lighting_origin_changes {
            self.origins_changes = scene.lighting_origin_changes;
            self.origins = None;
            self.last = None;
            self.last_cell = None;
        }
        let origins = *self.origins.get_or_insert_with(|| {
            hl2_simulation::lighting_origins::resolve(&game.world, &scene.keyvalue_overrides, id)
        });
        origins.map_or(point, |o| o.point(&game.scene.states, point))
    }
    /// The lighting offset of a relative lighting origin as the draw-to-lighting basis (the
    /// shader lights basis * position and normal): resolved by lighting_point first.
    fn lighting_basis(&self, game: &crate::gameplay::Gameplay) -> Option<Mat4> {
        let (rotation, translation) = self.origins.flatten()?.offset(&game.scene.states)?;
        // Source (x, y, z) is Bevy (x, z, -y): a proper rotation, so a quaternion's axis maps
        // the same way.
        let rotation = bevy::math::Quat::from_xyzw(rotation.x, rotation.z, -rotation.y, rotation.w);
        let translation = source_to_bevy(Vec3::from_array(translation.to_array()));
        Some(Mat4::from_rotation_translation(rotation, translation))
    }
    /// Keeps a cached light state's basis on the moving offset: the lighting point holds still
    /// in the mock car while the car (and its landmark) moves.
    fn rebase(
        &self,
        tag: u32,
        game: &crate::gameplay::Gameplay,
        (materials, slots): (
            &mut Assets<SourceMaterial>,
            &mut crate::material_slots::MaterialSlots,
        ),
        handle: &bevy::asset::Handle<SourceMaterial>,
    ) {
        let Some(basis) = self.lighting_basis(game) else {
            return;
        };
        let Some(mut lighting) = materials
            .get(handle)
            .map(|m| slots.draw_lighting(tag, m, handle.id()))
        else {
            return;
        };
        if lighting.basis != basis {
            lighting.basis = basis;
            if tag > 0 {
                slots.write_tagged_lighting(tag, lighting);
            } else {
                let key = (self.entity, self.half_lambert);
                slots.write_lighting(materials, handle, key, lighting);
            }
        }
    }
    /// Its material reflects the cubemap sample near it. A model loaded late has no mask for it
    /// yet, and without one a reflection is far too strong, so those do not reflect.
    pub(super) fn reflecting(mut self, reflects: bool) -> Self {
        self.cubemap = reflects.then_some(usize::MAX);
        self
    }
    /// Whether this draw chooses a dynamic per-location cubemap reflection sample.
    pub(super) fn reflects(&self) -> bool {
        self.cubemap.is_some()
    }
    /// Returns the new cubemap index and texture if moving into `origin`'s light cache cell changes
    /// this model's reflection sample; updates the tracked cell without touching `Assets`.
    fn next_cubemap(
        &mut self,
        origin: glam::Vec3,
        data: &modkit_core::lighting::LightingData,
        cubemaps: Option<&super::cubemaps::ModelCubemaps>,
    ) -> Option<(usize, Handle<Image>)> {
        let cell = modkit_core::lighting::cache_cell(origin);
        let moved = self.cell.replace(cell) != Some(cell);
        let held = self.cubemap.filter(|_| moved)?;
        cubemaps
            .and_then(|cubes| cubes.pick(data.cache_point(origin)))
            .filter(|(index, _)| *index != held)
    }
}

const MAX_CACHED_CELLS: usize = 4096;

#[inline]
fn cell_center(cell: [i32; 3]) -> glam::Vec3 {
    glam::Vec3::new(
        cell[0] as f32 + 16.,
        cell[1] as f32 + 16.,
        cell[2] as f32 + 64.,
    )
}

#[derive(Clone, Copy)]
struct FrameTag {
    origin: glam::Vec3,
    last_cell: Option<([i32; 3], bool)>,
    dynamic: bool,
    half_lambert: bool,
    scale: f32,
    illum: modkit_core::lighting::ModelIllumination,
}

#[derive(Default)]
pub struct StaticLightCache {
    data: Option<Weak<modkit_core::lighting::LightingData>>,
    origin_changes: u32,
    cells: HashMap<[i32; 3], modkit_core::lighting::LightState>,
    boosted_cells: HashMap<[i32; 3], modkit_core::lighting::LightState>,
    entries: HashMap<(Option<usize>, bool), (glam::Vec3, modkit_core::lighting::LightState)>,
    frame_tags: HashMap<u32, FrameTag>,
    scratch: modkit_core::lighting::LightState,
}

impl StaticLightCache {
    fn record_tag(
        &mut self,
        tag: u32,
        lit: &ModelLit,
        origin: glam::Vec3,
        dynamic: bool,
        illum: modkit_core::lighting::ModelIllumination,
    ) {
        if tag > 0 && !lit.reflects() && lit.entity.is_some() {
            self.frame_tags.insert(
                tag,
                FrameTag {
                    origin,
                    last_cell: lit.last_cell,
                    dynamic,
                    half_lambert: lit.half_lambert,
                    scale: lit.scale,
                    illum,
                },
            );
        }
    }

    fn take_tag(
        &self,
        tag: u32,
        lit: &mut ModelLit,
        illum: modkit_core::lighting::ModelIllumination,
    ) -> bool {
        if tag > 0
            && !lit.reflects()
            && lit.entity.is_some()
            && let Some(&entry) = self.frame_tags.get(&tag)
            && entry.half_lambert == lit.half_lambert
            && entry.scale == lit.scale
            && entry.illum == illum
        {
            lit.dynamic = entry.dynamic;
            lit.last = Some((entry.origin, lit.half_lambert));
            lit.last_cell = entry.last_cell;
            true
        } else {
            false
        }
    }

    fn prepare(&mut self, data: &Arc<modkit_core::lighting::LightingData>, origin_changes: u32) {
        let same_data = self
            .data
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|prev| Arc::ptr_eq(&prev, data));
        if !same_data || self.origin_changes != origin_changes {
            self.entries.clear();
            self.cells.clear();
            self.boosted_cells.clear();
            self.data = Some(Arc::downgrade(data));
            self.origin_changes = origin_changes;
        }
        self.frame_tags.clear();
    }

    fn cell_state(
        &mut self,
        data: &modkit_core::lighting::LightingData,
        cell: [i32; 3],
        ambient_boost: bool,
    ) -> &modkit_core::lighting::LightState {
        if self.cells.len() >= MAX_CACHED_CELLS {
            self.cells.clear();
            self.boosted_cells.clear();
        }
        let center = cell_center(cell);
        let base = self
            .cells
            .entry(cell)
            .or_insert_with(|| data.state_at(center));
        if !ambient_boost {
            return base;
        }
        self.boosted_cells.entry(cell).or_insert_with(|| {
            let mut state = base.clone();
            data.boost(&mut state, center);
            state
        })
    }

    fn ensure_entry_state(
        &mut self,
        data: &modkit_core::lighting::LightingData,
        entity: Option<usize>,
        origin: glam::Vec3,
        ambient_boost: bool,
    ) -> (Option<usize>, bool) {
        let key = (entity, ambient_boost);
        let stale = self
            .entries
            .get(&key)
            .is_none_or(|(cached, _)| cached.distance_squared(origin) >= 16.);
        if stale {
            let [x, y, _] = modkit_core::lighting::cache_cell(origin).map(|e| e as f32);
            let level = glam::Vec3::new(x + 16., y + 16., origin.z);
            let sample = if data.trace(origin, level).is_none() {
                level
            } else {
                origin
            };
            let mut state = data.state_at(sample);
            if ambient_boost {
                data.boost(&mut state, sample);
            }
            self.entries.insert(key, (origin, state));
        }
        key
    }

    fn sample_static_state(
        &mut self,
        data: &modkit_core::lighting::LightingData,
        entity: Option<usize>,
        origin: glam::Vec3,
        cell_center_ok: Option<[i32; 3]>,
        ambient_boost: bool,
    ) -> &modkit_core::lighting::LightState {
        if let Some(cell) = cell_center_ok {
            return self.cell_state(data, cell, ambient_boost);
        }
        let key = self.ensure_entry_state(data, entity, origin, ambient_boost);
        &self.entries[&key].1
    }

    fn dynamic_state(
        &mut self,
        data: &modkit_core::lighting::LightingData,
        entity: Option<usize>,
        origin: glam::Vec3,
        cell_center_ok: Option<[i32; 3]>,
        ambient_boost: bool,
        elights: &[modkit_core::lighting::WorldLight],
    ) -> &modkit_core::lighting::LightState {
        if let Some(cell) = cell_center_ok {
            self.cell_state(data, cell, ambient_boost);
            let base = if ambient_boost {
                &self.boosted_cells[&cell]
            } else {
                &self.cells[&cell]
            };
            self.scratch.clone_from(base);
        } else {
            let key = self.ensure_entry_state(data, entity, origin, ambient_boost);
            self.scratch.clone_from(&self.entries[&key].1);
        }
        for light in elights {
            data.add_dynamic(&mut self.scratch, origin, light);
        }
        &self.scratch
    }
}

struct LightingContext<'a> {
    game: &'a crate::gameplay::Gameplay,
    sim: &'a crate::simulation::Simulation,
    data: &'a modkit_core::lighting::LightingData,
    elights: &'a [modkit_core::lighting::WorldLight],
    cubemaps: Option<&'a super::cubemaps::ModelCubemaps>,
    beam: crate::flashlight::ViewmodelLight,
    view: Mat4,
}

type LitDraws<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut ModelLit,
        &'static MeshMaterial3d<SourceMaterial>,
        Option<&'static MeshTag>,
        &'static InheritedVisibility,
    ),
>;

type LightingResources<'w, 's> = (
    Option<Res<'w, crate::performance::Performance>>,
    Option<Res<'w, super::cubemaps::ModelCubemaps>>,
    ResMut<'w, crate::material_slots::MaterialSlots>,
    Local<'s, StaticLightCache>,
);

/// Recompute each lit model's light state when its illumination origin moves
/// (retail light cache at origin + rotation * illumposition * scale), and every frame
/// while an entity light (muzzle elight) reaches it (ComputeDynamicLighting).
pub fn present_lighting(
    game: Res<crate::gameplay::Gameplay>,
    sim: Res<crate::simulation::Simulation>,
    effects: Option<Res<crate::effects::Effects>>,
    flashlight: Res<crate::flashlight::HostFlashlight>,
    mut draws: LitDraws,
    mut materials: ResMut<Assets<SourceMaterial>>,
    (performance, cubemaps, mut slots, mut cache): LightingResources,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "model lighting");
    let Some(data_arc) = game.world.lighting.as_ref() else {
        return;
    };
    cache.prepare(data_arc, game.scene.lighting_origin_changes);
    let mut elights = effects
        .as_deref()
        .map_or_else(Vec::new, |e| e.elights(data_arc.dynamic_minlight));
    elights.extend(game.scene.dynamic_model_lights(data_arc.dynamic_minlight));
    let (yaw, pitch) = sim.view_angles();
    let ctx = LightingContext {
        game: &game,
        sim: &sim,
        data: data_arc,
        elights: &elights,
        cubemaps: cubemaps.as_deref(),
        beam: crate::flashlight::ViewmodelLight::from_light(flashlight.light()),
        view: viewmodel_basis(sim.view_origin(), yaw, pitch),
    };
    for (mut lit, material, tag, visible) in &mut draws {
        if !visible.get() && lit.last.is_some() {
            continue;
        }
        let assets = (&mut *materials, &mut *slots);
        let tag_val = tag.map_or(0, |t| t.0);
        present_draw_lighting(&ctx, (&mut lit, &material.0, tag_val), &mut cache, assets);
    }
}

fn model_illumination(
    lit: &mut ModelLit,
    game: &crate::gameplay::Gameplay,
) -> modkit_core::lighting::ModelIllumination {
    *lit.illumination.get_or_insert_with(|| {
        game.world
            .illumination
            .get(&lit.key)
            .copied()
            .unwrap_or_default()
    })
}

fn present_draw_lighting(
    ctx: &LightingContext<'_>,
    (lit, material, tag): (&mut ModelLit, &Handle<SourceMaterial>, u32),
    cache: &mut StaticLightCache,
    (materials, slots): (
        &mut Assets<SourceMaterial>,
        &mut crate::material_slots::MaterialSlots,
    ),
) {
    let illumination = model_illumination(lit, ctx.game);
    if cache.take_tag(tag, lit, illumination) {
        return;
    }
    let Some(origin) = resolve_lighting_origin(ctx, lit, illumination) else {
        return;
    };
    let lit_dynamic = !ctx.elights.is_empty()
        && ctx
            .elights
            .iter()
            .any(|l| modkit_core::lighting::dynamic_reaches(origin, l));
    let moved_less_than_4 = lit
        .last
        .is_some_and(|(last, hl)| last.distance_squared(origin) < 16. && hl == lit.half_lambert);
    let same_light = !lit_dynamic && !lit.dynamic && moved_less_than_4;
    if lit.entity.is_some() && same_light {
        finish_skipped_draw(
            ctx,
            (lit, tag, origin, lit_dynamic, illumination),
            cache,
            (materials, slots),
            material,
        );
        return;
    }
    let cell = modkit_core::lighting::cache_cell(origin);
    let reaches_center = (moved_less_than_4 && lit.last_cell == Some((cell, lit.half_lambert)))
        || ctx.data.trace(origin, cell_center(cell)).is_none();
    if lit.entity.is_some()
        && !lit_dynamic
        && !lit.dynamic
        && reaches_center
        && lit.last_cell == Some((cell, lit.half_lambert))
    {
        lit.last = Some((origin, lit.half_lambert));
        finish_skipped_draw(
            ctx,
            (lit, tag, origin, lit_dynamic, illumination),
            cache,
            (materials, slots),
            material,
        );
        return;
    }
    let Some(existing) = materials.get(material) else {
        return;
    };
    let current = slots.draw_lighting(tag, existing, material.id());
    let cell_center_ok = reaches_center.then_some(cell);
    let uniform = compute_draw_uniform(
        ctx,
        lit,
        (&current, origin, cell_center_ok, lit_dynamic, same_light),
        illumination,
        cache,
    );
    let beam_changed = lit.entity.is_none() && existing.viewmodel_flashlight != ctx.beam;
    let env_state = if lit.reflects() {
        (
            lit.next_cubemap(origin, ctx.data, ctx.cubemaps),
            existing.envmap.clone(),
            existing.envmap_tint.w,
        )
    } else {
        (None, Handle::default(), 1.)
    };
    lit.dynamic = lit_dynamic;
    lit.last = Some((origin, lit.half_lambert));
    lit.last_cell = reaches_center.then_some((cell, lit.half_lambert));
    if current != uniform {
        if tag > 0 {
            slots.write_tagged_lighting(tag, uniform);
        } else {
            slots.write_lighting(materials, material, (lit.entity, lit.half_lambert), uniform);
        }
    }
    apply_material_lighting_updates(
        lit,
        material,
        materials,
        (beam_changed, ctx.beam),
        env_state,
    );
    cache.record_tag(tag, lit, origin, lit_dynamic, illumination);
}

fn finish_skipped_draw(
    ctx: &LightingContext<'_>,
    (lit, tag, origin, lit_dynamic, illumination): (
        &mut ModelLit,
        u32,
        glam::Vec3,
        bool,
        modkit_core::lighting::ModelIllumination,
    ),
    cache: &mut StaticLightCache,
    (materials, slots): (
        &mut Assets<SourceMaterial>,
        &mut crate::material_slots::MaterialSlots,
    ),
    material: &Handle<SourceMaterial>,
) {
    if lit.origins.flatten().is_some_and(|o| o.relative.is_some()) {
        lit.rebase(tag, ctx.game, (materials, slots), material);
    }
    cache.record_tag(tag, lit, origin, lit_dynamic, illumination);
}

fn resolve_lighting_origin(
    ctx: &LightingContext<'_>,
    lit: &mut ModelLit,
    illumination: modkit_core::lighting::ModelIllumination,
) -> Option<glam::Vec3> {
    match lit.entity {
        Some(id) => {
            let state = ctx.game.scene.states.get(id)?;
            let (origin, rotation) = ctx
                .sim
                .physics
                .entity_pose(id)
                .unwrap_or((state.origin, state.rotation));
            let point = origin + rotation * (illumination.position * lit.scale);
            Some(lit.lighting_point(ctx.game, id, point))
        }
        None => Some(glam::Vec3::from_array(ctx.sim.eye().to_array())),
    }
}

fn compute_draw_uniform(
    ctx: &LightingContext<'_>,
    lit: &ModelLit,
    (current, origin, cell_center_ok, lit_dynamic, same_light): (
        &ModelLighting,
        glam::Vec3,
        Option<[i32; 3]>,
        bool,
        bool,
    ),
    illumination: modkit_core::lighting::ModelIllumination,
    cache: &mut StaticLightCache,
) -> ModelLighting {
    if lit.entity.is_none() && same_light && current.params.w == 2. {
        let mut uniform = *current;
        uniform.basis = ctx.view;
        return uniform;
    }
    let state = if lit_dynamic {
        cache.dynamic_state(
            ctx.data,
            lit.entity,
            origin,
            cell_center_ok,
            illumination.ambient_boost(),
            ctx.elights,
        )
    } else {
        cache.sample_static_state(
            ctx.data,
            lit.entity,
            origin,
            cell_center_ok,
            illumination.ambient_boost(),
        )
    };
    let mut uniform = ModelLighting::from_state(state, lit.half_lambert);
    uniform.basis = lit.lighting_basis(ctx.game).unwrap_or(uniform.basis);
    if lit.entity.is_none() {
        uniform.basis = ctx.view;
        uniform.params.w = 2.;
    }
    uniform
}

type EnvmapState = (Option<(usize, Handle<Image>)>, Handle<Image>, f32);

fn apply_material_lighting_updates(
    lit: &mut ModelLit,
    material: &Handle<SourceMaterial>,
    materials: &mut Assets<SourceMaterial>,
    (beam_changed, beam): (bool, crate::flashlight::ViewmodelLight),
    (env_update, existing_envmap, existing_envmap_w): EnvmapState,
) {
    let env_update = env_update.filter(|&(index, ref cube)| {
        lit.cubemap = Some(index);
        existing_envmap != *cube || existing_envmap_w != 1.
    });
    if !beam_changed && env_update.is_none() {
        return;
    }
    // Only borrow mutably when fields change so unchanged materials emit no AssetEvent::Modified.
    if let Some(mut mat) = materials.get_mut(material) {
        if beam_changed {
            mat.viewmodel_flashlight = beam;
        }
        if let Some((_, cube)) = env_update {
            mat.envmap = cube;
            mat.envmap_tint.w = 1.;
        }
    }
}

fn detach_tagged_material(
    commands: &mut bevy::prelude::Commands,
    materials: &mut Assets<SourceMaterial>,
    draw: Entity,
    mat: &Handle<SourceMaterial>,
    tag: Option<&MeshTag>,
) -> Handle<SourceMaterial> {
    if tag.is_some_and(|t| t.0 > 0)
        && materials.get(mat).is_some_and(|m| m.dynamic.w < 0.5)
        && let Some(copy) = super::copy_asset(materials, mat)
    {
        if let Some(mut cloned) = materials.get_mut(&copy) {
            cloned.dynamic.w = 1.;
        }
        commands.entity(draw).insert(MeshMaterial3d(copy.clone()));
        copy
    } else {
        mat.clone()
    }
}

type TaggedLitDraws<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static ModelLit,
        &'static MeshMaterial3d<SourceMaterial>,
        Option<&'static MeshTag>,
    ),
>;

/// C_ClientRagdoll::FadeOut: a retired ragdoll's model blends out (kRenderTransAlpha).
pub fn present_ragdoll_fades(
    mut commands: bevy::prelude::Commands,
    game: Res<crate::gameplay::Gameplay>,
    draws: TaggedLitDraws,
    mut materials: ResMut<Assets<SourceMaterial>>,
) {
    if game.ragdolls.fading() == 0 {
        return;
    }
    for (draw, lit, MeshMaterial3d(mat), tag) in &draws {
        let Some(alpha) = lit.entity.and_then(|id| game.ragdolls.fade_alpha(id)) else {
            continue;
        };
        if !materials.get(mat).is_some_and(|m| m.tint.w != alpha) {
            continue;
        }
        let target = detach_tagged_material(&mut commands, &mut materials, draw, mat, tag);
        if let Some(mut m) = materials.get_mut(&target) {
            m.tint.w = alpha;
            m.alpha = bevy::prelude::AlphaMode::Blend;
        }
    }
}

/// SetRenderColor at runtime (Scene::render_colors, e.g. a burning zombie's Scorch): the
/// entity's model materials take the color in place of their keyvalue modulation; its
/// client ragdoll keeps the entity id and so the color (C_ClientRagdoll copies it).
pub fn present_render_colors(
    mut commands: bevy::prelude::Commands,
    game: Res<crate::gameplay::Gameplay>,
    draws: TaggedLitDraws,
    mut materials: ResMut<Assets<SourceMaterial>>,
    mut base: bevy::ecs::system::Local<
        std::collections::HashMap<bevy::asset::AssetId<SourceMaterial>, bevy::math::Vec4>,
    >,
) {
    if game.scene.render_colors.is_empty() {
        return;
    }
    for (draw, lit, MeshMaterial3d(mat), tag) in &draws {
        let Some(id) = lit.entity else { continue };
        let (Some(color), Some(m)) = (game.scene.render_colors.get(&id), materials.get(mat)) else {
            continue;
        };
        let original = *base.entry(mat.id()).or_insert(m.tint);
        // The runtime color replaces the keyvalue color the material was built with.
        let keyvalue = super::entity_modulation(&game.world, id)
            .truncate()
            .max(Vec3::splat(1e-3));
        let rgb = Vec3::from_array(color.map(|c| f32::from(c) / 255.));
        let tint = (original.truncate() / keyvalue * rgb).extend(m.tint.w);
        if m.tint == tint {
            continue;
        }
        let target = detach_tagged_material(&mut commands, &mut materials, draw, mat, tag);
        base.entry(target.id()).or_insert(original);
        if let Some(mut m) = materials.get_mut(&target) {
            m.tint = tint;
        }
    }
}
/// The fixed +X viewmodel draw space mapped onto the player's world view.
fn viewmodel_basis(origin: Vec3, yaw: f32, pitch: f32) -> Mat4 {
    Transform::from_translation(source_to_bevy(origin))
        .looking_to(source_to_bevy(crate::source_direction(yaw, pitch)), Vec3::Y)
        .to_matrix()
        * Transform::IDENTITY
            .looking_to(Vec3::X, Vec3::Y)
            .to_matrix()
            .inverse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapon_draw_positions_and_normals_follow_the_world_view_without_framing_warp() {
        for (origin, yaw, pitch) in [
            (Vec3::ZERO, 0., 0.),
            (Vec3::new(-4040., -2460., 136.), 1.2, -0.3),
        ] {
            let basis = viewmodel_basis(origin, yaw, pitch);
            let forward = source_to_bevy(crate::source_direction(yaw, pitch));
            let point = basis.transform_point3(Vec3::X * 30.);
            assert!(point.distance(source_to_bevy(origin) + forward * 30.) < 0.001);
            let normal = basis.transform_vector3(Vec3::NEG_X);
            assert!(normal.distance(-forward) < 1e-5);
            assert!((normal.length() - 1.).abs() < 1e-5);
        }
    }

    #[test]
    fn stationary_lighting_and_viewmodel_emit_no_modified_material_events_on_repeat_frames() {
        use bevy::{
            app::{App, TaskPoolPlugin, Update},
            asset::{AssetApp, AssetEvent, AssetPlugin},
            ecs::message::{MessageCursor, Messages},
        };
        use std::sync::Arc;

        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
            .init_asset::<SourceMaterial>()
            .init_resource::<crate::material_slots::MaterialSlots>()
            .add_systems(Update, present_lighting);

        let mut lighting = modkit_core::lighting::LightingData::default();
        lighting.ambient.push(modkit_core::lighting::AmbientSample {
            cube: [glam::Vec3::splat(0.25); 6],
            position: [128, 128, 128],
        });
        let world = modkit_core::World {
            entities: vec![modkit_core::Entity {
                properties: vec![
                    ("classname".into(), "prop_dynamic".into()),
                    ("origin".into(), "10 20 0".into()),
                ],
            }],
            lighting: Some(Arc::new(lighting)),
            ..Default::default()
        };
        let sim =
            crate::simulation::Simulation::new(&world, glam::Vec3::Z * 64., 0., 0., false, vec![]);
        let game = crate::gameplay::Gameplay::synthetic(world);
        let vm_handle = app
            .world_mut()
            .resource_mut::<Assets<SourceMaterial>>()
            .add(crate::material_slots::test_material());
        let world_handle = app
            .world_mut()
            .resource_mut::<Assets<SourceMaterial>>()
            .add(crate::material_slots::test_material());

        app.insert_resource(game)
            .insert_resource(sim)
            .init_resource::<crate::flashlight::HostFlashlight>();
        app.world_mut().spawn((
            ModelLit::new(None, "vm".into(), 1., false),
            MeshMaterial3d(vm_handle.clone()),
            InheritedVisibility::VISIBLE,
        ));
        app.world_mut().spawn((
            ModelLit::new(Some(0), "prop".into(), 1., false),
            MeshMaterial3d(world_handle),
            InheritedVisibility::VISIBLE,
        ));

        let mut cursor = MessageCursor::<AssetEvent<SourceMaterial>>::default();
        let drain_modified = |app: &App, cursor: &mut MessageCursor<AssetEvent<SourceMaterial>>| {
            let messages = app
                .world()
                .resource::<Messages<AssetEvent<SourceMaterial>>>();
            cursor
                .read(messages)
                .filter_map(|event| match event {
                    AssetEvent::Modified { id } => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        // Frame 0: both materials receive their initial lighting uniform.
        app.update();
        assert_eq!(drain_modified(&app, &mut cursor).len(), 2);

        // Repeat frame with stationary player/camera/world emits no Modified events.
        app.update();
        assert!(drain_modified(&app, &mut cursor).is_empty());

        // Rotating the view updates only the viewmodel basis, leaving the world model untouched.
        let game_world = app
            .world()
            .resource::<crate::gameplay::Gameplay>()
            .world
            .clone();
        let turned = crate::simulation::Simulation::new(
            &game_world,
            glam::Vec3::Z * 64.,
            1.0,
            0.,
            false,
            vec![],
        );
        app.insert_resource(turned);
        app.update();
        assert_eq!(drain_modified(&app, &mut cursor), vec![vm_handle.id()]);

        // Next stationary frame after turning emits no Modified events again.
        app.update();
        assert!(drain_modified(&app, &mut cursor).is_empty());
    }

    #[test]
    fn static_light_cache_restores_static_lighting_and_dedups_shared_tags() {
        use bevy::{
            app::{App, TaskPoolPlugin, Update},
            asset::{AssetApp, AssetPlugin},
        };
        use std::sync::Arc;

        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
            .init_asset::<SourceMaterial>()
            .init_resource::<crate::material_slots::MaterialSlots>()
            .add_systems(Update, present_lighting);

        let lighting = modkit_core::lighting::LightingData {
            root: -1,
            leaves: vec![modkit_core::lighting::LightLeaf {
                contents: 0,
                flags: 0,
                mins: glam::Vec3::splat(-100.),
                maxs: glam::Vec3::splat(100.),
                ambient_count: 1,
                ambient_first: 0,
                sky_faces: Vec::new(),
                brushes: Vec::new(),
            }],
            ambient: vec![modkit_core::lighting::AmbientSample {
                cube: [glam::Vec3::splat(0.2); 6],
                position: [128, 128, 128],
            }],
            dynamic_minlight: 1.,
            ..Default::default()
        };
        let world = modkit_core::World {
            entities: vec![
                modkit_core::Entity {
                    properties: vec![
                        ("classname".into(), "prop_dynamic".into()),
                        ("origin".into(), "10 20 0".into()),
                    ],
                },
                modkit_core::Entity {
                    properties: vec![
                        ("classname".into(), "light_dynamic".into()),
                        ("origin".into(), "10 20 20".into()),
                        ("brightness".into(), "4".into()),
                        ("distance".into(), "200".into()),
                        ("_light".into(), "255 128 64 200".into()),
                    ],
                },
            ],
            lighting: Some(Arc::new(lighting)),
            ..Default::default()
        };
        let sim =
            crate::simulation::Simulation::new(&world, glam::Vec3::Z * 64., 0., 0., false, vec![]);
        let game = crate::gameplay::Gameplay::synthetic(world);
        let shared_mat = app
            .world_mut()
            .resource_mut::<Assets<SourceMaterial>>()
            .add(crate::material_slots::test_material());

        let draw_a = app
            .world_mut()
            .spawn((
                ModelLit::new(Some(0), "prop".into(), 1., false),
                MeshMaterial3d(shared_mat.clone()),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        let draw_b = app
            .world_mut()
            .spawn((
                ModelLit::new(Some(0), "prop".into(), 1., false),
                MeshMaterial3d(shared_mat.clone()),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        let tag = {
            let mut slots = app
                .world_mut()
                .resource_mut::<crate::material_slots::MaterialSlots>();
            let ta = slots.assign_draw_tag(draw_a, (Some(0), false)).unwrap();
            let tb = slots.assign_draw_tag(draw_b, (Some(0), false)).unwrap();
            assert_eq!(ta, tb);
            ta
        };
        app.world_mut().entity_mut(draw_a).insert(MeshTag(tag));
        app.world_mut().entity_mut(draw_b).insert(MeshTag(tag));

        app.insert_resource(game)
            .insert_resource(sim)
            .init_resource::<crate::flashlight::HostFlashlight>();

        // Frame 0: active light_dynamic illuminates both tagged draws via the shared tag.
        app.update();
        let dynamic_slot = {
            let mats = app.world().resource::<Assets<SourceMaterial>>();
            let slots = app
                .world()
                .resource::<crate::material_slots::MaterialSlots>();
            slots.draw_lighting(tag, mats.get(&shared_mat).unwrap(), shared_mat.id())
        };
        let static_expected = {
            let data = app
                .world()
                .resource::<crate::gameplay::Gameplay>()
                .world
                .lighting
                .as_ref()
                .unwrap();
            ModelLighting::from_state(&data.state_at(glam::Vec3::new(10., 20., 0.)), false)
        };
        assert_ne!(dynamic_slot, static_expected);

        // Extinguish dynamic light: static cache restores exact baseline lighting without rescan.
        app.world_mut()
            .resource_mut::<crate::gameplay::Gameplay>()
            .scene
            .states[1]
            .killed = true;
        app.update();
        let restored_slot = {
            let mats = app.world().resource::<Assets<SourceMaterial>>();
            let slots = app
                .world()
                .resource::<crate::material_slots::MaterialSlots>();
            slots.draw_lighting(tag, mats.get(&shared_mat).unwrap(), shared_mat.id())
        };
        assert_eq!(restored_slot, static_expected);
        assert!(!app.world().get::<ModelLit>(draw_b).unwrap().dynamic);
        assert_eq!(
            app.world().get::<ModelLit>(draw_b).unwrap().last,
            Some((glam::Vec3::new(10., 20., 0.), false))
        );
    }

    #[test]
    fn moving_or_lighting_origin_change_invalidates_static_light_cache() {
        use bevy::{
            app::{App, TaskPoolPlugin, Update},
            asset::{AssetApp, AssetPlugin},
        };
        use std::sync::Arc;

        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
            .init_asset::<SourceMaterial>()
            .init_resource::<crate::material_slots::MaterialSlots>()
            .add_systems(Update, present_lighting);

        let mut lighting = modkit_core::lighting::LightingData {
            root: 0,
            nodes: vec![modkit_core::lighting::LightNode {
                normal: glam::Vec3::X,
                distance: 0.,
                children: [-1, -2],
            }],
            leaves: vec![
                modkit_core::lighting::LightLeaf {
                    contents: 0,
                    flags: 0,
                    mins: glam::Vec3::splat(0.),
                    maxs: glam::Vec3::splat(200.),
                    ambient_count: 1,
                    ambient_first: 0,
                    sky_faces: Vec::new(),
                    brushes: Vec::new(),
                },
                modkit_core::lighting::LightLeaf {
                    contents: 0,
                    flags: 0,
                    mins: glam::Vec3::splat(-200.),
                    maxs: glam::Vec3::splat(0.),
                    ambient_count: 1,
                    ambient_first: 1,
                    sky_faces: Vec::new(),
                    brushes: Vec::new(),
                },
            ],
            ambient: vec![
                modkit_core::lighting::AmbientSample {
                    cube: [glam::Vec3::splat(0.8); 6],
                    position: [128, 128, 128],
                },
                modkit_core::lighting::AmbientSample {
                    cube: [glam::Vec3::splat(0.1); 6],
                    position: [128, 128, 128],
                },
            ],
            lights: Vec::new(),
            dynamic_minlight: 1.,
            brushes: Vec::new(),
            index: None,
        };
        lighting.build_index();

        let world = modkit_core::World {
            entities: vec![modkit_core::Entity {
                properties: vec![
                    ("classname".into(), "prop_dynamic".into()),
                    ("origin".into(), "50 0 0".into()),
                ],
            }],
            lighting: Some(Arc::new(lighting)),
            ..Default::default()
        };
        let sim =
            crate::simulation::Simulation::new(&world, glam::Vec3::Z * 64., 0., 0., false, vec![]);
        let game = crate::gameplay::Gameplay::synthetic(world);
        let mat = app
            .world_mut()
            .resource_mut::<Assets<SourceMaterial>>()
            .add(crate::material_slots::test_material());
        let draw = app
            .world_mut()
            .spawn((
                ModelLit::new(Some(0), "prop".into(), 1., false),
                MeshMaterial3d(mat.clone()),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        let tag = app
            .world_mut()
            .resource_mut::<crate::material_slots::MaterialSlots>()
            .assign_draw_tag(draw, (Some(0), false))
            .unwrap();
        app.world_mut().entity_mut(draw).insert(MeshTag(tag));
        app.insert_resource(game)
            .insert_resource(sim)
            .init_resource::<crate::flashlight::HostFlashlight>();

        app.update();
        let bright = {
            let mats = app.world().resource::<Assets<SourceMaterial>>();
            let slots = app
                .world()
                .resource::<crate::material_slots::MaterialSlots>();
            slots.draw_lighting(tag, mats.get(&mat).unwrap(), mat.id())
        };

        // Move across the leaf plane x = 0 into the dimmer leaf (-50, 0, 0).
        app.world_mut()
            .resource_mut::<crate::gameplay::Gameplay>()
            .scene
            .states[0]
            .origin = glam::Vec3::new(-50., 0., 0.);
        app.update();
        let dim = {
            let mats = app.world().resource::<Assets<SourceMaterial>>();
            let slots = app
                .world()
                .resource::<crate::material_slots::MaterialSlots>();
            slots.draw_lighting(tag, mats.get(&mat).unwrap(), mat.id())
        };
        assert_ne!(bright, dim);
    }

    #[test]
    fn entities_in_same_light_cache_cell_share_cached_lighting_unless_separated_by_wall() {
        use bevy::{
            app::{App, TaskPoolPlugin, Update},
            asset::{AssetApp, AssetPlugin},
        };
        use std::sync::Arc;

        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
            .init_asset::<SourceMaterial>()
            .init_resource::<crate::material_slots::MaterialSlots>()
            .add_systems(Update, present_lighting);

        // Leaf divider at x = 10: x >= 10 is open leaf 0 (bright + point light),
        // x < 10 is solid leaf 1 (dark, blocks trace to cell center (16, 16, 64)).
        let mut lighting = modkit_core::lighting::LightingData {
            root: 0,
            nodes: vec![modkit_core::lighting::LightNode {
                normal: glam::Vec3::X,
                distance: 10.,
                children: [-1, -2],
            }],
            leaves: vec![
                modkit_core::lighting::LightLeaf {
                    contents: 0,
                    flags: 0,
                    mins: glam::Vec3::new(10., 0., 0.),
                    maxs: glam::Vec3::new(100., 100., 128.),
                    ambient_count: 1,
                    ambient_first: 0,
                    sky_faces: Vec::new(),
                    brushes: Vec::new(),
                },
                modkit_core::lighting::LightLeaf {
                    contents: modkit_core::lighting::MASK_OPAQUE,
                    flags: 0,
                    mins: glam::Vec3::new(-100., 0., 0.),
                    maxs: glam::Vec3::new(10., 100., 128.),
                    ambient_count: 1,
                    ambient_first: 1,
                    sky_faces: Vec::new(),
                    brushes: Vec::new(),
                },
            ],
            ambient: vec![
                modkit_core::lighting::AmbientSample {
                    cube: [glam::Vec3::splat(0.75); 6],
                    position: [128, 128, 128],
                },
                modkit_core::lighting::AmbientSample {
                    cube: [glam::Vec3::splat(0.05); 6],
                    position: [128, 128, 128],
                },
            ],
            lights: vec![modkit_core::lighting::WorldLight {
                origin: glam::Vec3::new(50., 0., 64.),
                intensity: glam::Vec3::splat(1000.),
                normal: glam::Vec3::ZERO,
                kind: modkit_core::lighting::EmitType::Point,
                style: 0,
                stopdot: 0.,
                stopdot2: 0.,
                exponent: 0.,
                radius: 0.,
                attenuation: [0., 0., 1.],
                flags: 0,
            }],
            dynamic_minlight: 1.,
            brushes: Vec::new(),
            index: None,
        };
        lighting.build_index();

        let world = modkit_core::World {
            entities: vec![
                modkit_core::Entity {
                    properties: vec![
                        ("classname".into(), "prop_dynamic".into()),
                        ("origin".into(), "12 5 40".into()),
                    ],
                },
                modkit_core::Entity {
                    properties: vec![
                        ("classname".into(), "prop_dynamic".into()),
                        ("origin".into(), "25 25 40".into()),
                    ],
                },
                modkit_core::Entity {
                    properties: vec![
                        ("classname".into(), "prop_dynamic".into()),
                        ("origin".into(), "5 5 40".into()),
                    ],
                },
            ],
            lighting: Some(Arc::new(lighting)),
            ..Default::default()
        };
        let sim =
            crate::simulation::Simulation::new(&world, glam::Vec3::Z * 64., 0., 0., false, vec![]);
        let game = crate::gameplay::Gameplay::synthetic(world);
        let mut mats_res = app.world_mut().resource_mut::<Assets<SourceMaterial>>();
        let mat_a = mats_res.add(crate::material_slots::test_material());
        let mat_b = mats_res.add(crate::material_slots::test_material());
        let mat_c = mats_res.add(crate::material_slots::test_material());

        let spawn =
            |world: &mut bevy::ecs::world::World, id: usize, mat: Handle<SourceMaterial>| {
                let e = world
                    .spawn((
                        ModelLit::new(Some(id), "prop".into(), 1., false),
                        MeshMaterial3d(mat),
                        InheritedVisibility::VISIBLE,
                    ))
                    .id();
                let tag = world
                    .resource_mut::<crate::material_slots::MaterialSlots>()
                    .assign_draw_tag(e, (Some(id), false))
                    .unwrap();
                world.entity_mut(e).insert(MeshTag(tag));
                tag
            };
        let tag_a = spawn(app.world_mut(), 0, mat_a.clone());
        let tag_b = spawn(app.world_mut(), 1, mat_b.clone());
        let tag_c = spawn(app.world_mut(), 2, mat_c.clone());

        app.insert_resource(game)
            .insert_resource(sim)
            .init_resource::<crate::flashlight::HostFlashlight>();
        app.update();

        let expected_cell = {
            let data = app
                .world()
                .resource::<crate::gameplay::Gameplay>()
                .world
                .lighting
                .as_ref()
                .unwrap();
            ModelLighting::from_state(&data.state_at(glam::Vec3::new(16., 16., 64.)), false)
        };
        let mats = app.world().resource::<Assets<SourceMaterial>>();
        let slots = app
            .world()
            .resource::<crate::material_slots::MaterialSlots>();
        let lit_a = slots.draw_lighting(tag_a, mats.get(&mat_a).unwrap(), mat_a.id());
        let lit_b = slots.draw_lighting(tag_b, mats.get(&mat_b).unwrap(), mat_b.id());
        let lit_c = slots.draw_lighting(tag_c, mats.get(&mat_c).unwrap(), mat_c.id());

        assert_eq!(lit_a, expected_cell);
        assert_eq!(lit_b, expected_cell);
        assert_ne!(lit_c, expected_cell);
    }
}
