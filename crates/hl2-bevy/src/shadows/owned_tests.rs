//! Regression coverage for the canal corridor's static paper/junk receivers.
use glam::{Quat, Vec3};
use hl2_simulation::shadows::{
    frame::{Frame, Placement},
    receivers::{Cast, Decal, Receivers, Scratch},
};

#[test]
#[ignore = "requires an owned installed Half-Life 2 copy"]
fn canal_papers_are_static_props_and_receive_npc_style_shadows_only() {
    let root = source_assets::install::discover().unwrap();
    let loaded = crate::assets::load(&root, "hl2", "d1_canals_01").unwrap();
    let spawn = Vec3::from_array(loaded.world.spawn().0.to_array());
    let paper = loaded
        .world
        .model_instances
        .iter()
        .find(|instance| {
            instance.model.ends_with("garbage256_composite002b.mdl")
                && instance.origin.distance(spawn) < 650.
        })
        .expect("the corridor's paper/junk prop");
    assert!(
        paper.entity.is_none(),
        "a static prop, not a dynamic entity or BSP overlay"
    );
    let materials: std::collections::BTreeSet<_> = loaded.world.model_assets[&paper.asset_key()]
        .iter()
        .map(|surface| surface.material.as_str())
        .collect();
    let surfaces: Vec<_> = loaded
        .world
        .surfaces
        .iter()
        .filter(|surface| materials.contains(surface.material.as_str()))
        .collect();
    assert!(!surfaces.is_empty());
    assert!(
        surfaces
            .iter()
            .flat_map(|s| &s.vertices)
            .all(|v| v.skin.is_some()),
        "baked static-prop triangles retain receiver identity"
    );
    let triangle = surfaces
        .iter()
        .flat_map(|surface| {
            surface
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|indices| indices.map(|index| surface.vertices[index as usize].position))
        })
        .filter(|p| (p[1] - p[0]).cross(p[2] - p[0]).z.abs() > 1.)
        .min_by(|a, b| {
            let distance = |p: &[Vec3; 3]| ((p[0] + p[1] + p[2]) / 3. - spawn).length_squared();
            distance(a).total_cmp(&distance(b))
        })
        .expect("a floor-facing paper triangle");
    let center = (triangle[0] + triangle[1] + triangle[2]) / 3.;
    let frame = Frame::render_to_texture(
        (Vec3::new(-16., -16., 0.), Vec3::new(16., 16., 32.)),
        &Placement::new(center + Vec3::Z * 24., Quat::IDENTITY),
        Vec3::NEG_Z,
        75.,
    );
    let receivers = Receivers::new(surfaces, |_| true);
    let mut decal = Decal::default();
    let mut scratch = Scratch::default();
    receivers.decal(&frame, Cast::default(), &mut scratch, &mut decal);
    assert!(
        decal.is_empty(),
        "ordinary prop/blob projections skip static receivers"
    );
    receivers.decal(
        &frame,
        Cast {
            static_props: true,
            ..Default::default()
        },
        &mut scratch,
        &mut decal,
    );
    assert!(
        !decal.is_empty(),
        "NPC/player RTT projections can reach the same paper geometry"
    );
    eprintln!(
        "paper {:?}, sample {center:?}, {} NPC-style receiver triangles",
        paper.origin,
        decal.indices.len() / 3
    );
}
