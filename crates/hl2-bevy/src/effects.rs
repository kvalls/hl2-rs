//! Depth-tested host adapter for shared owned projectile sprites and impact marks.
use crate::{assets::LoadedMap, gameplay::Gameplay, movement::Simulation, rendering};
use bevy::{
    asset::RenderAssetUsages,
    camera::primitives::MeshAabb,
    mesh::{Indices, MeshVertexBufferLayoutRef},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, PrimitiveTopology,
        RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
use hl2_simulation::{projectile_visuals::ProjectileVisuals, projectiles::ProjectileKind};
use modkit_core::Surface;
use source_assets::{models, vpk::Vfs};
use std::collections::BTreeMap;

pub struct PreparedEffects {
    pub sprites: ProjectileVisuals,
    /// Rendered projectile models (SMG grenade, frag grenade) by model path.
    pub models: BTreeMap<&'static str, Vec<Surface>>,
    model_errors: Vec<String>,
}
impl PreparedEffects {
    pub fn load(vfs: &Vfs) -> Self {
        let mut model_errors = Vec::new();
        let mut loaded = BTreeMap::new();
        for path in [
            hl2_simulation::projectiles::GRENADE_MODEL,
            hl2_simulation::projectiles::FRAG_MODEL,
            hl2_simulation::weapon_crossbow::BOLT_MODEL,
            hl2_simulation::weapon_rpg::MISSILE_LAUNCH_MODEL,
            hl2_simulation::weapon_rpg::MISSILE_MODEL,
            hl2_simulation::weapon_bugbait::BAIT_MODEL,
        ] {
            match models::read_model(vfs, path, 0) {
                Ok(surfaces) => {
                    loaded.insert(path, surfaces);
                }
                Err(e) => model_errors.push(format!("{path}: {e:#}")),
            }
        }
        Self {
            sprites: ProjectileVisuals::new(vfs),
            models: loaded,
            model_errors,
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(EffectKey)]
pub struct EffectMaterial {
    #[texture(0)]
    #[sampler(1)]
    texture: Handle<Image>,
    #[uniform(2)]
    parameters: Vec4,
    kind: usize,
}
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct EffectKey {
    kind: usize,
}
impl From<&EffectMaterial> for EffectKey {
    fn from(m: &EffectMaterial) -> Self {
        Self { kind: m.kind }
    }
}
impl Material for EffectMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/effects.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        match self.kind {
            0 => AlphaMode::Opaque,
            1 => AlphaMode::Mask(0.5),
            _ => AlphaMode::Blend,
        }
    }
    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(blend) = effect_blend(key.bind_group_data.kind) {
            for target in descriptor
                .fragment
                .as_mut()
                .into_iter()
                .flat_map(|f| &mut f.targets)
                .flatten()
            {
                target.blend = Some(blend);
            }
        }
        Ok(())
    }
}
fn effect_blend(kind: usize) -> Option<BlendState> {
    match kind {
        3 => Some(BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::SrcAlpha,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent::OVER,
        }),
        4 => Some(BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::Dst,
                dst_factor: BlendFactor::Zero,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent {
                src_factor: BlendFactor::Zero,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
        }),
        _ => None,
    }
}
#[derive(Component)]
pub(crate) struct DecalDraw {
    id: usize,
}
#[derive(Component)]
pub(crate) struct GrenadeDraw {
    id: u64,
}
struct QuadDraw {
    entity: Entity,
    mesh: Handle<Mesh>,
    previous: Option<hl2_simulation::projectile_visuals::Quad>,
}
/// One projectile model surface: mesh and material.
type ModelPart = (Handle<Mesh>, Handle<rendering::SourceMaterial>);
#[derive(Resource)]
pub struct Effects {
    source: ProjectileVisuals,
    materials: BTreeMap<String, Handle<EffectMaterial>>,
    quads: Vec<QuadDraw>,
    marks: BTreeMap<usize, (Entity, Handle<Mesh>)>,
    grenade_meshes: BTreeMap<&'static str, Vec<ModelPart>>,
    grenades: BTreeMap<u64, Vec<Entity>>,
    /// Stuck crossbow bolt model instances by StuckBolt id.
    stuck: BTreeMap<u64, Vec<Entity>>,
    /// Viewmodel-space sprite/beam quads on the viewmodel camera's layer.
    viewmodel_quads: Vec<QuadDraw>,
    viewmodel_draws: usize,
    rng: u64,
    model_errors: Vec<String>,
    draws: usize,
    grenade_draws: usize,
    mark_draws: usize,
    pose_mismatches: usize,
    observed_decals: usize,
    observed_grenades: usize,
}
impl Effects {
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"sprite_draws":self.draws,"pooled_quads":self.quads.len(),
            "viewmodel_sprite_draws":self.viewmodel_draws,"stuck_bolt_draws":self.stuck.len(),
            "grenade_draws":self.grenade_draws,"decal_draws":self.mark_draws,"pose_mismatches":self.pose_mismatches, "observed_decals":self.observed_decals,"observed_grenades":self.observed_grenades,
            "ball_frames":self.source.ball_frames,"effect_frames":self.source.effect_frames,
            "particle_emitters":self.source.particles.diagnostics,"missing_particle_draws":self.source.missing_particle_draws,
            "sprite_errors":self.source.errors,"model_errors":self.model_errors,
            "limitations":"owned smoke/fire/ember/debris/electric emitters and shock rings; native RNG, ambient cubes, soft depth blending, beam tessellation, particle-manager scheduling, animated receivers and exact shading remain unfinished"})
    }
}
pub fn install(
    loaded: &LoadedMap,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<rendering::SourceMaterial>,
    effects: &mut Assets<EffectMaterial>,
    images: &mut Assets<Image>,
) {
    let mut handles = BTreeMap::new();
    for (name, sprite) in &loaded.effects.sprites.sprites {
        let texture = images.add(rendering::image(
            sprite.image.width,
            sprite.image.height,
            sprite.image.rgba.clone(),
            false,
        ));
        handles.insert(
            name.clone(),
            effects.add(EffectMaterial {
                texture,
                parameters: Vec4::new(
                    if sprite.kind == 1 { 0.5 } else { 0.001 },
                    f32::from(sprite.kind == 0),
                    1.,
                    0.,
                ),
                kind: sprite.kind,
            }),
        );
    }
    for (name, decal) in &loaded.gameplay.impacts.textures {
        let texture = images.add(rendering::image(
            decal.image.width,
            decal.image.height,
            decal.image.rgba.clone(),
            false,
        ));
        handles.insert(
            name.clone(),
            effects.add(EffectMaterial {
                texture,
                parameters: Vec4::new(0.001, 1., 2., 0.),
                kind: 4,
            }),
        );
    }
    let white = images.add(rendering::image(1, 1, vec![255; 4], false));
    let black_cube = images.add(rendering::black_cube());
    let mut grenade_meshes = BTreeMap::new();
    let mut bases = BTreeMap::new();
    for (model, surface) in loaded
        .effects
        .models
        .iter()
        .flat_map(|(model, surfaces)| surfaces.iter().map(move |s| (*model, s)))
    {
        let fallback = crate::assets::MaterialData::default();
        let definition = loaded.materials.get(&surface.material).unwrap_or(&fallback);
        let base = bases
            .entry(surface.material.clone())
            .or_insert_with(|| {
                definition.base.as_ref().map_or_else(
                    || white.clone(),
                    |i| images.add(rendering::image(i.width, i.height, i.rgba.clone(), true)),
                )
            })
            .clone();
        let material = materials.add(rendering::make_material(
            definition,
            base,
            white.clone(),
            None,
            (&white, &black_cube),
        ));
        grenade_meshes.entry(model).or_insert_with(Vec::new).push((
            meshes.add(rendering::mesh_from_surface(surface, definition)),
            material,
        ));
    }
    // Move CPU sprite state out of PreparedMap after all owned images are uploaded.
    commands.insert_resource(Effects {
        source: ProjectileVisuals::empty(),
        materials: handles,
        quads: Vec::new(),
        marks: BTreeMap::new(),
        grenade_meshes,
        grenades: BTreeMap::new(),
        stuck: BTreeMap::new(),
        viewmodel_quads: Vec::new(),
        viewmodel_draws: 0,
        rng: 0x2545_f491_4f6c_dd1d,
        model_errors: loaded.effects.model_errors.clone(),
        draws: 0,
        grenade_draws: 0,
        mark_draws: 0,
        pose_mismatches: 0,
        observed_decals: 0,
        observed_grenades: 0,
    });
}
pub fn adopt(source: ProjectileVisuals, commands: &mut Commands) {
    commands.queue(move |world: &mut World| {
        world.resource_mut::<Effects>().source = source;
    });
}
fn mesh(
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
    dynamic: bool,
) -> Mesh {
    let count = positions.len();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        if dynamic {
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD
        } else {
            RenderAssetUsages::RENDER_WORLD
        },
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.; 2]; count]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 1., 0.]; count]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}
fn quad_mesh(quad: &hl2_simulation::projectile_visuals::Quad) -> Mesh {
    mesh(
        quad.positions
            .iter()
            .map(|p| crate::source_to_bevy(Vec3::from_array(p.to_array())).to_array())
            .collect(),
        quad.uv.to_vec(),
        vec![quad.color.map(|c| f32::from(c) / 255.); 4],
        vec![0, 1, 2, 0, 2, 3],
        true,
    )
}
/// Fill a pool of quad draws with this frame's quads (materials must hold each
/// quad's material); `layer` puts them on another camera's render layer.
fn draw_pool(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    pool: &mut Vec<QuadDraw>,
    quads: &[hl2_simulation::projectile_visuals::Quad],
    materials: &BTreeMap<String, Handle<EffectMaterial>>,
    layer: Option<usize>,
) {
    let mut used = 0;
    for quad in quads {
        let Some(material) = materials.get(&quad.material).cloned() else {
            continue;
        };
        if used == pool.len() {
            let mesh = meshes.add(quad_mesh(quad));
            let mut entity = commands.spawn((
                crate::campaign::MapOwned,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::IDENTITY,
            ));
            if let Some(layer) = layer {
                entity.insert(bevy::camera::visibility::RenderLayers::layer(layer));
            }
            pool.push(QuadDraw {
                entity: entity.id(),
                mesh,
                previous: None,
            });
        }
        let slot = &mut pool[used];
        used += 1;
        if slot.previous.as_ref() != Some(quad) {
            let mesh = quad_mesh(quad);
            let bounds = mesh.compute_aabb();
            if let Some(mut old) = meshes.get_mut(&slot.mesh) {
                *old = mesh;
            }
            if let Some(bounds) = bounds {
                commands.entity(slot.entity).insert(bounds);
            }
            slot.previous = Some(quad.clone());
        }
        commands
            .entity(slot.entity)
            .insert((MeshMaterial3d(material), Visibility::Visible));
    }
    for slot in pool.iter().skip(used) {
        commands.entity(slot.entity).insert(Visibility::Hidden);
    }
}
/// The active viewmodel's posed rig (same clip choice as rendering::animate) and the
/// SDK viewmodel sprites/beams as viewmodel-space quads.
fn viewmodel_quads(
    game: &Gameplay,
    effects: &mut Effects,
) -> Vec<hl2_simulation::projectile_visuals::Quad> {
    use hl2_simulation::viewmodel_effects as vm;
    let described = game.inventory.viewmodel_effects(game.scene.time);
    if described.sprites.is_empty() && described.beams.is_empty() {
        return Vec::new();
    }
    let Some(rig) = game.weapons.get(&game.inventory.active).and_then(|w| {
        game.world
            .rigs
            .get(&format!("{}#0", w.viewmodel.to_lowercase()))
    }) else {
        return Vec::new();
    };
    let elapsed = (game.scene.time - game.inventory.animation_at).max(0.) as f32;
    let (clip, time) = if rig
        .clips
        .get(&game.inventory.animation)
        .is_some_and(|c| elapsed < c.duration())
    {
        (game.inventory.animation.as_str(), elapsed)
    } else {
        (game.inventory.idle_animation(), game.scene.time as f32)
    };
    let matrices =
        vm::viewmodel_matrices(rig, clip, time, &game.inventory.viewmodel_pose_values(rig));
    let sprites = &effects.source.sprites;
    let rng = &mut effects.rng;
    let mut random = || {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 7;
        *rng ^= *rng << 17;
        (*rng >> 40) as f32 / (1u32 << 24) as f32
    };
    vm::viewmodel_quads(
        &described,
        &|anchor| vm::anchor_position(rig, &matrices, anchor),
        &|material| sprites.get(material).map(|s| f32::from(s.image.width)),
        &mut random,
    )
}
pub fn present(
    mut commands: Commands,
    mut effects: ResMut<Effects>,
    game: Res<Gameplay>,
    sim: Res<Simulation>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut draws: Query<(&mut Transform, &mut Visibility)>,
) {
    let direction = glam::Vec3::from_array(crate::source_direction(sim.yaw, sim.pitch).to_array());
    effects.source.frame(
        &game.projectiles,
        glam::Vec3::from_array(sim.eye().to_array()),
        direction,
        game.scene.time,
        sim.paused(),
        &sim.physics,
    );
    effects.draws = effects.source.quads.len();
    let quads = std::mem::take(&mut effects.source.quads);
    let Effects {
        materials,
        quads: pool,
        ..
    } = &mut *effects;
    draw_pool(&mut commands, &mut meshes, pool, &quads, materials, None);
    effects.source.quads = quads;
    // Sprites and beams on the viewmodel (CSprite/CBeam viewmodel attachments).
    let viewmodel = viewmodel_quads(&game, &mut effects);
    effects.viewmodel_draws = viewmodel.len();
    let Effects {
        materials,
        viewmodel_quads: pool,
        ..
    } = &mut *effects;
    draw_pool(
        &mut commands,
        &mut meshes,
        pool,
        &viewmodel,
        materials,
        Some(1),
    );
    let dead: Vec<_> = effects
        .marks
        .keys()
        .copied()
        .filter(|id| !game.impacts.marks.iter().any(|m| m.id == *id))
        .collect();
    for id in dead {
        if let Some((entity, handle)) = effects.marks.remove(&id) {
            commands.entity(entity).despawn();
            meshes.remove(&handle);
        }
    }
    effects.mark_draws = 0;
    for mark in &game.impacts.marks {
        let (transform, visible) =
            game.scene
                .states
                .get(mark.entity)
                .map_or((Transform::IDENTITY, true), |s| {
                    let (p, r) = sim
                        .physics
                        .entity_pose(mark.entity)
                        .unwrap_or((s.origin, s.rotation));
                    let mut t = rendering::entity_transform(p, r);
                    t.scale = Vec3::splat(mark.scale);
                    (t, !s.killed && s.visible)
                });
        if !effects.marks.contains_key(&mark.id) {
            let mesh = meshes.add(mesh(
                mark.vertices
                    .iter()
                    .map(|v| {
                        crate::source_to_bevy(Vec3::from_array(v.position.to_array())).to_array()
                    })
                    .collect(),
                mark.vertices.iter().map(|v| v.uv.to_array()).collect(),
                vec![[1.; 4]; mark.vertices.len()],
                (0..mark.vertices.len() as u32).collect(),
                false,
            ));
            let material = effects.materials[&mark.material].clone();
            let entity = commands
                .spawn((
                    crate::campaign::MapOwned,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    transform,
                    DecalDraw { id: mark.id },
                ))
                .id();
            effects.marks.insert(mark.id, (entity, mesh));
        }
        let entity = effects.marks[&mark.id].0;
        commands.entity(entity).insert((
            transform,
            if visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
        ));
        effects.mark_draws += usize::from(visible);
    }
    let dead: Vec<_> = effects
        .grenades
        .keys()
        .copied()
        .filter(|id| {
            !game
                .projectiles
                .active
                .iter()
                .any(|p| p.id == *id && p.kind != ProjectileKind::CombineBall)
        })
        .collect();
    for id in dead {
        if let Some(entities) = effects.grenades.remove(&id) {
            for entity in entities {
                commands.entity(entity).despawn();
            }
        }
    }
    effects.grenade_draws = 0;
    for grenade in game
        .projectiles
        .active
        .iter()
        .filter(|p| p.kind != ProjectileKind::CombineBall)
    {
        let transform = rendering::entity_transform(
            grenade.position,
            hl2_simulation::physics::angles(grenade.angles),
        );
        if !effects.grenades.contains_key(&grenade.id) {
            let entities = effects
                .grenade_meshes
                .get(grenade.model())
                .into_iter()
                .flatten()
                .map(|(mesh, material)| {
                    commands
                        .spawn((
                            crate::campaign::MapOwned,
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(material.clone()),
                            transform,
                            GrenadeDraw { id: grenade.id },
                        ))
                        .id()
                })
                .collect();
            effects.grenades.insert(grenade.id, entities);
        }
        effects.grenade_draws += effects.grenades[&grenade.id].len();
        for entity in &effects.grenades[&grenade.id] {
            if let Ok((mut current, mut visible)) = draws.get_mut(*entity) {
                *current = transform;
                *visible = Visibility::Visible;
            } else {
                commands.entity(*entity).insert(transform);
            }
        }
    }
    // Stuck crossbow bolts (CCrossbowBolt left in the world): crossbow_bolt.mdl at the
    // recorded position/angles; the record cap drops the oldest instance.
    let live: std::collections::BTreeSet<u64> =
        game.projectiles.stuck_bolts.iter().map(|b| b.id).collect();
    let gone: Vec<u64> = effects
        .stuck
        .keys()
        .copied()
        .filter(|id| !live.contains(id))
        .collect();
    for id in gone {
        for entity in effects.stuck.remove(&id).unwrap_or_default() {
            commands.entity(entity).despawn();
        }
    }
    for bolt in &game.projectiles.stuck_bolts {
        if effects.stuck.contains_key(&bolt.id) {
            continue;
        }
        let transform = rendering::entity_transform(
            bolt.position,
            hl2_simulation::physics::angles(bolt.angles),
        );
        let entities = effects
            .grenade_meshes
            .get(hl2_simulation::weapon_crossbow::BOLT_MODEL)
            .into_iter()
            .flatten()
            .map(|(mesh, material)| {
                commands
                    .spawn((
                        crate::campaign::MapOwned,
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        transform,
                        Visibility::Visible,
                    ))
                    .id()
            })
            .collect();
        effects.stuck.insert(bolt.id, entities);
    }
}

fn agrees(actual: &Transform, expected: &Transform) -> bool {
    actual.translation.distance(expected.translation) < 0.001
        && actual.rotation.dot(expected.rotation).abs() > 0.99999
        && actual.scale.distance(expected.scale) < 0.001
}
pub fn observe(
    mut effects: ResMut<Effects>,
    game: Res<Gameplay>,
    sim: Res<Simulation>,
    decals: Query<(&DecalDraw, &Transform, &Visibility)>,
    grenades: Query<(&GrenadeDraw, &Transform)>,
) {
    let mut mismatches = 0;
    for (draw, transform, visible) in &decals {
        let Some(mark) = game.impacts.marks.iter().find(|m| m.id == draw.id) else {
            mismatches += 1;
            continue;
        };
        let (expected, should_show) =
            game.scene
                .states
                .get(mark.entity)
                .map_or((Transform::IDENTITY, true), |s| {
                    let (p, r) = sim
                        .physics
                        .entity_pose(mark.entity)
                        .unwrap_or((s.origin, s.rotation));
                    let mut t = rendering::entity_transform(p, r);
                    t.scale = Vec3::splat(mark.scale);
                    (t, !s.killed && s.visible)
                });
        mismatches += usize::from(
            !agrees(transform, &expected) || (*visible != Visibility::Hidden) != should_show,
        );
    }
    for (draw, transform) in &grenades {
        let Some(p) = game.projectiles.active.iter().find(|p| p.id == draw.id) else {
            mismatches += 1;
            continue;
        };
        let expected =
            rendering::entity_transform(p.position, hl2_simulation::physics::angles(p.angles));
        mismatches += usize::from(!agrees(transform, &expected));
    }
    effects.pose_mismatches = mismatches;
    effects.observed_decals = decals.iter().count();
    effects.observed_grenades = grenades.iter().count();
}
