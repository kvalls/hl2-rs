//! Read owned Source assets before constructing the Bevy app; no system performs file I/O.
use anyhow::{Context, Result, bail};
use modkit_core::World;
use source_assets::{
    bsp::Bsp,
    keyvalues::{self, Entry, Value},
    models,
    sky::UvTransform,
    vpk::{self, Vfs},
    vtf::{self, Image},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

const MAX_TEXTURE_DIMENSION: usize = 2048;
const MAX_TEXTURE_BYTES: usize = 64 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 512 * 1024 * 1024;
const MAX_MATERIALS: usize = 16384;
/// Animation frames, each with its authored mip chain (largest first).
pub type TextureFrames = Arc<Vec<Arc<Vec<Image>>>>;

#[derive(Clone, Debug)]
pub struct TwoTexture {
    pub base_frames: TextureFrames,
    pub second_frames: TextureFrames,
    pub second_path: String,
    pub frames: source_assets::proximity_material::Frames,
    pub scroll: source_assets::monitor_material::Animation,
}

pub struct LoadedMap {
    pub world: Arc<World>,
    pub gameplay: crate::gameplay::Gameplay,
    pub console: hl2_ui::console::Console,
    pub hud: hl2_ui::hud::WeaponHud,
    pub audio: crate::audio::PreparedAudio,
    pub sky: Option<source_assets::sky::Skybox>,
    pub effects: crate::effects::PreparedEffects,
    pub gaze: BTreeMap<String, source_assets::eyes::Gaze>,
    /// Facial flex data per NPC model asset key, read at setup.
    pub flexes: BTreeMap<String, Arc<source_assets::flexes::FaceModel>>,
    pub eyes: BTreeMap<(String, String), source_assets::eyes::Eyeball>,
    pub bsp: Bsp,
    pub revision: u32,
    pub materials: BTreeMap<String, MaterialData>,
    pub texture_errors: Vec<String>,
    pub models: serde_json::Value,
    /// The map's mounted content (its pak included) for runtime model loads.
    pub vfs: Arc<Vfs>,
}

pub fn map_metadata(loaded: &LoadedMap) -> serde_json::Value {
    let unique: BTreeMap<_, _> = loaded
        .materials
        .values()
        .filter_map(|m| {
            m.base_path
                .as_ref()
                .zip(m.base.as_ref())
                .map(|(k, i)| (k.clone(), i.rgba.len()))
        })
        .collect();
    serde_json::json!({"map":loaded.world.name,"bsp_revision":loaded.revision,"models":loaded.models,"texture_errors":loaded.texture_errors,"asset_warnings":loaded.world.warnings,
        "spawn_sky_visibility":format!("{:?}",loaded.bsp.sky_visibility(loaded.world.spawn().0)),
        "textures":{"source_materials":loaded.materials.len(),"unique_base_textures":unique.len(),"decoded_base_bytes":unique.values().sum::<usize>(),"decoded_base_budget":512*1024*1024,
            "owned_eye_dx8_fallbacks":loaded.materials.iter().filter(|(_,m)|m.eye_fallback).map(|(n,_)|n).collect::<Vec<_>>()}})
}

#[derive(Clone, Debug)]
pub struct MaterialData {
    pub base: Option<Arc<Image>>,
    /// Normalized VTF key shared by both decoded textures and GPU image handles.
    pub base_path: Option<String>,
    pub iris: Option<Arc<Image>>,
    pub iris_path: Option<String>,
    pub eye_fallback: bool,
    pub alpha_cutoff: Option<f32>,
    pub translucent: bool,
    pub additive: bool,
    pub two_sided: bool,
    pub opacity: f32,
    pub tint: [f32; 3],
    pub unlit: bool,
    pub camera: bool,
    pub camera_overlay: Option<Arc<Image>>,
    pub camera_overlay_path: Option<String>,
    pub camera_vertex_color: bool,
    pub camera_animation: source_assets::monitor_material::Animation,
    pub camera_color2: [f32; 3],
    /// VertexLitGeneric $halflambert.
    pub half_lambert: bool,
    /// Affine rows applied to the BSP/model base UVs before repeat sampling.
    pub uv_transform: [[f32; 3]; 2],
    /// LightmappedGeneric $envmap cubemap (HDR mode prefers the .hdr.vtf variant).
    pub envmap: Option<Arc<source_assets::vtf::Cube>>,
    pub envmap_path: Option<String>,
    pub envmap_tint: [f32; 3],
    pub envmap_contrast: [f32; 3],
    pub envmap_saturation: [f32; 3],
    pub fresnel_reflection: f32,
    /// $basealphaenvmapmask: the envmap is masked by 1 - base alpha.
    pub base_alpha_envmap_mask: bool,
    /// $envmapmask texture (RGB mask), or the $bumpmap: its normals perturb the reflection
    /// (`envmap_bump`) and, with $normalmapalphaenvmapmask, its alpha masks it
    /// (`envmap_mask_alpha`).
    pub envmap_mask: Option<Arc<Image>>,
    pub envmap_mask_path: Option<String>,
    pub envmap_mask_alpha: bool,
    pub envmap_bump: bool,
    /// $selfillum with $selfillumtint: diffuse = lerp(diffuse, tint * albedo, base alpha).
    pub self_illum: Option<[f32; 3]>,
    pub two_texture: Option<TwoTexture>,
}

impl Default for MaterialData {
    fn default() -> Self {
        Self {
            base: None,
            base_path: None,
            iris: None,
            iris_path: None,
            eye_fallback: false,
            alpha_cutoff: None,
            translucent: false,
            additive: false,
            two_sided: false,
            opacity: 1.,
            tint: [1.; 3],
            unlit: false,
            camera: false,
            camera_overlay: None,
            camera_overlay_path: None,
            camera_vertex_color: false,
            camera_animation: Default::default(),
            camera_color2: [1.; 3],
            half_lambert: false,
            uv_transform: UvTransform::default().rows(),
            envmap: None,
            envmap_path: None,
            envmap_tint: [1.; 3],
            envmap_contrast: [0.; 3],
            envmap_saturation: [1.; 3],
            fresnel_reflection: 1.,
            base_alpha_envmap_mask: false,
            envmap_mask: None,
            envmap_mask_path: None,
            envmap_mask_alpha: false,
            envmap_bump: false,
            self_illum: None,
            two_texture: None,
        }
    }
}

/// Build the thrown frag's Rapier body from w_grenade.phy (default); `--frag-swept-box`
/// keeps the legacy fitted swept box for comparisons.
pub static FRAG_RIGID_BODY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);
pub fn load(game: &Path, map: &str) -> Result<LoadedMap> {
    load_with_canvas(game, map, hl2_ui::canvas::Canvas::default(), true)
}
pub fn load_with_canvas(
    game: &Path,
    map: &str,
    canvas: hl2_ui::canvas::Canvas,
    new_game: bool,
) -> Result<LoadedMap> {
    let map = map.trim_end_matches(".bsp");
    if map.is_empty()
        || map.len() > 128
        || !map
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        bail!("map must be a bare Source map name");
    }
    let mut vfs = Vfs::mount(game).context("mount installed owned content")?;
    let bytes = vfs
        .read(&format!("maps/{map}.bsp"))?
        .with_context(|| format!("owned map {map}.bsp not found"))?;
    if bytes.len() > 512 * 1024 * 1024 {
        bail!("owned map exceeds 512 MiB read limit");
    }
    let bsp = Bsp::parse(&bytes).with_context(|| format!("decode {map}.bsp"))?;
    vfs.mount_pak(bsp.lump(40)).context("mount map pak")?;
    let mut world = bsp.world(map).context("decode Source world")?;
    world.warnings.extend(vfs.warnings.iter().cloned());
    normalize_bsp_render_winding(&mut world);
    // Static infodecals land on world brush faces only, before static props are merged.
    let mut decal_errors = Vec::new();
    add_static_decals(&mut world, &vfs, &mut decal_errors);
    // F4 / npc_create spawn pool: hidden NPCs every load-time system builds.
    hl2_simulation::ai::spawn::append_pool(&mut world);
    // CreateRagGib models (the zombie's legs), precached like the zombie's Precache.
    hl2_simulation::ai::spawn::append_gib_pool(&mut world);
    // CNPC_Citizen::SelectModel (citizen types, random heads), before models load.
    hl2_simulation::citizens::select_models(&mut world, hl2_simulation::ai::spawn::POOL_KEY);
    let model_report = models::append_models(&mut world, &vfs);
    let _gibs_without_physics = models::append_break_physics(&mut world, &vfs);
    // NPC weapon world models: their sequence events fire NPC weapons (DESIGN 14w).
    let weapon_models: Vec<String> = hl2_simulation::npc_weapons::definitions(&vfs)
        .map(|defs| {
            defs.into_values()
                .map(|d| d.world_model)
                .filter(|m| !m.is_empty())
                .collect()
        })
        .unwrap_or_default();
    for failure in models::append_rigs(&mut world, &vfs, &weapon_models) {
        world.warnings.push(failure);
    }
    // The player model's hitboxes take NPC bullets (CHL2_Player::Spawn).
    for failure in models::append_rigs(
        &mut world,
        &vfs,
        &[hl2_simulation::gameplay::PLAYER_MODEL.to_owned()],
    ) {
        world.warnings.push(failure);
    }
    // ... and drawn bone-merged in their owners' hands (CBaseCombatWeapon::Equip).
    for model in &weapon_models {
        let key = format!("{}#0", model.to_lowercase());
        if world.model_assets.contains_key(&key) {
            continue;
        }
        match models::read_model(&vfs, model, 0) {
            Ok(surfaces) => {
                world.model_assets.insert(key, surfaces);
            }
            Err(error) => world.warnings.push(format!("{model}: {error:#}")),
        }
    }
    // Dropped NPC weapons are VPhysics objects (CBaseCombatWeapon::FallInit): their
    // world models' .phy (a model without one is not dropped).
    for model in &weapon_models {
        let _ = models::append_model_physics(&mut world, &vfs, model);
    }
    // The thrown frag is a VPhysics object (CGrenadeFrag::CreateVPhysics): a Rapier body
    // from w_grenade.phy (12-sided prism as a round cylinder, the .phy's IVP inertia) and
    // SDK spawn orientation. Session 13 replay against five native throws (lob, roll,
    // overhand, plaza, wall) fits as well as or better than the old fitted swept box.
    if FRAG_RIGID_BODY.load(std::sync::atomic::Ordering::Relaxed)
        && let Err(error) =
            models::append_model_physics(&mut world, &vfs, hl2_simulation::projectiles::FRAG_MODEL)
    {
        world.warnings.push(format!(
            "{}: {error:#}",
            hl2_simulation::projectiles::FRAG_MODEL
        ));
    }
    let mut gameplay =
        crate::gameplay::Gameplay::load_with_campaign(world, &vfs, bsp.revision, new_game)?;
    let world = gameplay.world.clone();
    let effects = crate::effects::PreparedEffects::load(&vfs, &world);
    let mut names = rendered_material_names(&world);
    names.extend(
        effects
            .models
            .values()
            .flatten()
            .map(|s| s.material.clone()),
    );
    for model in &weapon_models {
        if let Some(surfaces) = world
            .model_assets
            .get(&format!("{}#0", model.to_lowercase()))
        {
            names.extend(surfaces.iter().map(|s| s.material.clone()));
        }
    }
    for weapon in gameplay.weapons.values() {
        for skin in 0..2 {
            if let Some(surfaces) = world
                .model_assets
                .get(&format!("{}#{skin}", weapon.viewmodel.to_lowercase()))
            {
                names.extend(surfaces.iter().map(|s| s.material.clone()));
            }
        }
    }
    if names.len() > MAX_MATERIALS {
        bail!("map exceeds {MAX_MATERIALS} unique material limit");
    }
    let mut materials = BTreeMap::new();
    let mut texture_errors = decal_errors;
    let mut decoded_bytes = 0usize;
    let mut texture_cache = BTreeMap::new();
    let mut frame_cache = BTreeMap::new();
    for name in names {
        let material = load_material(
            &vfs,
            &name,
            &mut texture_cache,
            &mut frame_cache,
            &mut decoded_bytes,
            &mut texture_errors,
        );
        materials.insert(name, material);
    }
    let mut eyes = BTreeMap::new();
    let mut gaze = BTreeMap::new();
    let mut flexes = BTreeMap::new();
    for (key, surfaces) in &world.model_assets {
        let model = key.split('#').next().unwrap_or(key);
        let mut face = None;
        if world.model_instances.iter().any(|i| {
            i.asset_key() == *key
                && i.entity
                    .is_some_and(|id| world.entities[id].class().starts_with("npc_"))
        }) {
            match vfs
                .read(model)
                .and_then(|d| d.ok_or_else(|| anyhow::anyhow!("model missing")))
                .and_then(|d| source_assets::flexes::read_flexes(&d))
            {
                Ok(model) if !model.meshes.is_empty() => {
                    face = Some(model);
                }
                Ok(_) => {}
                Err(e) => texture_errors.push(format!("{model}: facial flexes: {e:#}")),
            }
            match source_assets::eyes::load_gaze(&vfs, model) {
                Ok(metadata) => {
                    gaze.insert(key.clone(), metadata);
                }
                Err(e) => texture_errors.push(format!("{model}: gaze attachment metadata: {e:#}")),
            }
        }
        match source_assets::eyes::load(&vfs, model) {
            Ok(records) => {
                if let Some(flex) = face.take() {
                    flexes.insert(
                        key.clone(),
                        Arc::new(source_assets::flexes::FaceModel {
                            flex,
                            eyes: records.clone(),
                        }),
                    );
                }
                for eye in records {
                    if let Some(surface) = surfaces.get(eye.surface) {
                        eyes.insert((key.clone(), surface.material.clone()), eye);
                    } else {
                        texture_errors.push(format!("{model}: eyeball mesh missing"));
                    }
                }
            }
            Err(e) => texture_errors.push(format!("{model}: eyeball metadata: {e:#}")),
        }
    }
    let hud = hl2_ui::hud::WeaponHud::load(&vfs, canvas)?;
    let console = hl2_ui::console::Console::load(&vfs, hud.canvas.clone());
    let mut audio = crate::audio::PreparedAudio::load(&vfs, &gameplay);
    for (wave, sentence, seconds) in audio.sentences.drain(..) {
        gameplay
            .scene
            .lipsync
            .add_sentence(&wave, sentence, seconds);
    }
    let sky = match source_assets::sky::load(&vfs, &world.entities, 512) {
        Ok(sky) => sky,
        Err(e) => {
            texture_errors.push(format!("2D sky assets: {e:#}"));
            None
        }
    };
    Ok(LoadedMap {
        console,
        effects,
        eyes,
        gaze,
        flexes,
        sky,
        audio,
        hud,
        gameplay,
        revision: bsp.revision,
        bsp,
        world,
        materials,
        texture_errors,
        models: serde_json::to_value(model_report)?,
        vfs: Arc::new(vfs),
    })
}

/// A model loaded after the map (late precache, DESIGN 14r part B): its world tables
/// (`world`: one prop_physics instance through the map's model loader: mesh, rig,
/// .phy solid/pieces, propdata, breaks, illumination), the materials not already
/// loaded, and its studio hull.
pub struct PreparedModel {
    pub world: World,
    pub materials: BTreeMap<String, MaterialData>,
    pub hull: (glam::Vec3, glam::Vec3),
    /// Its breakable gibs/chunks by render key (the map preloads these per prop).
    pub gibs: BTreeMap<String, Vec<modkit_core::Surface>>,
}
/// Load a model on a worker thread (no GPU work): `known` materials are skipped.
pub fn prepare_model(
    vfs: &Vfs,
    model: &str,
    class: &str,
    known: &std::collections::BTreeSet<String>,
) -> Result<PreparedModel> {
    let mut world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), class.into()),
                ("model".into(), model.into()),
            ],
        }],
        ..Default::default()
    };
    let report = models::append_models(&mut world, vfs);
    if world.model_assets.is_empty() {
        bail!("{model}: {}", report.errors.join("; "));
    }
    let _ = models::append_break_physics(&mut world, vfs);
    let rig_failures = models::append_rigs(&mut world, vfs, &[]);
    world.warnings.extend(rig_failures);
    let hull = models::read_hull(vfs, model)?;
    let mut gibs = BTreeMap::new();
    for (path, skin) in crate::effects::gib_models(&world) {
        match crate::effects::read_effect_model(vfs, &path, skin) {
            Ok(surfaces) => {
                gibs.insert(hl2_simulation::gibs::render_key(&path, skin), surfaces);
            }
            Err(e) => world.warnings.push(format!("{path}: {e:#}")),
        }
    }
    let mut materials = BTreeMap::new();
    let (mut cache, mut frames, mut decoded, mut errors) =
        (BTreeMap::new(), BTreeMap::new(), 0usize, Vec::new());
    let names: std::collections::BTreeSet<String> = world
        .model_assets
        .values()
        .chain(gibs.values())
        .flatten()
        .map(|s| s.material.clone())
        .filter(|m| !known.contains(m))
        .collect();
    for name in names {
        let material = load_material(
            vfs,
            &name,
            &mut cache,
            &mut frames,
            &mut decoded,
            &mut errors,
        );
        materials.insert(name, material);
    }
    world.warnings.extend(errors);
    Ok(PreparedModel {
        world,
        materials,
        hull,
        gibs,
    })
}

/// CDecal::StaticDecal: infodecals without a targetname are applied at map load (named ones
/// wait for an input and are not handled yet). Size is the decal's base texture in world
/// units times `$decalscale`.
fn add_static_decals(world: &mut World, vfs: &Vfs, errors: &mut Vec<String>) {
    let decals: Vec<_> = world
        .entities
        .iter()
        .filter(|e| e.class() == "infodecal" && e.get("targetname").is_none_or(str::is_empty))
        .filter_map(|e| {
            Some((
                e.origin(),
                e.get("texture")?.replace('\\', "/").to_lowercase(),
            ))
        })
        .collect();
    let mut sizes = BTreeMap::<String, Option<glam::Vec2>>::new();
    let mut added = Vec::new();
    for (origin, material) in decals {
        let size =
            *sizes
                .entry(material.clone())
                .or_insert_with(|| match decal_size(vfs, &material) {
                    Ok(size) => Some(size),
                    Err(e) => {
                        errors.push(format!("infodecal {material}: {e:#}"));
                        None
                    }
                });
        if let Some(size) = size {
            added.extend(modkit_core::decals::static_decal(
                &world.surfaces,
                origin,
                size,
                5.,
                &material,
            ));
        }
    }
    world.surfaces.extend(added);
}
fn decal_size(vfs: &Vfs, material: &str) -> Result<glam::Vec2> {
    let definition = definition(vfs, material, 0)?;
    let base = definition
        .properties
        .get("$basetexture")
        .context("decal has no $basetexture")?;
    let path = asset_path(base, ".vtf")?;
    let data = vfs
        .read(&path)?
        .with_context(|| format!("VTF absent: {path}"))?;
    if data.get(..4) != Some(b"VTF\0") || data.len() < 20 {
        bail!("not a VTF: {path}");
    }
    let width = f32::from(u16::from_le_bytes([data[16], data[17]]));
    let height = f32::from(u16::from_le_bytes([data[18], data[19]]));
    let scale = scalar(&definition.properties, "$decalscale", 1.)?;
    Ok(glam::Vec2::new(width, height) * scale)
}
/// BSP surfedges retain clockwise triangles, but vmdl's Strip::indices already
/// reverses model triangles to CCW. Normalize BSP render copies before static
/// props are merged, keeping original collision copies and model data intact.
fn normalize_bsp_render_winding(world: &mut World) {
    for surface in world.surfaces.iter_mut().chain(
        world
            .brush_models
            .iter_mut()
            .flat_map(|model| &mut model.surfaces),
    ) {
        for triangle in surface.indices.as_chunks_mut::<3>().0 {
            triangle.swap(1, 2);
        }
    }
}

/// Initial static visibility only. Runtime I/O and animated render effects are separate.
pub(crate) fn visible_entity(world: &World, id: usize) -> bool {
    world.entities.get(id).is_some_and(|entity| {
        !entity.class().starts_with("trigger_") && !entity.class().starts_with("func_areaportal")
    })
}

fn rendered_material_names(world: &World) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut append = |surfaces: &[modkit_core::Surface]| {
        names.extend(
            surfaces
                .iter()
                .filter(|surface| !surface.indices.is_empty())
                .map(|surface| surface.material.clone()),
        );
    };
    // Includes displacements and already-transformed static props. terrain is a
    // duplicate collision representation, and unused model_assets are not draws.
    append(&world.surfaces);
    append(&world.overlays);
    for (id, entity) in world.entities.iter().enumerate() {
        if !visible_entity(world, id) {
            continue;
        }
        if let Some(brush) = entity
            .get("model")
            .and_then(|name| name.strip_prefix('*'))
            .and_then(|id| id.parse::<usize>().ok())
            .and_then(|id| world.brush_models.iter().find(|model| model.id == id))
        {
            append(&brush.surfaces);
        }
    }
    for instance in &world.model_instances {
        if !instance.entity.is_some_and(|id| visible_entity(world, id)) {
            continue;
        }
        if let Some(surfaces) = world.model_assets.get(&instance.asset_key()) {
            append(surfaces);
        }
    }
    names
}

struct Definition {
    shader: String,
    properties: BTreeMap<String, String>,
    proxies: Vec<Entry>,
}

fn asset_path(name: &str, extension: &str) -> Result<String> {
    if name.is_empty() || name.len() > 1024 {
        bail!("material asset name must contain 1..1024 bytes");
    }
    let name = vpk::normalize(name)?;
    let name = name
        .trim_start_matches("materials/")
        .trim_end_matches(extension);
    Ok(format!("materials/{name}{extension}"))
}

/// Direct material vars, with retail MaterialSystem key conditionals (`test?$var`, optional
/// `!`): a key whose test fails is skipped, and a passing one replaces the plain value.
fn direct_properties(entries: &[Entry]) -> BTreeMap<String, String> {
    let mut properties = BTreeMap::new();
    let mut conditional = Vec::new();
    for entry in entries {
        let Some(text) = entry.text() else {
            continue;
        };
        let key = entry.key.to_lowercase();
        match key.split_once('?') {
            Some((test, name)) if !test.is_empty() => {
                if material_condition(test) {
                    conditional.push((name.to_owned(), text.to_owned()));
                }
            }
            _ => {
                properties.insert(key, text.to_owned());
            }
        }
    }
    properties.extend(conditional);
    properties
}

/// Tests as evaluated on a DX9, sRGB-capable renderer in HDR mode (mat_hdr_level 2, the
/// Source default and the owner's setting). Unknown tests are false, as in retail (which
/// also warns).
fn material_condition(test: &str) -> bool {
    let (negate, test) = test.strip_prefix('!').map_or((false, test), |t| (true, t));
    let value = matches!(test, "srgb" | "hdr");
    value != negate
}

/// DirectX-level blocks inside a material (`">=dx90_20b" { ... }`, `"<dx90" { ... }`):
/// whether the block applies on a DX level 95 renderer (SM3 HDR, the owner's native
/// setting). `dx90_20b` (ps2.0b) sits between levels 90 and 95. Other keys are not
/// DX-level blocks (None).
fn dx_level_block(key: &str) -> Option<bool> {
    const LEVEL: f32 = 95.;
    let key = key.to_ascii_lowercase();
    let (op, rest) = ["<=", ">=", "<", ">"]
        .into_iter()
        .find_map(|op| key.strip_prefix(op).map(|rest| (op, rest)))
        .unwrap_or(("=", key.as_str()));
    let digits = rest.strip_prefix("dx")?;
    let (number, b20) = digits
        .strip_suffix("_20b")
        .map_or((digits, false), |n| (n, true));
    let level = number.parse::<f32>().ok()? + if b20 { 0.5 } else { 0. };
    Some(match op {
        "<=" => LEVEL <= level,
        ">=" => LEVEL >= level,
        "<" => LEVEL < level,
        ">" => LEVEL > level,
        _ => LEVEL == level,
    })
}
/// Direct material vars plus the DX-level blocks that apply (their vars win).
fn material_properties(entries: &[Entry]) -> BTreeMap<String, String> {
    let mut properties = direct_properties(entries);
    for entry in entries {
        if matches!(entry.value, Value::Block(_)) && dx_level_block(&entry.key) == Some(true) {
            properties.extend(direct_properties(entry.children()));
        }
    }
    properties
}

// Reuse the bounded KeyValues decoder, preserving direct material parameters and
// Patch insert/replace semantics. DX-level blocks are evaluated for level 95; the authored
// Eyes_dx8 fallback is selected for EyeRefract; other shader fallbacks/proxies remain
// unsupported.
fn definition(vfs: &Vfs, name: &str, depth: usize) -> Result<Definition> {
    if depth > 8 {
        bail!("VMT include cycle/depth limit");
    }
    let path = asset_path(name, ".vmt")?;
    let data = vfs
        .read(&path)?
        .with_context(|| format!("VMT absent: {path}"))?;
    if data.len() > 1024 * 1024 {
        bail!("VMT exceeds 1 MiB limit: {path}");
    }
    let entries = keyvalues::parse(&keyvalues::decode_text(&data)?)?;
    if entries.len() != 1 || !matches!(entries[0].value, Value::Block(_)) {
        bail!("VMT must contain one material block: {path}");
    }
    let root = &entries[0];
    if root.key.eq_ignore_ascii_case("eyerefract")
        && let Some(fallback) = root.get("Eyes_dx8")
    {
        let mut properties = direct_properties(root.children());
        properties.extend(direct_properties(fallback.children()));
        return Ok(Definition {
            shader: "eyes_dx8".into(),
            proxies: Vec::new(),
            properties,
        });
    }
    if !root.key.eq_ignore_ascii_case("patch") {
        return Ok(Definition {
            shader: root.key.clone(),
            properties: material_properties(root.children()),
            proxies: root
                .get("Proxies")
                .map_or(vec![], |p| p.children().to_vec()),
        });
    }
    let include = root
        .get("include")
        .and_then(Entry::text)
        .context("VMT Patch has no include")?;
    let mut inherited = definition(vfs, include, depth + 1)?;
    for operation in ["insert", "replace"] {
        if let Some(block) = root.get(operation) {
            if !matches!(block.value, Value::Block(_)) {
                bail!("VMT Patch {operation} must be a block");
            }
            for (key, value) in direct_properties(block.children()) {
                if inherited.properties.contains_key(&key) == (operation == "replace") {
                    inherited.properties.insert(key, value);
                }
            }
        }
    }
    Ok(inherited)
}

fn scalar(properties: &BTreeMap<String, String>, key: &str, default: f32) -> Result<f32> {
    let Some(value) = properties.get(key) else {
        return Ok(default);
    };
    let number: f32 = value
        .parse()
        .with_context(|| format!("invalid VMT {key}"))?;
    if !number.is_finite() {
        bail!("nonfinite VMT {key}");
    }
    Ok(number)
}

fn metadata(definition: &Definition) -> Result<MaterialData> {
    let p = &definition.properties;
    let mut material = MaterialData {
        translucent: scalar(p, "$translucent", 0.)?.trunc() != 0.,
        additive: scalar(p, "$additive", 0.)?.trunc() != 0.,
        two_sided: scalar(p, "$nocull", 0.)?.trunc() != 0.,
        half_lambert: scalar(p, "$halflambert", 0.)?.trunc() != 0.,
        opacity: scalar(p, "$alpha", 1.)?.clamp(0., 1.),
        unlit: definition.shader.eq_ignore_ascii_case("unlitgeneric"),
        eye_fallback: definition.shader.eq_ignore_ascii_case("eyes_dx8"),
        ..Default::default()
    };
    if scalar(p, "$alphatest", 0.)?.trunc() != 0. {
        let reference = scalar(p, "$alphatestreference", 0.5)?;
        material.alpha_cutoff = Some(if reference > 0. { reference } else { 0.5 });
    }
    if let Some(color) = p.get("$color") {
        let bytes = color.trim().starts_with('{');
        let values = color
            .trim()
            .trim_matches(['[', ']', '{', '}'])
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("invalid VMT $color")?;
        if values.len() != 3 || values.iter().any(|v| !v.is_finite()) {
            bail!("VMT $color must contain three finite channels");
        }
        material.tint = std::array::from_fn(|i| values[i] / if bytes { 255. } else { 1. });
    }
    if let Some(transform) = p.get("$basetexturetransform") {
        material.uv_transform = UvTransform::parse(transform)?.rows();
    }
    Ok(material)
}

/// Envmap parameters with SDK lightmappedgeneric_dx9_helper.cpp defaults. Contrast and
/// saturation are float3 in the shader (one value fills all channels); the tint is used as
/// authored (not gamma-converted).
fn envmap_parameters(definition: &Definition, material: &mut MaterialData) -> Result<()> {
    let p = &definition.properties;
    let vector = |key: &str, default: f32| -> Result<[f32; 3]> {
        p.get(key).map_or(Ok([default; 3]), |v| {
            vector3(v).with_context(|| format!("invalid VMT {key}"))
        })
    };
    material.envmap_tint = vector("$envmaptint", 1.)?;
    material.envmap_contrast = vector("$envmapcontrast", 0.)?;
    material.envmap_saturation = vector("$envmapsaturation", 1.)?;
    material.fresnel_reflection = scalar(p, "$fresnelreflection", 1.)?;
    material.base_alpha_envmap_mask = scalar(p, "$basealphaenvmapmask", 0.)?.trunc() != 0.;
    Ok(())
}

/// A VMT vector: `[r g b]` floats, `{r g b}` bytes, or one scalar for all channels.
fn vector3(value: &str) -> Result<[f32; 3]> {
    let value = value.trim();
    let bytes = value.starts_with('{');
    let values = value
        .trim_matches(['[', ']', '{', '}'])
        .split_whitespace()
        .map(str::parse::<f32>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let values = match values[..] {
        [v] => [v; 3],
        [r, g, b] => [r, g, b],
        _ => bail!("expected one or three channels"),
    };
    if values.iter().any(|v| !v.is_finite()) {
        bail!("nonfinite channel");
    }
    Ok(values.map(|v| v / if bytes { 255. } else { 1. }))
}

/// $selfillum tint for LightmappedGeneric/VertexLitGeneric (SDK dx9 helpers): the base alpha
/// selects self-illumination, so an opaque base texture clears the flag. $selfillummask and
/// $selfillumfresnel (alternate VertexLitGeneric modes) are not supported.
fn self_illum(definition: &Definition, base: Option<&Image>) -> Result<Option<[f32; 3]>> {
    let p = &definition.properties;
    let shader = definition.shader.to_ascii_lowercase();
    if !matches!(shader.as_str(), "lightmappedgeneric" | "vertexlitgeneric")
        || scalar(p, "$selfillum", 0.)?.trunc() == 0.
        || !base.is_some_and(|base| base.translucent)
    {
        return Ok(None);
    }
    p.get("$selfillumtint")
        .map_or(Ok([1.; 3]), |tint| {
            vector3(tint).context("invalid VMT $selfillumtint")
        })
        .map(Some)
}

/// How a texture shapes a LightmappedGeneric envmap (SDK lightmappedgeneric_dx9_helper.cpp).
#[derive(Clone, Copy, Debug, PartialEq)]
struct EnvmapMask {
    /// The texture's alpha masks the envmap ($normalmapalphaenvmapmask).
    alpha: bool,
    /// The texture is the $bumpmap; its normals perturb the reflection.
    bump: bool,
}

/// A $bumpmap undefines $envmapmask; its normals then shape the reflection (except $ssbump,
/// whose encoding is not decoded yet) and only $normalmapalphaenvmapmask masks by its alpha.
fn envmap_mask(definition: &Definition) -> Option<(&str, EnvmapMask)> {
    let p = &definition.properties;
    let flag = |key: &str| scalar(p, key, 0.).is_ok_and(|v| v.trunc() != 0.);
    match p.get("$bumpmap") {
        Some(bump) => {
            let mask = EnvmapMask {
                alpha: flag("$normalmapalphaenvmapmask"),
                bump: !flag("$ssbump"),
            };
            (mask.alpha || mask.bump).then_some((bump.as_str(), mask))
        }
        None => p.get("$envmapmask").map(|mask| {
            (
                mask.as_str(),
                EnvmapMask {
                    alpha: false,
                    bump: false,
                },
            )
        }),
    }
}

/// The $envmap cubemap of a LightmappedGeneric material. `env_cubemap` (resolved by the
/// engine at runtime for entities) is not supported yet; map compiles bake world faces into
/// patch VMTs naming a sample. HDR mode loads the .hdr.vtf variant when present.
fn load_envmap(
    vfs: &Vfs,
    definition: &Definition,
    decoded_bytes: &mut usize,
) -> Result<Option<(String, source_assets::vtf::Cube)>> {
    if !definition.shader.eq_ignore_ascii_case("lightmappedgeneric") {
        return Ok(None);
    }
    let Some(name) = definition.properties.get("$envmap") else {
        return Ok(None);
    };
    if name.eq_ignore_ascii_case("env_cubemap") {
        return Ok(None);
    }
    let path = asset_path(name, ".vtf")?;
    let hdr = format!("{}.hdr.vtf", path.trim_end_matches(".vtf"));
    let (path, data) = match vfs.read(&hdr)? {
        Some(data) => (hdr, data),
        None => {
            let data = vfs
                .read(&path)?
                .with_context(|| format!("envmap absent: {path}"))?;
            (path, data)
        }
    };
    if data.len() > MAX_TEXTURE_BYTES {
        bail!("encoded envmap exceeds 64 MiB limit: {path}");
    }
    let cube = source_assets::vtf::decode_cube(&data, MAX_TEXTURE_DIMENSION)
        .with_context(|| format!("decode {path}"))?;
    let total = decoded_bytes
        .checked_add(cube.faces.iter().map(Vec::len).sum())
        .context("decoded VTF byte count overflow")?;
    if total > MAX_DECODED_BYTES {
        bail!("map decoded texture budget exceeds 512 MiB");
    }
    *decoded_bytes = total;
    Ok(Some((path, cube)))
}

fn load_material(
    vfs: &Vfs,
    name: &str,
    cache: &mut BTreeMap<String, Arc<Image>>,
    frame_cache: &mut BTreeMap<String, TextureFrames>,
    decoded_bytes: &mut usize,
    errors: &mut Vec<String>,
) -> MaterialData {
    let definition = match definition(vfs, name, 0) {
        Ok(definition) => definition,
        Err(error) => {
            errors.push(format!("{name}: {error:#}"));
            return MaterialData::default();
        }
    };
    let mut material = match metadata(&definition) {
        Ok(material) => material,
        Err(error) => {
            errors.push(format!("{name}: {error:#}"));
            MaterialData::default()
        }
    };
    if definition
        .properties
        .get("$basetexture")
        .is_some_and(|base| base.eq_ignore_ascii_case("_rt_camera"))
    {
        material.camera = true;
        material.unlit = true;
        material.base_path = Some("_rt_camera".into());
        material.camera_vertex_color =
            scalar(&definition.properties, "$vertexcolor", 0.).unwrap_or(0.) != 0.;
        material.camera_animation =
            source_assets::monitor_material::Animation::parse_with_properties(
                &definition.proxies,
                &definition.properties,
            );
        if let Some(color) = definition.properties.get("$color2") {
            let color = color
                .trim()
                .trim_matches(['[', ']'])
                .split_whitespace()
                .map(str::parse::<f32>)
                .collect::<std::result::Result<Vec<_>, _>>();
            match color {
                Ok(color) if color.len() == 3 && color.iter().all(|v| v.is_finite()) => {
                    material.camera_color2.copy_from_slice(&color)
                }
                _ => errors.push(format!("{name}: invalid monitor $color2")),
            }
        }
        if let Some(second) = definition.properties.get("$texture2") {
            let texture = (|| -> Result<_> {
                let path = asset_path(second, ".vtf")?;
                let image = cached_texture(cache, decoded_bytes, &path, || {
                    vfs.read(&path)?.context("owned monitor overlay absent")
                })?;
                Ok((path, image))
            })();
            match texture {
                Ok((path, image)) => {
                    material.camera_overlay_path = Some(path);
                    material.camera_overlay = Some(image);
                }
                Err(e) => errors.push(format!("{name}: monitor overlay: {e:#}")),
            }
        }
        return material;
    }
    if definition.shader.eq_ignore_ascii_case("UnlitTwoTexture") {
        let textures = (|| -> Result<_> {
            let frames = source_assets::proximity_material::Frames::parse(
                &definition.proxies,
                &definition.properties,
            )?;
            let mut load = |key: &str| -> Result<(String, TextureFrames)> {
                let path = asset_path(
                    definition
                        .properties
                        .get(key)
                        .context("two-texture material texture absent")?,
                    ".vtf",
                )?;
                let cache_key = format!(
                    "{path}#{}",
                    if frames.active() { "frames" } else { "single" }
                );
                if let Some(images) = frame_cache.get(&cache_key) {
                    return Ok((path, images.clone()));
                }
                let data = vfs.read(&path)?.context("two-texture VTF absent")?;
                if data.len() > MAX_TEXTURE_BYTES {
                    bail!("encoded VTF exceeds 64 MiB");
                }
                // Source samples the authored mips with distance; the shield frames'
                // smaller mips are part of their far look.
                let mut images = vtf::decode_frame_mips(
                    &data,
                    MAX_TEXTURE_DIMENSION,
                    MAX_DECODED_BYTES.saturating_sub(*decoded_bytes),
                )?;
                if !frames.active() {
                    images.truncate(1);
                }
                let bytes = images.iter().flatten().map(|i| i.rgba.len()).sum::<usize>();
                if bytes > MAX_DECODED_BYTES.saturating_sub(*decoded_bytes) {
                    bail!("map decoded texture budget exceeds 512 MiB");
                }
                *decoded_bytes += bytes;
                let images = Arc::new(images.into_iter().map(Arc::new).collect::<Vec<_>>());
                frame_cache.insert(cache_key, images.clone());
                Ok((path, images))
            };
            let (base_path, base_frames) = load("$basetexture")?;
            let (second_path, second_frames) = load("$texture2")?;
            let scroll = source_assets::monitor_material::Animation::parse(&definition.proxies);
            Ok((
                base_path,
                TwoTexture {
                    base_frames,
                    second_frames,
                    second_path,
                    frames,
                    scroll,
                },
            ))
        })();
        match textures {
            Ok((path, textures)) => {
                material.base_path = Some(path);
                material.base = textures
                    .base_frames
                    .first()
                    .and_then(|chain| chain.first())
                    .map(|top| Arc::new(top.clone()));
                material.two_texture = Some(textures);
                material.unlit = true;
            }
            Err(error) => errors.push(format!("{name}: two-texture: {error:#}")),
        }
        return material;
    }
    let image = (|| -> Result<(String, Arc<Image>)> {
        let base = definition
            .properties
            .get("$basetexture")
            .or_else(|| definition.properties.get("$refracttinttexture"))
            .context("VMT has no supported base texture")?;
        let path = asset_path(base, ".vtf")?;
        let image = cached_texture(cache, decoded_bytes, &path, || {
            vfs.read(&path)?
                .with_context(|| format!("VTF absent: {path}"))
        })?;
        Ok((path, image))
    })();
    match image {
        Ok((path, image)) => {
            material.base_path = Some(path);
            material.base = Some(image);
        }
        Err(error) => errors.push(format!("{name}: {error:#}")),
    }
    match self_illum(&definition, material.base.as_deref()) {
        Ok(tint) => material.self_illum = tint,
        Err(error) => errors.push(format!("{name}: selfillum: {error:#}")),
    }
    let envmap = envmap_parameters(&definition, &mut material)
        .and_then(|()| load_envmap(vfs, &definition, decoded_bytes));
    let envmap = envmap.and_then(|cube| {
        let Some(cube) = cube else {
            return Ok(None);
        };
        let mask = envmap_mask(&definition)
            .map(|(texture, kind)| -> Result<_> {
                let path = asset_path(texture, ".vtf")?;
                let image = cached_texture(cache, decoded_bytes, &path, || {
                    vfs.read(&path)?
                        .with_context(|| format!("envmap mask absent: {path}"))
                })?;
                Ok((path, image, kind))
            })
            .transpose()?;
        Ok(Some((cube, mask)))
    });
    match envmap {
        Ok(Some(((path, cube), mask))) => {
            material.envmap_path = Some(path);
            material.envmap = Some(Arc::new(cube));
            if let Some((path, image, kind)) = mask {
                material.envmap_mask_path = Some(path);
                material.envmap_mask = Some(image);
                material.envmap_mask_alpha = kind.alpha;
                material.envmap_bump = kind.bump;
            }
            // SDK ShaderInit: an opaque base texture clears BASEALPHAENVMAPMASK.
            if !material.base.as_ref().is_some_and(|base| base.translucent) {
                material.base_alpha_envmap_mask = false;
            }
        }
        Ok(None) => {}
        // Without its mask an envmap would be far too strong, so it is dropped entirely.
        Err(error) => errors.push(format!("{name}: envmap: {error:#}")),
    }
    if definition.shader.eq_ignore_ascii_case("eyes") || material.eye_fallback {
        let iris = (|| -> Result<(String, Arc<Image>)> {
            let path = asset_path(
                definition
                    .properties
                    .get("$iris")
                    .context("Eyes VMT missing $iris")?,
                ".vtf",
            )?;
            let image = cached_texture(cache, decoded_bytes, &path, || {
                vfs.read(&path)?.context("owned iris VTF absent")
            })?;
            Ok((path, image))
        })();
        match iris {
            Ok((path, image)) => {
                material.iris_path = Some(path);
                material.iris = Some(image);
            }
            Err(e) => errors.push(format!("{name}: iris: {e:#}")),
        }
    }
    material
}

fn cached_texture(
    cache: &mut BTreeMap<String, Arc<Image>>,
    decoded_bytes: &mut usize,
    path: &str,
    read: impl FnOnce() -> Result<Vec<u8>>,
) -> Result<Arc<Image>> {
    if let Some(image) = cache.get(path) {
        return Ok(Arc::clone(image));
    }
    let data = read()?;
    if data.len() > MAX_TEXTURE_BYTES {
        bail!("encoded VTF exceeds 64 MiB limit: {path}");
    }
    // The VTF adapter picks an existing mip rather than resampling. Reject
    // a texture with no mip within our cap before allocating its RGBA pixels.
    let width = usize::from(u16::from_le_bytes(
        data.get(16..18)
            .context("truncated VTF width")?
            .try_into()?,
    ));
    let height = usize::from(u16::from_le_bytes(
        data.get(18..20)
            .context("truncated VTF height")?
            .try_into()?,
    ));
    let mips = usize::from(*data.get(56).context("truncated VTF mip count")?);
    if !(1..=15).contains(&mips) {
        bail!("invalid VTF mip count: {path}");
    }
    if (width.max(height) >> (mips - 1)) > MAX_TEXTURE_DIMENSION {
        bail!("VTF has no mip within the 2048 dimension limit: {path}");
    }
    let image =
        vtf::decode(&data, MAX_TEXTURE_DIMENSION).with_context(|| format!("decode {path}"))?;
    let total = decoded_bytes
        .checked_add(image.rgba.len())
        .context("decoded VTF byte count overflow")?;
    if total > MAX_DECODED_BYTES {
        bail!("map decoded texture budget exceeds 512 MiB");
    }
    *decoded_bytes = total;
    let image = Arc::new(image);
    cache.insert(path.to_owned(), Arc::clone(&image));
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dx_level_blocks_apply_like_a_dx95_renderer() {
        assert_eq!(dx_level_block(">=dx90_20b"), Some(true));
        assert_eq!(dx_level_block("<dx90_20b"), Some(false));
        assert_eq!(dx_level_block("<dx90"), Some(false));
        assert_eq!(dx_level_block(">dx90"), Some(true));
        assert_eq!(dx_level_block("dx95"), Some(true));
        assert_eq!(dx_level_block("Proxies"), None);
        // combine_soldier/combinesoldiersheet.vmt: the base texture lives only there.
        let text = "\"VertexLitGeneric\" { \"$model\" 1 \">=dx90_20b\" { \"$baseTexture\" \"a\" } \"<dx90_20b\" { \"$baseTexture\" \"b\" } }";
        let entries = keyvalues::parse(text).unwrap();
        let p = material_properties(entries[0].children());
        assert_eq!(p.get("$basetexture").map(String::as_str), Some("a"));
    }

    #[test]
    fn self_illum_needs_a_translucent_base() {
        let definition = |shader: &str, pairs: &[(&str, &str)]| Definition {
            shader: shader.into(),
            properties: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            proxies: Vec::new(),
        };
        let image = |translucent| Image {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
            format: 15,
            translucent,
        };
        let lit = definition("LightmappedGeneric", &[("$selfillum", "1")]);
        assert_eq!(self_illum(&lit, Some(&image(true))).unwrap(), Some([1.; 3]));
        assert_eq!(self_illum(&lit, Some(&image(false))).unwrap(), None);
        assert_eq!(self_illum(&lit, None).unwrap(), None);
        let tinted = definition(
            "VertexLitGeneric",
            &[("$selfillum", "1"), ("$selfillumtint", "[2 1 .5]")],
        );
        assert_eq!(
            self_illum(&tinted, Some(&image(true))).unwrap(),
            Some([2., 1., 0.5])
        );
        let unlit = definition("UnlitGeneric", &[("$selfillum", "1")]);
        assert_eq!(self_illum(&unlit, Some(&image(true))).unwrap(), None);
    }

    #[test]
    fn envmap_mask_follows_sdk_bump_rules() {
        let definition = |pairs: &[(&str, &str)]| Definition {
            shader: "LightmappedGeneric".into(),
            properties: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            proxies: Vec::new(),
        };
        // $bumpmap undefines $envmapmask; only $normalmapalphaenvmapmask masks then.
        let kind = |alpha, bump| EnvmapMask { alpha, bump };
        let d = definition(&[("$bumpmap", "b"), ("$envmapmask", "m")]);
        assert_eq!(envmap_mask(&d), Some(("b", kind(false, true))));
        let d = definition(&[("$bumpmap", "b"), ("$ssbump", "1")]);
        assert_eq!(envmap_mask(&d), None);
        let d = definition(&[("$bumpmap", "b"), ("$normalmapalphaenvmapmask", "1")]);
        assert_eq!(envmap_mask(&d), Some(("b", kind(true, true))));
        let d = definition(&[("$envmapmask", "m")]);
        assert_eq!(envmap_mask(&d), Some(("m", kind(false, false))));
        assert_eq!(envmap_mask(&definition(&[])), None);
        let mut m = MaterialData::default();
        let d = definition(&[("$envmapsaturation", "[1 1 1]"), ("$envmapcontrast", ".5")]);
        envmap_parameters(&d, &mut m).unwrap();
        assert_eq!(
            (m.envmap_saturation, m.envmap_contrast),
            ([1.; 3], [0.5; 3])
        );
        assert!(envmap_parameters(&definition(&[("$envmaptint", "[1 2]")]), &mut m).is_err());
    }

    #[test]
    fn conditional_material_vars_follow_retail_tests() {
        let entries = keyvalues::parse(
            r#""srgb?$color2" "[2.5 2.5 2.5]" "$color2" "[1 1 1]" "hdr?$alpha" "0.5"
            "!hdr?$nocull" "1" "ldr?$additive" "1" "360?$translucent" "1" "dx9?$x" "1""#,
        )
        .unwrap();
        let p = direct_properties(&entries);
        assert_eq!(p["$color2"], "[2.5 2.5 2.5]");
        assert_eq!(p["$alpha"], "0.5");
        for absent in ["$nocull", "$additive", "$translucent", "$x", "srgb?$color2"] {
            assert!(!p.contains_key(absent), "{absent}");
        }
    }

    #[test]
    fn bsp_normalization_faces_up_and_preserves_model_and_collision_copies() {
        let mut surface = modkit_core::Surface {
            flex_source: None,
            background: false,
            material: "synthetic".into(),
            lightmap: None,
            vertices: [[0., 0., 0.], [0., 1., 0.], [1., 0., 0.]]
                .into_iter()
                .map(|position| modkit_core::Vertex {
                    normal: Default::default(),
                    position: position.into(),
                    uv: Default::default(),
                    color: [255; 4],
                    light_uv: Default::default(),
                    skin: None,
                })
                .collect(),
            indices: vec![0, 1, 2],
        };
        let collision = surface.clone();
        surface.indices = vec![0, 2, 1];
        let model = surface;
        let mut world = World {
            surfaces: vec![collision.clone()],
            terrain: vec![collision.clone()],
            model_assets: BTreeMap::from([("synthetic-model".into(), vec![model.clone()])]),
            ..Default::default()
        };
        normalize_bsp_render_winding(&mut world);
        let triangle = &world.surfaces[0];
        let positions: Vec<_> = triangle
            .indices
            .iter()
            .map(|&i| triangle.vertices[i as usize].position)
            .collect();
        assert!(
            (positions[1] - positions[0])
                .cross(positions[2] - positions[0])
                .z
                > 0.
        );
        assert_eq!(world.terrain[0].indices, collision.indices);
        assert_eq!(
            world.model_assets["synthetic-model"][0].indices,
            model.indices
        );
    }

    fn tiny_vtf() -> Vec<u8> {
        let mut bytes = vec![0; 80];
        bytes[..4].copy_from_slice(b"VTF\0");
        bytes[4..8].copy_from_slice(&7u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&80u32.to_le_bytes());
        bytes[16..18].copy_from_slice(&1u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&1u16.to_le_bytes());
        bytes[24..26].copy_from_slice(&1u16.to_le_bytes());
        bytes[56] = 1;
        bytes[57..61].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[63..65].copy_from_slice(&1u16.to_le_bytes());
        bytes.extend([2, 4, 8, 16]);
        bytes
    }

    #[test]
    fn texture_aliases_share_pixels_and_still_reuse_when_budget_is_full() {
        let mut cache = BTreeMap::new();
        let mut used = MAX_DECODED_BYTES - 4;
        let first_key = asset_path("Materials\\Shared.VTF", ".vtf").unwrap();
        let alias_key = asset_path("shared", ".vtf").unwrap();
        assert_eq!(first_key, alias_key);
        let first = cached_texture(&mut cache, &mut used, &first_key, || Ok(tiny_vtf())).unwrap();
        assert_eq!(first.rgba, [2, 4, 8, 16]);
        assert_eq!(used, MAX_DECODED_BYTES);
        let alias = cached_texture(&mut cache, &mut used, &alias_key, || {
            panic!("an alias must not read or decode its existing texture again")
        })
        .unwrap();
        assert!(Arc::ptr_eq(&first, &alias));
        assert_eq!(used, MAX_DECODED_BYTES);
        assert!(
            cached_texture(&mut cache, &mut used, "materials/new.vtf", || {
                Ok(tiny_vtf())
            })
            .is_err()
        );
        assert_eq!(used, MAX_DECODED_BYTES);
        assert_eq!(
            cache.len(),
            1,
            "an over-budget texture must not be retained"
        );
    }

    #[test]
    fn selection_keeps_toggleable_hidden_entities_but_excludes_sky_and_collision_copies() {
        use modkit_core::{BrushModel, Entity, ModelInstance, Surface, Vertex};
        let surface = |name: &str, background: bool| Surface {
            flex_source: None,
            material: name.into(),
            background,
            vertices: [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
                .into_iter()
                .map(|position| Vertex {
                    normal: Default::default(),
                    position: position.into(),
                    uv: Default::default(),
                    light_uv: Default::default(),
                    color: [255; 4],
                    skin: None,
                })
                .collect(),
            indices: vec![0, 1, 2],
            lightmap: None,
        };
        let entity = |class: &str, model: &str, hidden: bool| Entity {
            properties: vec![
                ("classname".into(), class.into()),
                ("model".into(), model.into()),
                ("rendermode".into(), if hidden { "10" } else { "0" }.into()),
            ],
        };
        let instance = |model: &str, entity: Option<usize>, background: bool| ModelInstance {
            model: model.into(),
            entity,
            background,
            origin: Default::default(),
            angles: Default::default(),
            scale: 1.,
            skin: 0,
            body: 0,
            kind: "prop_dynamic".into(),
            solid: false,
            solid_mode: None,
        };
        let world = World {
            surfaces: vec![
                surface("world", false),
                surface("static-baked", false),
                surface("sky", true),
            ],
            terrain: vec![surface("collision-only", false)],
            entities: vec![
                entity("func_detail", "*1", false),
                entity("trigger_once", "*2", false),
                entity("func_areaportal", "*3", false),
                entity("func_brush", "*4", true),
                entity("func_brush", "*5", false),
                entity("prop_dynamic", "visible.mdl", false),
                entity("prop_dynamic", "hidden.mdl", true),
                entity("prop_dynamic", "background.mdl", false),
            ],
            background_entities: vec![4, 7],
            brush_models: (1..=6)
                .map(|id| BrushModel {
                    id,
                    surfaces: vec![surface(&format!("brush-{id}"), false)],
                    ..Default::default()
                })
                .collect(),
            model_instances: vec![
                instance("visible.mdl", Some(5), false),
                instance("hidden.mdl", Some(6), false),
                instance("background.mdl", Some(7), true),
                instance("static.mdl", None, false),
            ],
            model_assets: ["visible", "hidden", "background", "static", "unused"]
                .into_iter()
                .map(|name| {
                    (
                        format!("{name}.mdl#0"),
                        vec![surface(&format!("model-{name}"), false)],
                    )
                })
                .collect(),
            ..Default::default()
        };
        assert_eq!(
            rendered_material_names(&world),
            BTreeSet::from([
                "world".into(),
                "static-baked".into(),
                "sky".into(),
                "brush-5".into(),
                "model-background".into(),
                "brush-1".into(),
                "brush-4".into(),
                "model-hidden".into(),
                "model-visible".into(),
            ])
        );
    }

    #[test]
    fn synthetic_vmt_keeps_cutout_tint_opacity_and_affine_uv_parameters() {
        let entries = keyvalues::parse(
            r#""VertexLitGeneric" {
            "$alpha" "0.4" "$nocull" "1" "$color" "{128 64 255}"
            "$basetexturetransform" "center 0 0 scale 2 3 translate 0.1 -0.2"
            "$translucent" "1" "$alphatest" "1" "$alphatestreference" "0.3"
        }"#,
        )
        .unwrap();
        let definition = Definition {
            shader: entries[0].key.clone(),
            properties: direct_properties(entries[0].children()),
            proxies: Vec::new(),
        };
        let material = metadata(&definition).unwrap();
        assert!(material.translucent && material.two_sided);
        assert_eq!(material.alpha_cutoff, Some(0.3));
        assert_eq!(material.opacity, 0.4);
        assert_eq!(material.tint, [128. / 255., 64. / 255., 1.]);
        assert_eq!(material.uv_transform, [[2., 0., 0.1], [0., 3., -0.2]]);
    }
    #[test]
    fn material_rejects_nonfinite_parameters_and_escaping_asset_paths() {
        let definition = Definition {
            shader: "UnlitGeneric".into(),
            properties: BTreeMap::from([("$alpha".into(), "NaN".into())]),
            proxies: Vec::new(),
        };
        assert!(metadata(&definition).is_err());
        assert!(asset_path("../../outside", ".vmt").is_err());
    }
}
#[cfg(test)]
/// Replays native frag throws (private oracle ent_text samples on d1_trainstation_02,
/// sessions 11 and 13) through the shipped frag body. The native spin is random, so each
/// case runs seven spins and the best fit must stay within the bound.
mod owned_frag_replay {
    use glam::Vec3;
    use hl2_simulation::{
        entities::Scene,
        gameplay::{FragThrow, frag_launch},
        physics::Physics,
        projectiles::{ProjectileKind, ProjectileSpawn, Projectiles},
    };
    fn forward(pitch: f32, yaw: f32) -> Vec3 {
        let (p, y) = (pitch.to_radians(), yaw.to_radians());
        Vec3::new(p.cos() * y.cos(), p.cos() * y.sin(), -p.sin())
    }
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_frag_throws_follow_native_samples() {
        let root = source_assets::install::discover().unwrap();
        let loaded = super::load(&root, "d1_trainstation_02").unwrap();
        let world = loaded.world.clone();
        let lob = vec![
            (0.195, [-3060.6, -2042.3, 98.5]),
            (0.36, [-3014.4, -2059.3, 67.6]),
            (0.537, [-2982.2, -2066.6, 68.9]),
            (0.688, [-2958.1, -2068.5, 69.7]),
            (0.897, [-2926.1, -2068.6, 61.4]),
            (1.059, [-2901.4, -2068.5, 38.0]),
            (1.207, [-2881.3, -2065.5, 27.2]),
            (1.355, [-2859.0, -2060.9, 13.5]),
            (1.524, [-2845.1, -2059.5, 2.1]),
            (2.257, [-2842.8, -2071.9, 2.1]),
        ];
        let roll = vec![
            (0.209, [-3013.6, -2050.9, 66.5]),
            (0.346, [-2940.1, -2077.7, 67.2]),
            (0.517, [-2855.0, -2108.8, 53.5]),
            (0.697, [-2767.8, -2140.6, 21.5]),
            (0.837, [-2713.0, -2158.1, 3.5]),
            (0.969, [-2669.9, -2167.8, -0.7]),
            (1.113, [-2630.4, -2175.8, -1.3]),
            (1.299, [-2591.9, -2182.9, -1.6]),
            (1.458, [-2564.1, -2188.8, -1.1]),
            (1.601, [-2546.6, -2196.0, -1.3]),
            (1.746, [-2532.2, -2206.3, -1.8]),
            (1.893, [-2519.6, -2216.4, -1.7]),
            (2.091, [-2504.1, -2227.2, -1.8]),
            (2.253, [-2495.4, -2235.8, -1.7]),
        ];
        let high = vec![
            (0.199, [-2954.4, -2080.9, 231.4]),
            (0.374, [-2807.1, -2134.6, 315.8]),
            (0.524, [-2695.9, -2175.1, 365.2]),
            (0.678, [-2593.6, -2212.2, 397.7]),
            (0.847, [-2489.1, -2250.2, 416.1]),
            (1.027, [-2382.3, -2289.2, 416.8]),
            (1.166, [-2298.1, -2319.8, 402.9]),
            (1.335, [-2210.7, -2351.5, 373.4]),
        ];
        let plaza = vec![
            (0.961, [-2287.2, -1852.6, 143.0]),
            (1.443, [-2060.8, -1796.1, 29.4]),
            (1.927, [-2074.0, -1769.5, 26.1]),
            (2.425, [-2087.1, -1751.8, 18.1]),
            (2.887, [-2098.9, -1747.5, 18.1]),
        ];
        let wall = vec![
            (0.661, [-3157.6, -2002.0, 163.5]),
            (1.639, [-3134.0, -1999.3, 66.2]),
            (2.085, [-3123.3, -1992.8, 66.1]),
        ];
        let only = std::env::var("FRAG_REPLAY_ONLY").unwrap_or_default();
        let cases = [
            // (name, kind, eye z, pitch, yaw, native spawn delay, samples, best RMS bound)
            ("lob", FragThrow::Lob, 128., 25., -20., 0.092, lob, 35.),
            ("roll", FragThrow::Roll, 92., 25., -20., 0.092, roll, 30.),
            ("high", FragThrow::Throw, 128., -30., -20., 0.045, high, 8.),
            (
                "plaza",
                FragThrow::Throw,
                128.,
                -10.,
                12.,
                0.045,
                plaza,
                25.,
            ),
            ("wall", FragThrow::Throw, 128., 0., 180., 0.045, wall, 55.),
        ];
        let verbose = std::env::var("FRAG_REPLAY_VERBOSE").is_ok();
        let mut total = 0.;
        for (name, kind, eyez, pitch, yaw, delay, samples, bound) in cases {
            if !only.is_empty() && !only.contains(name) {
                continue;
            }
            let mut rms_all = vec![];
            for spin in [-0.9f32, -0.5, -0.2, 0., 0.3, 0.6, 0.95] {
                let mut scene = Scene::new(&world);
                let mut physics = Physics::new(&world);
                physics.tick(0.015);
                let (ex, ey) = if name == "wall" {
                    (-3000., -1998.)
                } else {
                    (-3104., -2018.)
                };
                let eye = Vec3::new(ex, ey, eyez);
                let feet = Vec3::new(ex, ey, 64.);
                let (position, velocity, angular, angles) = frag_launch(
                    &physics,
                    kind,
                    eye,
                    forward(pitch, yaw),
                    Vec3::ZERO,
                    feet,
                    Some(Vec3::Z),
                    spin,
                );
                let mut projectiles = Projectiles::default();
                projectiles.spawn(
                    ProjectileSpawn {
                        angles: Some(angles),
                        kind: ProjectileKind::FragGrenade,
                        position,
                        velocity,
                        angular_velocity: angular,
                        at: 0.,
                        damage: 125.,
                        radius: 250.,
                        mass: 0.,
                        lifetime: 3.2,
                    },
                    &mut scene,
                );
                let mut track = vec![(0f32, position)];
                for tick in 1..200 {
                    scene.time = f64::from(tick) * 0.015;
                    projectiles.tick(
                        &world,
                        &mut scene,
                        &mut physics,
                        Vec3::splat(9999.),
                        false,
                        0.015,
                    );
                    physics.tick(0.015);
                    scene.sounds.clear();
                    if let Some(f) = projectiles.active.first() {
                        track.push((tick as f32 * 0.015, f.position));
                    }
                }
                let at = |t: f32| -> Vec3 {
                    let i = track
                        .iter()
                        .position(|(tt, _)| *tt >= t)
                        .unwrap_or(track.len() - 1)
                        .max(1);
                    let (t0, p0) = track[i - 1];
                    let (t1, p1) = track[i];
                    p0.lerp(p1, ((t - t0) / (t1 - t0).max(1e-6)).clamp(0., 1.))
                };
                let mut sq = 0.;
                for (t, p) in &samples {
                    let ours = at(*t as f32 - delay as f32 + 0.015);
                    let d = ours.distance(Vec3::from_array(*p));
                    if verbose {
                        println!(
                            "  {name} spin {spin} t {t} ours {:.1} {:.1} {:.1} native {:?} d {:.1}",
                            ours.x, ours.y, ours.z, p, d
                        );
                    }
                    sq += d * d;
                }
                let rms = (sq / samples.len() as f32).sqrt();
                let end = at(samples.last().unwrap().0 as f32 - delay as f32)
                    .distance(Vec3::from_array(samples.last().unwrap().1));
                rms_all.push(rms);
                println!("{name} spin {spin:5.2} rms {rms:6.1} end {end:6.1}");
            }
            let mean = rms_all.iter().sum::<f32>() / rms_all.len() as f32;
            total += mean;
            let best = rms_all.iter().cloned().fold(f32::MAX, f32::min);
            println!("{name} MEAN rms {mean:.1} best {best:.1}");
            assert!(best < bound, "{name}: best RMS {best} >= {bound}");
        }
        println!("TOTAL {total:.1}");
    }
}
#[cfg(test)]
/// Route planning cost on d1_trainstation_02 (performance report: a cop chasing an
/// unreachable player): routes from the can cop's post to every graph node. Timing is
/// machine dependent; the bound is loose.
mod owned_route_cost {
    use glam::Vec3;
    use hl2_simulation::{
        npc::{ActorPose, GoalKey, GoalState, MoveRequest, MoveStyle},
        physics::Physics,
    };
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_tiny_hull_falls_to_the_hall_floor() {
        use hl2_simulation::npc_probe::{NpcCollisionWorld, Query};
        let root = source_assets::install::discover().unwrap();
        let loaded = super::load(&root, "d1_trainstation_02").unwrap();
        let world = loaded.world.clone();
        let mut physics = Physics::new(&world);
        physics.tick(0.015);
        let hull = hl2_simulation::ai_hull::HullType::Tiny.bounds();
        for z in [104.03f32, 103., 102.34, 100., 90., 80., 70.] {
            let start = Vec3::new(-3356., -2300., z);
            let hit = physics.npc_trace_hull(start, start - Vec3::Z * 50., hull, Query::default());
            println!(
                "z {z}: fraction {} start_solid {} entity {:?}",
                hit.trace.fraction, hit.trace.start_solid, hit.entity
            );
        }
    }
    /// A zombie torso (HULL_TINY) plans the same hall routes as a whole zombie (HULL_HUMAN).
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_tiny_hull_routes_like_the_human_hull() {
        let root = source_assets::install::discover().unwrap();
        let mut loaded = super::load(&root, "d1_trainstation_02").unwrap();
        let world = loaded.world.clone();
        let mut physics = Physics::new(&world);
        physics.tick(0.015);
        let template = |label: &str| {
            hl2_simulation::ai::spawn::templates(&world, label)
                .first()
                .copied()
                .unwrap()
        };
        let npcs = &mut loaded.gameplay.npcs;
        let from = Vec3::new(-3356., -2241., 64.);
        let mut cases = Vec::new();
        for dx in [-300., -150., 0., 150., 300.] {
            for dy in [-300., -150., 150., 300.] {
                cases.push(Vec3::new(from.x + dx, from.y + dy, 64.));
            }
        }
        let mut differ = 0;
        for (i, to) in cases.iter().enumerate() {
            let mut states = Vec::new();
            for (j, label) in ["npc_zombie", "npc_zombie_torso"].into_iter().enumerate() {
                let actor = template(label);
                let key = GoalKey {
                    scene: usize::MAX,
                    event: 1000 + i * 2 + j,
                    actor,
                };
                let state = npcs.request(
                    key,
                    MoveRequest {
                        target_entity: actor,
                        target_feet: *to,
                        style: MoveStyle::Run,
                        event_distance: 0.,
                        force_short: false,
                    },
                    ActorPose {
                        entity: actor,
                        feet: from,
                        yaw_degrees: 270.,
                        scripted_by: None,
                    },
                    &physics,
                    &[],
                );
                states.push(matches!(state, GoalState::Blocked(_)));
                npcs.cancel(key);
                npcs.forget(key);
            }
            println!(
                "to {to:?}: human blocked {} tiny blocked {}",
                states[0], states[1]
            );
            differ += usize::from(states[0] != states[1]);
        }
        // Session 37: both hulls reach (or miss) the same 20 hall targets.
        assert_eq!(differ, 0);
    }
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_failed_routes_cost() {
        let root = source_assets::install::discover().unwrap();
        let mut loaded = super::load(&root, "d1_trainstation_02").unwrap();
        let world = loaded.world.clone();
        let mut physics = Physics::new(&world);
        physics.tick(0.015);
        let cop = world
            .entities
            .iter()
            .position(|e| e.get("targetname") == Some("cupcop"))
            .unwrap();
        let feet = Vec3::new(-3751., -921., 64.03);
        let npcs = &mut loaded.gameplay.npcs;
        let targets: Vec<Vec3> = npcs
            .graph()
            .unwrap()
            .nodes
            .iter()
            .map(|n| n.origin)
            .collect();
        let (mut ok, mut failed, mut worst) = (0, 0, (0u64, 0u64, Vec3::ZERO));
        for (i, target) in targets.iter().enumerate() {
            let before = npcs.path_stats();
            let key = GoalKey {
                scene: usize::MAX,
                event: i,
                actor: cop,
            };
            let state = npcs.request(
                key,
                MoveRequest {
                    target_entity: cop,
                    target_feet: *target,
                    style: MoveStyle::Run,
                    event_distance: 0.,
                    force_short: false,
                },
                ActorPose {
                    entity: cop,
                    feet,
                    yaw_degrees: 0.,
                    scripted_by: None,
                },
                &physics,
                &[],
            );
            let after = npcs.path_stats();
            npcs.cancel(key);
            npcs.forget(key);
            if matches!(state, GoalState::Blocked(_)) {
                failed += 1;
                let cost = (
                    after.micros - before.micros,
                    after.link_probes - before.link_probes,
                );
                if cost.0 > worst.0 {
                    worst = (cost.0, cost.1, *target);
                }
            } else {
                ok += 1;
            }
        }
        let stats = npcs.path_stats();
        println!(
            "nodes {} ok {ok} failed {failed} total {:.1} ms probes {}; worst failure {:.1} ms {} probes to {:?}",
            targets.len(),
            stats.micros as f64 / 1000.,
            stats.link_probes,
            worst.0 as f64 / 1000.,
            worst.1,
            worst.2
        );
        // Session 31: graph-only link usability (was 1002 link sweeps and ~1.9 s for
        // one failed route); a fresh controller has no stale links to recheck.
        assert_eq!(stats.link_probes, 0);
        assert!(worst.0 < 250_000, "worst failed route {} us", worst.0);
    }
}
