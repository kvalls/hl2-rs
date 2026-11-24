//! Impact-mark decal spawning, pose updates, and cleanup.
use super::{DecalDraw, Effects, mesh};
use crate::{gameplay::Gameplay, rendering, simulation::Simulation};
use bevy::prelude::{Assets, Commands, Mesh, Mesh3d, MeshMaterial3d, Transform, Vec3, Visibility};

/// Synchronize spawned impact-mark decal entities with the simulation's active marks.
pub(super) fn present(
    commands: &mut Commands,
    effects: &mut Effects,
    game: &Gameplay,
    sim: &Simulation,
    meshes: &mut Assets<Mesh>,
) {
    let dead: Vec<_> = effects
        .marks
        .keys()
        .copied()
        .filter(|id| !game.impacts.marks.iter().any(|m| m.id == *id))
        .collect();
    for id in dead {
        if let Some((entity, handle)) = effects.marks.remove(&id) {
            commands.entity(entity).try_despawn();
            meshes.remove(&handle);
        }
    }
    effects.mark_draws = 0;
    for mark in &game.impacts.marks {
        let (transform, visible) =
            game.scene
                .states
                .get(mark.entity)
                .map_or((Transform::IDENTITY, true), |s| {
                    let (p, r) = sim
                        .physics
                        .entity_pose(mark.entity)
                        .unwrap_or((s.origin, s.rotation));
                    let mut t = rendering::entity_transform(p, r);
                    t.scale = Vec3::splat(mark.scale);
                    (t, !s.killed && s.visible)
                });
        if !effects.marks.contains_key(&mark.id) {
            let mesh = meshes.add(mesh(
                mark.vertices
                    .iter()
                    .map(|v| {
                        crate::source_to_bevy(Vec3::from_array(v.position.to_array())).to_array()
                    })
                    .collect(),
                mark.vertices.iter().map(|v| v.uv.to_array()).collect(),
                // Linear light; lit decals apply it after the sRGB decode.
                vec![
                    {
                        let [r, g, b] = mark.tint.map(|c| c.max(0.));
                        [r, g, b, 1.]
                    };
                    mark.vertices.len()
                ],
                (0..mark.vertices.len() as u32).collect(),
                false,
            ));
            let material = effects.materials[&mark.material].clone();
            let entity = commands
                .spawn((
                    crate::campaign::MapOwned,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    transform,
                    (DecalDraw { id: mark.id }, bevy::light::NotShadowCaster),
                ))
                .id();
            effects.marks.insert(mark.id, (entity, mesh));
        }
        let entity = effects.marks[&mark.id].0;
        commands.entity(entity).insert((
            transform,
            if visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
        ));
        effects.mark_draws += usize::from(visible);
    }
}
