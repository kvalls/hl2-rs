//! Startup-only drawing selection; automatic direct mode is limited to measured hardware.
use bevy::{
    camera::Camera,
    core_pipeline::prepass::MotionVectorPrepass,
    ecs::{
        change_detection::DetectChanges,
        schedule::{ScheduleCleanupPolicy, Schedules},
    },
    light::cluster::GlobalClusterSettings,
    mesh::Mesh3d,
    pbr::PreviousGlobalTransform,
    prelude::{
        App, Camera3d, Commands, Entity, GlobalTransform, Mut, Or, PreUpdate, Query, Ref, With,
        Without, World,
    },
    render::{renderer::RenderAdapterInfo, view::NoIndirectDrawing},
};

type MovableMeshFilter = (
    With<Mesh3d>,
    Or<(
        With<crate::rendering::SourceEntity>,
        With<crate::rendering::WeaponMesh>,
    )>,
);

type MotionCameras<'w, 's> =
    Query<'w, 's, &'static Camera, (With<Camera3d>, With<MotionVectorPrepass>)>;

type NewMovableMeshes<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static GlobalTransform),
    (MovableMeshFilter, Without<PreviousGlobalTransform>),
>;

type TrackedMovableMeshes<'w, 's> = Query<
    'w,
    's,
    (
        Ref<'static, GlobalTransform>,
        &'static mut PreviousGlobalTransform,
    ),
    MovableMeshFilter,
>;

/// Replaces Bevy's unfiltered [`bevy::pbr::update_mesh_previous_global_transforms`] in
/// [`PreUpdate`] with [`update_mesh_previous_global_transforms`], which runs only while an active
/// camera carries [`MotionVectorPrepass`] and tracks only movable entity/weapon meshes.
pub(crate) fn install_previous_transforms(app: &mut App) {
    app.world_mut()
        .resource_scope(|world, mut schedules: Mut<Schedules>| {
            let _ = schedules.remove_systems_in_set(
                PreUpdate,
                bevy::pbr::update_mesh_previous_global_transforms,
                world,
                ScheduleCleanupPolicy::RemoveSystemsOnly,
            );
        });
    app.add_systems(PreUpdate, update_mesh_previous_global_transforms);
}

pub(crate) fn update_mesh_previous_global_transforms(
    mut commands: Commands,
    views: MotionCameras,
    new_meshes: NewMovableMeshes,
    mut meshes: TrackedMovableMeshes,
) {
    if !views.iter().any(|camera| camera.is_active) {
        return;
    }
    for (entity, transform) in &new_meshes {
        commands
            .entity(entity)
            .try_insert(PreviousGlobalTransform(transform.affine()));
    }
    for (transform, mut previous) in &mut meshes {
        if transform.is_changed_after(previous.last_changed()) {
            let affine = transform.affine();
            if previous.0 != affine {
                *previous = PreviousGlobalTransform(affine);
            }
        }
    }
}

fn selected(macos: bool, adapter: &str, metal: bool, requested: Option<&str>) -> bool {
    match requested {
        Some("1") => true,
        Some("0") => false,
        _ => macos && metal && adapter == "Apple M1 Pro",
    }
}
fn requested<'a>(
    profile: bool,
    general: Option<&'a str>,
    profiling: Option<&'a str>,
) -> Option<&'a str> {
    if profile {
        profiling.or(general)
    } else {
        general
    }
}

pub(crate) fn configure(world: &mut World) {
    let profile = world
        .get_resource::<crate::Options>()
        .is_some_and(|o| o.profile && o.background);
    let general = std::env::var("HL2RS_DIRECT_DRAW").ok();
    let profiling = profile
        .then(|| std::env::var("HL2RS_PROFILE_DIRECT_DRAW").ok())
        .flatten();
    let adapter = world.get_resource::<RenderAdapterInfo>();
    let direct = selected(
        cfg!(target_os = "macos"),
        adapter.map_or("", |a| a.name.as_str()),
        adapter.is_some_and(|a| format!("{:?}", a.backend) == "Metal"),
        requested(profile, general.as_deref(), profiling.as_deref()),
    );
    if direct {
        world.register_required_components::<Camera3d, NoIndirectDrawing>();
    }
    if let Some(performance) = world.get_resource::<crate::performance::Performance>() {
        performance.set_camera_drawing(direct);
    }
    bevy::log::info!("camera indirect drawing disabled: {direct}");
    // Few clustered lights exist (the flashlight), so assigning them on the CPU is far cheaper than
    // GPU clustering's five passes and readback for every camera, each frame.
    let gpu_clustering = std::env::var("HL2RS_GPU_CLUSTERING").is_ok_and(|value| value == "1");
    if !gpu_clustering && let Some(mut settings) = world.get_resource_mut::<GlobalClusterSettings>()
    {
        settings.gpu_clustering = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_policy_is_limited_to_the_measured_metal_adapter() {
        assert!(selected(true, "Apple M1 Pro", true, None));
        assert!(!selected(false, "Apple M1 Pro", true, None));
        assert!(!selected(true, "Apple M1 Pro", false, None));
        for name in ["", "Apple M2", "Intel GPU"] {
            assert!(!selected(true, name, true, None));
        }
    }
    #[test]
    fn startup_overrides_are_explicit_and_profile_specific() {
        assert!(selected(false, "unknown", false, Some("1")));
        assert!(!selected(true, "Apple M1 Pro", true, Some("0")));
        assert_eq!(requested(false, None, Some("1")), None);
        assert_eq!(requested(false, Some("0"), Some("1")), Some("0"));
        assert_eq!(requested(true, Some("0"), Some("1")), Some("1"));
        assert_eq!(requested(true, Some("0"), None), Some("0"));
    }

    #[test]
    fn previous_transforms_track_only_movable_meshes_when_motion_vectors_are_active() {
        use bevy::{
            asset::Handle, ecs::system::RunSystemOnce, math::Vec3, transform::components::Transform,
        };

        let mut world = World::new();
        let camera = world
            .spawn((
                Camera3d::default(),
                Camera {
                    is_active: true,
                    ..Default::default()
                },
            ))
            .id();
        let world_mesh = world
            .spawn((
                Mesh3d(Handle::default()),
                GlobalTransform::from(Transform::from_xyz(1.0, 2.0, 3.0)),
            ))
            .id();
        let entity_mesh = world
            .spawn((
                Mesh3d(Handle::default()),
                crate::rendering::SourceEntity(7),
                GlobalTransform::from(Transform::from_xyz(4.0, 5.0, 6.0)),
            ))
            .id();

        // Without MotionVectorPrepass on an active camera, nothing is inserted.
        world
            .run_system_once(update_mesh_previous_global_transforms)
            .unwrap();
        assert!(world.get::<PreviousGlobalTransform>(world_mesh).is_none());
        assert!(world.get::<PreviousGlobalTransform>(entity_mesh).is_none());

        // With MotionVectorPrepass on the active camera, only the movable entity mesh is tracked.
        world.entity_mut(camera).insert(MotionVectorPrepass);
        world
            .run_system_once(update_mesh_previous_global_transforms)
            .unwrap();
        assert!(world.get::<PreviousGlobalTransform>(world_mesh).is_none());
        assert_eq!(
            world.get::<PreviousGlobalTransform>(entity_mesh).unwrap().0,
            GlobalTransform::from(Transform::from_xyz(4.0, 5.0, 6.0)).affine()
        );

        // Moving the entity mesh updates its PreviousGlobalTransform on the next PreUpdate pass.
        world.increment_change_tick();
        let moved = GlobalTransform::from(Transform::from_translation(Vec3::new(10.0, 20.0, 30.0)));
        world.entity_mut(entity_mesh).insert(moved);
        world
            .run_system_once(update_mesh_previous_global_transforms)
            .unwrap();
        assert_eq!(
            world.get::<PreviousGlobalTransform>(entity_mesh).unwrap().0,
            moved.affine()
        );
    }
}
