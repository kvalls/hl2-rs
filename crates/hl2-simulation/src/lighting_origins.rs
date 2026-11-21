//! Model lighting points set by an entity's `LightingOrigin` and `LightingOriginHack` keys
//! (SDK server baseanimating.cpp CBaseAnimating::Activate, SetLightingOrigin and
//! SetLightingOriginRelative; client c_baseanimating.cpp OnInternalDrawModel and
//! C_InfoLightingRelative::GetLightingOffset).
use crate::entities::State;
use glam::{Quat, Vec3};
use modkit_core::World;

/// Where a model takes its lighting from instead of its own illumination point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightingOrigin {
    /// `LightingOrigin`: the named entity's origin is the lighting point.
    pub at: Option<usize>,
    /// `LightingOriginHack`: an info_lighting_relative and its `LightingLandmark`. The lighting
    /// point keeps its offset from the landmark, carried to the info_lighting_relative (the
    /// offset is info * inverse(landmark)). d1_trainstation_01's seats and racks name one: the
    /// landmark rides the moving car, the info entity stands in a shaded copy of the car, so
    /// the moving seats are lit as if inside it rather than by the world they pass.
    pub relative: Option<(usize, usize)>,
}

fn named(world: &World, name: &str) -> Option<usize> {
    world.entities.iter().position(|e| {
        e.get("targetname")
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })
}

/// Resolves entity `id`'s keys once, as Activate does: the first entity of each name. A
/// LightingOriginHack whose target is not an info_lighting_relative is ignored (the SDK only
/// warns), and one without a landmark offsets nothing (GetLightingOffset's identity).
pub fn resolve(world: &World, id: usize) -> Option<LightingOrigin> {
    let entity = world.entities.get(id)?;
    let key = |k: &str| entity.get(k).filter(|v| !v.is_empty());
    let at = key("LightingOrigin").and_then(|name| named(world, name));
    let relative = key("LightingOriginHack")
        .and_then(|name| named(world, name))
        .filter(|&info| world.entities[info].get("classname") == Some("info_lighting_relative"))
        .and_then(|info| {
            let landmark = world.entities[info].get("LightingLandmark")?;
            Some((info, named(world, landmark)?))
        });
    (at.is_some() || relative.is_some()).then_some(LightingOrigin { at, relative })
}

/// An entity's current pose, unless it is gone.
fn live_pose(states: &[State], id: usize) -> Option<(Vec3, Quat)> {
    states
        .get(id)
        .filter(|s| !s.killed)
        .map(|s| (s.origin, s.rotation))
}

impl LightingOrigin {
    /// The lighting point for a model whose own illumination point is `point`, from the
    /// entities' current poses; an entity that is gone leaves the point as it is.
    pub fn point(&self, states: &[State], point: Vec3) -> Vec3 {
        self.point_with(|id| live_pose(states, id), point)
    }

    /// The lighting offset (info * inverse(landmark)) as a rotation and translation: the
    /// whole transform the model is lit through (pLightingOffset), so the mock car's ceiling
    /// spots light the moving car's seats from the same places above them.
    pub fn offset(&self, states: &[State]) -> Option<(Quat, Vec3)> {
        self.offset_with(|id| live_pose(states, id))
    }

    fn offset_with(&self, pose: impl Fn(usize) -> Option<(Vec3, Quat)>) -> Option<(Quat, Vec3)> {
        let (info, mark) = self.relative?;
        Some(relative_offset(pose(info)?, pose(mark)?))
    }

    fn point_with(&self, pose: impl Fn(usize) -> Option<(Vec3, Quat)>, point: Vec3) -> Vec3 {
        let point = self.at.and_then(&pose).map_or(point, |(origin, _)| origin);
        self.offset_with(pose)
            .map_or(point, |(rotation, translation)| {
                rotation * point + translation
            })
    }
}

/// The lighting offset info * inverse(landmark) as a rotation and translation.
fn relative_offset(info: (Vec3, Quat), landmark: (Vec3, Quat)) -> (Quat, Vec3) {
    let rotation = info.1 * landmark.1.inverse();
    (rotation, info.0 - rotation * landmark.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::Entity;

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// d1_trainstation_01: a seat parented to the moving train names info_lighting_relative
    /// lighting_traincar, whose landmark target_lighting_traincar rides the train too.
    #[test]
    fn a_relative_lighting_origin_follows_the_landmark_to_the_info_entity() {
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "prop_dynamic"),
                    ("lightingoriginhack", "lighting_traincar"),
                ]),
                entity(&[
                    ("classname", "info_lighting_relative"),
                    ("targetname", "lighting_traincar"),
                    ("LightingLandmark", "target_lighting_traincar"),
                ]),
                entity(&[
                    ("classname", "info_target"),
                    ("targetname", "target_lighting_traincar"),
                ]),
            ],
            ..Default::default()
        };
        let origin = resolve(&world, 0).expect("resolved");
        assert_eq!(origin.relative, Some((1, 2)));
        assert_eq!(origin.at, None);
        let info = Vec3::new(-6768., -6016., 24.);
        // The car (and its landmark) moves; the seat keeps its offset from the landmark.
        for shift in [0., 900., 2400.] {
            let landmark = Vec3::new(-5360. + shift, -1856., 25.);
            let poses = [Vec3::ZERO, info, landmark].map(|p| (p, Quat::IDENTITY));
            let seat = Vec3::new(-5615.72 + shift, -1903.72, 16.28);
            let lit = origin.point_with(|id| poses.get(id).copied(), seat);
            assert!(lit.distance(info + (seat - landmark)) < 0.01, "{lit}");
        }
        // A yawed landmark carries the offset through its rotation.
        let turned = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let poses = [
            (Vec3::ZERO, Quat::IDENTITY),
            (info, Quat::IDENTITY),
            (Vec3::ZERO, turned),
        ];
        let lit = origin.point_with(|id| poses.get(id).copied(), Vec3::new(0., 10., 0.));
        assert!(lit.distance(info + Vec3::new(10., 0., 0.)) < 0.01, "{lit}");
        // The offset maps the moving seat onto its place in the mock car.
        let poses = [Vec3::ZERO, info, Vec3::new(-4000., -1856., 25.)].map(|p| (p, Quat::IDENTITY));
        let (rotation, translation) = origin
            .offset_with(|id| poses.get(id).copied())
            .expect("offset");
        let seat = Vec3::new(-4255.72, -1903.72, 16.28);
        let mapped = rotation * seat + translation;
        assert!(
            mapped.distance(Vec3::new(-7023.72, -6063.72, 15.28)) < 0.01,
            "{mapped}"
        );
        // Without its landmark (killed) the point is left alone.
        let lit = origin.point_with(|id| poses.get(id).copied().filter(|_| id != 2), Vec3::ONE);
        assert_eq!(lit, Vec3::ONE);
    }

    /// A LightingOriginHack must name an info_lighting_relative; LightingOrigin names any entity.
    #[test]
    fn lighting_origin_targets_are_checked_like_activate() {
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "prop_dynamic"),
                    ("LightingOriginHack", "spot"),
                    ("LightingOrigin", "spot"),
                ]),
                entity(&[("classname", "info_target"), ("targetname", "Spot")]),
                entity(&[("classname", "prop_dynamic")]),
            ],
            ..Default::default()
        };
        let origin = resolve(&world, 0).expect("resolved");
        assert_eq!(
            origin,
            LightingOrigin {
                at: Some(1),
                relative: None
            }
        );
        assert_eq!(resolve(&world, 2), None);
        let poses = [
            (Vec3::ZERO, Quat::IDENTITY),
            (Vec3::new(5., 6., 7.), Quat::IDENTITY),
        ];
        let lit = origin.point_with(|id| poses.get(id).copied(), Vec3::ONE);
        assert_eq!(lit, Vec3::new(5., 6., 7.));
    }
}
