use crate::bevy::{BlurFrame, Hl2AdvancedOptionsPlugin, PrepareBlurSet, ReconstructionSet};
use bevy::prelude::{
    App, Commands, Entity, IntoScheduleConfigs, PostStartup, PostUpdate, Query, Res, ResMut,
    Visibility, With, Without,
};

pub fn install(_app: &mut App) {
    _app.add_plugins(Hl2AdvancedOptionsPlugin)
        .add_systems(PostStartup, tag_views)
        .add_systems(PostUpdate, tag_views.before(snapshot))
        .add_systems(
            PostUpdate,
            snapshot
                .before(PrepareBlurSet)
                .after(hl2_bevy::camera::present_player_view)
                .after(hl2_bevy::rendering::present_weapons)
                .after(hl2_bevy::rendering::present_entities),
        );
    install_diagnostics(_app);
    if let Some(render) = _app.get_sub_app_mut(bevy::render::RenderApp) {
        render.configure_sets(
            bevy::core_pipeline::schedule::Core3d,
            ReconstructionSet.before(hl2_bevy::bloom::BloomSet),
        );
    }
}

fn snapshot(
    mut frame: ResMut<BlurFrame>,
    ui: Option<Res<hl2_bevy::ui_host::Console>>,
    game: Option<Res<hl2_bevy::gameplay::Gameplay>>,
    sim: Option<Res<hl2_bevy::simulation::Simulation>>,
    draws: Query<
        (&hl2_bevy::rendering::WeaponMesh, &Visibility),
        With<bevy::mesh::skinning::SkinnedMesh>,
    >,
) {
    let enabled = sim.is_some_and(|s| !s.paused() && !s.background_level);
    let mode = ui.map(|u| u.source.config.video.advanced.motion_blur_mode());
    let Some(game) = game else {
        frame.viewmodel = false;
        frame.world = false;
        return;
    };
    frame.map = std::sync::Arc::as_ptr(&game.world) as usize;
    frame.time = game.scene.time;
    if frame.weapon != game.inventory.active {
        frame.weapon.clone_from(&game.inventory.active);
    }
    frame.world = enabled && mode.is_some_and(|m| m.world());
    frame.viewmodel = enabled
        && mode.is_some_and(|m| m.viewmodel())
        && !game.inventory.carrying()
        && draws
            .iter()
            .any(|(w, v)| w.0 == game.inventory.active && *v != Visibility::Hidden);
}

fn tag_views(
    mut commands: Commands,
    world: Query<Entity, (With<hl2_bevy::FlyCamera>, Without<crate::bevy::WorldCamera>)>,
    weapon: Query<
        Entity,
        (
            With<hl2_bevy::camera::ViewmodelCamera>,
            Without<crate::bevy::WeaponCamera>,
        ),
    >,
) {
    for entity in &world {
        commands.entity(entity).insert(crate::bevy::WorldCamera);
    }
    for entity in &weapon {
        commands.entity(entity).insert(crate::bevy::WeaponCamera);
    }
}
fn install_diagnostics(app: &mut App) {
    let Some(performance) = app
        .world()
        .get_resource::<hl2_bevy::performance::Performance>()
        .cloned()
    else {
        return;
    };
    let sink = crate::bevy::diagnostics::Diagnostics(std::sync::Arc::new(Reporter(performance)));
    app.insert_resource(sink.clone());
    if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render.insert_resource(sink);
        crate::bevy::install_memory_probe(render);
    }
}
struct Reporter(hl2_bevy::performance::Performance);
impl crate::bevy::diagnostics::Reporter for Reporter {
    fn memory(&self, report: crate::bevy::BlurMemory) {
        self.0.record_contribution(
            "blur_memory",
            serde_json::to_value(report).expect("render allocation report"),
        );
    }
    fn reduction(&self, world: bool, processed: u64, full: u64) {
        self.0.record_reduction(world, processed, full);
    }
    fn stage(&self, name: &'static str, milliseconds: f64) {
        self.0.record_stage(name, milliseconds);
    }
}
