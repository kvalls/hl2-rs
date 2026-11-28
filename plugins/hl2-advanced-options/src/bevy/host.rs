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
                .after(hl2_bevy::rendering::present_entities)
                .before(hl2_bevy::rendering::variant_warmup::warm_hidden_variants),
        );
    install_diagnostics(_app);
    if let Some(render) = _app.get_sub_app_mut(bevy::render::RenderApp) {
        render.configure_sets(
            bevy::core_pipeline::schedule::Core3d,
            // Each pass that swaps the view's main textures is ordered against the others.
            ReconstructionSet
                .after(hl2_bevy::bloom::MotionBlurSet)
                .before(hl2_bevy::bloom::BloomSet),
        );
    }
}

fn frame_flags(
    mode: Option<crate::motion_blur::MotionBlurMode>,
    sim: Option<&hl2_bevy::simulation::Simulation>,
    warmup: Option<&hl2_bevy::rendering::variant_warmup::VariantWarmup>,
    carrying: bool,
    active_shown: bool,
) -> (bool, bool) {
    let enabled = sim.is_some_and(|s| !s.paused() && !s.background_level);
    let warming = warmup.is_some_and(|w| w.active())
        && sim.is_some_and(|s| !s.background_level && s.loading());
    let world = (enabled || warming) && mode.is_some_and(|m| m.world());
    let viewmodel =
        mode.is_some_and(|m| m.viewmodel()) && (warming || (enabled && !carrying && active_shown));
    (world, viewmodel)
}

fn snapshot(
    mut frame: ResMut<BlurFrame>,
    ui: Option<Res<hl2_bevy::ui_host::Console>>,
    game: Option<Res<hl2_bevy::gameplay::Gameplay>>,
    sim: Option<Res<hl2_bevy::simulation::Simulation>>,
    warmup: Option<Res<hl2_bevy::rendering::variant_warmup::VariantWarmup>>,
    draws: Query<
        (&hl2_bevy::rendering::WeaponMesh, &Visibility),
        With<bevy::mesh::skinning::SkinnedMesh>,
    >,
) {
    let mode = ui.map(|u| crate::motion_blur::selected(&u.source.config.video.advanced));
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
    let active_shown = draws
        .iter()
        .any(|(w, v)| w.0 == game.inventory.active && *v != Visibility::Hidden);
    (frame.world, frame.viewmodel) = frame_flags(
        mode,
        sim.as_deref(),
        warmup.as_deref(),
        game.inventory.carrying(),
        active_shown,
    );
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

#[cfg(test)]
mod tests {
    use super::frame_flags;
    use crate::motion_blur::MotionBlurMode;

    #[test]
    fn loading_warmup_keeps_viewmodel_prepass_active_when_viewmodel_blur_is_enabled() {
        let mut sim = hl2_bevy::simulation::Simulation::new(
            &modkit_core::World::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            false,
            vec![],
        );
        sim.loading = true;
        let warmup = hl2_bevy::rendering::variant_warmup::VariantWarmup::new();

        assert_eq!(
            frame_flags(
                Some(MotionBlurMode::Advanced),
                Some(&sim),
                Some(&warmup),
                false,
                false,
            ),
            (false, true)
        );
        assert_eq!(
            frame_flags(
                Some(MotionBlurMode::Advanced),
                Some(&sim),
                None,
                false,
                true,
            ),
            (false, false)
        );
        assert_eq!(
            frame_flags(
                Some(MotionBlurMode::Disabled),
                Some(&sim),
                Some(&warmup),
                false,
                false,
            ),
            (false, false)
        );

        sim.background_level = true;
        assert_eq!(
            frame_flags(
                Some(MotionBlurMode::Advanced),
                Some(&sim),
                Some(&warmup),
                false,
                false,
            ),
            (false, false)
        );
    }
}
