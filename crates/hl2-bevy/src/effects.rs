//! Depth-tested host adapter for shared owned projectile sprites and impact marks.
mod gibs;
mod material;
mod model_reading;
use crate::{assets::LoadedMap, gameplay::Gameplay, rendering, simulation::Simulation};
use bevy::{
    asset::RenderAssetUsages,
    camera::primitives::MeshAabb,
    mesh::Indices,
    prelude::*,
    render::render_resource::{AsBindGroup, PrimitiveTopology},
};
pub use gibs::{gib_models, present_gibs};
use hl2_simulation::{projectile_visuals::ProjectileVisuals, projectiles::ProjectileKind};
pub(crate) use model_reading::read_effect_model_lod;
use modkit_core::Surface;
use source_assets::vpk::Vfs;
use std::collections::BTreeMap;

const TRILINEAR: hl2_ui::config::Filtering = hl2_ui::config::Filtering::Trilinear;

pub struct PreparedEffects {
    pub sprites: ProjectileVisuals,
    /// Rendered projectile and gib models (SMG grenade, frag grenade, the map's
    /// breakable-prop gibs) by model path.
    pub models: BTreeMap<String, Vec<Surface>>,
    model_errors: Vec<String>,
}

impl PreparedEffects {
    #[allow(dead_code)]
    pub fn load(vfs: &Vfs, world: &modkit_core::World) -> Self {
        Self::load_with_lod(vfs, world, 0)
    }

    pub fn load_with_lod(vfs: &Vfs, world: &modkit_core::World, root_lod: usize) -> Self {
        let mut model_errors = Vec::new();
        let mut loaded = BTreeMap::new();
        let gibs = gib_models(world);
        let fixed = [
            hl2_simulation::projectiles::GRENADE_MODEL,
            hl2_simulation::projectiles::FRAG_MODEL,
            hl2_simulation::weapon_crossbow::BOLT_MODEL,
            hl2_simulation::weapon_rpg::MISSILE_LAUNCH_MODEL,
            hl2_simulation::weapon_rpg::MISSILE_MODEL,
            hl2_simulation::weapon_bugbait::BAIT_MODEL,
        ]
        .into_iter()
        .map(|path| (path.to_string(), 0));
        for (path, skin) in fixed.chain(gibs) {
            let key = hl2_simulation::gibs::render_key(&path, skin);
            if loaded.contains_key(&key) {
                continue;
            }
            match read_effect_model_lod(vfs, &path, skin, root_lod) {
                Ok(surfaces) => {
                    loaded.insert(key, surfaces);
                }
                Err(e) => model_errors.push(format!("{key}: {e:#}")),
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
    /// kRenderGlow: no depth test or write.
    ignore_z: bool,
}
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct EffectKey {
    kind: usize,
    ignore_z: bool,
}
impl From<&EffectMaterial> for EffectKey {
    fn from(m: &EffectMaterial) -> Self {
        Self {
            kind: m.kind,
            ignore_z: m.ignore_z,
        }
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
pub(crate) type ModelPart = (Handle<Mesh>, Handle<rendering::SourceMaterial>);
impl Effects {
    /// Gib models loaded after the map (DESIGN 14r part B) by render key, textured from
    /// the model cache like the map's gibs.
    pub fn add_models(
        &mut self,
        models: BTreeMap<String, Vec<Surface>>,
        cache: &rendering::ModelCache,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<rendering::SourceMaterial>,
    ) {
        for (key, surfaces) in models {
            if self.grenade_meshes.contains_key(&key) {
                continue;
            }
            let parts = surfaces
                .iter()
                .map(|surface| {
                    let fallback = crate::assets::MaterialData::default();
                    let definition = cache
                        .definitions
                        .get(&surface.material)
                        .unwrap_or(&fallback);
                    let base = definition
                        .base_path
                        .as_ref()
                        .and_then(|p| cache.textures.get(p))
                        .unwrap_or(&cache.white)
                        .clone();
                    let material = materials.add(rendering::make_material(
                        definition,
                        base,
                        cache.white.clone(),
                        None,
                        (&cache.white, &cache.black_cube, &cache.flashlight),
                    ));
                    (
                        meshes.add(rendering::mesh_from_surface(surface, definition)),
                        material,
                    )
                })
                .collect();
            self.grenade_meshes.insert(key, parts);
        }
    }
}
#[derive(Resource)]
pub struct Effects {
    source: ProjectileVisuals,
    materials: BTreeMap<String, Handle<EffectMaterial>>,
    quads: Vec<QuadDraw>,
    marks: BTreeMap<usize, (Entity, Handle<Mesh>)>,
    grenade_meshes: BTreeMap<String, Vec<ModelPart>>,
    /// Gib draws by gib id: entity, its own material (lighting and fade alpha) and the
    /// material's authored alpha mode.
    gibs: BTreeMap<u64, Vec<(Entity, Handle<rendering::SourceMaterial>, AlphaMode)>>,
    /// Lighting origin and alpha last applied per gib.
    gib_state: BTreeMap<u64, (glam::Vec3, u8)>,
    gib_draws: usize,
    grenades: BTreeMap<u64, Vec<Entity>>,
    /// Stuck crossbow bolt model instances by StuckBolt id.
    stuck: BTreeMap<u64, Vec<Entity>>,
    /// Viewmodel-space sprite/beam quads on the viewmodel camera's layer.
    viewmodel_quads: Vec<QuadDraw>,
    viewmodel_draws: usize,
    rng: u64,
    /// The newest crossbow load effect already given its CrossbowLoad sparks.
    load_sparks: f64,
    /// The newest gravity gun EFFECT_LAUNCH already given its GaussExplosion.
    launch_sparks: f64,
    /// The newest shot effect consumed (Gameplay::shot_effects) and the start time of the
    /// newest viewmodel fire animation given its AE_MUZZLEFLASH.
    shot_serial: u64,
    player_flash: f64,
    /// The newest Inventory::muzzle_flash_count given its viewmodel elight.
    player_muzzle_lights: u64,
    /// Scene time of the last elight decay (CL_DecayLights runs once per frame).
    elight_time: f64,
    model_errors: Vec<String>,
    draws: usize,
    grenade_draws: usize,
    mark_draws: usize,
    pose_mismatches: usize,
    observed_decals: usize,
    observed_grenades: usize,
}
impl Effects {
    /// A loaded decal material (impact textures) by name.
    pub(crate) fn decal_material(&self, name: &str) -> Option<Handle<EffectMaterial>> {
        self.materials.get(name).cloned()
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"sprite_draws":self.draws,"pooled_quads":self.quads.len(),
            "viewmodel_sprite_draws":self.viewmodel_draws,"stuck_bolt_draws":self.stuck.len(),
            "grenade_draws":self.grenade_draws,"gib_draws":self.gib_draws,"decal_draws":self.mark_draws,"pose_mismatches":self.pose_mismatches, "observed_decals":self.observed_decals,"observed_grenades":self.observed_grenades,
            "muzzle_flashes":self.source.muzzle.flashes,"tracers":self.source.muzzle.tracers_made,
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
                    // kRenderTransAlphaAdd still adds the colour of zero-alpha texels.
                    match sprite.kind {
                        1 => 0.5,
                        hl2_simulation::projectile_visuals::KIND_ALPHA_ADD => -1.,
                        _ => 0.001,
                    },
                    f32::from(sprite.kind == 0),
                    1.,
                    0.,
                ),
                kind: sprite.kind,
                ignore_z: false,
            }),
        );
        // The RPG laser dot draws in kRenderGlow mode (no depth test).
        if name == hl2_simulation::weapon_rpg::DOT_SPRITE {
            let glow = EffectMaterial {
                ignore_z: true,
                ..effects
                    .get(&handles[name])
                    .cloned()
                    .expect("material just added")
            };
            handles.insert(
                format!("{name}{}", hl2_simulation::projectile_visuals::GLOW_SUFFIX),
                effects.add(glow),
            );
        }
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
            effects.add(if decal.translucent {
                // Alpha-blended, lit through the mark's vertex colour (linear light) and
                // the view exposure like the world (w = 1).
                EffectMaterial {
                    texture,
                    parameters: Vec4::new(0.001, 0., 1., 1.),
                    kind: 2,
                    ignore_z: false,
                }
            } else {
                // DecalModulate (mod2x).
                EffectMaterial {
                    texture,
                    parameters: Vec4::new(0.001, 1., 2., 0.),
                    kind: 4,
                    ignore_z: false,
                }
            }),
        );
    }
    let cookie = loaded.flashlight_cookie.as_deref();
    let (white, black_cube, flashlight, _) = rendering::fallback_images(images, cookie, TRILINEAR);
    let mut grenade_meshes = BTreeMap::new();
    let mut bases = BTreeMap::new();
    for (model, surface) in loaded
        .effects
        .models
        .iter()
        .flat_map(|(model, surfaces)| surfaces.iter().map(move |s| (model.clone(), s)))
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
            (&white, &black_cube, &flashlight),
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
        load_sparks: f64::NEG_INFINITY,
        launch_sparks: f64::NEG_INFINITY,
        shot_serial: 0,
        player_flash: f64::NEG_INFINITY,
        player_muzzle_lights: 0,
        elight_time: 0.,
        model_errors: loaded.effects.model_errors.clone(),
        draws: 0,
        grenade_draws: 0,
        mark_draws: 0,
        pose_mismatches: 0,
        observed_decals: 0,
        observed_grenades: 0,
        gibs: BTreeMap::new(),
        gib_state: BTreeMap::new(),
        gib_draws: 0,
    });
}
pub fn adopt(source: ProjectileVisuals, commands: &mut Commands) {
    commands.queue(move |world: &mut World| {
        let mut e = world.resource_mut::<Effects>();
        e.source = source;
        e.quads.clear();
        e.viewmodel_quads.clear();
        e.marks.clear();
        e.grenades.clear();
        e.stuck.clear();
        e.gibs.clear();
        e.gib_state.clear();
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
            if let (Some(bounds), Ok(mut cmd)) = (bounds, commands.get_entity(slot.entity)) {
                cmd.insert(bounds);
            }
            slot.previous = Some(quad.clone());
        }
        if let Ok(mut cmd) = commands.get_entity(slot.entity) {
            cmd.insert((MeshMaterial3d(material), Visibility::Visible));
        }
    }
    for slot in pool.iter().skip(used) {
        if let Ok(mut cmd) = commands.get_entity(slot.entity) {
            cmd.insert(Visibility::Hidden);
        }
    }
}
/// The active viewmodel's posed rig (same clip choice as rendering::animate) and the
/// SDK viewmodel sprites/beams as viewmodel-space quads.
fn viewmodel_quads(
    game: &Gameplay,
    effects: &mut Effects,
    sim: &Simulation,
    aspect: f32,
) -> Vec<hl2_simulation::projectile_visuals::Quad> {
    use hl2_simulation::viewmodel_effects as vm;
    let mut described = game.inventory.viewmodel_effects(game.scene.time);
    let new_load = (game.inventory.active == "weapon_crossbow")
        .then(|| {
            game.inventory
                .crossbow
                .load_effects
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .filter(|at| *at > effects.load_sparks);
    // AE_MUZZLEFLASH on the viewmodel's fire sequence (cycle 0 in the owned models).
    let elapsed_fire = (game.scene.time - game.inventory.animation_at).max(0.) as f32;
    let flash_due = (game.inventory.animation_at > effects.player_flash)
        .then_some(())
        .and_then(|()| {
            game.weapons
                .get(&game.inventory.active)
                .and_then(|w| {
                    game.world
                        .rigs
                        .get(&format!("{}#0", w.viewmodel.to_lowercase()))
                })
                .and_then(|rig| {
                    let clip = rig.clips.get(&game.inventory.animation)?;
                    clip.events
                        .iter()
                        .filter(|e| e.cycle * clip.duration() <= elapsed_fire)
                        .find_map(|e| {
                            use hl2_simulation::muzzle_effects as fx;
                            if e.name.eq_ignore_ascii_case("AE_MUZZLEFLASH") {
                                return fx::parse(&e.options);
                            }
                            // Obsolete CL_EVENT_MUZZLEFLASH0-3 (the RPG): attachment n.
                            let (kind, index) = fx::parse_obsolete(e.id, &e.options)?;
                            Some((kind, rig.attachments.get(index)?.name.clone()))
                        })
                })
        });
    let first_person_live = effects
        .source
        .muzzle
        .emitters
        .iter()
        .any(|e| e.first_person);
    // CBasePlayer::DoMuzzleFlash -> the viewmodel's ProcessMuzzleFlashEvent elight.
    let muzzle_light_due = game.inventory.muzzle_flash_count != effects.player_muzzle_lights;
    if described.sprites.is_empty()
        && described.beams.is_empty()
        && new_load.is_none()
        && flash_due.is_none()
        && !first_person_live
        && !muzzle_light_due
    {
        effects.player_muzzle_lights = game.inventory.muzzle_flash_count;
        return Vec::new();
    }
    let Some(rig) = game.weapons.get(&game.inventory.active).and_then(|w| {
        game.world
            .rigs
            .get(&format!("{}#0", w.viewmodel.to_lowercase()))
    }) else {
        return Vec::new();
    };
    let matrices = game.inventory.viewmodel_matrices(rig, game.scene.time);
    let fov = game.inventory.view_fov(game.scene.time);
    let framing = sim.viewmodel_framing(&game.inventory.active, aspect);
    let raw_viewmodel_fov =
        vm::viewmodel_fov(sim.viewmodel_fov, game.inventory.fov.default_fov, fov);
    let viewmodel_fov =
        crate::camera::viewmodel_projection(raw_viewmodel_fov, framing.fov_offset, 1000., aspect)
            .0
            .to_degrees();
    let world_fov = sim.world_camera_fov(sim.view_fov(fov), aspect).to_degrees();
    let offset = glam::Vec3::from_array(framing.offset);
    // DoLoadEffect -> "CrossbowLoad" (client c_weapon_crossbow.cpp): FX_ElectricSpark at
    // viewmodel attachment 1, a world position from FormatViewModelAttachment (right/up
    // offsets scaled by tan(fov / 2) / tan(viewmodel fov / 2), depth kept).
    let forward = glam::Vec3::from_array(crate::source_direction(sim.yaw, sim.pitch).to_array());
    let right = forward.cross(glam::Vec3::Z).normalize_or(glam::Vec3::Y);
    let up = right.cross(forward);
    let factor = (world_fov.to_radians() * 0.5).tan() / (viewmodel_fov.to_radians() * 0.5).tan();
    let eye = glam::Vec3::from_array(sim.eye().to_array());
    if muzzle_light_due {
        effects.player_muzzle_lights = game.inventory.muzzle_flash_count;
        // GetAttachment(1) on the viewmodel is FormatViewModelAttachment'ed into the world.
        if let Some(local) = vm::anchor_position(rig, &matrices, &vm::Anchor::Index(1))
            && viewmodel_fov > 0.
        {
            let world = eye + forward * local.x - right * local.y * factor + up * local.z * factor;
            let rng = &mut effects.rng;
            let mut random = || {
                *rng ^= *rng << 13;
                *rng ^= *rng >> 7;
                *rng ^= *rng << 17;
                (*rng >> 40) as f32 / (1u32 << 24) as f32
            };
            effects.source.muzzle.elights.muzzle_flash(
                VIEWMODEL_LIGHT_KEY,
                world,
                false,
                game.scene.time,
                &mut random,
            );
        }
    }
    if let Some(at) = new_load {
        effects.load_sparks = at;
        if let Some(local) = vm::anchor_position(rig, &matrices, &vm::Anchor::Index(1))
            && raw_viewmodel_fov > 0.
        {
            let local = local + offset;
            let world = eye + forward * local.x - right * local.y * factor + up * local.z * factor;
            effects.source.spawn_sparks(world, game.scene.time);
        }
    }
    // DoEffectLaunch: te GaussExplosion 4 units before the end point (random direction)
    // once, and for 0.1 s a CBeam orangelight1 from the end point to attachment 1 (width
    // 6.4 -> 12.8, brightness 255, noise 2). The world end point is moved into viewmodel
    // space by the inverse of FormatViewModelAttachment so it projects where it is.
    if game.inventory.active == "weapon_physcannon"
        && let Some((at, end)) = game.inventory.physcannon.last_launch
    {
        let shot = (end - eye).normalize_or(forward);
        if at > effects.launch_sparks {
            effects.launch_sparks = at;
            let r = &mut effects.rng;
            let mut unit = || {
                *r ^= *r << 13;
                *r ^= *r >> 7;
                *r ^= *r << 17;
                (*r >> 40) as f32 / (1u32 << 24) as f32 * 2. - 1.
            };
            let direction = glam::Vec3::new(unit(), unit(), unit());
            effects
                .source
                .spawn_gauss_sparks(end - shot * 4., direction, game.scene.time);
        }
        if game.scene.time - at < 0.1 && raw_viewmodel_fov > 0. {
            let d = end - eye;
            described.beams.push(vm::ViewmodelBeam {
                start: vm::Anchor::Local(
                    glam::Vec3::new(d.dot(forward), -d.dot(right) / factor, d.dot(up) / factor)
                        - offset,
                ),
                end: vm::Anchor::Index(1),
                material: "sprites/orangelight1",
                width: 6.4,
                end_width: (12.8, 12.8),
                brightness: 255.,
                color: [255, 255, 255],
                noise: (2., 2.),
                shade_in: false,
            });
        }
    }
    if let Some((kind, attachment)) = flash_due {
        effects.player_flash = game.inventory.animation_at;
        if let Some(frame) = rig
            .attachment(&attachment)
            .and_then(|a| rig.attachment_matrix(&matrices, a))
        {
            let rng = &mut effects.rng;
            let mut random = || {
                *rng ^= *rng << 13;
                *rng ^= *rng >> 7;
                *rng ^= *rng << 17;
                (*rng >> 40) as f32 / (1u32 << 24) as f32
            };
            effects.source.muzzle.flash(
                kind,
                true,
                frame,
                game.inventory.animation_at,
                &mut random,
            );
        }
    }
    // First-person flashes draw in viewmodel space at the viewmodel camera's scale.
    let mut flashes = effects.source.muzzle.particle_quads(
        true,
        game.scene.time,
        -glam::Vec3::Y,
        glam::Vec3::Z,
        vm::attachment_size_factor(viewmodel_fov, fov),
    );
    flashes.retain(|q| effects.source.sprites.contains_key(&q.material));
    let sprites = &effects.source.sprites;
    let rng = &mut effects.rng;
    let mut random = || {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 7;
        *rng ^= *rng << 17;
        (*rng >> 40) as f32 / (1u32 << 24) as f32
    };
    let mut quads = vm::viewmodel_quads(
        &described,
        &|anchor| vm::anchor_position(rig, &matrices, anchor),
        &|material| sprites.get(material).map(|s| f32::from(s.image.width)),
        &mut random,
        vm::attachment_size_factor(viewmodel_fov, world_fov),
    );
    quads.extend(flashes);
    quads
}
/// New muzzle flashes and tracers from the simulation's shot effect log.
/// The viewmodel's entity index in muzzle elight keys (any index no map entity uses).
const VIEWMODEL_LIGHT_KEY: usize = 0xF_FFFF;
impl Effects {
    /// Live entity lights (elights) as dworldlight_t for model lighting.
    pub(crate) fn elights(&self, floor: f32) -> Vec<modkit_core::lighting::WorldLight> {
        self.source
            .muzzle
            .elights
            .active()
            .map(|l| l.dynamic_light().world_light(floor))
            .collect()
    }
}
fn shot_effects(game: &Gameplay, effects: &mut Effects) {
    use crate::gameplay::ShotEffect;
    use hl2_simulation::muzzle_effects as fx;
    let Effects {
        source,
        rng,
        shot_serial,
        ..
    } = effects;
    let mut random = || {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 7;
        *rng ^= *rng << 17;
        (*rng >> 40) as f32 / (1u32 << 24) as f32
    };
    for (serial, at, effect) in &game.shot_effects {
        if *serial <= *shot_serial {
            continue;
        }
        *shot_serial = *serial;
        match effect {
            ShotEffect::NpcFlash(kind, frame) => {
                source.muzzle.flash(*kind, false, *frame, *at, &mut random)
            }
            ShotEffect::MuzzleLight(npc, origin, combine) => {
                source
                    .muzzle
                    .elights
                    .muzzle_flash(*npc, *origin, *combine, *at, &mut random)
            }
            ShotEffect::NpcTracer(start, end, ar2) => source.muzzle.tracer(if *ar2 {
                fx::npc_ar2_tracer(*start, *end, *at, &mut random)
            } else {
                fx::npc_tracer(*start, *end, *at, &mut random)
            }),
            ShotEffect::PlayerTracer(start, end, right, ar2) => source.muzzle.tracer(if *ar2 {
                fx::player_ar2_tracer(*start, *end, *right, *at, &mut random)
            } else {
                fx::player_tracer(*start, *end, *right, *at, &mut random)
            }),
            ShotEffect::Ar2Impact(origin, normal) => {
                source
                    .muzzle
                    .surface_quads
                    .push(fx::SurfaceQuad::ar2_impact(
                        *origin,
                        *normal,
                        *at,
                        &mut random,
                    ))
            }
        }
    }
}

pub fn present(
    mut commands: Commands,
    mut effects: ResMut<Effects>,
    game: Res<Gameplay>,
    sim: Res<Simulation>,
    frame: Option<Res<crate::video::GameFrame>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut draws: Query<(&mut Transform, &mut Visibility)>,
) {
    let (yaw, pitch) = sim.view_angles();
    let direction = glam::Vec3::from_array(crate::source_direction(yaw, pitch).to_array());
    // CL_DecayLights before this frame's new elights.
    let frametime = (game.scene.time - effects.elight_time).max(0.) as f32;
    effects.elight_time = game.scene.time;
    effects
        .source
        .muzzle
        .elights
        .decay(game.scene.time, frametime);
    shot_effects(&game, &mut effects);
    effects.source.frame(
        &game.projectiles,
        glam::Vec3::from_array(sim.view_origin().to_array()),
        direction,
        game.scene.time,
        !sim.world_running(),
        &sim.physics,
    );
    // C_EntityFlame's burning_character on burning props (control points: the prop's
    // center and world bounds).
    {
        let eye = glam::Vec3::from_array(sim.eye().to_array());
        let flames: Vec<(usize, hl2_simulation::pcf_runtime::ControlPoints)> = game
            .scene
            .fires
            .flames
            .keys()
            .filter_map(|&id| {
                let state = game.scene.states.get(id).filter(|s| !s.killed)?;
                let (min, max) = sim.physics.entity_world_bounds(id).unwrap_or((
                    state.origin - glam::Vec3::splat(16.),
                    state.origin + glam::Vec3::splat(16.),
                ));
                Some((
                    id,
                    hl2_simulation::pcf_runtime::ControlPoints {
                        origin: (min + max) * 0.5,
                        model_min: min,
                        model_max: max,
                    },
                ))
            })
            .collect();
        effects
            .source
            .burning(&flames, eye, direction, game.scene.time, sim.paused());
    }
    // Active stunsticks' halos (C_WeaponStunStick::DrawModel), new sizes every frame.
    if effects
        .source
        .sprites
        .contains_key(hl2_simulation::muzzle_effects::STUNSTICK_GLOW)
    {
        let right = direction.cross(glam::Vec3::Z).normalize_or(glam::Vec3::Y);
        let up = right.cross(direction).normalize_or(glam::Vec3::Z);
        let Effects { source, rng, .. } = &mut *effects;
        let mut random = || {
            *rng ^= *rng << 13;
            *rng ^= *rng >> 7;
            *rng ^= *rng << 17;
            (*rng >> 40) as f32 / (1u32 << 24) as f32
        };
        for frame in &game.stunstick_glows {
            source
                .quads
                .extend(hl2_simulation::muzzle_effects::stunstick_glow(
                    *frame,
                    right,
                    up,
                    &mut random,
                ));
        }
    }
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
    let aspect = crate::video::frame_aspect(frame.as_deref());
    let viewmodel = viewmodel_quads(&game, &mut effects, &sim, aspect);
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
                // Linear light; lit decals apply it after the sRGB decode.
                vec![
                    {
                        let [r, g, b] = mark.tint.map(|c| c.max(0.));
                        [r, g, b, 1.]
                    };
                    mark.vertices.len()
                ],
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
    present_grenades(&mut commands, &mut effects, &game, &mut draws);
    present_stuck_bolts(&mut commands, &mut effects, &game);
}

fn present_grenades(
    commands: &mut Commands,
    effects: &mut Effects,
    game: &Gameplay,
    draws: &mut Query<(&mut Transform, &mut Visibility)>,
) {
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
            } else if let Ok(mut cmd) = commands.get_entity(*entity) {
                cmd.insert(transform);
            }
        }
    }
}

fn present_stuck_bolts(commands: &mut Commands, effects: &mut Effects, game: &Gameplay) {
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
            if let Ok(mut cmd) = commands.get_entity(entity) {
                cmd.despawn();
            }
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
