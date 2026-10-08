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

#[derive(Debug)]
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
        }
    }
}

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
    let model_report = models::append_models(&mut world, &vfs);
    let mut gameplay =
        crate::gameplay::Gameplay::load_with_campaign(world, &vfs, bsp.revision, new_game)?;
    let world = gameplay.world.clone();
    let effects = crate::effects::PreparedEffects::load(&vfs);
    let mut names = rendered_material_names(&world);
    names.extend(effects.grenade.iter().map(|s| s.material.clone()));
    for weapon in gameplay.weapons.values() {
        if let Some(surfaces) = world
            .model_assets
            .get(&format!("{}#0", weapon.viewmodel.to_lowercase()))
        {
            names.extend(surfaces.iter().map(|s| s.material.clone()));
        }
    }
    if names.len() > MAX_MATERIALS {
        bail!("map exceeds {MAX_MATERIALS} unique material limit");
    }
    let mut materials = BTreeMap::new();
    let mut texture_errors = decal_errors;
    let mut decoded_bytes = 0usize;
    let mut texture_cache = BTreeMap::new();
    for name in names {
        let material = load_material(
            &vfs,
            &name,
            &mut texture_cache,
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

// Reuse the bounded KeyValues decoder, preserving direct material parameters and
// Patch insert/replace semantics. Only the authored Eyes_dx8 fallback for EyeRefract is selected; other DX blocks/proxies remain unsupported.
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
            properties: direct_properties(root.children()),
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
            source_assets::monitor_material::Animation::parse(&definition.proxies);
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
