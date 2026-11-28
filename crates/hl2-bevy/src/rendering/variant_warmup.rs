//! Shader-variant warm-up after a map install (O9). Bevy compiles render pipelines on the
//! render thread on macOS, so the first draw of a hidden template (a spawn-pool NPC, a
//! template-maker NPC, a hidden prop) used to compile its variants mid-play, the moment it
//! appeared. For the first frames after the install, every hidden entity draw is drawn by the
//! main view itself (so the pipeline keys match its MSAA, prepasses and tonemapping exactly)
//! far below the map with frustum culling off, where nothing reaches the screen; the next
//! `present_entities` restores its pose and hidden state. Hidden viewmodel draws (weapons not
//! yet shown, other skins) are drawn the same frames by the viewmodel camera, which a shown
//! weapon turns on; the loading screen stays up until the warm-up ends.
use super::{SourceEntity, WeaponMesh};
use bevy::{
    camera::visibility::NoFrustumCulling,
    ecs::{hierarchy::Children, query::Has},
    prelude::{
        Commands, Component, Entity, Mesh3d, Query, Res, ResMut, Resource, Transform, Visibility,
        With, Without,
    },
};

/// Frames of warm-up left after the current map was installed.
#[derive(Resource)]
pub struct VariantWarmup {
    frames: u8,
}
impl VariantWarmup {
    /// Four frames: the first queues the variants, the next draw with them in place. A skinned
    /// draw keys its pipelines on the previous frame's skin (`HAS_PREVIOUS_SKIN`), which it
    /// has only from its second drawn frame, so those variants queue a frame later; two frames
    /// left them to compile on first sight in play.
    pub fn new() -> Self {
        Self { frames: 4 }
    }
    /// True while warm-up frames remain to draw before cleanup.
    pub fn active(&self) -> bool {
        self.frames > 0
    }
}

impl Default for VariantWarmup {
    fn default() -> Self {
        Self::new()
    }
}

/// Marks what the warm-up changed, so exactly that is undone.
#[derive(Component)]
pub struct Warming;

/// Marks a viewmodel draw the warm-up shows.
#[derive(Component)]
pub(crate) struct WarmingWeapon;

type WeaponDraws<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static mut Visibility, Has<WarmingWeapon>),
    (With<WeaponMesh>, With<Mesh3d>),
>;

/// Shows every hidden viewmodel draw during the warm-up. Runs after `present_weapons` (which
/// hides them again each frame) and before `sync_viewmodel_camera`, which then turns the
/// viewmodel camera on and hands it the bloom and colour correction, as when a weapon is
/// drawn in play, so the pipelines compile with play's keys. Afterwards `present_weapons`'
/// own visibility stands again.
pub(crate) fn warm_weapon_variants(
    mut commands: Commands,
    warmup: Option<Res<VariantWarmup>>,
    sim: Option<Res<crate::simulation::Simulation>>,
    mut draws: WeaponDraws,
) {
    let warming = warmup.is_some_and(|w| w.active())
        && sim
            .as_deref()
            .is_none_or(|s| !s.background_level && s.loading());
    for (entity, mut visibility, marked) in &mut draws {
        if !warming {
            if marked {
                commands
                    .entity(entity)
                    .remove::<(WarmingWeapon, NoFrustumCulling)>();
            }
            continue;
        }
        if *visibility == Visibility::Hidden {
            *visibility = Visibility::Inherited;
            if !marked {
                commands
                    .entity(entity)
                    .insert((WarmingWeapon, NoFrustumCulling));
            }
        }
    }
}

type Roots<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static mut Transform, &'static mut Visibility),
    (With<SourceEntity>, Without<crate::gpu_skinning::Joint>),
>;

/// Runs after `present_entities`, before transforms propagate and visibility is checked.
pub fn warm_hidden_variants(
    mut commands: Commands,
    (warmup, backlog): (Option<ResMut<VariantWarmup>>, Option<Res<PipelineBacklog>>),
    mut roots: Roots,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    warming: Query<Entity, With<Warming>>,
) {
    let Some(mut warmup) = warmup else { return };
    if let Some(backlog) = &backlog {
        backlog.set_warming(warmup.frames > 0);
    }
    if warmup.frames == 0 {
        for entity in &warming {
            commands
                .entity(entity)
                .remove::<(Warming, NoFrustumCulling)>();
        }
        commands.remove_resource::<VariantWarmup>();
        return;
    }
    warmup.frames -= 1;
    let below = crate::coords::source_to_bevy(bevy::math::Vec3::from_array(
        hl2_simulation::ai::spawn::POOL_ORIGIN.to_array(),
    ));
    for (root, mut transform, mut visibility) in &mut roots {
        if *visibility != Visibility::Hidden {
            continue;
        }
        *visibility = Visibility::Inherited;
        transform.translation = below;
        for entity in std::iter::once(root).chain(children.iter_descendants(root)) {
            if meshes.contains(entity) || entity == root {
                commands.entity(entity).insert((Warming, NoFrustumCulling));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{VariantWarmup, Warming, warm_hidden_variants};
    use crate::rendering::SourceEntity;
    use bevy::{
        camera::visibility::NoFrustumCulling,
        ecs::system::RunSystemOnce,
        prelude::{Mesh3d, Transform, Vec3, Visibility, World},
    };

    #[test]
    fn hidden_draws_warm_far_below_for_four_frames_then_everything_is_undone() {
        let mut world = World::new();
        let hidden = world
            .spawn((SourceEntity(0), Transform::default(), Visibility::Hidden))
            .id();
        let mesh = world.spawn(Mesh3d::default()).id();
        world.entity_mut(hidden).add_child(mesh);
        let shown = world
            .spawn((SourceEntity(1), Transform::default(), Visibility::Inherited))
            .id();
        world.insert_resource(VariantWarmup::new());
        for _ in 0..4 {
            // `present_entities` hides it again each frame before the warm-up runs.
            *world.get_mut::<Visibility>(hidden).unwrap() = Visibility::Hidden;
            world.run_system_once(warm_hidden_variants).unwrap();
            assert_eq!(
                world.get::<Visibility>(hidden),
                Some(&Visibility::Inherited)
            );
            assert!(world.get::<Transform>(hidden).unwrap().translation.length() > 10_000.);
            for e in [hidden, mesh] {
                assert!(world.get::<NoFrustumCulling>(e).is_some());
            }
            assert!(world.get::<NoFrustumCulling>(shown).is_none());
            assert_eq!(
                world.get::<Transform>(shown).unwrap().translation,
                Vec3::ZERO
            );
        }
        world.run_system_once(warm_hidden_variants).unwrap();
        for e in [hidden, mesh] {
            assert!(world.get::<NoFrustumCulling>(e).is_none());
            assert!(world.get::<Warming>(e).is_none());
        }
        assert!(world.get_resource::<VariantWarmup>().is_none());
    }

    #[test]
    fn hidden_weapons_warm_for_four_frames_and_clean_up() {
        use super::{PipelineBacklog, WarmingWeapon, WeaponMesh, warm_weapon_variants};
        let mut world = World::new();
        let hidden = world
            .spawn((
                WeaponMesh("crowbar".into()),
                Mesh3d::default(),
                Visibility::Hidden,
            ))
            .id();
        let shown = world
            .spawn((
                WeaponMesh("pistol".into()),
                Mesh3d::default(),
                Visibility::Inherited,
            ))
            .id();
        world.insert_resource(VariantWarmup::new());
        world.insert_resource(PipelineBacklog::default());
        for _ in 0..4 {
            *world.get_mut::<Visibility>(hidden).unwrap() = Visibility::Hidden;
            world.run_system_once(warm_weapon_variants).unwrap();
            world.run_system_once(warm_hidden_variants).unwrap();
            assert_eq!(
                world.get::<Visibility>(hidden),
                Some(&Visibility::Inherited)
            );
            assert!(world.get::<WarmingWeapon>(hidden).is_some());
            assert!(world.get::<NoFrustumCulling>(hidden).is_some());
            assert!(world.get::<WarmingWeapon>(shown).is_none());
            assert!(world.get::<NoFrustumCulling>(shown).is_none());
            assert!(world.resource::<PipelineBacklog>().warming());
        }
        *world.get_mut::<Visibility>(hidden).unwrap() = Visibility::Hidden;
        world.run_system_once(warm_weapon_variants).unwrap();
        world.run_system_once(warm_hidden_variants).unwrap();
        assert_eq!(world.get::<Visibility>(hidden), Some(&Visibility::Hidden));
        assert!(world.get::<WarmingWeapon>(hidden).is_none());
        assert!(world.get::<NoFrustumCulling>(hidden).is_none());
        assert!(!world.resource::<PipelineBacklog>().warming());
        assert!(world.get_resource::<VariantWarmup>().is_none());
    }

    #[test]
    fn background_level_does_not_warm_weapons() {
        use super::{WarmingWeapon, WeaponMesh, warm_weapon_variants};
        let mut world = World::new();
        let hidden = world
            .spawn((
                WeaponMesh("crowbar".into()),
                Mesh3d::default(),
                Visibility::Hidden,
            ))
            .id();
        world.insert_resource(VariantWarmup::new());
        world.insert_resource(crate::simulation::Simulation::new(
            &modkit_core::World::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            true,
            vec![],
        ));
        world.run_system_once(warm_weapon_variants).unwrap();
        assert_eq!(world.get::<Visibility>(hidden), Some(&Visibility::Hidden));
        assert!(world.get::<WarmingWeapon>(hidden).is_none());
        assert!(world.get::<NoFrustumCulling>(hidden).is_none());
    }
}

/// Pipelines the render world still has waiting to compile, as of its last frame, and whether
/// warm-up frames are still to draw; shared with the main world so a held loading screen can
/// wait for both (`campaign::loading::advance_reveal`).
#[derive(Resource, Clone, Default)]
pub struct PipelineBacklog(
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
);
impl PipelineBacklog {
    pub(crate) fn waiting(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub(crate) fn warming(&self) -> bool {
        self.1.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn set_warming(&self, warming: bool) {
        self.1.store(warming, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Publishes the render world's pipeline queue length every frame.
pub(crate) struct PipelineBacklogPlugin;
impl bevy::app::Plugin for PipelineBacklogPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        let backlog = PipelineBacklog::default();
        app.insert_resource(backlog.clone());
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            use bevy::ecs::schedule::IntoScheduleConfigs;
            render_app.insert_resource(backlog).add_systems(
                bevy::render::Render,
                publish_backlog.in_set(bevy::render::RenderSystems::Cleanup),
            );
        }
    }
}

fn publish_backlog(
    cache: Option<Res<bevy::render::render_resource::PipelineCache>>,
    backlog: Res<PipelineBacklog>,
) {
    let waiting = cache.map_or(0, |c| c.waiting_pipelines().count());
    backlog
        .0
        .store(waiting, std::sync::atomic::Ordering::Relaxed);
}
#[cfg(test)]
impl PipelineBacklog {
    pub(crate) fn set_waiting(&self, waiting: usize) {
        self.0.store(waiting, std::sync::atomic::Ordering::Relaxed);
    }
}
