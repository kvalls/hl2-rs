//! Model ambient/local light presentation and the viewmodel's draw-to-world lighting basis.
use super::{ModelLighting, SourceMaterial};
use crate::source_to_bevy;
use bevy::{
    asset::{Assets, Handle},
    camera::visibility::InheritedVisibility,
    ecs::{
        component::Component,
        system::{Query, Res, ResMut},
    },
    image::Image,
    math::{Mat4, Vec3},
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
        game: &crate::gameplay::Gameplay,
        materials: &mut Assets<SourceMaterial>,
        handle: &bevy::asset::Handle<SourceMaterial>,
    ) {
        let Some(basis) = self.lighting_basis(game) else {
            return;
        };
        if materials
            .get(handle)
            .is_some_and(|m| m.lighting.basis != basis)
            && let Some(mut material) = materials.get_mut(handle)
        {
            material.lighting.basis = basis;
        }
    }
    /// Its material reflects the cubemap sample near it. A model loaded late has no mask for it
    /// yet, and without one a reflection is far too strong, so those do not reflect.
    pub(super) fn reflecting(mut self, reflects: bool) -> Self {
        self.cubemap = reflects.then_some(usize::MAX);
        self
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

/// Recompute each lit model's light state when its illumination origin moves
/// (retail light cache at origin + rotation * illumposition * scale), and every frame
/// while an entity light (muzzle elight) reaches it (ComputeDynamicLighting).
pub fn present_lighting(
    game: Res<crate::gameplay::Gameplay>,
    sim: Res<crate::simulation::Simulation>,
    effects: Option<Res<crate::effects::Effects>>,
    flashlight: Res<crate::flashlight::HostFlashlight>,
    mut draws: Query<(
        &mut ModelLit,
        &MeshMaterial3d<SourceMaterial>,
        &InheritedVisibility,
    )>,
    mut materials: ResMut<Assets<SourceMaterial>>,
    (performance, cubemaps): (
        Option<Res<crate::performance::Performance>>,
        Option<Res<super::cubemaps::ModelCubemaps>>,
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
    for (mut lit, material, visible) in &mut draws {
        if !visible.get() && lit.last.is_some() {
            continue;
        }
        present_draw_lighting(&ctx, &mut lit, &material.0, &mut states, &mut materials);
    }
}

fn present_draw_lighting(
    ctx: &LightingContext<'_>,
    lit: &mut ModelLit,
    material: &Handle<SourceMaterial>,
    states: &mut BTreeMap<Option<usize>, (glam::Vec3, modkit_core::lighting::LightState)>,
    materials: &mut Assets<SourceMaterial>,
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
            lit.rebase(ctx.game, materials, material);
        }
        return;
    }
    let Some(existing) = materials.get(material) else {
        return;
    };
    let uniform = if lit.entity.is_none() && same_light && existing.lighting.params.w == 2. {
        let mut uniform = existing.lighting;
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
    let lighting_changed = existing.lighting != uniform;
    let beam_changed = lit.entity.is_none() && existing.viewmodel_flashlight != ctx.beam;
    let env_update = lit.next_cubemap(origin, ctx.data, ctx.cubemaps);
    lit.dynamic = lit_dynamic;
    lit.last = Some((origin, lit.half_lambert));
    if !lighting_changed && !beam_changed && env_update.is_none() {
        return;
    }
    // Only borrow mutably when fields change so unchanged materials emit no AssetEvent::Modified.
    if let Some(mut mat) = materials.get_mut(material) {
        if lighting_changed {
            mat.lighting = uniform;
        }
        if beam_changed {
            mat.viewmodel_flashlight = ctx.beam;
        }
        if let Some((index, cube)) = env_update {
            mat.envmap = cube;
            mat.envmap_tint.w = 1.;
            lit.cubemap = Some(index);
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
