use super::*;
use crate::shader_rows::ShaderRows;
use crate::shadows::material::ShadowParameters;
use bevy::{
    app::{App, Startup, TaskPoolPlugin},
    asset::{AssetApp, AssetEvent, AssetPlugin},
    ecs::{
        message::{MessageCursor, Messages},
        query::{Changed, Or},
        system::SystemState,
        world::Mut,
    },
    mesh::VertexAttributeValues,
};
use hl2_simulation::shadows::receivers::Decal;

type ChangedDrawsQuery = Query<
    'static,
    'static,
    Entity,
    (
        With<ShadowDraw>,
        Or<(
            Changed<Visibility>,
            Changed<Aabb>,
            Changed<Mesh3d>,
            Changed<MeshTag>,
            Changed<MeshMaterial3d<ShadowMaterial>>,
        )>,
    ),
>;

struct DrawInfo {
    visibility: Visibility,
    aabb: Aabb,
    mesh: Mesh3d,
    tag: MeshTag,
    material: Handle<ShadowMaterial>,
}

struct Fixture {
    app: App,
    batches: ShadowBatches,
    atlas: Handle<ShadowMaterial>,
    blob: Handle<ShadowMaterial>,
    mesh_events: MessageCursor<AssetEvent<Mesh>>,
    buffer_events: MessageCursor<AssetEvent<ShaderBuffer>>,
    changed_draws: SystemState<ChangedDrawsQuery>,
}

impl Fixture {
    fn new(install_buffer: bool) -> Self {
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<ShadowMaterial>()
            .init_asset::<ShaderBuffer>();
        if install_buffer {
            app.add_systems(Startup, install_shadow_buffer);
        }
        app.update();

        let (atlas, blob) = {
            let mut materials = app.world_mut().resource_mut::<Assets<ShadowMaterial>>();
            let mut make = |channel: Vec4| {
                materials.add(ShadowMaterial {
                    texture: Handle::default(),
                    parameters: ShadowParameters {
                        color: Vec4::ONE,
                        jitter: Vec4::ZERO,
                        channel,
                    },
                    verts: SHADOW_VERTS,
                })
            };
            (make(Vec4::X), make(Vec4::W))
        };
        let changed_draws = SystemState::new(app.world_mut());
        let mut fixture = Self {
            app,
            batches: ShadowBatches::default(),
            atlas,
            blob,
            mesh_events: MessageCursor::default(),
            buffer_events: MessageCursor::default(),
            changed_draws,
        };
        fixture.drain_events();
        fixture
    }

    fn sync(&mut self, casters: &[Caster]) {
        self.app
            .world_mut()
            .resource_scope(|world, mut meshes: Mut<Assets<Mesh>>| {
                world.resource_scope(|world, mut buffers: Mut<Assets<ShaderBuffer>>| {
                    let mut state = SystemState::<(Commands, Draws)>::new(world);
                    let (mut commands, mut draws) =
                        state.get_mut(world).expect("shadow draw sync system state");
                    self.batches.sync(
                        casters,
                        (Some(self.atlas.clone()), Some(self.blob.clone())),
                        (&mut commands, &mut meshes, &mut buffers),
                        &mut draws,
                    );
                    state.apply(world);
                });
            });
        self.app.update();
    }

    fn drain_events(&mut self) -> (Vec<AssetEvent<Mesh>>, Vec<AssetEvent<ShaderBuffer>>) {
        let meshes = self
            .mesh_events
            .read(self.app.world().resource::<Messages<AssetEvent<Mesh>>>())
            .cloned()
            .collect();
        let buffers = self
            .buffer_events
            .read(
                self.app
                    .world()
                    .resource::<Messages<AssetEvent<ShaderBuffer>>>(),
            )
            .cloned()
            .collect();
        (meshes, buffers)
    }

    fn changed_draw_count(&mut self) -> usize {
        self.changed_draws
            .get(self.app.world())
            .expect("changed draws query")
            .iter()
            .count()
    }

    fn draw_entities(&mut self) -> Vec<DrawInfo> {
        let mut state = SystemState::<
            Query<
                (
                    &Visibility,
                    &Aabb,
                    &Mesh3d,
                    &MeshTag,
                    &MeshMaterial3d<ShadowMaterial>,
                ),
                With<ShadowDraw>,
            >,
        >::new(self.app.world_mut());
        state
            .get(self.app.world())
            .expect("draw entities query")
            .iter()
            .map(|(v, a, m, t, mat)| DrawInfo {
                visibility: *v,
                aabb: *a,
                mesh: m.clone(),
                tag: t.clone(),
                material: mat.0.clone(),
            })
            .collect()
    }

    fn shadow_rows(&self) -> Vec<Vec4> {
        let buffers = self.app.world().resource::<Assets<ShaderBuffer>>();
        let bytes = buffers
            .get(&SHADOW_VERTS)
            .and_then(|b| b.bytes())
            .expect("SHADOW_VERTS data");
        bytes
            .as_chunks::<16>()
            .0
            .iter()
            .map(|chunk| {
                let f: [f32; 4] = chunk
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|src| f32::from_le_bytes(*src))
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap();
                Vec4::from_array(f)
            })
            .collect()
    }
}

fn quad_decal(x: f32, y: f32, fade: u8) -> Decal {
    Decal {
        positions: vec![
            glam::Vec3::new(x, y, 0.1),
            glam::Vec3::new(x + 10.0, y, 0.1),
            glam::Vec3::new(x + 10.0, y + 10.0, 0.1),
            glam::Vec3::new(x, y + 10.0, 0.1),
        ],
        normals: vec![glam::Vec3::Z; 4],
        uvs: vec![
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(1.0, 0.0),
            glam::Vec2::new(1.0, 1.0),
            glam::Vec2::new(0.0, 1.0),
        ],
        fades: vec![fade, fade, fade, fade],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

fn large_decal(tris: usize) -> Decal {
    let mut positions = Vec::with_capacity(tris * 3);
    let mut normals = Vec::with_capacity(tris * 3);
    let mut uvs = Vec::with_capacity(tris * 3);
    let mut fades = Vec::with_capacity(tris * 3);
    let mut indices = Vec::with_capacity(tris * 3);
    for t in 0..tris {
        let x = t as f32;
        let base = (t * 3) as u32;
        positions.extend([
            glam::Vec3::new(x, 0.0, 0.1),
            glam::Vec3::new(x + 0.5, 0.0, 0.1),
            glam::Vec3::new(x, 0.5, 0.1),
        ]);
        normals.extend([glam::Vec3::Z; 3]);
        uvs.extend([glam::Vec2::ZERO, glam::Vec2::X, glam::Vec2::Y]);
        fades.extend([64, 64, 64]);
        indices.extend([base, base + 1, base + 2]);
    }
    Decal {
        positions,
        normals,
        uvs,
        fades,
        indices,
    }
}

#[test]
fn packs_atlas_and_blob_into_two_draws_with_exact_rows_tag_and_aabb() {
    let mut f = Fixture::new(true);
    let atlas_rect_a = (Vec2::new(0.25, 0.5), Vec2::splat(0.25));
    let atlas_rect_b = (Vec2::new(0.5, 0.0), Vec2::splat(0.5));
    let blob_rect = (Vec2::ZERO, Vec2::ONE);
    let casters = vec![
        Caster::synthetic(true, true, atlas_rect_a, quad_decal(0.0, 0.0, 51)),
        Caster::synthetic(false, true, atlas_rect_a, quad_decal(500.0, 500.0, 255)),
        Caster::synthetic(true, false, blob_rect, quad_decal(20.0, 0.0, 102)),
        Caster::synthetic(true, true, atlas_rect_b, quad_decal(100.0, 10.0, 255)),
        Caster::synthetic(true, false, blob_rect, Decal::default()),
        Caster::synthetic(true, false, blob_rect, quad_decal(40.0, -5.0, 0)),
    ];

    f.sync(&casters);
    let draws = f.draw_entities();
    assert_eq!(draws.len(), 2);

    let atlas_draw = draws
        .iter()
        .find(|d| d.material == f.atlas)
        .expect("atlas draw");
    let blob_draw = draws
        .iter()
        .find(|d| d.material == f.blob)
        .expect("blob draw");
    assert_eq!(atlas_draw.visibility, Visibility::Inherited);
    assert_eq!(blob_draw.visibility, Visibility::Inherited);
    assert_eq!(atlas_draw.tag, MeshTag::new(4));
    assert_eq!(blob_draw.tag, MeshTag::new((4 << 16) | 4));

    let expected_aabb = |list: &[&Caster]| {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for c in list {
            for p in &c.decal.positions {
                let bp = crate::source_to_bevy(Vec3::from_array(p.to_array()));
                min = min.min(bp);
                max = max.max(bp);
            }
        }
        Aabb::from_min_max(min, max)
    };
    assert_eq!(atlas_draw.aabb, expected_aabb(&[&casters[0], &casters[3]]));
    assert_eq!(blob_draw.aabb, expected_aabb(&[&casters[2], &casters[5]]));

    let rows = f.shadow_rows();
    assert_eq!(rows.len(), TOTAL_ROWS);
    let expected_verts: Vec<_> = [&casters[0], &casters[3], &casters[2], &casters[5]]
        .into_iter()
        .flat_map(|caster| decal_vertices(layer(caster, caster.use_atlas).unwrap()))
        .collect();
    assert_eq!(expected_verts.len(), 24);
    for (v, (pos, _, uv, fade)) in expected_verts.iter().enumerate() {
        assert_eq!(rows[v * 2], Vec4::new(pos.x, pos.y, pos.z, *fade));
        assert_eq!(rows[v * 2 + 1], Vec4::new(uv.x, uv.y, 0.0, 0.0));
    }
    assert!(
        rows[expected_verts.len() * 2..]
            .iter()
            .all(|r| *r == Vec4::ZERO)
    );
}

#[test]
fn moving_caster_updates_shadow_verts_with_zero_mesh_rewrites() {
    let mut f = Fixture::new(true);
    let mut casters = vec![
        Caster::synthetic(
            true,
            true,
            (Vec2::ZERO, Vec2::ONE),
            quad_decal(0.0, 0.0, 20),
        ),
        Caster::synthetic(
            true,
            false,
            (Vec2::ZERO, Vec2::ONE),
            quad_decal(10.0, 0.0, 40),
        ),
    ];
    f.sync(&casters);
    let initial_meshes = f.app.world().resource::<Assets<Mesh>>().len();
    let _ = f.drain_events();

    casters[0].decal = quad_decal(35.0, 12.0, 128);
    f.sync(&casters);
    let (mesh_events, buffer_events) = f.drain_events();
    assert!(
        mesh_events.is_empty(),
        "expected 0 mesh events, got {mesh_events:?}"
    );
    assert_eq!(
        f.app.world().resource::<Assets<Mesh>>().len(),
        initial_meshes
    );
    assert!(
        buffer_events
            .iter()
            .any(|e| matches!(e, AssetEvent::Modified { id } if *id == SHADOW_VERTS.id())),
        "expected SHADOW_VERTS modified event"
    );
    let moved_pos = crate::source_to_bevy(Vec3::new(35.0, 12.0, 0.1));
    let rows = f.shadow_rows();
    assert_eq!(
        rows[0],
        Vec4::new(moved_pos.x, moved_pos.y, moved_pos.z, 128.0 / 255.0)
    );
}

#[test]
fn idle_repeat_sync_has_zero_dirty_rows_and_zero_ecs_mutations() {
    let mut f = Fixture::new(true);
    let casters = vec![
        Caster::synthetic(
            true,
            true,
            (Vec2::ZERO, Vec2::ONE),
            quad_decal(1.0, 2.0, 50),
        ),
        Caster::synthetic(
            true,
            false,
            (Vec2::ZERO, Vec2::ONE),
            quad_decal(5.0, 6.0, 90),
        ),
    ];
    f.sync(&casters);
    let _ = f.changed_draw_count();
    let _ = f.drain_events();

    f.sync(&casters);
    let (mesh_events, buffer_events) = f.drain_events();
    assert!(f.batches.dirty.is_none());
    assert!(mesh_events.is_empty(), "mesh events: {mesh_events:?}");
    assert!(buffer_events.is_empty(), "buffer events: {buffer_events:?}");
    assert_eq!(f.changed_draw_count(), 0);
}

#[test]
fn hiding_all_hides_both_draws_and_overflow_falls_back_to_cpu_mesh() {
    let mut f = Fixture::new(true);
    let mut casters = vec![
        Caster::synthetic(
            true,
            true,
            (Vec2::ZERO, Vec2::ONE),
            quad_decal(0.0, 0.0, 20),
        ),
        Caster::synthetic(
            true,
            false,
            (Vec2::ZERO, Vec2::ONE),
            quad_decal(10.0, 0.0, 40),
        ),
    ];
    f.sync(&casters);
    assert_eq!(f.draw_entities().len(), 2);

    for c in &mut casters {
        c.shown = false;
    }
    f.sync(&casters);
    for draw in f.draw_entities() {
        assert_eq!(draw.visibility, Visibility::Hidden);
    }
    let _ = f.changed_draw_count();
    f.sync(&casters);
    assert_eq!(f.changed_draw_count(), 0);

    let overflow_tris = MAX_SHADOW_TRIS + 1;
    let overflow_casters = vec![Caster::synthetic(
        true,
        true,
        (Vec2::ZERO, Vec2::ONE),
        large_decal(overflow_tris),
    )];
    f.sync(&overflow_casters);
    let draws = f.draw_entities();
    let atlas_draw = draws
        .iter()
        .find(|d| d.material == f.atlas)
        .expect("atlas draw");
    let blob_draw = draws
        .iter()
        .find(|d| d.material == f.blob)
        .expect("blob draw");
    assert_eq!(atlas_draw.visibility, Visibility::Inherited);
    assert_eq!(atlas_draw.tag, MeshTag::new(0));
    assert_eq!(blob_draw.visibility, Visibility::Hidden);
    let meshes = f.app.world().resource::<Assets<Mesh>>();
    let mesh = meshes.get(&atlas_draw.mesh.0).expect("cpu overflow mesh");
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("missing positions");
    };
    assert_eq!(pos.len(), overflow_tris * 3);

    let mut no_buf = Fixture::new(false);
    no_buf.sync(&[Caster::synthetic(
        true,
        false,
        (Vec2::ZERO, Vec2::ONE),
        quad_decal(2.0, 3.0, 64),
    )]);
    let no_buf_draws = no_buf.draw_entities();
    assert_eq!(no_buf_draws.len(), 1);
    assert_eq!(no_buf_draws[0].tag, MeshTag::new(0));
}

#[test]
fn handoff_uses_both_receiver_projections_and_complementary_weights() {
    let mut caster = Caster::synthetic(true, true, (Vec2::ZERO, Vec2::ONE), quad_decal(0., 0., 0));
    caster.transition.weight = 0.25;
    caster.transition.blob = quad_decal(10., 0., 0);
    let atlas = layer(&caster, true).unwrap();
    let blob = layer(&caster, false).unwrap();
    assert_eq!((atlas.opacity, blob.opacity), (0.25, 0.75));
    assert_ne!(atlas.decal.positions, blob.decal.positions);
    caster.transition.weight = 1.;
    assert!(layer(&caster, false).is_none());
}
