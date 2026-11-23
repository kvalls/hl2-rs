//! Model ambient/local light presentation and the viewmodel's draw-to-world lighting basis.
use super::{ModelLighting, SourceMaterial};
use crate::source_to_bevy;
use bevy::{
    asset::{Assets, Handle},
    camera::visibility::InheritedVisibility,
    ecs::{
        component::Component,
        entity::Entity,
        system::{Query, Res, ResMut},
    },
    image::Image,
    math::{Mat4, Vec3},
    mesh::MeshTag,
    pbr::MeshMaterial3d,
    transform::components::Transform,
};
use std::collections::BTreeMap;

/// A model lit by its own light state.
#[derive(Component)]
pub struct ModelLit {
    pub(super) entity: Option<usize>,
    pub(super) key: String,
    pub(super) scale: f32,
    pub(super) half_lambert: bool,
    last: Option<(glam::Vec3, bool)>,
    /// Entity lights (muzzle elights) lit this draw last time: relight once they end.
    dynamic: bool,
    /// The cubemap sample it reflects, for a material that reflects one; usize::MAX before any.
    cubemap: Option<usize>,
    /// The light cache cell that sample was chosen in: the choice holds until it leaves the cell.
    cell: Option<[i32; 3]>,
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
            dynamic: false,
            cubemap: None,
            cell: None,
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
    (performance, cubemaps, mut slots): (
        Option<Res<crate::performance::Performance>>,
        Option<Res<super::cubemaps::ModelCubemaps>>,
        ResMut<crate::material_slots::MaterialSlots>,
    ),
) {
    let _timing = crate::performance::scope(performance.as_deref(), "model lighting");
    let Some(data) = game.world.lighting.as_deref() else {
        return;
    };
    let mut elights = effects
        .as_deref()
        .map_or_else(Vec::new, |e| e.elights(data.dynamic_minlight));
    elights.extend(game.scene.dynamic_model_lights(data.dynamic_minlight));
    let mut states =
        BTreeMap::<Option<usize>, (glam::Vec3, modkit_core::lighting::LightState)>::new();
    let (yaw, pitch) = sim.view_angles();
    let ctx = LightingContext {
        game: &game,
        sim: &sim,
        data,
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
        present_draw_lighting(&ctx, (&mut lit, &material.0, tag_val), &mut states, assets);
    }
}

fn present_draw_lighting(
    ctx: &LightingContext<'_>,
    (lit, material, tag): (&mut ModelLit, &Handle<SourceMaterial>, u32),
    states: &mut BTreeMap<Option<usize>, (glam::Vec3, modkit_core::lighting::LightState)>,
    (materials, slots): (
        &mut Assets<SourceMaterial>,
        &mut crate::material_slots::MaterialSlots,
    ),
) {
    let illumination = ctx
        .game
        .world
        .illumination
        .get(&lit.key)
        .copied()
        .unwrap_or_default();
    let origin = match lit.entity {
        Some(id) => {
            let Some(state) = ctx.game.scene.states.get(id) else {
                return;
            };
            let (origin, rotation) = ctx
                .sim
                .physics
                .entity_pose(id)
                .unwrap_or((state.origin, state.rotation));
            let point = origin + rotation * (illumination.position * lit.scale);
            lit.lighting_point(ctx.game, id, point)
        }
        None => glam::Vec3::from_array(ctx.sim.eye().to_array()),
    };
    let lit_dynamic = ctx
        .elights
        .iter()
        .any(|l| modkit_core::lighting::dynamic_reaches(origin, l));
    let same_light = !lit_dynamic
        && !lit.dynamic
        && lit.last.is_some_and(|(last, hl)| {
            last.distance_squared(origin) < 16. && hl == lit.half_lambert
        });
    if lit.entity.is_some() && same_light {
        if lit.origins.flatten().is_some_and(|o| o.relative.is_some()) {
            lit.rebase(tag, ctx.game, (materials, slots), material);
        }
        return;
    }
    let Some(existing) = materials.get(material) else {
        return;
    };
    let current = slots.draw_lighting(tag, existing, material.id());
    let uniform = if lit.entity.is_none() && same_light && current.params.w == 2. {
        let mut uniform = current;
        uniform.basis = ctx.view;
        uniform
    } else {
        let (_, state) = states.entry(lit.entity).or_insert_with(|| {
            let mut state = ctx.data.state_at(origin);
            if illumination.ambient_boost() {
                ctx.data.boost(&mut state, origin);
            }
            for light in ctx.elights {
                ctx.data.add_dynamic(&mut state, origin, light);
            }
            (origin, state)
        });
        let mut uniform = ModelLighting::from_state(state, lit.half_lambert);
        uniform.basis = lit.lighting_basis(ctx.game).unwrap_or(uniform.basis);
        if lit.entity.is_none() {
            uniform.basis = ctx.view;
            uniform.params.w = 2.;
        }
        uniform
    };
    let lighting_changed = current != uniform;
    let beam_changed = lit.entity.is_none() && existing.viewmodel_flashlight != ctx.beam;
    let (existing_envmap, existing_envmap_w) = (existing.envmap.clone(), existing.envmap_tint.w);
    let env_update = lit.next_cubemap(origin, ctx.data, ctx.cubemaps);
    lit.dynamic = lit_dynamic;
    lit.last = Some((origin, lit.half_lambert));
    if lighting_changed {
        if tag > 0 {
            slots.write_tagged_lighting(tag, uniform);
        } else {
            let key = (lit.entity, lit.half_lambert);
            slots.write_lighting(materials, material, key, uniform);
        }
    }
    apply_material_lighting_updates(
        lit,
        material,
        materials,
        (beam_changed, ctx.beam),
        (env_update, existing_envmap, existing_envmap_w),
    );
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
}
