//! Off-thread map preparation and stage diagnostics.
use super::{
    FRAG_RIGID_BODY, LoadedMap, MAX_MATERIALS, MaterialData, add_static_decals, load_material,
    names, normalize_bsp_render_winding,
};
use anyhow::{Context, Result, bail};
use modkit_core::World;
use source_assets::{bsp::Bsp, models, vpk::Vfs};
use std::time::Instant;
use std::{collections::BTreeMap, path::Path, sync::Arc};

struct Timings {
    started: Instant,
    last: Instant,
    stages: BTreeMap<String, f64>,
}
impl Timings {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last: now,
            stages: BTreeMap::new(),
        }
    }
    fn mark(&mut self, name: &str) {
        let now = Instant::now();
        self.stages.insert(
            name.into(),
            now.duration_since(self.last).as_secs_f64() * 1000.,
        );
        self.last = now;
    }
    fn finish(mut self) -> BTreeMap<String, f64> {
        self.stages
            .insert("total".into(), self.started.elapsed().as_secs_f64() * 1000.);
        self.stages
    }
}

pub(crate) struct LoadInput<'a> {
    pub root: &'a Path,
    pub game: &'a str,
    pub map: &'a str,
    pub canvas: source_ui::canvas::Canvas,
    pub new_game: bool,
    pub config: &'a hl2_ui::config::Config,
    pub progress: &'a hl2_ui::gameui::loading::ProgressReport,
}
#[derive(Default)]
pub(crate) struct LoadMode {
    pub background: bool,
    pub cancel: super::work::Cancellation,
    pub audio_reuse: Arc<crate::audio::reuse::AudioReuse>,
}

pub fn load_with_progress(
    root: &Path,
    game: &str,
    map: &str,
    canvas: source_ui::canvas::Canvas,
    new_game: bool,
    config: &hl2_ui::config::Config,
    progress: &hl2_ui::gameui::loading::ProgressReport,
) -> Result<LoadedMap> {
    load(
        LoadInput {
            root,
            game,
            map,
            canvas,
            new_game,
            config,
            progress,
        },
        LoadMode::default(),
    )
}

pub(crate) fn load(input: LoadInput<'_>, mode: LoadMode) -> Result<LoadedMap> {
    let LoadInput {
        root,
        game,
        map,
        canvas,
        new_game,
        config,
        progress,
    } = input;
    mode.cancel.check()?;
    let map = map.trim_end_matches(".bsp");
    let mut timing = Timings::new();
    let root_lod = config.video.root_lod();
    let picmip = config.video.picmip();
    let hdr = config.video.hdr.is_hdr();
    let (vfs, bsp) = mount(root, game, map, hdr, progress, &mut timing)?;
    let vfs = Arc::new(vfs);
    progress.report(0.35, "LoadingProgress_PrecacheWorld");
    let PreparedWorld {
        world,
        model_report,
        weapon_models,
        decal_errors,
    } = prepare_world(&bsp, &vfs, map, config, &mut timing, &mode)?;
    progress.report(0.50, "LoadingProgress_SignonDataLocal");
    let mut gameplay = prepare_gameplay(world, &vfs, bsp.revision, new_game, config)?;
    let world = gameplay.world.clone();
    timing.mark("gameplay");
    mode.cancel.check()?;
    let (audio_work, aux_work) = parallel_resources(
        vfs.clone(),
        &gameplay,
        canvas,
        mode.audio_reuse.clone(),
        hdr,
        &mode.cancel,
    )?;
    timing.mark("parallel_plan");
    let effects = crate::effects::PreparedEffects::load_with_lod(&vfs, &world, root_lod);
    timing.mark("effects");
    let (materials, mut texture_errors) = materials(
        &vfs,
        &world,
        (&effects, &weapon_models, &gameplay),
        decal_errors,
        (picmip, hdr),
        (progress, &mode.cancel),
    )?;
    mode.cancel.check()?;
    timing.mark("materials");
    progress.report(0.90, "LoadingProgress_SignonLocal");
    let super::loading_auxiliary::Auxiliary {
        hud,
        faces,
        sky,
        cookie: flashlight_cookie,
        errors,
        timings: aux_timings,
    } = aux_work.wait()?;
    texture_errors.extend(errors);
    timing.mark("aux_wait");
    let (audio, pending_audio) = complete_audio(audio_work, mode.background, &mut gameplay)?;
    timing.mark("audio");
    progress.report(1.0, "LoadingProgress_SignonLocal");
    let shadow_receivers = Arc::new(crate::shadows::receivers(&world, &materials));
    timing.mark("shadows_cookie");
    mode.cancel.check()?;
    let mut load_timings = timing.finish();
    load_timings.extend(aux_timings);
    Ok(LoadedMap {
        load_timings,
        shadow_receivers,
        flashlight_cookie,
        game: game.to_owned(),
        effects,
        eyes: faces.eyes,
        gaze: faces.gaze,
        flexes: faces.flexes,
        sky,
        audio,
        pending_audio,
        hud,
        gameplay,
        revision: bsp.revision,
        bsp,
        world,
        materials,
        texture_errors,
        models: serde_json::to_value(model_report)?,
        vfs,
    })
}

fn prepare_gameplay(
    world: World,
    vfs: &Vfs,
    revision: u32,
    new_game: bool,
    config: &hl2_ui::config::Config,
) -> Result<crate::gameplay::Gameplay> {
    let mut gameplay =
        crate::gameplay::Gameplay::load_with_campaign(world, vfs, revision, new_game)?;
    let world = gameplay.world.clone();
    hl2_simulation::commentary::CommentaryState::spawn_init(
        &mut gameplay.scene,
        &world,
        config.gameplay.commentary,
        config.video.hdr.is_hdr(),
        new_game,
    );
    Ok(gameplay)
}

type MapAudio = (
    crate::audio::PreparedAudio,
    Option<super::work::Work<crate::audio::PreparedAudio>>,
);
fn complete_audio(
    audio_work: super::work::Work<crate::audio::PreparedAudio>,
    background: bool,
    gameplay: &mut crate::gameplay::Gameplay,
) -> Result<MapAudio> {
    let (mut audio, pending_audio) = if background {
        (crate::audio::PreparedAudio::silent(), Some(audio_work))
    } else {
        (audio_work.wait()?, None)
    };
    for (wave, sentence, seconds) in audio.sentences.drain(..) {
        gameplay
            .scene
            .lipsync
            .add_sentence(&wave, sentence, seconds);
    }
    Ok((audio, pending_audio))
}

fn parallel_resources(
    vfs: Arc<Vfs>,
    gameplay: &crate::gameplay::Gameplay,
    canvas: source_ui::canvas::Canvas,
    audio_reuse: Arc<crate::audio::reuse::AudioReuse>,
    hdr: bool,
    cancel: &super::work::Cancellation,
) -> Result<(
    super::work::Work<crate::audio::PreparedAudio>,
    super::work::Work<super::loading_auxiliary::Auxiliary>,
)> {
    cancel.check()?;
    let world = gameplay.world.clone();
    let aux_vfs = vfs.clone();
    let aux_work = super::work::Work::spawn("hl2-visual-auxiliary", move |cancel| {
        super::loading_auxiliary::load(&aux_vfs, &world, canvas, hdr, &cancel)
    })
    .map_err(anyhow::Error::msg)?;
    let plan = crate::audio::PreparedAudio::plan(&vfs, gameplay);
    cancel.check()?;
    let audio_vfs = vfs.clone();
    let audio_work = super::work::Work::spawn("hl2-audio-preparation", move |cancel| {
        plan.decode(&audio_vfs, &cancel, &audio_reuse)
    })
    .map_err(anyhow::Error::msg)?;
    Ok((audio_work, aux_work))
}

struct PreparedWorld {
    world: World,
    model_report: models::ModelReport,
    weapon_models: Vec<String>,
    decal_errors: Vec<String>,
}
fn prepare_world(
    bsp: &Bsp,
    vfs: &Vfs,
    map: &str,
    config: &hl2_ui::config::Config,
    timing: &mut Timings,
    mode: &LoadMode,
) -> Result<PreparedWorld> {
    mode.cancel.check()?;
    let mut world = bsp.world(map).context("decode Source world")?;
    world.warnings.extend(vfs.warnings.iter().cloned());
    normalize_bsp_render_winding(&mut world);
    // Static infodecals land on world brush faces only, before static props are merged.
    let mut decal_errors = Vec::new();
    add_static_decals(&mut world, vfs, &mut decal_errors);
    // F4 / npc_create spawn pool: hidden NPCs every load-time system builds.
    if !mode.background {
        hl2_simulation::ai::spawn::append_pool(&mut world);
    }
    // CreateRagGib models (the zombie's legs), precached like the zombie's Precache.
    hl2_simulation::ai::spawn::append_gib_pool(&mut world);
    // CNPC_Citizen::SelectModel (citizen types, random heads), before models load.
    hl2_simulation::citizens::select_models(&mut world, hl2_simulation::ai::spawn::POOL_KEY);
    let commentary_enabled = config.gameplay.commentary;
    if commentary_enabled {
        hl2_simulation::commentary::append_world_commentary(&mut world, vfs, map);
    }
    timing.mark("world");
    let (model_report, weapon_models) = prepare_models(&mut world, vfs, config.video.root_lod());
    mode.cancel.check()?;
    timing.mark("models");
    Ok(PreparedWorld {
        world,
        model_report,
        weapon_models,
        decal_errors,
    })
}

fn mount(
    root: &Path,
    game: &str,
    map: &str,
    hdr: bool,
    progress: &hl2_ui::gameui::loading::ProgressReport,
    timing: &mut Timings,
) -> Result<(Vfs, Bsp)> {
    if map.is_empty()
        || map.len() > 128
        || !map
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        bail!("map must be a bare Source map name");
    }
    progress.report(0.05, "LoadingProgress_SpawningServer");
    if game.is_empty()
        || !game
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        bail!("game must be a bare installation folder name");
    }
    let mut vfs = Vfs::mount_game(root, game).context("mount installed owned content")?;
    timing.mark("mount");
    progress.report(0.15, "LoadingProgress_LoadMap");
    let bytes = vfs
        .read(&format!("maps/{map}.bsp"))?
        .with_context(|| format!("owned map {game}/maps/{map}.bsp not found"))?;
    if bytes.len() > 512 * 1024 * 1024 {
        bail!("owned map exceeds 512 MiB read limit");
    }
    let bsp = Bsp::parse_with(&bytes, hdr).with_context(|| format!("decode {map}.bsp"))?;
    vfs.mount_pak(bsp.lump(40)).context("mount map pak")?;
    timing.mark("bsp");
    Ok((vfs, bsp))
}

fn prepare_models(
    world: &mut World,
    vfs: &Vfs,
    root_lod: usize,
) -> (models::ModelReport, Vec<String>) {
    let model_report = models::append_models_lod(world, vfs, root_lod);
    let _gibs_without_physics = models::append_break_physics(world, vfs);
    // NPC weapon world models: their sequence events fire NPC weapons (DESIGN 14w).
    let weapon_models: Vec<String> = hl2_simulation::npc_weapons::definitions(vfs)
        .map(|defs| {
            defs.into_values()
                .map(|d| d.world_model)
                .filter(|m| !m.is_empty())
                .collect()
        })
        .unwrap_or_default();
    for failure in models::append_rigs(world, vfs, &weapon_models) {
        world.warnings.push(failure);
    }
    // The player model's hitboxes take NPC bullets (CHL2_Player::Spawn).
    for failure in models::append_rigs(
        world,
        vfs,
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
        match models::read_model_lod(vfs, model, 0, root_lod) {
            Ok(surfaces) => {
                world.model_assets.insert(key, surfaces);
            }
            Err(error) => world.warnings.push(format!("{model}: {error:#}")),
        }
    }
    // Dropped NPC weapons are VPhysics objects (CBaseCombatWeapon::FallInit): their
    // world models' .phy (a model without one is not dropped).
    for model in &weapon_models {
        let _ = models::append_model_physics(world, vfs, model);
    }
    // The thrown frag is a VPhysics object (CGrenadeFrag::CreateVPhysics): a Rapier body
    // from w_grenade.phy (12-sided prism as a round cylinder, the .phy's IVP inertia) and
    // SDK spawn orientation. Session 13 replay against five native throws (lob, roll,
    // overhand, plaza, wall) fits as well as or better than the old fitted swept box.
    if FRAG_RIGID_BODY.load(std::sync::atomic::Ordering::Relaxed)
        && let Err(error) =
            models::append_model_physics(world, vfs, hl2_simulation::projectiles::FRAG_MODEL)
    {
        world.warnings.push(format!(
            "{}: {error:#}",
            hl2_simulation::projectiles::FRAG_MODEL
        ));
    }
    (model_report, weapon_models)
}

fn materials(
    vfs: &Vfs,
    world: &World,
    (effects, weapon_models, gameplay): (
        &crate::effects::PreparedEffects,
        &[String],
        &crate::gameplay::Gameplay,
    ),
    decal_errors: Vec<String>,
    (picmip, hdr): (i32, bool),
    (progress, cancel): (
        &hl2_ui::gameui::loading::ProgressReport,
        &super::work::Cancellation,
    ),
) -> Result<(BTreeMap<String, MaterialData>, Vec<String>)> {
    let names = names::map_material_names(world, effects, weapon_models, gameplay);
    if names.len() > MAX_MATERIALS {
        bail!("map exceeds {MAX_MATERIALS} unique material limit");
    }
    let mut materials = BTreeMap::new();
    let mut texture_errors = decal_errors;
    let mut decoded_bytes = 0usize;
    let mut texture_cache = BTreeMap::new();
    let mut frame_cache = BTreeMap::new();
    let total_materials = names.len().max(1);
    for (i, name) in names.into_iter().enumerate() {
        cancel.check()?;
        if i % 10 == 0 || i + 1 == total_materials {
            let frac = 0.55 + 0.30 * (i as f32 / total_materials as f32);
            progress.report(frac, "LoadingProgress_LoadResources");
        }
        let material = load_material(
            vfs,
            &name,
            &mut texture_cache,
            &mut frame_cache,
            &mut decoded_bytes,
            &mut texture_errors,
            (picmip, hdr),
        );
        materials.insert(name, material);
    }
    Ok((materials, texture_errors))
}
