//! Decals on static props land on the drawn mesh (Source SDK game/client/fx_impact.cpp Impact:
//! AddDecalToStaticProp with the shot's ray continued 8 units past the trace's end; the engine's
//! studio AddDecal paints the model's triangles along that ray). Our trace stops at a static
//! prop's VPhysics hull, which can sit several units off the drawn mesh (a tree's trunk inside
//! its hull), so the hit moves to where the continued shot meets the prop's front-facing
//! triangles.
use crate::physics::RayHit;
use glam::Vec3;
use modkit_core::studio_decals::{ray_distance, SkinVertex};
use modkit_core::World;

/// fx_impact.cpp: the decal ray runs this far past the trace's end.
const PAST_HIT: f32 = 8.;
/// How far in front of the hull hit a drawn face may lie (a mesh poking out of its hull): our
/// bound, as impacts' STATIC_PROP_GAP.
const BEFORE_HIT: f32 = 4.;
/// Static props whose origin is farther than this from the hit are not tested (our bound,
/// above any HL2 static prop's reach from its origin).
const NEAR: f32 = 1024.;

/// `hit` moved onto the drawn triangles of a static prop along `direction`: the nearest
/// front-facing one from BEFORE_HIT before the hit to PAST_HIT past it, with that triangle's
/// normal. None when no static prop's mesh lies on that stretch.
pub fn on_mesh(world: &World, hit: &RayHit, direction: Vec3) -> Option<RayHit> {
    let direction = direction.normalize_or_zero();
    if direction == Vec3::ZERO {
        return None;
    }
    let start = hit.position - direction * BEFORE_HIT;
    let mut best: Option<(f32, Vec3)> = None;
    let props = world.model_instances.iter().filter(|instance| {
        instance.entity.is_none()
            && !instance.background
            && instance.origin.distance(hit.position) < NEAR
    });
    for instance in props {
        let Some(surfaces) = world.model_assets.get(&instance.asset_key()) else {
            continue;
        };
        let rotation = crate::physics::angles(instance.angles);
        let place = |p: Vec3| instance.origin + rotation * (p * instance.scale);
        for surface in surfaces {
            let vertices: Vec<SkinVertex> = surface
                .vertices
                .iter()
                .map(|v| {
                    let posed = place(v.position);
                    SkinVertex {
                        posed,
                        bind: posed,
                        joints: [0; 4],
                        weights: [1., 0., 0., 0.],
                    }
                })
                .collect();
            for tri in surface.indices.as_chunks::<3>().0 {
                let reach = BEFORE_HIT + PAST_HIT;
                let Some(t) = ray_distance(&vertices, tri, start, direction, reach) else {
                    continue;
                };
                if best.is_none_or(|(nearest, _)| t < nearest) {
                    // ray_distance's front faces: (b - a) x (c - a) turns back toward the shot.
                    let [a, b, c] = tri.map(|i| vertices[i as usize].posed);
                    best = Some((t, (b - a).cross(c - a).normalize_or_zero()));
                }
            }
        }
    }
    best.map(|(t, normal)| RayHit {
        position: start + direction * t,
        normal,
        ..*hit
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A static prop whose drawn face (facing -x, 16 square) stands at `x`.
    fn world(x: f32) -> World {
        let vertex = |y: f32, z: f32| modkit_core::Vertex {
            position: Vec3::new(x, y, z),
            uv: glam::Vec2::ZERO,
            color: [255; 4],
            light_uv: glam::Vec2::ZERO,
            skin: None,
            normal: Vec3::NEG_X,
        };
        let mut world = World::default();
        world.model_assets.insert(
            "tree.mdl#0".into(),
            vec![modkit_core::Surface {
                background: false,
                material: "bark".into(),
                vertices: vec![
                    vertex(-8., -8.),
                    vertex(-8., 8.),
                    vertex(8., -8.),
                    vertex(8., 8.),
                ],
                indices: vec![0, 1, 2, 2, 1, 3],
                lightmap: None,
                flex_source: None,
            }],
        );
        world.model_instances.push(modkit_core::ModelInstance {
            background: false,
            model: "tree.mdl".into(),
            origin: Vec3::ZERO,
            angles: Vec3::ZERO,
            skin: 0,
            body: 0,
            scale: 1.,
            kind: "static_prop".into(),
            solid: true,
            solid_mode: Some(6),
            entity: None,
        });
        world
    }

    #[test]
    fn a_hull_hit_moves_along_the_shot_onto_the_drawn_face() {
        let hit = RayHit {
            entity: usize::MAX,
            position: Vec3::new(6., 1., 2.),
            normal: Vec3::NEG_X,
            ..Default::default()
        };
        // The face 4 units inside the hull: within the 8 the decal ray runs past the hit.
        let moved = on_mesh(&world(10.), &hit, Vec3::X).expect("face within reach");
        assert!(moved.position.distance(Vec3::new(10., 1., 2.)) < 1e-4);
        assert!(moved.normal.distance(Vec3::NEG_X) < 1e-4);
        // A face 2 units outside the hull is found too; one 9 units inside is not.
        assert!(on_mesh(&world(4.), &hit, Vec3::X).is_some());
        assert!(on_mesh(&world(15.), &hit, Vec3::X).is_none());
        // Shot from the far side, the face's back is not a front face.
        let behind = RayHit {
            position: Vec3::new(14., 1., 2.),
            ..hit
        };
        assert!(on_mesh(&world(10.), &behind, Vec3::NEG_X).is_none());
    }
}
