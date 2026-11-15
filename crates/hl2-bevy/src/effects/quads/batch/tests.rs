use super::{super::Geometry, Batches, EffectMaterial};
use bevy::{
    camera::primitives::{Aabb, MeshAabb},
    camera::visibility::RenderLayers,
    ecs::system::SystemState,
    mesh::{Indices, VertexAttributeValues},
    prelude::{Assets, Commands, Handle, Mesh, MeshMaterial3d, Vec4, Visibility, World},
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
    materials: BTreeMap<String, Handle<EffectMaterial>>,
    additive: BTreeSet<String>,
    batches: Batches,
}

impl Fixture {
    fn new() -> Self {
        let mut assets = Assets::<EffectMaterial>::default();
        let materials = ["glow", "flare", "smoke"].map(|name| {
            let material = assets.add(EffectMaterial {
                texture: Handle::default(),
                parameters: Vec4::ZERO,
                kind: 3,
                ignore_z: false,
            });
            (name.to_owned(), material)
        });
        Self {
            world: World::new(),
            meshes: Assets::default(),
            materials: materials.into(),
            additive: ["glow", "flare"].map(String::from).into(),
            batches: Batches::default(),
        }
    }

    /// Draws a frame; how many batch meshes it rewrote.
    fn draw(&mut self, quads: &[Quad]) -> usize {
        let mut state = SystemState::<Commands>::new(&mut self.world);
        let mut commands = state.get_mut(&mut self.world).expect("fixture commands");
        let rewritten = self.batches.draw(
            (&mut commands, &mut self.meshes),
            quads,
            (&self.materials, &self.additive),
            Some(1),
        );
        state.apply(&mut self.world);
        rewritten
    }

    fn mesh(&self, material: &str) -> &Mesh {
        self.meshes
            .get(&self.batches.by_material[material].mesh)
            .unwrap()
    }

    fn visibility(&self, material: &str) -> Visibility {
        let entity = self.world.entity(self.batches.by_material[material].entity);
        *entity.get::<Visibility>().unwrap()
    }
}

/// The mesh holds exactly these quads, four corners each, in this order.
fn assert_quads(mesh: &Mesh, quads: &[Quad]) {
    let geometry: Vec<Geometry> = quads.iter().map(Geometry::from).collect();
    let positions: Vec<_> = geometry.iter().flat_map(Geometry::bevy_positions).collect();
    assert_eq!(
        mesh.attribute(Mesh::ATTRIBUTE_POSITION),
        Some(&VertexAttributeValues::Float32x3(positions))
    );
    let uvs: Vec<_> = quads.iter().flat_map(|quad| quad.uv).collect();
    assert_eq!(
        mesh.attribute(Mesh::ATTRIBUTE_UV_0),
        Some(&VertexAttributeValues::Float32x2(uvs))
    );
    let colors: Vec<_> = geometry.iter().flat_map(|g| [g.unit_color(); 4]).collect();
    assert_eq!(
        mesh.attribute(Mesh::ATTRIBUTE_COLOR),
        Some(&VertexAttributeValues::Float32x4(colors))
    );
    let indices: Vec<u32> = (0..quads.len() as u32)
        .flat_map(|quad| [0, 1, 2, 0, 2, 3].map(|corner| quad * 4 + corner))
        .collect();
    assert_eq!(mesh.indices(), Some(&Indices::U32(indices)));
    for attribute in [Mesh::ATTRIBUTE_NORMAL, Mesh::ATTRIBUTE_UV_1] {
        assert_eq!(mesh.attribute(attribute).unwrap().len(), quads.len() * 4);
    }
}

#[test]
fn additive_quads_share_one_mesh_per_material_in_input_order() {
    let mut f = Fixture::new();
    let frame = [
        quad("glow", 0.),
        quad("smoke", 5.),
        quad("flare", 10.),
        quad("glow", 20.),
        quad("unavailable", 30.),
    ];
    assert_eq!(f.draw(&frame), 2);
    // Only the additive materials are batched; an unknown material makes no batch.
    assert_eq!(
        f.batches.by_material.keys().collect::<Vec<_>>(),
        ["flare", "glow"]
    );
    assert_eq!(f.batches.quads(), 3);
    assert_quads(f.mesh("glow"), &[frame[0].clone(), frame[3].clone()]);
    assert_quads(f.mesh("flare"), &[frame[2].clone()]);
    let entity = f.world.entity(f.batches.by_material["glow"].entity);
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
    let bounds = f.mesh("glow").compute_aabb().unwrap();
    assert_eq!(entity.get::<Aabb>().unwrap().min(), bounds.min());
    assert_eq!(entity.get::<Aabb>().unwrap().max(), bounds.max());
}

#[test]
fn a_mesh_is_rewritten_only_when_a_quad_of_its_material_changed() {
    let mut f = Fixture::new();
    let mut frame = [quad("glow", 0.), quad("flare", 10.), quad("glow", 20.)];
    assert_eq!(f.draw(&frame), 2);
    assert_eq!(f.draw(&frame), 0, "an unchanged frame uploads nothing");
    frame[2].color[3] = 9;
    assert_eq!(f.draw(&frame), 1, "only the glow batch changed");
    assert_quads(f.mesh("glow"), &[frame[0].clone(), frame[2].clone()]);
    // The same count is written over the mesh's storage; another count rebuilds it.
    let storage = |f: &Fixture| match f.mesh("glow").attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(positions)) => positions.as_ptr(),
        _ => panic!("positions"),
    };
    let before = storage(&f);
    frame[0].positions[0].x -= 4.;
    assert_eq!(f.draw(&frame), 1);
    assert_eq!(storage(&f), before);
    assert_eq!(f.draw(&frame[..2]), 1);
    assert_quads(f.mesh("glow"), &[frame[0].clone()]);
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
    assert_eq!(f.draw(&frame), 0, "the hidden batch kept its mesh");
    assert_eq!(f.visibility("glow"), Visibility::Visible);
    // A replaced material handle reaches the batch's draw.
    let replacement = f.materials["flare"].clone();
    f.materials.insert("glow".into(), replacement.clone());
    assert_eq!(f.draw(&frame), 0);
    let entity = f.world.entity(f.batches.by_material["glow"].entity);
    assert_eq!(
        entity.get::<MeshMaterial3d<EffectMaterial>>().unwrap().0,
        replacement
    );
}
