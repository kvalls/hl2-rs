use super::*;

#[test]
fn the_metric_grows_with_depth_and_field_of_view_and_shrinks_with_resolution() {
    // 90 degrees vertical: tan 45 = 1. 768 lines, 384 units deep: 100 * 384 / 768 = 50.
    assert!((metric(384., 1., 768.) - 50.).abs() < 1e-4);
    // Twice the lines: a finer level at the same distance (retail measures pixels).
    assert!((metric(384., 1., 1536.) - 25.).abs() < 1e-4);
    // Behind the camera or at the eye: the material system's 0.001 guard (finest level).
    assert!(metric(-50., 1., 768.) < 1e-3);
    assert_eq!(metric(-50., 1., 768.), metric(0., 1., 768.));
}

#[test]
fn the_level_is_the_last_whose_switch_point_the_metric_reached() {
    // Barney's levels: switch points 0, 10, 20, 30, 40 (as authored in the model).
    let switches = [0., 10., 20., 30., 40.];
    assert_eq!(level_for(&switches, 0.), 0);
    assert_eq!(level_for(&switches, 9.99), 0);
    assert_eq!(
        level_for(&switches, 10.),
        1,
        "a switch point reached switches"
    );
    assert_eq!(level_for(&switches, 25.), 2);
    assert_eq!(level_for(&switches, 1000.), 4);
    assert_eq!(level_for(&[0.], 1000.), 0, "a single level");
    // From a root LOD of 2 (model detail Low) the levels start there.
    assert_eq!(level_for(&[0., 30., 40.], 25.), 0);
    assert_eq!(level_for(&[0., 30., 40.], 35.), 1);
}

#[test]
fn level_switches_and_template_copies_share_immutable_vertex_tables() {
    use bevy::camera::PerspectiveProjection;
    use bevy::ecs::{system::RunSystemOnce, world::World as BevyWorld};

    let root_vertices = Arc::new(Vertices {
        bind: vec![(glam::Vec3::ONE, None)],
        normals: vec![glam::Vec3::Z],
        flex: vec![Some([0, 0, 1])],
    });
    let coarser_vertices = Arc::new(Vertices {
        bind: vec![(glam::Vec3::ZERO, None)],
        normals: vec![glam::Vec3::Y],
        flex: vec![Some([0, 0, 2])],
    });
    let template = AnimatedMesh {
        sampled: None,
        gpu: None,
        entity: Some(1),
        weapon: None,
        key: "models/alyx.mdl".to_owned(),
        scale: 1.0,
        vertices: Arc::clone(&root_vertices),
        deformed: Some(vec![glam::Vec3::X]),
        morph: None,
        flex_state: Default::default(),
    };
    let copied = template.copy_for(2, None);
    assert!(Arc::ptr_eq(&template.vertices, &copied.vertices));

    let mut world = BevyWorld::new();
    let camera = world
        .spawn((
            crate::FlyCamera,
            Transform::from_xyz(0., 0., 0.).looking_to(Vec3::NEG_Z, Vec3::Y),
            Projection::Perspective(PerspectiveProjection {
                fov: std::f32::consts::FRAC_PI_2,
                ..Default::default()
            }),
        ))
        .id();
    let draw = world
        .spawn((
            Transform::from_xyz(0., 0., -500.),
            ModelLevels {
                switches: vec![0., 20.],
                levels: vec![
                    Level {
                        mesh: Handle::default(),
                        vertices: Some(Arc::clone(&root_vertices)),
                        triangles: 100,
                    },
                    Level {
                        mesh: Handle::default(),
                        vertices: Some(Arc::clone(&coarser_vertices)),
                        triangles: 40,
                    },
                ],
                current: 0,
            },
            Mesh3d(Handle::default()),
            template,
        ))
        .id();

    world.run_system_once(switch_levels).unwrap();
    let animation = world.get::<AnimatedMesh>(draw).unwrap();
    assert!(Arc::ptr_eq(&animation.vertices, &coarser_vertices));
    assert!(animation.deformed.is_none());

    world
        .entity_mut(camera)
        .insert(Transform::from_xyz(0., 0., -490.).looking_to(Vec3::NEG_Z, Vec3::Y));
    world.run_system_once(switch_levels).unwrap();
    let animation = world.get::<AnimatedMesh>(draw).unwrap();
    assert!(Arc::ptr_eq(&animation.vertices, &root_vertices));
}

#[test]
fn actors_with_too_many_morph_channels_keep_private_meshes() {
    use source_assets::flexes::{FaceModel, FlexModel, MeshFlex, MeshFlexes, VertAnim};

    let key = String::from("test");
    let flex = FlexModel {
        meshes: vec![MeshFlexes {
            bodypart: 0,
            model: 0,
            mesh: 0,
            flexes: (0..=bevy::mesh::morph::MAX_MORPH_WEIGHTS)
                .map(|descriptor| MeshFlex {
                    descriptor,
                    pair: 0,
                    targets: [0., 1., 1., 2.],
                    vertices: vec![VertAnim {
                        index: 0,
                        speed: 0,
                        side: 0,
                        delta: glam::Vec3::Y,
                        normal: glam::Vec3::ZERO,
                    }],
                })
                .collect(),
        }],
        ..Default::default()
    };
    let face = FaceModel {
        slots: flex.vertex_slots(),
        flex,
        eyes: vec![],
    };
    let mut shared = SharedLevels::for_map(&BTreeMap::from([(key.clone(), Arc::new(face))]));
    let mut meshes = Assets::default();
    let mut spawn = |id| {
        let mut batch = Batch {
            positions: vec![[0.; 3]],
            normals: vec![glam::Vec3::Y],
            skin: vec![(glam::Vec3::ZERO, None)],
            flex: vec![Some([0, 0, 0])],
            ..Default::default()
        };
        let mut anim =
            AnimatedMesh::from_batch(&mut batch, (Some(id), None), (key.clone(), 1.), true);
        anim.gpu = Some(crate::gpu_skinning::Skeleton {
            joints: vec![bevy::prelude::Entity::PLACEHOLDER],
            inverse_bindposes: Handle::default(),
        });
        let mesh = shared.mesh_for_draw(batch, "face", Some(&mut anim), &mut meshes, false);
        assert!(anim.morph.is_none());
        mesh
    };
    assert_ne!(spawn(1), spawn(2));
}
