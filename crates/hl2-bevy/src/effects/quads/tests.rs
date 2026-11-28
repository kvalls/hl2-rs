use super::{EffectMaterial, Geometry, Quad, batch, draw_runs, gpu};
use crate::shader_rows::ShaderRows;
use bevy::{
    asset::RenderAssetUsages,
    camera::primitives::{Aabb, MeshAabb},
    ecs::system::SystemState,
    mesh::{Indices, MeshTag, VertexAttributeValues},
    prelude::{
        Assets, Commands, DetectChanges, Handle, Mesh, Mesh3d, MeshMaterial3d, Vec3, Vec4,
        Visibility, World,
    },
    render::storage::ShaderBuffer,
};
use std::collections::{BTreeMap, BTreeSet};

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

fn positions(quad: &Quad) -> [[f32; 3]; 4] {
    Geometry::from(quad).bevy_positions()
}

fn color(quad: &Quad) -> [f32; 4] {
    Geometry::from(quad).unit_color()
}

fn expected_bounds(quads: &[Quad]) -> Aabb {
    let points = quads
        .iter()
        .flat_map(|q| positions(q).into_iter().map(Vec3::from_array));
    Aabb::enclosing(points).unwrap()
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

fn decode_buffer_row(bytes: &[u8], row: usize) -> Vec4 {
    let base = row * 16;
    let read = |offset: usize| -> f32 {
        f32::from_le_bytes(bytes[base + offset..base + offset + 4].try_into().unwrap())
    };
    Vec4::new(read(0), read(4), read(8), read(12))
}

fn assert_gpu_quads(buffer: &ShaderBuffer, start: usize, quads: &[Quad]) {
    let bytes = buffer.bytes().expect("quad buffer bytes");
    for (i, q) in quads.iter().enumerate() {
        let p = positions(q);
        let uv = q.uv;
        let c = color(q);
        let base = (start + i) * gpu::ROWS_PER_QUAD;
        assert_eq!(
            decode_buffer_row(bytes, base),
            Vec4::new(p[0][0], p[0][1], p[0][2], uv[0][0])
        );
        assert_eq!(
            decode_buffer_row(bytes, base + 1),
            Vec4::new(p[1][0], p[1][1], p[1][2], uv[1][0])
        );
        assert_eq!(
            decode_buffer_row(bytes, base + 2),
            Vec4::new(p[2][0], p[2][1], p[2][2], uv[2][0])
        );
        assert_eq!(
            decode_buffer_row(bytes, base + 3),
            Vec4::new(p[3][0], p[3][1], p[3][2], uv[3][0])
        );
        assert_eq!(
            decode_buffer_row(bytes, base + 4),
            Vec4::new(uv[0][1], uv[1][1], uv[2][1], uv[3][1])
        );
        assert_eq!(decode_buffer_row(bytes, base + 5), Vec4::from_array(c));
    }
}

#[test]
fn changed_sprite_keeps_storage_and_exact_geometry() {
    let mut quad = quad();
    let mut mesh = batch::batch_mesh(&[(&quad).into()]);
    let attributes = [
        Mesh::ATTRIBUTE_POSITION,
        Mesh::ATTRIBUTE_UV_0,
        Mesh::ATTRIBUTE_COLOR,
    ];
    let storage = attributes.map(|a| mesh.attribute(a).unwrap().get_bytes().as_ptr());
    quad.positions[2] = glam::Vec3::new(6., -4., 12.);
    quad.uv[2] = [0.25, 0.75];
    quad.color = [0, 255, 31, 255];
    assert!(batch::patch(&mut mesh, &[(&quad).into()]));
    assert_geometry(&mesh, &quad);
    assert_eq!(
        storage,
        attributes.map(|a| mesh.attribute(a).unwrap().get_bytes().as_ptr())
    );
    let bounds = mesh.get_aabb().unwrap();
    assert_eq!(bounds.min().to_array(), [-2., 3., -1.]);
    assert_eq!(bounds.max().to_array(), [6., 12., 4.]);
}

#[test]
fn invalid_storage_is_rebuilt_as_a_complete_quad() {
    let mut f = Fixture::new();
    let filler: Vec<Quad> = (0..gpu::MAX_QUADS).map(|_| quad()).collect();
    let mut overflow = quad();
    overflow.material = "second".into();
    let mut frame = filler.clone();
    frame.push(overflow.clone());
    f.draw(&frame);
    let overflow_batch = f.pool.last().unwrap();
    let handle = overflow_batch
        .cpu_mesh
        .clone()
        .expect("overflow uses cpu_mesh");
    assert_eq!(
        f.world.entity(overflow_batch.entity).get::<MeshTag>(),
        Some(&MeshTag::new(0))
    );
    f.meshes
        .get_mut(&handle)
        .unwrap()
        .remove_attribute(Mesh::ATTRIBUTE_UV_0);
    overflow.uv[1] = [0.5, 0.];
    let mut next_frame = filler;
    next_frame.push(overflow.clone());
    f.draw(&next_frame);
    assert_geometry(f.meshes.get(&handle).unwrap(), &overflow);
}

struct Fixture {
    world: World,
    meshes: Assets<Mesh>,
    buffers: Assets<ShaderBuffer>,
    gpu: gpu::QuadGpu,
    materials: BTreeMap<String, Handle<EffectMaterial>>,
    pool: Vec<batch::Batch>,
}

impl Fixture {
    fn new() -> Self {
        let mut buffers = Assets::<ShaderBuffer>::default();
        let mut buf = ShaderBuffer::default();
        buf.set_rows(&vec![Vec4::ZERO; gpu::MAX_QUADS * gpu::ROWS_PER_QUAD]);
        let _ = buffers.insert(gpu::QUAD_ROWS.id(), buf);
        let mut assets = Assets::<EffectMaterial>::default();
        let materials = ["first", "second"].map(|name| {
            let material = assets.add(EffectMaterial {
                texture: Handle::default(),
                parameters: Vec4::ZERO,
                quad_rows: gpu::QUAD_ROWS,
                kind: 2,
                ignore_z: false,
            });
            (name.to_owned(), material)
        });
        Self {
            world: World::new(),
            meshes: Assets::default(),
            buffers,
            gpu: gpu::QuadGpu::default(),
            materials: materials.into(),
            pool: Vec::new(),
        }
    }

    fn draw(&mut self, quads: &[Quad]) -> usize {
        self.gpu.begin_frame();
        let mut state = SystemState::<(Commands, gpu::QuadDraws)>::new(&mut self.world);
        let (mut commands, mut quad_draws) =
            state.get_mut(&mut self.world).expect("fixture commands");
        let drawn = draw_runs(
            &mut commands,
            &mut self.meshes,
            (&mut self.gpu, &mut quad_draws),
            &mut self.pool,
            quads,
            (&self.materials, &BTreeSet::new()),
            Some(1),
        );
        state.apply(&mut self.world);
        self.gpu.flush(&mut self.buffers);
        drawn
    }
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
    f.gpu.begin_frame();
    let mut state = SystemState::<(Commands, gpu::QuadDraws)>::new(&mut f.world);
    let (mut commands, mut quad_draws) = state.get_mut(&mut f.world).expect("fixture commands");
    draw_runs(
        &mut commands,
        &mut f.meshes,
        (&mut f.gpu, &mut quad_draws),
        &mut f.pool,
        std::slice::from_ref(&quad),
        (&f.materials, &BTreeSet::new()),
        Some(1),
    );
    state.apply(&mut f.world);
    assert_eq!(f.gpu.dirty(), None);
    let entity = f.world.entity(entity);
    assert_eq!(
        entity.get::<MeshMaterial3d<EffectMaterial>>().unwrap().0,
        replacement
    );
    assert!(!entity.get_ref::<Aabb>().unwrap().is_changed());
    assert!(!entity.get_ref::<MeshTag>().unwrap().is_changed());
    assert!(!entity.get_ref::<Mesh3d>().unwrap().is_changed());
}

#[test]
fn consecutive_quads_of_one_material_share_a_draw_in_input_order() {
    let mut f = Fixture::new();
    let first = quad();
    let mut second = quad();
    second.positions.iter_mut().for_each(|p| p.x += 5.);
    let mut other = quad();
    other.material = "second".into();
    let mut last = quad();
    last.positions.iter_mut().for_each(|p| p.y += 3.);
    let drawn = f.draw(&[first.clone(), second.clone(), other.clone(), last.clone()]);
    assert_eq!(drawn, 4);
    // Runs: first + second (bucket 2), other (bucket 1), last (bucket 1).
    assert_eq!((f.pool.len(), f.meshes.len()), (3, 2));
    assert_eq!(f.pool[1].mesh, f.pool[2].mesh);
    assert_eq!(
        f.world.entity(f.pool[0].entity).get::<MeshTag>(),
        Some(&MeshTag::new(2))
    );
    assert_eq!(
        f.world.entity(f.pool[1].entity).get::<MeshTag>(),
        Some(&MeshTag::new((2 << 16) | 1))
    );
    assert_eq!(
        f.world.entity(f.pool[2].entity).get::<MeshTag>(),
        Some(&MeshTag::new((3 << 16) | 1))
    );
    let buf = f.buffers.get(&gpu::QUAD_ROWS).unwrap();
    assert_gpu_quads(buf, 0, &[first, second, other, last]);
    let material = |i: usize| {
        f.world
            .entity(f.pool[i].entity)
            .get::<MeshMaterial3d<EffectMaterial>>()
            .unwrap()
            .0
            .clone()
    };
    assert_eq!(material(0), f.materials["first"]);
    assert_eq!(material(1), f.materials["second"]);
    assert_eq!(material(2), f.materials["first"]);
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
    assert_eq!(f.meshes.len(), 1);
    assert_eq!(handles[0], handles[1]);
    for (index, q) in [first.clone(), second.clone()].iter().enumerate() {
        let entity = f.world.entity(entities[index]);
        assert!(entity.contains::<gpu::QuadDraw>());
        assert!(entity.contains::<bevy::camera::visibility::NoAutoAabb>());
        assert_eq!(entity.get::<Visibility>(), Some(&Visibility::Visible));
        assert!(
            entity.contains::<bevy::light::NotShadowCaster>(),
            "sprites cast no shadow"
        );
        assert_eq!(
            entity.get::<MeshMaterial3d<EffectMaterial>>().unwrap().0,
            f.materials[&q.material]
        );
        assert_eq!(
            entity
                .get::<bevy::camera::visibility::RenderLayers>()
                .unwrap(),
            &bevy::camera::visibility::RenderLayers::layer(1)
        );
        assert_eq!(
            entity.get::<MeshTag>(),
            Some(&MeshTag::new(((index as u32) << 16) | 1))
        );
        let bounds = expected_bounds(std::slice::from_ref(q));
        assert_eq!(entity.get::<Aabb>().unwrap().min(), bounds.min());
        assert_eq!(entity.get::<Aabb>().unwrap().max(), bounds.max());
    }
    f.world.clear_trackers();
    f.draw(&[first.clone(), second]);
    let entity0 = f.world.entity(entities[0]);
    assert!(!entity0.get_ref::<Visibility>().unwrap().is_changed());
    assert!(
        !entity0
            .get_ref::<MeshMaterial3d<EffectMaterial>>()
            .unwrap()
            .is_changed()
    );
    assert!(!entity0.get_ref::<Aabb>().unwrap().is_changed());
    assert!(!entity0.get_ref::<Mesh3d>().unwrap().is_changed());
    assert!(!entity0.get_ref::<MeshTag>().unwrap().is_changed());

    first.material = "second".into();
    first.positions.iter_mut().for_each(|p| p.x += 20.);
    f.draw(std::slice::from_ref(&first));
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
    assert_gpu_quads(f.buffers.get(&gpu::QUAD_ROWS).unwrap(), 0, &[first]);
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
    assert_eq!(f.meshes.len(), 1);
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
    f.gpu.begin_frame();
    let mut state = SystemState::<(Commands, gpu::QuadDraws)>::new(&mut f.world);
    let (mut commands, mut quad_draws) = state.get_mut(&mut f.world).expect("fixture commands");
    let additive = BTreeSet::from(["first".to_owned()]);
    let quads = [quad(), second, quad()];
    layer.draw(
        &mut commands,
        &mut f.meshes,
        (&mut f.gpu, &mut quad_draws),
        &quads,
        (&f.materials, &additive),
        None,
    );
    state.apply(&mut f.world);
    f.gpu.flush(&mut f.buffers);
    assert_eq!(layer.counts(), (3, 2));
    assert_eq!(f.meshes.len(), 2);
    layer.clear();
    assert_eq!(layer.counts(), (0, 0));
}

#[test]
fn gpu_pack_encodes_rows_buckets_and_skips_dirty_on_idle_repeat() {
    let mut f = Fixture::new();
    let q0 = quad();
    let mut q1 = quad();
    q1.positions[0].x += 4.;
    let g0: Geometry = (&q0).into();
    let g1: Geometry = (&q1).into();

    f.gpu.begin_frame();
    let (tag, aabb) = f.gpu.pack(&[g0, g1]).unwrap();
    assert_eq!(tag, 2);
    assert_eq!(aabb, expected_bounds(&[q0.clone(), q1.clone()]));
    assert_eq!(f.gpu.rows().len(), gpu::MAX_QUADS * gpu::ROWS_PER_QUAD);
    assert_eq!(f.gpu.rows()[5], Vec4::from_array(color(&q0)));
    assert_eq!(f.gpu.dirty(), Some((0, 12)));
    f.gpu.flush(&mut f.buffers);
    assert_eq!(f.gpu.dirty(), None);
    assert_gpu_quads(
        f.buffers.get(&gpu::QUAD_ROWS).unwrap(),
        0,
        &[q0.clone(), q1.clone()],
    );

    // Idle repeat leaves dirty == None (zero ShaderBuffer writes).
    f.gpu.begin_frame();
    assert_eq!(f.gpu.pack(&[g0, g1]).unwrap().0, 2);
    assert_eq!(f.gpu.dirty(), None);

    // Changing only the second quad dirties only its 6 rows.
    q1.color = [10, 20, 30, 40];
    f.gpu.begin_frame();
    f.gpu.pack(&[g0, (&q1).into()]).unwrap();
    assert_eq!(f.gpu.dirty(), Some((6, 12)));
    f.gpu.flush(&mut f.buffers);
    assert_gpu_quads(f.buffers.get(&gpu::QUAD_ROWS).unwrap(), 0, &[q0, q1]);

    // Bucket template meshes are immutable RENDER_WORLD with corner indices in position.
    assert!(f.gpu.template(0, &mut f.meshes).is_none());
    assert!(f.gpu.template(gpu::MAX_BUCKET + 1, &mut f.meshes).is_none());
    let t3 = f.gpu.template(3, &mut f.meshes).unwrap();
    let t4 = f.gpu.template(4, &mut f.meshes).unwrap();
    assert_eq!(t3, t4);
    let mesh = f.meshes.get(&t4).unwrap();
    assert_eq!(mesh.asset_usage, RenderAssetUsages::RENDER_WORLD);
    assert_eq!(
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().len(),
        4 * 4
    );
    assert_eq!(mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap().len(), 4 * 4);
    assert_eq!(mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().len(), 4 * 4);
    assert_eq!(mesh.attribute(Mesh::ATTRIBUTE_UV_1).unwrap().len(), 4 * 4);
    assert_eq!(mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap().len(), 4 * 4);
    assert_eq!(
        mesh.attribute(Mesh::ATTRIBUTE_POSITION),
        Some(&VertexAttributeValues::Float32x3(
            (0..4)
                .flat_map(|q| (0..4).map(move |c| [q as f32, c as f32, 0.0]))
                .collect()
        ))
    );
}

#[test]
fn runs_split_at_max_bucket_preserving_input_order() {
    let mut f = Fixture::new();
    let quads: Vec<Quad> = (0..gpu::MAX_BUCKET + 3)
        .map(|i| {
            let mut q = quad();
            q.positions[0].x += i as f32;
            q
        })
        .collect();
    assert_eq!(f.draw(&quads), gpu::MAX_BUCKET + 3);
    assert_eq!(f.pool.len(), 2);
    assert_eq!(
        f.world.entity(f.pool[0].entity).get::<MeshTag>(),
        Some(&MeshTag::new(gpu::MAX_BUCKET as u32))
    );
    assert_eq!(
        f.world.entity(f.pool[1].entity).get::<MeshTag>(),
        Some(&MeshTag::new(((gpu::MAX_BUCKET as u32) << 16) | 3))
    );
    assert_gpu_quads(f.buffers.get(&gpu::QUAD_ROWS).unwrap(), 0, &quads);
}

#[test]
fn overflow_falls_back_to_cpu_mesh_and_recovers_to_gpu_template() {
    let mut f = Fixture::new();
    let mut quads: Vec<Quad> = (0..gpu::MAX_QUADS).map(|_| quad()).collect();
    let mut extra = quad();
    extra.material = "second".into();
    extra.positions[0].z += 12.;
    quads.push(extra.clone());

    assert_eq!(f.draw(&quads), gpu::MAX_QUADS + 1);
    let overflow = f.pool.last().unwrap();
    let cpu_handle = overflow
        .cpu_mesh
        .clone()
        .expect("overflow creates cpu_mesh");
    assert_eq!(
        f.world.entity(overflow.entity).get::<MeshTag>(),
        Some(&MeshTag::new(0))
    );
    assert_eq!(
        f.world.entity(overflow.entity).get::<Visibility>(),
        Some(&Visibility::Visible)
    );
    assert_geometry(f.meshes.get(&cpu_handle).unwrap(), &extra);

    // Next frame fits again: drops cpu_mesh and returns to GPU template + non-zero MeshTag.
    f.draw(std::slice::from_ref(&extra));
    assert!(f.pool[0].cpu_mesh.is_none());
    assert!(!f.meshes.contains(&cpu_handle));
    assert_eq!(
        f.world.entity(f.pool[0].entity).get::<MeshTag>(),
        Some(&MeshTag::new(1))
    );
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
            let mut mesh = batch::batch_mesh(&[(&quad).into()]);
            let start = Instant::now();
            for i in 0..iterations {
                quad.positions[0].x = (i % 97) as f32;
                let geometry: Geometry = (&quad).into();
                if reuse {
                    batch::patch(black_box(&mut mesh), black_box(&[geometry]));
                } else {
                    mesh = batch::batch_mesh(black_box(&[geometry]));
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
