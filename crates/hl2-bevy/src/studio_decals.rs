//! Studio decals on GPU-skinned NPCs (DESIGN 13f): each queued request is clipped from
//! the entity's skinned draws as they are posed now (joint GlobalTransforms x inverse
//! binds) and spawned as a mesh sharing the draw's joints, so it follows the animation.
use crate::{effects::Effects, gameplay::Gameplay, rendering::SourceEntity};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{
        Indices, PrimitiveTopology, VertexAttributeValues,
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    },
    prelude::*,
};
use modkit_core::studio_decals::{SkinVertex, project, ray_distance};
use std::collections::BTreeMap;

/// r_maxmodeldecal: decals kept per model (oldest removed first).
const MAX_MODEL_DECALS: usize = 50;
/// Bind-space lift off the skin so the decal does not z-fight it.
const LIFT: f32 = 0.25;
/// The skin re-shoot starts this far before the trace hit (units) and reaches three
/// times as far.
const RESHOOT_BACK: f32 = 16.;

#[derive(Component)]
pub struct StudioDecalDraw {
    entity: usize,
}
#[derive(Resource, Default)]
pub struct StudioDecals {
    /// Decal draws per entity, oldest first: (request id, draw entities).
    by_entity: BTreeMap<usize, Vec<(usize, Vec<Entity>)>>,
    pub built: usize,
    pub empty: usize,
    /// The newest built decal: material, triangles, vertex tint.
    last: Option<(String, usize, [f32; 4])>,
}
impl StudioDecals {
    /// A new map was installed: its draws were despawned with the old map (`MapOwned`), and
    /// entity indices now name the new map's entities, so the per-entity lists start empty.
    pub fn new_map(&mut self) {
        self.by_entity.clear();
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"built": self.built, "empty": self.empty,
            "live": self.by_entity.values().map(Vec::len).sum::<usize>(), "last": self.last})
    }
}

#[allow(clippy::too_many_arguments)]
pub fn present(
    mut commands: Commands,
    mut state: ResMut<StudioDecals>,
    mut game: ResMut<Gameplay>,
    effects: Res<Effects>,
    mut meshes: ResMut<Assets<Mesh>>,
    binds: Res<Assets<SkinnedMeshInverseBindposes>>,
    draws: Query<(&SourceEntity, &Mesh3d, &SkinnedMesh)>,
    joints: Query<&GlobalTransform>,
    mut decal_draws: Query<(&StudioDecalDraw, &mut Visibility)>,
) {
    for request in std::mem::take(&mut game.impacts.studio_requests) {
        let Some(material) = effects.decal_material(&request.material) else {
            continue;
        };
        let center = crate::source_to_bevy(Vec3::from_array(request.position.to_array()));
        let direction = crate::source_to_bevy(Vec3::from_array(request.direction.to_array()));
        let tint = game
            .world
            .lighting
            .as_deref()
            .map_or(Vec4::ONE, |lighting| {
                let normal = -request.direction.normalize_or_zero();
                lighting
                    .state_at(request.position + normal)
                    .vertex_light(request.position, normal, false)
                    .extend(1.)
                    .to_array()
                    .into()
            });
        // The entity's skinned draws as posed now.
        let posed: Vec<_> = draws
            .iter()
            .filter(|(owner, ..)| owner.0 == request.entity)
            .filter_map(|(_, mesh, skin)| {
                let source = meshes.get(&mesh.0)?;
                let vertices = skin_vertices(source, skin, &binds, &joints)?;
                let indices: Vec<u32> = match source.indices() {
                    Some(Indices::U32(i)) => i.clone(),
                    Some(Indices::U16(i)) => i.iter().map(|&i| u32::from(i)).collect(),
                    None => (0..vertices.len() as u32).collect(),
                };
                Some((skin.clone(), vertices, indices))
            })
            .collect();
        // Re-shoot the ray at the skin: the trace hit is on the convex trace shape.
        let along = glam::Vec3::from_array(direction.normalize_or_zero().to_array());
        let start = glam::Vec3::from_array(center.to_array()) - along * RESHOOT_BACK;
        let center = posed
            .iter()
            .filter_map(|(_, vertices, indices)| {
                ray_distance(vertices, indices, start, along, RESHOOT_BACK * 3.)
            })
            .min_by(f32::total_cmp)
            .map_or(glam::Vec3::from_array(center.to_array()), |t| {
                start + along * t
            });
        let mut spawned = Vec::new();
        let mut triangles = 0;
        for (skin, vertices, indices) in &posed {
            let decal = project(
                vertices,
                indices,
                center,
                along,
                request.radius,
                request.angle,
            );
            if decal.is_empty() {
                continue;
            }
            let count = decal.len();
            triangles += count / 3;
            let mut built = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            );
            built.insert_attribute(
                Mesh::ATTRIBUTE_POSITION,
                decal
                    .iter()
                    .map(|v| (v.bind + v.normal * LIFT).to_array())
                    .collect::<Vec<_>>(),
            );
            built.insert_attribute(
                Mesh::ATTRIBUTE_NORMAL,
                decal
                    .iter()
                    .map(|v| v.normal.to_array())
                    .collect::<Vec<_>>(),
            );
            built.insert_attribute(
                Mesh::ATTRIBUTE_UV_0,
                decal.iter().map(|v| v.uv.to_array()).collect::<Vec<_>>(),
            );
            built.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0f32; 2]; count]);
            built.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![tint.to_array(); count]);
            built.insert_attribute(
                Mesh::ATTRIBUTE_JOINT_INDEX,
                VertexAttributeValues::Uint16x4(decal.iter().map(|v| v.joints).collect()),
            );
            built.insert_attribute(
                Mesh::ATTRIBUTE_JOINT_WEIGHT,
                decal.iter().map(|v| v.weights).collect::<Vec<_>>(),
            );
            if built.generate_skinned_mesh_bounds().is_err() {
                continue;
            }
            spawned.push(
                commands
                    .spawn((
                        crate::campaign::MapOwned,
                        Mesh3d(meshes.add(built)),
                        MeshMaterial3d(material.clone()),
                        skin.clone(),
                        bevy::camera::visibility::DynamicSkinnedMeshBounds,
                        Transform::IDENTITY,
                        Visibility::Visible,
                        StudioDecalDraw {
                            entity: request.entity,
                        },
                    ))
                    .id(),
            );
        }
        if spawned.is_empty() {
            state.empty += 1;
            continue;
        }
        state.built += 1;
        state.last = Some((request.material.clone(), triangles, tint.to_array()));
        let list = state.by_entity.entry(request.entity).or_default();
        list.push((request.id, spawned));
        while list.len() > MAX_MODEL_DECALS {
            for entity in list.remove(0).1 {
                commands.entity(entity).despawn();
            }
        }
    }
    // Decals hide with their entity (killed or script-hidden).
    for (draw, mut visibility) in &mut decal_draws {
        let shown = game
            .scene
            .states
            .get(draw.entity)
            .is_some_and(|s| s.visible && !s.killed);
        let wanted = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// The draw's vertices as posed now (skinning matrices from the joints' last
/// propagated transforms) beside their bind positions and influences.
fn skin_vertices(
    mesh: &Mesh,
    skin: &SkinnedMesh,
    binds: &Assets<SkinnedMeshInverseBindposes>,
    joints: &Query<&GlobalTransform>,
) -> Option<Vec<SkinVertex>> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return None;
    };
    let Some(VertexAttributeValues::Uint16x4(indices)) =
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
    else {
        return None;
    };
    let Some(VertexAttributeValues::Float32x4(weights)) =
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
    else {
        return None;
    };
    let inverse = binds.get(&skin.inverse_bindposes)?;
    let matrices: Vec<Mat4> = skin
        .joints
        .iter()
        .zip(inverse.iter())
        .map(|(joint, bind)| {
            joints
                .get(*joint)
                .map_or(Mat4::IDENTITY, |g| g.to_matrix() * *bind)
        })
        .collect();
    Some(
        positions
            .iter()
            .zip(indices)
            .zip(weights)
            .map(|((p, j), w)| {
                let bind = Vec3::from_array(*p);
                let mut posed = Vec3::ZERO;
                for k in 0..4 {
                    if w[k] > 0.
                        && let Some(m) = matrices.get(usize::from(j[k]))
                    {
                        posed += m.transform_point3(bind) * w[k];
                    }
                }
                SkinVertex {
                    posed: glam::Vec3::from_array(posed.to_array()),
                    bind: glam::Vec3::from_array(*p),
                    joints: *j,
                    weights: *w,
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_map_forgets_the_old_maps_decals() {
        let mut decals = StudioDecals::default();
        decals
            .by_entity
            .insert(7, vec![(1, vec![Entity::PLACEHOLDER])]);
        decals.built = 1;
        assert_eq!(decals.report()["live"], 1);
        decals.new_map();
        assert_eq!(
            decals.report()["live"],
            0,
            "stale draws are never evicted again"
        );
        assert_eq!(decals.built, 1, "diagnostic counters keep counting");
    }
}
