//! Prepared brush material resources; frame updates perform no asset I/O.
use crate::{gameplay::Gameplay, rendering::SourceMaterial, simulation::Simulation};
use bevy::prelude::*;

#[derive(Component)]
pub struct TwoTextureDraw {
    pub entity: Option<usize>,
    pub center: glam::Vec3,
    pub frames: source_assets::proximity_material::Frames,
    pub scroll: source_assets::monitor_material::Animation,
    pub base: Vec<Handle<Image>>,
    pub second: Vec<Handle<Image>>,
}
pub fn present(
    game: Res<Gameplay>,
    sim: Res<Simulation>,
    draws: Query<(&TwoTextureDraw, &MeshMaterial3d<SourceMaterial>)>,
    mut materials: ResMut<Assets<SourceMaterial>>,
) {
    let eye = glam::Vec3::from_array(sim.eye().to_array());
    let player_center = eye - glam::Vec3::Z * sim.player.eye_height
        + glam::Vec3::Z * if sim.player.crouched { 18. } else { 36. };
    for (draw, handle) in &draws {
        let center = draw
            .entity
            .and_then(|id| game.scene.states.get(id))
            .map_or(draw.center, |s| s.origin + s.rotation * draw.center);
        let frames = draw.frames.sample(
            center.distance(player_center),
            [draw.base.len(), draw.second.len()],
        );
        let (_, uv) = draw.scroll.sample([1.; 3], game.scene.time as f32);
        let secondary_uv = Mat3::from_cols(
            Vec3::new(uv[0][0], uv[1][0], 0.),
            Vec3::new(uv[0][1], uv[1][1], 0.),
            Vec3::new(uv[0][2], uv[1][2], 1.),
        );
        if materials.get(&handle.0).is_some_and(|m| {
            m.base != draw.base[frames[0]]
                || m.iris != draw.second[frames[1]]
                || m.secondary_uv != secondary_uv
        }) && let Some(mut m) = materials.get_mut(&handle.0)
        {
            m.base = draw.base[frames[0]].clone();
            m.iris = draw.second[frames[1]].clone();
            m.secondary_uv = secondary_uv;
        }
    }
}

/// Each two-texture draw's frames and texture transform (run reports).
pub fn report<'a>(
    draws: impl Iterator<
        Item = (
            &'a bevy::ecs::name::Name,
            &'a TwoTextureDraw,
            &'a bevy::pbr::MeshMaterial3d<crate::rendering::SourceMaterial>,
        ),
    >,
    materials: &bevy::asset::Assets<crate::rendering::SourceMaterial>,
    time: f64,
) -> serde_json::Value {
    let entries: Vec<_> = draws
        .filter_map(|(name, draw, handle)| {
            let material = materials.get(&handle.0)?;
            Some(
                serde_json::json!({"name":name.as_str(),"entity":draw.entity,
                "base_frames":draw.base.len(),"second_frames":draw.second.len(),
                "base_frame":draw.base.iter().position(|h|h==&material.base),
                "second_frame":draw.second.iter().position(|h|h==&material.iris),
                "secondary_uv":material.secondary_uv.to_cols_array(),"time":time}),
            )
        })
        .collect();
    serde_json::json!(entries)
}
