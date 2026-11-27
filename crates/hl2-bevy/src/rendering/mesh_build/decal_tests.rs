use super::{PrepareContext, collect_map_batches, resolve_draws};
use crate::{
    Status,
    rendering::{Batch, material},
};
use bevy::prelude::{AlphaMode, Assets, Handle, Mat4, Transform, Vec3};
use bevy::{
    camera::primitives::MeshAabb, core_pipeline::core_3d::TransparentSortingInfo3d, pbr::Material,
    render::render_phase::ViewRangefinder3d,
};
use modkit_core::Surface;
use std::collections::BTreeMap;

const FIRST: &str = "decals/decalgraffiti009a";
const LAST: &str = "decals/decalgraffiti001c";

#[test]
#[ignore = "requires an owned Half-Life 2 installation"]
fn canal_graffiti_keeps_placement_order_from_every_view_direction() {
    let root = source_assets::install::discover().unwrap();
    let config = hl2_ui::config::Config::default();
    let mut map = crate::assets::load_with_config(
        &root,
        "hl2",
        "d1_canals_03",
        Default::default(),
        true,
        &config,
    )
    .unwrap();
    let status = Status::default();
    let (mut batches, _) = collect_map_batches(&map, &status, false, false);
    batches.retain(|((_, name, _, _), _)| matches!(name.as_str(), FIRST | LAST));
    let pages: Vec<_> = batches.iter().map(|((_, _, page, _), _)| *page).collect();
    // Rebuild the previous material/lightmap batches for the same BSP triangles.
    let old = Surface::batch(
        map.world
            .surfaces
            .iter()
            .filter(|s| matches!(s.material.as_str(), FIRST | LAST))
            .cloned(),
    );
    let old_centers: BTreeMap<_, _> = old
        .into_iter()
        .map(|(key, surface)| {
            let mut batch = Batch::default();
            batch.append(&surface, Mat4::IDENTITY, None);
            (key, Vec3::from(batch.mesh().get_aabb().unwrap().center))
        })
        .collect();
    let mut geometry = crate::rendering::MapGeometry::planned(batches, &map, false);
    let image = Handle::default();
    let uploaded = material::upload_map_textures(
        &mut map,
        &mut Assets::default(),
        &image,
        (config.video.advanced.filtering, false),
        &image,
    );
    let cx = PrepareContext {
        loaded: &map,
        uploaded: &uploaded,
        lightmaps: &[],
        white: &image,
        black_cube: &image,
        flashlight: &image,
        missing: &image,
        plain_materials: false,
        cpu_skinning: false,
    };
    let mut materials = Assets::default();
    let mut world = bevy::prelude::World::new();
    resolve_draws(
        &mut geometry,
        &cx,
        &mut world.commands(),
        (&mut materials, &mut Default::default()),
        &mut Assets::default(),
        &status,
    );
    let mut graffiti = BTreeMap::new();
    for (draw, page) in geometry.draws.into_iter().zip(pages) {
        // Bevy replaces `final_aabb` with the position bounds when it uploads the mesh
        // and sorts by that center: check the uploaded copy, not the main-world mesh.
        let uploaded = draw.batch.unwrap().mesh().take_gpu_data().unwrap();
        let mat = materials.get(&draw.material).unwrap();
        assert_eq!(mat.alpha_mode(), AlphaMode::Blend);
        let center = Vec3::from(uploaded.get_aabb().unwrap().center);
        let original = old_centers[&(draw.name.clone(), page, false)];
        graffiti
            .entry(draw.name)
            .or_insert(Vec::new())
            .push((center, mat.depth_bias(), original));
    }
    assert_draw_order(&graffiti[FIRST], &graffiti[LAST]);
}

fn assert_draw_order(bottom: &[(Vec3, f32, Vec3)], top: &[(Vec3, f32, Vec3)]) {
    let mut matched = 0;
    let mut reproduced_flip = false;
    for &(center, bias, old_center) in bottom {
        for &(other, top_bias, old_other) in top {
            if center != other {
                continue;
            }
            matched += 1;
            let mut previous = None;
            // The owner's reports (345/2, 359/4, 1084/-6.5, 1083/13.4 in --pitch terms)
            // are examples: sweep the whole sphere of view directions.
            let sweep =
                (0..24).flat_map(|y| (-8..=8).map(move |p| (y as f32 * 15., p as f32 * 10.)));
            for (yaw, pitch) in sweep {
                let forward = crate::coords::source_to_bevy(crate::coords::source_direction(
                    yaw.to_radians(),
                    pitch.to_radians(),
                ));
                let camera = Transform::IDENTITY.looking_to(forward, Vec3::Y);
                let range = ViewRangefinder3d::from_world_from_view(&camera.compute_affine());
                let distance = |mesh_center, depth_bias| {
                    TransparentSortingInfo3d::Sorted {
                        mesh_center,
                        depth_bias,
                    }
                    .sort_distance(&range)
                };
                assert!(distance(center, bias) < distance(other, top_bias));
                let before = distance(old_center, 0.) < distance(old_other, 0.);
                reproduced_flip |= previous.is_some_and(|p| p != before);
                previous = Some(before);
            }
        }
    }
    assert_eq!(
        matched, 2,
        "both overlapping receiver pieces must keep BSP order"
    );
    assert!(
        reproduced_flip,
        "the old batches must reproduce the reported angle-dependent flip"
    );
}
