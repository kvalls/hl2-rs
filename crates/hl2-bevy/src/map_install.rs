//! Installing a loaded map: its draws, effects, sky, audio and map-owned resources.
use crate::{
    Status, assets, audio, canvas_renderer, console_host, effects, eyes, monitors, rendering,
    runtime_props, shadows, sky, studio_decals, visibility,
};
use bevy::{
    asset::Assets,
    audio::AudioSource,
    ecs::{system::Commands, world::World},
    image::Image,
    mesh::Mesh,
};

pub(crate) type StartupAssets<'w> = (
    bevy::ecs::system::ResMut<'w, Assets<Mesh>>,
    bevy::ecs::system::ResMut<'w, Assets<rendering::SourceMaterial>>,
    bevy::ecs::system::ResMut<'w, Assets<Image>>,
    bevy::ecs::system::ResMut<'w, Assets<AudioSource>>,
    bevy::ecs::system::ResMut<'w, Assets<effects::EffectMaterial>>,
    bevy::ecs::system::ResMut<'w, Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
);

pub(crate) type MapAssets<'a> = (
    &'a mut Assets<Mesh>,
    &'a mut Assets<rendering::SourceMaterial>,
    &'a mut Assets<Image>,
    &'a mut Assets<AudioSource>,
    &'a mut Assets<effects::EffectMaterial>,
    &'a mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
);

pub(crate) fn install_loaded_map(
    loaded: assets::LoadedMap,
    commands: &mut Commands,
    assets: &mut StartupAssets,
    status: &Status,
    options: &crate::Options,
    filtering: hl2_ui::config::Filtering,
    target: Option<&bevy::asset::Handle<Image>>,
) {
    let (meshes, materials, images, sounds, effect_materials, inverse_binds) = assets;
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
        ),
        status,
        (
            options.cpu_skinning,
            options.no_pvs,
            options.world_partition,
        ),
        (filtering, options.mute),
        target,
    );
}

pub(crate) fn install_map(
    loaded: assets::LoadedMap,
    commands: &mut Commands,
    (meshes, materials, images, sounds, effect_materials, inverse_binds): MapAssets,
    status: &Status,
    (cpu_skinning, no_pvs, world_partition): (bool, bool, bool),
    (filtering, mute): (hl2_ui::config::Filtering, bool),
    target: Option<&bevy::asset::Handle<Image>>,
) {
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
    let camera_target = monitors::install(&loaded, commands, images);
    rendering::spawn_map_filtered(
        &loaded,
        commands,
        (meshes, inverse_binds),
        materials,
        images,
        status,
        (&camera_target, cpu_skinning, world_partition),
        filtering,
    );
    effects::install(
        &loaded,
        commands,
        meshes,
        materials,
        effect_materials,
        images,
    );
    commands.insert_resource(eyes::Eyes::new(&loaded));
    commands.insert_resource(shadows::MapShadows::new(&loaded));
    commands.insert_resource(console_host::MapEntityNames::of(&loaded.world.entities));
    sky::install(&loaded, commands, meshes, materials, images, target);
    commands.insert_resource(sky::Sky::new(
        loaded.bsp,
        loaded.world.background_camera.clone(),
        loaded.sky.as_ref(),
    ));
    effects::adopt(loaded.effects.sprites, commands);
    commands.insert_resource(loaded.gameplay);
    // Studio decal draws went with the old map; its entity indices now name the new map's.
    commands.queue(|world: &mut World| {
        if let Some(mut decals) = world.get_resource_mut::<studio_decals::StudioDecals>() {
            decals.new_map();
        }
    });
    commands.insert_resource(canvas_renderer::Hud::new(loaded.hud));
    audio::install(commands, sounds, loaded.audio, mute);
}
