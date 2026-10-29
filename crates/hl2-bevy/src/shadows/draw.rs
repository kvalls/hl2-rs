//! A caster's shadow draw: its decal mesh, rewritten in place, and its visibility.
use super::{Caster, ShadowDraw, ShadowMaterial};
use crate::campaign::MapOwned;
use bevy::{
    asset::{Assets, Handle, RenderAssetUsages},
    camera::{
        primitives::{Aabb, MeshAabb},
        visibility::{ViewVisibility, Visibility},
    },
    ecs::{
        query::With,
        system::{Commands, Query},
    },
    math::{Vec2, Vec3},
    mesh::{Indices, Mesh, Mesh3d, MeshVertexAttribute, PrimitiveTopology, VertexAttributeValues},
    pbr::MeshMaterial3d,
    transform::components::Transform,
};
use hl2_simulation::shadows::receivers::Decal;

pub(super) type Draws<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Visibility,
        &'static mut Aabb,
        &'static mut MeshMaterial3d<ShadowMaterial>,
        &'static ViewVisibility,
    ),
    With<ShadowDraw>,
>;

/// Shows or hides the caster's draw.
pub(super) fn show(caster: &Caster, draws: &mut Draws, shown: bool) {
    let Some((entity, _)) = caster.draw else {
        return;
    };
    if let Ok((mut visibility, ..)) = draws.get_mut(entity) {
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

/// Writes the caster's decal into its mesh, texture coordinates mapped into `rect` (origin, size)
/// of `material`'s texture; the mesh and draw are made on the first write.
pub(super) fn write(
    caster: &mut Caster,
    (commands, meshes): (&mut Commands, &mut Assets<Mesh>),
    (material, rect): (Handle<ShadowMaterial>, (Vec2, Vec2)),
    draws: &mut Draws,
) {
    match &caster.draw {
        Some((entity, handle)) => {
            let Some(mut mesh) = meshes.get_mut(handle) else {
                return;
            };
            fill(&mut mesh, &caster.decal, rect);
            if let Ok((_, mut current, mut drawn, _)) = draws.get_mut(*entity) {
                if let Some(aabb) = mesh.compute_aabb() {
                    *current = aabb;
                }
                if drawn.0 != material {
                    drawn.0 = material;
                }
            }
        }
        None => {
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
            );
            fill(&mut mesh, &caster.decal, rect);
            let aabb = mesh.compute_aabb().unwrap_or_default();
            let handle = meshes.add(mesh);
            let entity = commands
                .spawn((
                    Mesh3d(handle.clone()),
                    MeshMaterial3d(material),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    aabb,
                    ShadowDraw,
                    MapOwned,
                ))
                .id();
            caster.draw = Some((entity, handle));
        }
    }
}

/// The decal as mesh attributes in Bevy's axes, reusing the mesh's buffers.
fn fill(mesh: &mut Mesh, decal: &Decal, (origin, size): (Vec2, Vec2)) {
    let point = |p: &glam::Vec3| crate::source_to_bevy(Vec3::from_array(p.to_array())).to_array();
    set3(
        mesh,
        Mesh::ATTRIBUTE_POSITION,
        decal.positions.iter().map(point),
    );
    set3(
        mesh,
        Mesh::ATTRIBUTE_NORMAL,
        decal.normals.iter().map(point),
    );
    let uv = |uv: &glam::Vec2| (origin + Vec2::from_array(uv.to_array()) * size).to_array();
    match mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(uvs)) => {
            uvs.clear();
            uvs.extend(decal.uvs.iter().map(uv));
        }
        _ => mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            decal.uvs.iter().map(uv).collect::<Vec<_>>(),
        ),
    }
    let fade = |f: &u8| [1., 1., 1., f32::from(*f) / 255.];
    match mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR) {
        Some(VertexAttributeValues::Float32x4(colors)) => {
            colors.clear();
            colors.extend(decal.fades.iter().map(fade));
        }
        _ => mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            decal.fades.iter().map(fade).collect::<Vec<_>>(),
        ),
    }
    match mesh.indices_mut() {
        Some(Indices::U32(indices)) => {
            indices.clear();
            indices.extend_from_slice(&decal.indices);
        }
        _ => mesh.insert_indices(Indices::U32(decal.indices.clone())),
    }
}

fn set3(mesh: &mut Mesh, attribute: MeshVertexAttribute, values: impl Iterator<Item = [f32; 3]>) {
    match mesh.attribute_mut(attribute) {
        Some(VertexAttributeValues::Float32x3(buffer)) => {
            buffer.clear();
            buffer.extend(values);
        }
        _ => mesh.insert_attribute(attribute, values.collect::<Vec<_>>()),
    }
}
