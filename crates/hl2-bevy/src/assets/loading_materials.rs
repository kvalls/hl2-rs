//! Three-pass parallel VMT/VTF material loader for map preparation.
use super::{
    Definition, EnvmapMask, MAX_DECODED_BYTES, MAX_MATERIALS, MAX_TEXTURE_BYTES,
    MAX_TEXTURE_DIMENSION, MaterialData, TextureFrames, TwoTexture, asset_path, definition,
    envmap_mask, envmap_parameters, metadata, names, reflects_sample, scalar, self_illum,
};
use anyhow::{Context, Result, bail};
use modkit_core::World;
use source_assets::{
    vpk::Vfs,
    vtf::{self, Cube, GpuImage},
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(super) fn decode_2d_vtf(
    path: &str,
    data: &[u8],
    picmip: i32,
    allow_bc: bool,
) -> Result<GpuImage> {
    if data.len() > MAX_TEXTURE_BYTES {
        bail!("encoded VTF exceeds 64 MiB limit: {path}");
    }
    // The VTF adapter picks an existing mip rather than resampling. Reject
    // a texture with no mip within our cap before allocating its decoded pixels.
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
    vtf::decode_gpu_picmip(data, MAX_TEXTURE_DIMENSION, picmip, allow_bc)
        .with_context(|| format!("decode {path}"))
}

pub(super) fn envmap_candidates(
    definition: &Definition,
    hdr: bool,
) -> Result<Option<(String, String, String)>> {
    if !definition.shader.eq_ignore_ascii_case("lightmappedgeneric") {
        return Ok(None);
    }
    let Some(name) = definition.properties.get("$envmap") else {
        return Ok(None);
    };
    if name.eq_ignore_ascii_case("env_cubemap") {
        return Ok(None);
    }
    let plain = asset_path(name, ".vtf")?;
    let hdr_path = format!("{}.hdr.vtf", plain.trim_end_matches(".vtf"));
    let (preferred, fallback) = if hdr {
        (hdr_path, plain.clone())
    } else {
        (plain.clone(), hdr_path)
    };
    Ok(Some((plain, preferred, fallback)))
}

pub(super) fn decode_envmap_cube(
    vfs: &Vfs,
    plain: &str,
    preferred: &str,
    fallback: &str,
    picmip: i32,
) -> Result<(String, Cube)> {
    let (path, data) = match vfs.read(preferred)? {
        Some(data) => (preferred.to_owned(), data),
        None => match vfs.read(fallback)? {
            Some(data) => (fallback.to_owned(), data),
            None => bail!("envmap absent: {plain}"),
        },
    };
    if data.len() > MAX_TEXTURE_BYTES {
        bail!("encoded envmap exceeds 64 MiB limit: {path}");
    }
    let cube = vtf::decode_cube_picmip(&data, MAX_TEXTURE_DIMENSION, picmip)
        .with_context(|| format!("decode {path}"))?;
    Ok((path, cube))
}

pub(super) fn load(
    vfs: &Vfs,
    world: &World,
    (effects, weapon_models, gameplay): (
        &crate::effects::PreparedEffects,
        &[String],
        &crate::gameplay::Gameplay,
    ),
    decal_errors: Vec<String>,
    quality: (i32, bool, bool),
    (progress, cancel): (
        &hl2_ui::gameui::loading::ProgressReport,
        &super::work::Cancellation,
    ),
) -> Result<(BTreeMap<String, MaterialData>, Vec<String>)> {
    let names = names::map_material_names(world, effects, weapon_models, gameplay);
    if names.len() > MAX_MATERIALS {
        bail!("map exceeds {MAX_MATERIALS} unique material limit");
    }
    load_names(
        vfs,
        &names,
        decal_errors,
        quality,
        MAX_DECODED_BYTES,
        (progress, cancel),
    )
}

fn load_names(
    vfs: &Vfs,
    names: &BTreeSet<String>,
    decal_errors: Vec<String>,
    (picmip, hdr, allow_bc): (i32, bool, bool),
    budget: usize,
    (progress, cancel): (
        &hl2_ui::gameui::loading::ProgressReport,
        &super::work::Cancellation,
    ),
) -> Result<(BTreeMap<String, MaterialData>, Vec<String>)> {
    progress.report(0.55, "LoadingProgress_LoadResources");
    let (planned, jobs) = plan_materials(vfs, names, hdr, cancel)?;
    cancel.check()?;
    progress.report(0.62, "LoadingProgress_LoadResources");
    let mut decoded = decode_parallel(vfs, jobs, (picmip, allow_bc), cancel)?;
    cancel.check()?;
    progress.report(0.80, "LoadingProgress_LoadResources");
    let mut texture_errors = decal_errors;
    let materials = assemble_materials(planned, &mut decoded, budget, &mut texture_errors, cancel)?;
    progress.report(0.85, "LoadingProgress_LoadResources");
    Ok((materials, texture_errors))
}

struct NormalPlan {
    definition: Definition,
    material: MaterialData,
    meta_err: Option<String>,
    base: Result<String, String>,
    base2: Option<Result<String, String>>,
    envmap_err: Option<String>,
    envmap_key: Option<String>,
    from_sample: bool,
    mask: Option<(Result<String, String>, EnvmapMask)>,
    iris: Option<Result<String, String>>,
}

enum PlannedKind {
    Failed(String),
    Camera {
        material: MaterialData,
        color2_err: Option<String>,
        overlay: Option<Result<String, String>>,
    },
    TwoTexture {
        material: MaterialData,
        setup: Result<PlannedTwoTexture, String>,
    },
    Normal(Box<NormalPlan>),
}

struct PlannedTwoTexture {
    base_path: String,
    base_key: String,
    second_path: String,
    second_key: String,
    frames: source_assets::proximity_material::Frames,
    scroll: source_assets::monitor_material::Animation,
}

struct PlannedMaterial {
    name: String,
    kind: PlannedKind,
}

#[derive(Default)]
struct JobPlan {
    vtf_2d: BTreeSet<String>,
    frames: BTreeMap<String, (String, bool)>,
    envmaps: BTreeMap<String, (String, String, String)>,
}

enum DecodeJob {
    Vtf2d(String),
    Frames {
        key: String,
        path: String,
        active: bool,
    },
    Envmap {
        key: String,
        plain: String,
        preferred: String,
        fallback: String,
    },
}

enum DecodedValue {
    Vtf2d(String, Result<GpuImage, Option<String>>),
    Frames(String, Result<(TextureFrames, usize), String>),
    Envmap(String, Result<(String, Cube), String>),
}

#[derive(Default)]
struct DecodedAssets {
    vtf_2d: BTreeMap<String, Result<GpuImage, Option<String>>>,
    frames: BTreeMap<String, Result<(TextureFrames, usize), String>>,
    envmaps: BTreeMap<String, Result<(String, Cube), String>>,
}

fn plan_materials(
    vfs: &Vfs,
    names: &BTreeSet<String>,
    hdr: bool,
    cancel: &super::work::Cancellation,
) -> Result<(Vec<PlannedMaterial>, Vec<DecodeJob>)> {
    let mut planned = Vec::with_capacity(names.len());
    let mut job_plan = JobPlan::default();
    for name in names {
        cancel.check()?;
        planned.push(plan_one(vfs, name, hdr, &mut job_plan));
    }
    let mut jobs =
        Vec::with_capacity(job_plan.vtf_2d.len() + job_plan.frames.len() + job_plan.envmaps.len());
    for path in job_plan.vtf_2d {
        jobs.push(DecodeJob::Vtf2d(path));
    }
    for (key, (path, active)) in job_plan.frames {
        jobs.push(DecodeJob::Frames { key, path, active });
    }
    for (key, (plain, preferred, fallback)) in job_plan.envmaps {
        jobs.push(DecodeJob::Envmap {
            key,
            plain,
            preferred,
            fallback,
        });
    }
    Ok((planned, jobs))
}

fn plan_one(vfs: &Vfs, name: &str, hdr: bool, jobs: &mut JobPlan) -> PlannedMaterial {
    let def = match definition(vfs, name, 0, hdr) {
        Ok(def) => def,
        Err(error) => {
            return PlannedMaterial {
                name: name.to_owned(),
                kind: PlannedKind::Failed(format!("{name}: {error:#}")),
            };
        }
    };
    let (mut material, meta_err) = match metadata(&def) {
        Ok(material) => (material, None),
        Err(error) => (MaterialData::default(), Some(format!("{name}: {error:#}"))),
    };
    if def
        .properties
        .get("$basetexture")
        .is_some_and(|base| base.eq_ignore_ascii_case("_rt_camera"))
    {
        return plan_camera(name, &def, material, jobs);
    }
    if def.shader.eq_ignore_ascii_case("UnlitTwoTexture") {
        return plan_two_texture(name, &def, material, jobs);
    }
    let base = def
        .properties
        .get("$basetexture")
        .or_else(|| def.properties.get("$refracttinttexture"))
        .context("VMT has no supported base texture")
        .and_then(|b| asset_path(b, ".vtf"))
        .map_err(|e| format!("{name}: {e:#}"));
    if let Ok(path) = &base {
        jobs.vtf_2d.insert(path.clone());
    }
    let base2 = def.properties.get("$basetexture2").map(|v| {
        let res = asset_path(v, ".vtf").map_err(|e| format!("{name}: $basetexture2: {e:#}"));
        if let Ok(path) = &res {
            jobs.vtf_2d.insert(path.clone());
        }
        res
    });
    let (envmap_err, envmap_key, from_sample, mask) =
        plan_reflection(name, &def, &mut material, hdr, jobs);
    let iris = (def.shader.eq_ignore_ascii_case("eyes") || material.eye_fallback).then(|| {
        let res = def
            .properties
            .get("$iris")
            .context("Eyes VMT missing $iris")
            .and_then(|v| asset_path(v, ".vtf"))
            .map_err(|e| format!("{name}: iris: {e:#}"));
        if let Ok(path) = &res {
            jobs.vtf_2d.insert(path.clone());
        }
        res
    });
    PlannedMaterial {
        name: name.to_owned(),
        kind: PlannedKind::Normal(Box::new(NormalPlan {
            definition: def,
            material,
            meta_err,
            base,
            base2,
            envmap_err,
            envmap_key,
            from_sample,
            mask,
            iris,
        })),
    }
}

fn plan_camera(
    name: &str,
    def: &Definition,
    mut material: MaterialData,
    jobs: &mut JobPlan,
) -> PlannedMaterial {
    material.camera = true;
    material.unlit = true;
    material.base_path = Some("_rt_camera".into());
    material.camera_vertex_color = scalar(&def.properties, "$vertexcolor", 0.).unwrap_or(0.) != 0.;
    material.camera_animation = source_assets::monitor_material::Animation::parse_with_properties(
        &def.proxies,
        &def.properties,
    );
    let mut color2_err = None;
    if let Some(color) = def.properties.get("$color2") {
        let parsed = color
            .trim()
            .trim_matches(['[', ']'])
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<std::result::Result<Vec<_>, _>>();
        match parsed {
            Ok(c) if c.len() == 3 && c.iter().all(|v| v.is_finite()) => {
                material.camera_color2.copy_from_slice(&c);
            }
            _ => color2_err = Some(format!("{name}: invalid monitor $color2")),
        }
    }
    let overlay = def.properties.get("$texture2").map(|second| {
        let res = asset_path(second, ".vtf").map_err(|e| format!("{name}: monitor overlay: {e:#}"));
        if let Ok(path) = &res {
            jobs.vtf_2d.insert(path.clone());
        }
        res
    });
    PlannedMaterial {
        name: name.to_owned(),
        kind: PlannedKind::Camera {
            material,
            color2_err,
            overlay,
        },
    }
}

fn plan_two_texture(
    name: &str,
    def: &Definition,
    material: MaterialData,
    jobs: &mut JobPlan,
) -> PlannedMaterial {
    let setup = (|| -> Result<PlannedTwoTexture> {
        let frames =
            source_assets::proximity_material::Frames::parse(&def.proxies, &def.properties)?;
        let suffix = if frames.active() { "frames" } else { "single" };
        let mut queue = |key: &str| -> Result<(String, String)> {
            let path = asset_path(
                def.properties
                    .get(key)
                    .context("two-texture material texture absent")?,
                ".vtf",
            )?;
            let cache_key = format!("{path}#{suffix}");
            jobs.frames
                .entry(cache_key.clone())
                .or_insert_with(|| (path.clone(), frames.active()));
            Ok((path, cache_key))
        };
        let (base_path, base_key) = queue("$basetexture")?;
        let (second_path, second_key) = queue("$texture2")?;
        let scroll = source_assets::monitor_material::Animation::parse(&def.proxies);
        Ok(PlannedTwoTexture {
            base_path,
            base_key,
            second_path,
            second_key,
            frames,
            scroll,
        })
    })()
    .map_err(|e| format!("{name}: two-texture: {e:#}"));
    PlannedMaterial {
        name: name.to_owned(),
        kind: PlannedKind::TwoTexture { material, setup },
    }
}

type ReflectionPlan = (
    Option<String>,
    Option<String>,
    bool,
    Option<(Result<String, String>, EnvmapMask)>,
);

fn plan_reflection(
    name: &str,
    def: &Definition,
    material: &mut MaterialData,
    hdr: bool,
    jobs: &mut JobPlan,
) -> ReflectionPlan {
    if let Err(e) = envmap_parameters(def, material) {
        return (Some(format!("{name}: envmap: {e:#}")), None, false, None);
    }
    let envmap_key = match envmap_candidates(def, hdr) {
        Ok(Some((plain, preferred, fallback))) => {
            let key = format!("{preferred}|{fallback}");
            jobs.envmaps
                .entry(key.clone())
                .or_insert((plain, preferred, fallback));
            Some(key)
        }
        Ok(None) => None,
        Err(e) => return (Some(format!("{name}: envmap: {e:#}")), None, false, None),
    };
    let from_sample = reflects_sample(def);
    let mask = if envmap_key.is_some() || from_sample {
        envmap_mask(def).map(|(texture, kind)| {
            let res = asset_path(texture, ".vtf").map_err(|e| format!("{name}: envmap: {e:#}"));
            if let Ok(path) = &res {
                jobs.vtf_2d.insert(path.clone());
            }
            (res, kind)
        })
    } else {
        None
    };
    (None, envmap_key, from_sample, mask)
}

fn decode_parallel(
    vfs: &Vfs,
    jobs: Vec<DecodeJob>,
    (picmip, allow_bc): (i32, bool),
    cancel: &super::work::Cancellation,
) -> Result<DecodedAssets> {
    if jobs.is_empty() {
        return Ok(DecodedAssets::default());
    }
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8)
        .min(jobs.len());
    let next = AtomicUsize::new(0);
    let chunks: Vec<Vec<DecodedValue>> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            handles.push(scope.spawn(|| {
                crate::priority::set_current_thread(crate::priority::Priority::Background);
                let mut out = Vec::new();
                while !cancel.cancelled() {
                    let idx = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(idx) else {
                        break;
                    };
                    out.push(run_decode_job(vfs, job, picmip, allow_bc));
                }
                out
            }));
        }
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    cancel.check()?;
    let mut assets = DecodedAssets::default();
    for item in chunks.into_iter().flatten() {
        match item {
            DecodedValue::Vtf2d(k, v) => {
                assets.vtf_2d.insert(k, v);
            }
            DecodedValue::Frames(k, v) => {
                assets.frames.insert(k, v);
            }
            DecodedValue::Envmap(k, v) => {
                assets.envmaps.insert(k, v);
            }
        }
    }
    Ok(assets)
}

fn run_decode_job(vfs: &Vfs, job: &DecodeJob, picmip: i32, allow_bc: bool) -> DecodedValue {
    match job {
        DecodeJob::Vtf2d(path) => {
            let res = match vfs.read(path) {
                Ok(Some(data)) => {
                    decode_2d_vtf(path, &data, picmip, allow_bc).map_err(|e| Some(format!("{e:#}")))
                }
                Ok(None) => Err(None),
                Err(e) => Err(Some(format!("{e:#}"))),
            };
            DecodedValue::Vtf2d(path.clone(), res)
        }
        DecodeJob::Frames { key, path, active } => {
            let res = decode_frame_chain(vfs, path, picmip, *active).map_err(|e| format!("{e:#}"));
            DecodedValue::Frames(key.clone(), res)
        }
        DecodeJob::Envmap {
            key,
            plain,
            preferred,
            fallback,
        } => {
            let res = decode_envmap_cube(vfs, plain, preferred, fallback, picmip)
                .map_err(|e| format!("{e:#}"));
            DecodedValue::Envmap(key.clone(), res)
        }
    }
}

fn decode_frame_chain(
    vfs: &Vfs,
    path: &str,
    picmip: i32,
    active: bool,
) -> Result<(TextureFrames, usize)> {
    let data = vfs.read(path)?.context("two-texture VTF absent")?;
    if data.len() > MAX_TEXTURE_BYTES {
        bail!("encoded VTF exceeds 64 MiB");
    }
    // Source samples the authored mips with distance; the shield frames' smaller mips are
    // part of their far look.
    let mut images =
        vtf::decode_frame_mips_picmip(&data, MAX_TEXTURE_DIMENSION, picmip, MAX_DECODED_BYTES)?;
    if !active {
        images.truncate(1);
    }
    let bytes = images.iter().flatten().map(|i| i.rgba.len()).sum::<usize>();
    let frames = Arc::new(images.into_iter().map(Arc::new).collect::<Vec<_>>());
    Ok((frames, bytes))
}

#[derive(Default)]
struct AssemblyCache {
    textures: BTreeMap<String, Arc<GpuImage>>,
    frames: BTreeMap<String, TextureFrames>,
    envmaps: BTreeMap<String, (String, Arc<Cube>)>,
    decoded_bytes: usize,
    budget: usize,
}

impl AssemblyCache {
    fn take_2d(
        &mut self,
        decoded: &mut DecodedAssets,
        path: &str,
        absent_msg: &str,
    ) -> std::result::Result<Arc<GpuImage>, String> {
        if let Some(image) = self.textures.get(path) {
            return Ok(Arc::clone(image));
        }
        let Some(entry) = decoded.vtf_2d.get_mut(path) else {
            return Err(absent_msg.to_owned());
        };
        let image = match entry {
            Ok(img) => img,
            Err(None) => return Err(absent_msg.to_owned()),
            Err(Some(err)) => return Err(err.clone()),
        };
        let total = self
            .decoded_bytes
            .checked_add(image.data.len())
            .ok_or_else(|| "decoded VTF byte count overflow".to_owned())?;
        if total > self.budget {
            return Err("map decoded texture budget exceeds 512 MiB".to_owned());
        }
        let Ok(owned) = std::mem::replace(entry, Err(Some("consumed".into()))) else {
            unreachable!()
        };
        self.decoded_bytes = total;
        let arc = Arc::new(owned);
        self.textures.insert(path.to_owned(), Arc::clone(&arc));
        Ok(arc)
    }

    fn take_frames(
        &mut self,
        decoded: &mut DecodedAssets,
        key: &str,
    ) -> std::result::Result<TextureFrames, String> {
        if let Some(frames) = self.frames.get(key) {
            return Ok(Arc::clone(frames));
        }
        let Some(entry) = decoded.frames.get_mut(key) else {
            return Err("two-texture VTF absent".to_owned());
        };
        let bytes = match entry {
            Ok((_, bytes)) => *bytes,
            Err(err) => return Err(err.clone()),
        };
        let total = self
            .decoded_bytes
            .checked_add(bytes)
            .ok_or_else(|| "decoded VTF byte count overflow".to_owned())?;
        if total > self.budget {
            return Err("map decoded texture budget exceeds 512 MiB".to_owned());
        }
        let Ok((frames, _)) = std::mem::replace(entry, Err("consumed".into())) else {
            unreachable!()
        };
        self.decoded_bytes = total;
        self.frames.insert(key.to_owned(), Arc::clone(&frames));
        Ok(frames)
    }

    fn take_envmap(
        &mut self,
        decoded: &mut DecodedAssets,
        key: &str,
    ) -> std::result::Result<(String, Arc<Cube>), String> {
        if let Some((path, cube)) = self.envmaps.get(key) {
            return Ok((path.clone(), Arc::clone(cube)));
        }
        let Some(entry) = decoded.envmaps.get_mut(key) else {
            return Err("envmap absent".to_owned());
        };
        let cube_bytes = match entry {
            Ok((_, cube)) => cube.bytes(),
            Err(err) => return Err(err.clone()),
        };
        let total = self
            .decoded_bytes
            .checked_add(cube_bytes)
            .ok_or_else(|| "decoded VTF byte count overflow".to_owned())?;
        if total > self.budget {
            return Err("map decoded texture budget exceeds 512 MiB".to_owned());
        }
        let Ok((path, cube)) = std::mem::replace(entry, Err("consumed".into())) else {
            unreachable!()
        };
        self.decoded_bytes = total;
        let arc = Arc::new(cube);
        self.envmaps
            .insert(key.to_owned(), (path.clone(), Arc::clone(&arc)));
        Ok((path, arc))
    }
}

pub(super) fn apply_two_texture(material: &mut MaterialData, path: String, textures: TwoTexture) {
    material.base_path = Some(path);
    material.base = textures
        .base_frames
        .first()
        .and_then(|chain| chain.first())
        .map(|top| Arc::new(GpuImage::from(top.clone())));
    material.two_texture = Some(textures);
    material.unlit = true;
}

fn assemble_camera(
    name: &str,
    mut material: MaterialData,
    color2_err: Option<String>,
    overlay: Option<Result<String, String>>,
    decoded: &mut DecodedAssets,
    cache: &mut AssemblyCache,
    errors: &mut Vec<String>,
) -> MaterialData {
    if let Some(err) = color2_err {
        errors.push(err);
    }
    if let Some(res) = overlay {
        match res.and_then(|path| {
            let img = cache
                .take_2d(decoded, &path, "owned monitor overlay absent")
                .map_err(|e| format!("{name}: monitor overlay: {e}"))?;
            Ok((path, img))
        }) {
            Ok((path, img)) => {
                material.camera_overlay_path = Some(path);
                material.camera_overlay = Some(img);
            }
            Err(err) => errors.push(err),
        }
    }
    material
}

fn assemble_two_texture(
    name: &str,
    mut material: MaterialData,
    setup: Result<PlannedTwoTexture, String>,
    decoded: &mut DecodedAssets,
    cache: &mut AssemblyCache,
    errors: &mut Vec<String>,
) -> MaterialData {
    match setup.and_then(|s| {
        let base_frames = cache
            .take_frames(decoded, &s.base_key)
            .map_err(|e| format!("{name}: two-texture: {e}"))?;
        let second_frames = cache
            .take_frames(decoded, &s.second_key)
            .map_err(|e| format!("{name}: two-texture: {e}"))?;
        Ok((
            s.base_path,
            TwoTexture {
                base_frames,
                second_frames,
                second_path: s.second_path,
                frames: s.frames,
                scroll: s.scroll,
            },
        ))
    }) {
        Ok((path, textures)) => apply_two_texture(&mut material, path, textures),
        Err(err) => errors.push(err),
    }
    material
}

fn assemble_materials(
    planned: Vec<PlannedMaterial>,
    decoded: &mut DecodedAssets,
    budget: usize,
    errors: &mut Vec<String>,
    cancel: &super::work::Cancellation,
) -> Result<BTreeMap<String, MaterialData>> {
    let mut cache = AssemblyCache {
        budget,
        ..Default::default()
    };
    let mut materials = BTreeMap::new();
    for item in planned {
        cancel.check()?;
        let name = item.name;
        let material = match item.kind {
            PlannedKind::Failed(err) => {
                errors.push(err);
                MaterialData::default()
            }
            PlannedKind::Camera {
                material,
                color2_err,
                overlay,
            } => assemble_camera(
                &name, material, color2_err, overlay, decoded, &mut cache, errors,
            ),
            PlannedKind::TwoTexture { material, setup } => {
                assemble_two_texture(&name, material, setup, decoded, &mut cache, errors)
            }
            PlannedKind::Normal(plan) => assemble_normal(&name, *plan, decoded, &mut cache, errors),
        };
        materials.insert(name, material);
    }
    Ok(materials)
}

fn assemble_normal(
    name: &str,
    mut plan: NormalPlan,
    decoded: &mut DecodedAssets,
    cache: &mut AssemblyCache,
    errors: &mut Vec<String>,
) -> MaterialData {
    let material = &mut plan.material;
    if let Some(err) = plan.meta_err {
        errors.push(err);
    }
    match plan.base.and_then(|path| {
        let img = cache
            .take_2d(decoded, &path, &format!("VTF absent: {path}"))
            .map_err(|e| format!("{name}: {e}"))?;
        Ok((path, img))
    }) {
        Ok((path, img)) => {
            material.base_path = Some(path);
            material.base = Some(img);
        }
        Err(err) => errors.push(err),
    }
    if let Some(res) = plan.base2 {
        match res.and_then(|path| {
            let img = cache
                .take_2d(decoded, &path, &format!("VTF absent: {path}"))
                .map_err(|e| format!("{name}: $basetexture2: {e}"))?;
            Ok((path, img))
        }) {
            Ok((path, img)) => material.base2 = Some((path, img)),
            Err(err) => errors.push(err),
        }
    }
    match self_illum(&plan.definition, material.base.as_deref()) {
        Ok(tint) => material.self_illum = tint,
        Err(err) => errors.push(format!("{name}: selfillum: {err:#}")),
    }
    if let Some(err) = plan.envmap_err {
        errors.push(err);
    } else if plan.envmap_key.is_some() || plan.from_sample {
        let cube = plan
            .envmap_key
            .map(|key| {
                cache
                    .take_envmap(decoded, &key)
                    .map_err(|e| format!("{name}: envmap: {e}"))
            })
            .transpose();
        let mask_res = cube.and_then(|cube| {
            let loaded_mask = plan
                .mask
                .map(|(res, kind)| {
                    res.and_then(|path| {
                        let img = cache
                            .take_2d(decoded, &path, &format!("envmap mask absent: {path}"))
                            .map_err(|e| format!("{name}: envmap: {e}"))?;
                        Ok((path, img, kind))
                    })
                })
                .transpose()?;
            Ok((cube, loaded_mask))
        });
        match mask_res {
            Ok((cube, loaded_mask)) => {
                if let Some((path, cube)) = cube {
                    material.envmap_path = Some(path);
                    material.envmap = Some(cube);
                }
                material.model_envmap = plan.from_sample;
                if let Some((path, img, kind)) = loaded_mask {
                    material.envmap_mask_path = Some(path);
                    material.envmap_mask = Some(img);
                    material.envmap_mask_alpha = kind.alpha;
                    material.envmap_bump = kind.bump;
                }
                // SDK ShaderInit: an opaque base texture clears BASEALPHAENVMAPMASK.
                if !material.base.as_ref().is_some_and(|base| base.translucent) {
                    material.base_alpha_envmap_mask = false;
                }
            }
            Err(err) => errors.push(err),
        }
    }
    if let Some(res) = plan.iris {
        match res.and_then(|path| {
            let img = cache
                .take_2d(decoded, &path, &format!("VTF absent: {path}"))
                .map_err(|e| format!("{name}: iris: {e}"))?;
            Ok((path, img))
        }) {
            Ok((path, img)) => {
                material.iris_path = Some(path);
                material.iris = Some(img);
            }
            Err(err) => errors.push(err),
        }
    }
    plan.material
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempGame {
        root: PathBuf,
    }
    impl TempGame {
        fn new(label: &str) -> Self {
            static SEQ: AtomicUsize = AtomicUsize::new(0);
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "hl2rs_parallel_mat_{label}_{}_{}",
                std::process::id(),
                n
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("hl2/materials")).unwrap();
            std::fs::write(
                root.join("hl2/gameinfo.txt"),
                b"\"GameInfo\" { \"game\" \"Half-Life 2\" }",
            )
            .unwrap();
            Self { root }
        }
        fn write_vmt(&self, name: &str, texture: &str) {
            let path = self.root.join("hl2/materials").join(format!("{name}.vmt"));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let body = format!("\"LightmappedGeneric\" {{ \"$basetexture\" \"{texture}\" }}");
            std::fs::write(path, body).unwrap();
        }
        fn write_tiny_vtf(&self, name: &str, pixel: [u8; 4]) {
            let path = self.root.join("hl2/materials").join(format!("{name}.vtf"));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
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
            bytes.extend(pixel);
            std::fs::write(path, bytes).unwrap();
        }
    }
    impl Drop for TempGame {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn parallel_loader_deduplicates_shared_vtf_and_enforces_budget_deterministically() {
        let game = TempGame::new("dedup_budget");
        game.write_tiny_vtf("shared", [10, 20, 30, 255]);
        game.write_tiny_vtf("second", [40, 50, 60, 255]);
        game.write_tiny_vtf("over_budget", [70, 80, 90, 255]);
        game.write_vmt("a_first", "shared");
        game.write_vmt("b_alias", "Materials\\Shared.VTF");
        game.write_vmt("c_second", "second");
        game.write_vmt("d_over", "over_budget");

        let vfs = Vfs::mount_game(&game.root, "hl2").unwrap();
        let names = BTreeSet::from([
            "a_first".to_owned(),
            "b_alias".to_owned(),
            "c_second".to_owned(),
            "d_over".to_owned(),
        ]);
        let progress = hl2_ui::gameui::loading::ProgressReport::default();
        let cancel = super::super::work::Cancellation::default();
        // Budget = 8 bytes: `shared` (4B, reused by b_alias) + `second` (4B) fit; `over_budget` fails.
        let (materials, errors) = load_names(
            &vfs,
            &names,
            Vec::new(),
            (0, false, false),
            8,
            (&progress, &cancel),
        )
        .unwrap();

        let first = materials["a_first"].base.as_ref().unwrap();
        let alias = materials["b_alias"].base.as_ref().unwrap();
        assert!(Arc::ptr_eq(first, alias), "alias shares single decoded Arc");
        assert_eq!(first.data, [10, 20, 30, 255]);
        assert_eq!(
            materials["c_second"].base.as_ref().unwrap().data,
            [40, 50, 60, 255]
        );
        assert!(materials["d_over"].base.is_none());
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("d_over") && errors[0].contains("budget"),
            "{errors:?}"
        );
    }

    #[test]
    fn parallel_loader_stops_promptly_when_cancelled() {
        let game = TempGame::new("cancel");
        game.write_tiny_vtf("tex", [1, 2, 3, 255]);
        game.write_vmt("mat", "tex");
        let vfs = Vfs::mount_game(&game.root, "hl2").unwrap();
        let names = BTreeSet::from(["mat".to_owned()]);
        let progress = hl2_ui::gameui::loading::ProgressReport::default();
        let cancel = super::super::work::Cancellation::default();
        cancel.cancel();
        let res = load_names(
            &vfs,
            &names,
            Vec::new(),
            (0, false, false),
            MAX_DECODED_BYTES,
            (&progress, &cancel),
        );
        assert!(res.is_err());
    }
}
