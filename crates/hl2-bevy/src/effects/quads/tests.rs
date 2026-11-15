use super::{EffectMaterial, Quad, QuadDraw, color, draw_pool, fill, positions, quad_mesh};
use bevy::prelude::{
    Assets, Commands, DetectChanges, Handle, Mesh, MeshMaterial3d, Vec4, Visibility, World,
};
use bevy::{
    camera::primitives::{Aabb, MeshAabb},
    ecs::system::SystemState,
    mesh::{Indices, VertexAttributeValues},
};
use std::collections::BTreeMap;
use std::collections::BTreeSet;

fn quad() -> Quad {
    Quad {
        material: "first".into(),
        positions: [
            glam::Vec3::new(-2., 1., 3.),
            glam::Vec3::new(5., 1., 3.),
            glam::Vec3::new(5., 1., 9.),
            glam::Vec3::new(-2., 1., 9.),
        ],
        color: [255, 127, 0, 64],
        uv: [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
    }
}

fn assert_geometry(mesh: &Mesh, quad: &Quad) {
    assert_eq!(
        mesh.attribute(Mesh::ATTRIBUTE_POSITION),
        Some(&VertexAttributeValues::Float32x3(positions(quad).to_vec()))
    );
    assert_eq!(
        mesh.attribute(Mesh::ATTRIBUTE_UV_0),
        Some(&VertexAttributeValues::Float32x2(quad.uv.to_vec()))
    );
    assert_eq!(
        mesh.attribute(Mesh::ATTRIBUTE_COLOR),
        Some(&VertexAttributeValues::Float32x4(vec![color(quad); 4]))
    );
    assert_eq!(
        mesh.attribute(Mesh::ATTRIBUTE_NORMAL),
        Some(&VertexAttributeValues::Float32x3(vec![[0., 1., 0.]; 4]))
    );
    assert_eq!(
        mesh.attribute(Mesh::ATTRIBUTE_UV_1),
        Some(&VertexAttributeValues::Float32x2(vec![[0., 0.]; 4]))
    );
    assert!(matches!(mesh.indices(), Some(Indices::U32(i)) if i == &[0, 1, 2, 0, 2, 3]));
}

#[test]
fn changed_sprite_keeps_storage_and_exact_geometry() {
    let mut quad = quad();
    let mut mesh = quad_mesh(&quad);
    let attributes = [
        Mesh::ATTRIBUTE_POSITION,
        Mesh::ATTRIBUTE_UV_0,
        Mesh::ATTRIBUTE_COLOR,
    ];
    let storage = attributes.map(|a| mesh.attribute(a).unwrap().get_bytes().as_ptr());
    quad.positions[2] = glam::Vec3::new(6., -4., 12.);
    quad.uv[2] = [0.25, 0.75];
    quad.color = [0, 255, 31, 255];
    fill(&mut mesh, &quad);
    assert_geometry(&mesh, &quad);
    assert_eq!(
        storage,
        attributes.map(|a| mesh.attribute(a).unwrap().get_bytes().as_ptr())
    );
    // Bounds follow the transformed positions, including a displaced corner.
    let bounds = mesh.compute_aabb().unwrap();
    assert_eq!(bounds.min().to_array(), [-2., 3., -1.]);
    assert_eq!(bounds.max().to_array(), [6., 12., 4.]);
}

#[test]
fn invalid_storage_is_rebuilt_as_a_complete_quad() {
    let quad = quad();
    let mut mesh = quad_mesh(&quad);
    mesh.remove_attribute(Mesh::ATTRIBUTE_UV_0);
    fill(&mut mesh, &quad);
    assert_geometry(&mesh, &quad);
}

struct Fixture {
    world: World,
    meshes: Assets<Mesh>,
    materials: BTreeMap<String, Handle<EffectMaterial>>,
    pool: Vec<QuadDraw>,
}

#[test]
fn replacing_a_material_handle_does_not_require_a_geometry_change() {
    let mut f = Fixture::new();
    let quad = quad();
    f.draw(std::slice::from_ref(&quad));
    let entity = f.pool[0].entity;
    f.world.clear_trackers();
    let replacement = f.materials["second"].clone();
    f.materials
        .insert(quad.material.clone(), replacement.clone());
    f.draw(std::slice::from_ref(&quad));
    let entity = f.world.entity(entity);
    assert_eq!(
        entity.get::<MeshMaterial3d<EffectMaterial>>().unwrap().0,
        replacement
    );
    assert!(!entity.get_ref::<Aabb>().unwrap().is_changed());
}

impl Fixture {
    fn new() -> Self {
        let mut assets = Assets::<EffectMaterial>::default();
        let materials = ["first", "second"].map(|name| {
            let material = assets.add(EffectMaterial {
                texture: Handle::default(),
                parameters: Vec4::ZERO,
                kind: 2,
                ignore_z: false,
            });
            (name.to_owned(), material)
        });
        Self {
            world: World::new(),
            meshes: Assets::default(),
            materials: materials.into(),
            pool: Vec::new(),
        }
    }

    fn draw(&mut self, quads: &[Quad]) {
        let mut state = SystemState::<Commands>::new(&mut self.world);
        let mut commands = state.get_mut(&mut self.world).expect("fixture commands");
        draw_pool(
            &mut commands,
            &mut self.meshes,
            &mut self.pool,
            quads,
            (&self.materials, &BTreeSet::new()),
            Some(1),
        );
        state.apply(&mut self.world);
    }
}

#[test]
fn pool_preserves_material_bounds_layers_and_show_hide_transitions() {
    let mut f = Fixture::new();
    let mut first = quad();
    let mut second = quad();
    second.material = "second".into();
    second.positions.iter_mut().for_each(|p| p.z += 10.);
    f.draw(&[first.clone(), second.clone()]);
    let entities: Vec<_> = f.pool.iter().map(|s| s.entity).collect();
    let handles: Vec<_> = f.pool.iter().map(|s| s.mesh.clone()).collect();
    assert_eq!(f.meshes.len(), 2);
    for (index, quad) in [first.clone(), second.clone()].iter().enumerate() {
        let entity = f.world.entity(entities[index]);
        assert_eq!(entity.get::<Visibility>(), Some(&Visibility::Visible));
        assert_eq!(
            entity.get::<MeshMaterial3d<EffectMaterial>>().unwrap().0,
            f.materials[&quad.material]
        );
        assert_eq!(
            entity
                .get::<bevy::camera::visibility::RenderLayers>()
                .unwrap(),
            &bevy::camera::visibility::RenderLayers::layer(1)
        );
        let mesh = f.meshes.get(&handles[index]).unwrap();
        assert_geometry(mesh, quad);
        let expected_bounds = mesh.compute_aabb().unwrap();
        assert_eq!(entity.get::<Aabb>().unwrap().min(), expected_bounds.min());
        assert_eq!(entity.get::<Aabb>().unwrap().max(), expected_bounds.max());
    }
    f.world.clear_trackers();
    f.draw(&[first.clone(), second]);
    assert!(
        !f.world
            .entity(entities[0])
            .get_ref::<Visibility>()
            .unwrap()
            .is_changed()
    );
    assert!(
        !f.world
            .entity(entities[0])
            .get_ref::<MeshMaterial3d<EffectMaterial>>()
            .unwrap()
            .is_changed()
    );
    assert!(
        !f.world
            .entity(entities[0])
            .get_ref::<Aabb>()
            .unwrap()
            .is_changed()
    );
    first.material = "second".into();
    first.positions.iter_mut().for_each(|p| p.x += 20.);
    f.draw(&[first.clone()]);
    assert_eq!(
        f.world.entity(entities[1]).get::<Visibility>(),
        Some(&Visibility::Hidden)
    );
    assert_eq!(
        f.world
            .entity(entities[0])
            .get::<MeshMaterial3d<EffectMaterial>>()
            .unwrap()
            .0,
        f.materials["second"]
    );
    assert_geometry(f.meshes.get(&handles[0]).unwrap(), &first);
    f.draw(&[]);
    assert!(
        entities
            .iter()
            .all(|&id| f.world.entity(id).get::<Visibility>() == Some(&Visibility::Hidden))
    );
    f.draw(&[quad()]);
    assert_eq!(
        f.world.entity(entities[0]).get::<Visibility>(),
        Some(&Visibility::Visible)
    );
    assert_eq!(f.pool[0].mesh, handles[0]);
    assert_eq!(f.meshes.len(), 2);
    // Unknown sprite materials neither occupy a slot nor leave a stale draw visible.
    let mut unknown = quad();
    unknown.material = "unavailable".into();
    f.draw(&[unknown]);
    assert!(
        entities
            .iter()
            .all(|&id| f.world.entity(id).get::<Visibility>() == Some(&Visibility::Hidden))
    );
}

#[test]
fn a_layer_batches_the_additive_materials_and_pools_the_rest() {
    let mut f = Fixture::new();
    let mut second = quad();
    second.material = "second".into();
    let mut layer = super::QuadLayer::default();
    let mut state = SystemState::<Commands>::new(&mut f.world);
    let mut commands = state.get_mut(&mut f.world).expect("fixture commands");
    let additive = BTreeSet::from(["first".to_owned()]);
    let quads = [quad(), second, quad()];
    layer.draw(
        &mut commands,
        &mut f.meshes,
        &quads,
        (&f.materials, &additive),
        None,
    );
    state.apply(&mut f.world);
    // One pooled draw for the other material, one batch mesh holding both additive quads.
    assert_eq!(layer.counts(), (3, 2));
    assert_eq!(f.meshes.len(), 2);
    layer.clear();
    assert_eq!(layer.counts(), (0, 0));
}

#[test]
#[ignore = "CPU allocation experiment; run explicitly without competing workloads"]
fn benchmark_quad_geometry_updates() {
    use std::{hint::black_box, time::Instant};
    let mut quad = quad();
    let iterations = 100_000;
    let mut rebuilt = Vec::new();
    let mut patched = Vec::new();
    for round in 0..6 {
        for reuse in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let mut mesh = quad_mesh(&quad);
            let start = Instant::now();
            for i in 0..iterations {
                quad.positions[0].x = (i % 97) as f32;
                if reuse {
                    fill(black_box(&mut mesh), black_box(&quad));
                } else {
                    mesh = quad_mesh(black_box(&quad));
                }
                black_box(&mesh);
            }
            let micros = start.elapsed().as_secs_f64() * 1_000_000. / f64::from(iterations);
            assert_geometry(&mesh, &quad);
            if reuse {
                patched.push(micros);
            } else {
                rebuilt.push(micros);
            }
        }
    }
    println!("quad geometry microseconds/update: rebuild={rebuilt:?}; persistent={patched:?}");
}
