//! Installing a loaded map: its draws, effects, sky, audio and map-owned resources.
use crate::audio::mixer::MixerStream;
use crate::{
    Status, assets, audio, canvas_renderer, console_host, effects, eyes, monitors, rendering,
    runtime_npcs, runtime_props, shadows, sky, studio_decals, visibility,
};
use bevy::{
    asset::Assets,
    ecs::{system::Commands, world::World},
    image::Image,
    mesh::Mesh,
};

pub(crate) type StartupAssets<'w> = (
    bevy::ecs::system::ResMut<'w, Assets<Mesh>>,
    bevy::ecs::system::ResMut<'w, Assets<rendering::SourceMaterial>>,
    bevy::ecs::system::ResMut<'w, Assets<Image>>,
    bevy::ecs::system::ResMut<'w, Assets<MixerStream>>,
    bevy::ecs::system::ResMut<'w, Assets<effects::EffectMaterial>>,
    bevy::ecs::system::ResMut<'w, Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
    bevy::ecs::system::ResMut<'w, crate::material_slots::MaterialSlots>,
);

pub(crate) type MapAssets<'a> = (
    &'a mut Assets<Mesh>,
    &'a mut Assets<rendering::SourceMaterial>,
    &'a mut Assets<Image>,
    &'a mut Assets<MixerStream>,
    &'a mut Assets<effects::EffectMaterial>,
    &'a mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    &'a mut crate::material_slots::MaterialSlots,
);

/// Milliseconds for the load report: the whole install (`install_ms`) and each step
/// (`install_<step>`).
pub(crate) type InstallTimes = std::collections::BTreeMap<String, f64>;

/// Times the install and its consecutive steps.
struct Laps {
    started: std::time::Instant,
    last: std::time::Instant,
    times: InstallTimes,
}
impl Laps {
    fn new() -> Self {
        let now = std::time::Instant::now();
        Self {
            started: now,
            last: now,
            times: InstallTimes::new(),
        }
    }
    fn mark(&mut self, step: &str) {
        let now = std::time::Instant::now();
        let ms = (now - self.last).as_secs_f64() * 1000.;
        self.times.insert(format!("install_{step}"), ms);
        self.last = now;
    }
    fn finish(mut self) -> InstallTimes {
        let ms = self.started.elapsed().as_secs_f64() * 1000.;
        self.times.insert("install_ms".into(), ms);
        self.times
    }
}

pub(crate) fn install_loaded_map(
    loaded: assets::LoadedMap,
    commands: &mut Commands,
    assets: &mut StartupAssets,
    status: &Status,
    options: &crate::Options,
    (filtering, audio_reuse): (
        hl2_ui::config::TextureSampling,
        std::sync::Arc<crate::audio::reuse::AudioReuse>,
    ),
    target: Option<&bevy::asset::Handle<Image>>,
) -> InstallTimes {
    let (meshes, materials, images, sounds, effect_materials, inverse_binds, slots) = assets;
    install_map(
        loaded,
        commands,
        (
            &mut **meshes,
            &mut **materials,
            &mut **images,
            &mut **sounds,
            &mut **effect_materials,
            &mut **inverse_binds,
            &mut **slots,
        ),
        status,
        (
            options.cpu_skinning,
            options.no_pvs,
            options.world_partition,
        ),
        (filtering, options.mute, audio_reuse),
        target,
    )
}

pub(crate) fn install_map(
    mut loaded: assets::LoadedMap,
    commands: &mut Commands,
    (meshes, materials, images, sounds, effect_materials, inverse_binds, slots): MapAssets,
    status: &Status,
    (cpu_skinning, no_pvs, world_partition): (bool, bool, bool),
    (filtering, mute, audio_reuse): (
        hl2_ui::config::TextureSampling,
        bool,
        std::sync::Arc<crate::audio::reuse::AudioReuse>,
    ),
    target: Option<&bevy::asset::Handle<Image>>,
) -> InstallTimes {
    let mut laps = Laps::new();
    {
        let mut stats = status.0.lock().expect("map stats");
        stats.meshes = 0;
        stats.partitioned_world_batches = 0;
        stats.gpu_skinned_meshes = 0;
        stats.cpu_skinned_meshes = 0;
        stats.skin_joints = 0;
        stats.triangles = 0;
        stats.materials = 0;
        stats.skipped_background_surfaces = 0;
    }
    commands.insert_resource(visibility::SourceVisibility::new(
        loaded.bsp.visibility_index(),
        no_pvs,
    ));
    commands.insert_resource(rendering::FlexModels(loaded.flexes.clone()));
    commands.insert_resource(runtime_props::RuntimeProps::new(loaded.vfs.clone()));
    commands.insert_resource(runtime_npcs::RuntimeNpcs::new(
        loaded.vfs.clone(),
        audio_reuse,
    ));
    let camera_target = monitors::install(&loaded, commands, images);
    laps.mark("monitors");
    effects::install(
        &loaded,
        commands,
        meshes,
        materials,
        effect_materials,
        images,
    );
    laps.mark("effects");
    commands.insert_resource(shadows::MapShadows::new(&loaded));
    slots.clear_lights();
    rendering::spawn_map_filtered(
        &mut loaded,
        commands,
        (meshes, inverse_binds),
        (materials, slots),
        images,
        status,
        (&camera_target, cpu_skinning, world_partition),
        filtering,
    );
    laps.mark("draws");
    commands.insert_resource(eyes::Eyes::new(&loaded));
    commands.insert_resource(console_host::MapEntityNames::of(&loaded.world.entities));
    sky::install(&loaded, commands, meshes, materials, images, target);
    laps.mark("sky");
    commands.insert_resource(sky::Sky::new(
        loaded.bsp,
        loaded.world.background_camera.clone(),
        loaded.sky.as_ref(),
    ));
    effects::adopt(loaded.effects.sprites, commands);
    audio::deferred::stage(commands, loaded.pending_audio);
    let extension_vfs = loaded.vfs.clone();
    commands.insert_resource(loaded.gameplay);
    commands.queue(move |world: &mut World| {
        world.resource_scope(
            |world, mut simulation: bevy::prelude::Mut<crate::simulation::Simulation>| {
                if let Some(mut game) = world.get_resource_mut::<crate::gameplay::Gameplay>() {
                    for extension in &mut simulation.extensions {
                        extension.map_installed(&mut game, &extension_vfs);
                    }
                }
            },
        );
    });
    // Studio decal draws went with the old map; its entity indices now name the new map's.
    commands.queue(|world: &mut World| {
        if let Some(mut decals) = world.get_resource_mut::<studio_decals::StudioDecals>() {
            decals.new_map();
        }
    });
    commands.insert_resource(canvas_renderer::Hud::new(loaded.canvas, loaded.hud));
    audio::install(commands, sounds, loaded.audio, mute);
    laps.mark("rest");
    laps.finish()
}
