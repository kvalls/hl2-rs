use super::{super::Geometry, super::gpu, Batches, EffectMaterial};
use crate::shader_rows::ShaderRows;
use bevy::{
    camera::primitives::Aabb,
    camera::visibility::RenderLayers,
    ecs::system::SystemState,
    mesh::MeshTag,
    prelude::{Assets, Commands, Handle, Mesh, MeshMaterial3d, Vec3, Vec4, Visibility, World},
    render::storage::ShaderBuffer,
};
use hl2_simulation::projectile_visuals::Quad;
use std::collections::{BTreeMap, BTreeSet};

fn quad(material: &str, x: f32) -> Quad {
    Quad {
        material: material.into(),
        positions: [
            glam::Vec3::new(x, 1., 3.),
            glam::Vec3::new(x + 7., 1., 3.),
            glam::Vec3::new(x + 7., 1., 9.),
            glam::Vec3::new(x, 1., 9.),
        ],
        color: [255, 127, 0, 64],
        uv: [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
    }
}

struct Fixture {
    world: World,
    meshes: Assets<Mesh>,
    buffers: Assets<ShaderBuffer>,
    gpu: gpu::QuadGpu,
    materials: BTreeMap<String, Handle<EffectMaterial>>,
    additive: BTreeSet<String>,
    batches: Batches,
}

impl Fixture {
    fn new() -> Self {
        let mut buffers = Assets::<ShaderBuffer>::default();
        let mut buf = ShaderBuffer::default();
        buf.set_rows(&vec![Vec4::ZERO; gpu::MAX_QUADS * gpu::ROWS_PER_QUAD]);
        let _ = buffers.insert(gpu::QUAD_ROWS.id(), buf);
        let mut assets = Assets::<EffectMaterial>::default();
        let materials = ["glow", "flare", "smoke"].map(|name| {
            let material = assets.add(EffectMaterial {
                texture: Handle::default(),
                parameters: Vec4::ZERO,
                quad_rows: gpu::QUAD_ROWS,
                kind: 3,
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
            additive: ["glow", "flare"].map(String::from).into(),
            batches: Batches::default(),
        }
    }

    /// Draws a frame; returns how many CPU fallback meshes were rewritten.
    fn draw(&mut self, quads: &[Quad]) -> usize {
        self.gpu.begin_frame();
        let rewritten = self.draw_without_flush(quads);
        self.gpu.flush(&mut self.buffers);
        rewritten
    }

    fn draw_without_flush(&mut self, quads: &[Quad]) -> usize {
        let mut state = SystemState::<(Commands, gpu::QuadDraws)>::new(&mut self.world);
        let (mut commands, mut quad_draws) =
            state.get_mut(&mut self.world).expect("fixture commands");
        let rewritten = self.batches.draw(
            (&mut commands, &mut self.meshes),
            (&mut self.gpu, &mut quad_draws),
            quads,
            (&self.materials, &self.additive),
            Some(1),
        );
        state.apply(&mut self.world);
        rewritten
    }

    fn tag(&self, material: &str) -> u32 {
        self.world
            .entity(self.batches.by_material[material].entity)
            .get::<MeshTag>()
            .unwrap()
            .value
    }

    fn visibility(&self, material: &str) -> Visibility {
        let entity = self.world.entity(self.batches.by_material[material].entity);
        *entity.get::<Visibility>().unwrap()
    }
}

fn decode_row(bytes: &[u8], row: usize) -> Vec4 {
    let base = row * 16;
    let read = |offset: usize| -> f32 {
        f32::from_le_bytes(bytes[base + offset..base + offset + 4].try_into().unwrap())
    };
    Vec4::new(read(0), read(4), read(8), read(12))
}

fn assert_gpu_slice(buffer: &ShaderBuffer, tag: u32, quads: &[Quad]) {
    let start = (tag >> 16) as usize;
    let count = (tag & 0xffff) as usize;
    assert_eq!(count, quads.len());
    let bytes = buffer.bytes().expect("quad buffer bytes");
    for (i, q) in quads.iter().enumerate() {
        let g = Geometry::from(q);
        let p = g.bevy_positions();
        let uv = g.uv;
        let c = g.unit_color();
        let base = (start + i) * gpu::ROWS_PER_QUAD;
        assert_eq!(
            decode_row(bytes, base),
            Vec4::new(p[0][0], p[0][1], p[0][2], uv[0][0])
        );
        assert_eq!(
            decode_row(bytes, base + 1),
            Vec4::new(p[1][0], p[1][1], p[1][2], uv[1][0])
        );
        assert_eq!(
            decode_row(bytes, base + 2),
            Vec4::new(p[2][0], p[2][1], p[2][2], uv[2][0])
        );
        assert_eq!(
            decode_row(bytes, base + 3),
            Vec4::new(p[3][0], p[3][1], p[3][2], uv[3][0])
        );
        assert_eq!(
            decode_row(bytes, base + 4),
            Vec4::new(uv[0][1], uv[1][1], uv[2][1], uv[3][1])
        );
        assert_eq!(decode_row(bytes, base + 5), Vec4::from_array(c));
    }
}

#[test]
fn additive_quads_share_one_draw_per_material_in_input_order() {
    let mut f = Fixture::new();
    let frame = [
        quad("glow", 0.),
        quad("smoke", 5.),
        quad("flare", 10.),
        quad("glow", 20.),
        quad("unavailable", 30.),
    ];
    assert_eq!(f.draw(&frame), 0);
    // Only the additive materials are batched; an unknown material makes no batch.
    assert_eq!(
        f.batches.by_material.keys().collect::<Vec<_>>(),
        ["flare", "glow"]
    );
    assert_eq!(f.batches.quads(), 3);
    let buf = f.buffers.get(&gpu::QUAD_ROWS).unwrap();
    assert_gpu_slice(buf, f.tag("glow"), &[frame[0].clone(), frame[3].clone()]);
    assert_gpu_slice(buf, f.tag("flare"), &[frame[2].clone()]);

    let entity = f.world.entity(f.batches.by_material["glow"].entity);
    assert!(entity.contains::<gpu::QuadDraw>());
    assert_eq!(entity.get::<Visibility>(), Some(&Visibility::Visible));
    assert!(
        entity.contains::<bevy::light::NotShadowCaster>(),
        "sprites cast no shadow"
    );
    assert_eq!(entity.get::<RenderLayers>(), Some(&RenderLayers::layer(1)));
    assert_eq!(
        entity.get::<MeshMaterial3d<EffectMaterial>>().unwrap().0,
        f.materials["glow"]
    );
    let points = [frame[0].clone(), frame[3].clone()]
        .iter()
        .flat_map(|q| {
            Geometry::from(q)
                .bevy_positions()
                .into_iter()
                .map(Vec3::from_array)
        })
        .collect::<Vec<_>>();
    let bounds = Aabb::enclosing(points).unwrap();
    assert_eq!(entity.get::<Aabb>().unwrap().min(), bounds.min());
    assert_eq!(entity.get::<Aabb>().unwrap().max(), bounds.max());
}

#[test]
fn changed_quads_rewrite_no_meshes_and_idle_repeat_skips_buffer_upload() {
    let mut f = Fixture::new();
    let mut frame = [quad("glow", 0.), quad("flare", 10.), quad("glow", 20.)];
    assert_eq!(f.draw(&frame), 0);
    let template_count = f.meshes.len();

    // Idle repeat produces zero dirty buffer rows and zero mesh rewrites.
    f.gpu.begin_frame();
    assert_eq!(f.draw_without_flush(&frame), 0);
    assert_eq!(f.gpu.dirty(), None);
    f.gpu.flush(&mut f.buffers);

    // Changing a quad rewrites zero Assets<Mesh> and dirties only the changed slice.
    frame[2].color[3] = 9;
    assert_eq!(f.draw(&frame), 0);
    assert_eq!(f.meshes.len(), template_count);
    assert_gpu_slice(
        f.buffers.get(&gpu::QUAD_ROWS).unwrap(),
        f.tag("glow"),
        &[frame[0].clone(), frame[2].clone()],
    );

    let before_handle = f.batches.by_material["glow"].mesh.clone();
    frame[0].positions[0].x -= 4.;
    assert_eq!(f.draw(&frame), 0);
    assert_eq!(f.batches.by_material["glow"].mesh, before_handle);

    // Shrinking to 1 quad switches to the bucket-1 template without rewriting any mesh.
    assert_eq!(f.draw(&frame[..2]), 0);
    assert_gpu_slice(
        f.buffers.get(&gpu::QUAD_ROWS).unwrap(),
        f.tag("glow"),
        &[frame[0].clone()],
    );
}

#[test]
fn a_batch_without_quads_is_hidden_and_returns_without_an_upload() {
    let mut f = Fixture::new();
    let frame = [quad("glow", 0.), quad("flare", 10.)];
    f.draw(&frame);
    assert_eq!(f.draw(&frame[1..]), 0);
    assert_eq!(f.visibility("glow"), Visibility::Hidden);
    assert_eq!(f.visibility("flare"), Visibility::Visible);
    assert_eq!(f.batches.quads(), 1);
    assert_eq!(f.draw(&frame), 0);
    assert_eq!(f.visibility("glow"), Visibility::Visible);

    // A replaced material handle reaches the batch's draw without touching the buffer.
    let replacement = f.materials["flare"].clone();
    f.materials.insert("glow".into(), replacement.clone());
    f.gpu.begin_frame();
    assert_eq!(f.draw_without_flush(&frame), 0);
    assert_eq!(f.gpu.dirty(), None);
    let entity = f.world.entity(f.batches.by_material["glow"].entity);
    assert_eq!(
        entity.get::<MeshMaterial3d<EffectMaterial>>().unwrap().0,
        replacement
    );
}

#[test]
fn batch_exceeding_max_bucket_falls_back_to_cpu_mesh_with_tag_zero() {
    let mut f = Fixture::new();
    let big: Vec<Quad> = (0..=gpu::MAX_BUCKET)
        .map(|i| quad("glow", i as f32))
        .collect();
    assert_eq!(f.draw(&big), 1);
    assert_eq!(f.tag("glow"), 0);
    assert!(f.batches.by_material["glow"].cpu_mesh.is_some());
    assert_eq!(f.draw(&big), 0);
}
