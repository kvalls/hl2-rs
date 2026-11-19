//! func_physbox: a brush entity simulated as one VPhysics object.

use super::{
    authored_mass_properties, authored_solid, brush_shape, restitution, surface_material, Physics,
    DEBRIS_GROUPS, MASK_SOLID,
};
use modkit_core::{BrushModel, Entity, World};
use rapier3d::parry::bounding_volume::BoundingVolume;
use rapier3d::prelude::{
    ActiveHooks, CoefficientCombineRule, ColliderBuilder, Isometry, Real, RigidBodyBuilder,
    SharedShape,
};

/// SDK physobj.h spawnflags.
const ASLEEP: u32 = 0x1000;
const DEBRIS: u32 = 0x4000;
const MOTION_DISABLED: u32 = 0x8000;
const NEVER_PICK_UP: u32 = 0x20_0000;

fn number(e: &Entity, key: &str) -> f32 {
    e.get(key)
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.)
}

/// CPhysBox::CreateVPhysics leaves the object's motion enabled unless
/// SF_PHYSBOX_MOTIONDISABLED is set or damage or a force is needed to enable it.
fn moves(e: &Entity, flags: u32) -> bool {
    flags & MOTION_DISABLED == 0
        && number(e, "damagetoenablemotion") <= 0.
        && number(e, "forcetoenablemotion") <= 0.
}

/// ShouldDampRotation (physobj.cpp): a solid whose longest bounding-box side is more
/// than 9 times the length of the other two sides taken as a vector is long and thin;
/// CreateVPhysics raises its rotdamping to 1 so it does not roll endlessly.
fn long_and_thin(shapes: &[SharedShape]) -> bool {
    let Some(aabb) = shapes
        .iter()
        .map(|s| s.compute_local_aabb())
        .reduce(|a, b| a.merged(&b))
    else {
        return false;
    };
    let mut size = [0usize, 1, 2].map(|i| aabb.maxs[i] - aabb.mins[i]);
    let largest = (0..3).fold(0, |best, i| if size[i] > size[best] { i } else { best });
    let long = size[largest];
    size[largest] = 0.;
    let rest = (size[0] * size[0] + size[1] * size[1] + size[2] * size[2]).sqrt();
    rest > 0. && long / rest > 9.
}

impl Physics {
    /// func_physbox (SDK physobj.cpp CPhysBox::Spawn and CreateVPhysics): one dynamic
    /// body at the entity's pose made from its brush model's solid brushes, with the
    /// compiled solid's parameters from the map's PHYSCOLLIDE text (mass times
    /// massScale, damping, surfaceprop), awake unless SF_PHYSBOX_ASLEEP, in the debris
    /// group under SF_PHYSBOX_DEBRIS. Returns false, leaving the brushes static, when
    /// motion starts disabled or the map has no solid for the model. Not modeled: the
    /// EnableMotion/DisableMotion inputs, damage or force enabling motion, the mass
    /// center override, overridescript, FVPHYSICS_DMG_SLICE, the physcannon flags other
    /// than SF_PHYSBOX_NEVER_PICK_UP.
    pub(super) fn add_physbox(
        &mut self,
        world: &World,
        id: usize,
        e: &Entity,
        model: &BrushModel,
        position: Isometry<Real>,
    ) -> bool {
        let flags = e
            .get("spawnflags")
            .and_then(|f| f.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let Some(compiled) = world.brush_model_physics.get(&model.id) else {
            return false;
        };
        if e.class() != "func_physbox" || !moves(e, flags) {
            return false;
        }
        let pieces: Vec<_> = model
            .brushes
            .iter()
            .filter(|b| b.contents & MASK_SOLID != 0)
            .filter_map(|b| Some((brush_shape(b)?, b.contents)))
            .collect();
        if pieces.is_empty() {
            return false;
        }
        let shapes: Vec<_> = pieces.iter().map(|(s, _)| s.clone()).collect();
        let mut solid = authored_solid(compiled, |key| {
            e.get(key).and_then(|v| v.trim().parse::<f32>().ok())
        });
        if solid.rotdamping < 1. && long_and_thin(&shapes) {
            solid.rotdamping = 1.;
        }
        let body = self.bodies.insert(
            RigidBodyBuilder::dynamic()
                .additional_mass_properties(authored_mass_properties(
                    &shapes,
                    solid.mass,
                    solid.inertia,
                ))
                .position(position)
                .sleeping(flags & ASLEEP != 0)
                .linear_damping(solid.damping)
                .angular_damping(solid.rotdamping),
        );
        let material = surface_material(world, &solid.surfaceprop);
        for (shape, contents) in pieces {
            let builder = match material {
                Some(m) => ColliderBuilder::new(shape)
                    .friction(m.friction)
                    .restitution(restitution(m))
                    .friction_combine_rule(CoefficientCombineRule::Multiply)
                    .restitution_combine_rule(CoefficientCombineRule::Multiply),
                None => ColliderBuilder::new(shape).friction(0.8),
            }
            .user_data(id as u128 + 1)
            // No contact skin: vbsp builds every non-world brush model's solid shrunk by
            // VPHYSICS_SHRINK (0.5) so that VPhysics's separation radius puts its surface
            // back at the brush faces (SDK utils/vbsp/ivp.cpp); the prop margin
            // (VPHYSICS_CONTACT_MARGIN) is for .phy hulls, which are not shrunk.
            .density(0.)
            .active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS);
            let c = self
                .colliders
                .insert_with_parent(builder, body, &mut self.bodies);
            if flags & DEBRIS != 0 {
                self.colliders[c].set_collision_groups(DEBRIS_GROUPS);
            }
            self.entity_colliders.entry(id).or_default().push(c);
            self.npc_collider_contents.insert(c, contents);
            self.collider_materials.insert(c, solid.surfaceprop.clone());
        }
        // The compiled solid's stored drag fractions (zeros from vbsp: no air drag).
        if let Some(fractions) = solid.drag_fractions {
            self.drag_fractions.insert(body, fractions);
        }
        self.prop_materials.insert(id, solid.surfaceprop);
        self.ccd_gated.insert(body, 0.);
        self.dynamic.insert(id, body);
        if flags & NEVER_PICK_UP != 0 {
            self.no_pickup.insert(id);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rapier3d::prelude::*;

    #[test]
    fn only_a_long_thin_solid_damps_rotation() {
        let bar = SharedShape::cuboid(10., 0.5, 0.5);
        let cube = SharedShape::cuboid(1., 1., 1.);
        assert!(long_and_thin(&[bar]));
        assert!(!long_and_thin(std::slice::from_ref(&cube)));
        assert!(!long_and_thin(&[]));
        assert!(!long_and_thin(&[cube]));
    }

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// A turnstile-like physbox: one bar from the vertical hinge axis at x = 0 to x = 30,
    /// 35 kg of metal (synthetic values shaped like d1_trainstation_01's turnstyle_1).
    fn turnstile(spawnflags: &str, lump: bool) -> World {
        use glam::Vec3;
        let mut world = World {
            entities: vec![
                entity(&[
                    ("classname", "func_physbox"),
                    ("targetname", "turn"),
                    ("model", "*1"),
                    ("origin", "0 0 50"),
                    ("spawnflags", spawnflags),
                ]),
                entity(&[
                    ("classname", "phys_hinge"),
                    ("attach1", "turn"),
                    ("origin", "0 0 60"),
                    ("hingeaxis", "0 0 20"),
                    ("spawnflags", "9"),
                ]),
            ],
            brush_models: vec![BrushModel {
                id: 1,
                surfaces: Vec::new(),
                brushes: vec![super::super::tests::bounds_brush(
                    Vec3::new(-2., -2., -10.),
                    Vec3::new(30., 2., 10.),
                )],
                mins: Vec3::new(-2., -2., -10.),
                maxs: Vec3::new(30., 2., 10.),
            }],
            ..Default::default()
        };
        if lump {
            world.brush_model_physics.insert(
                1,
                modkit_core::PhysicsSolid {
                    mass: 35.188103,
                    surfaceprop: "metal".into(),
                    ..Default::default()
                },
            );
        }
        world
    }

    #[test]
    fn a_hinged_physbox_turns_about_its_axis_when_pushed() {
        use glam::Vec3;
        let mut physics = Physics::new(&turnstile("0", true));
        assert_eq!(
            (physics.constraints, physics.constraint_errors.len()),
            (1, 0)
        );
        let body = physics.dynamic[&0];
        assert!((physics.bodies[body].mass() - 35.188103).abs() < 0.01);
        // A push at the bar's end, across it.
        physics.impulse_at(0, Vec3::new(0., 4., 0.), Vec3::new(28., 0., 50.));
        for _ in 0..30 {
            physics.tick(1. / 66.);
        }
        let (origin, rotation) = physics.entity_pose(0).unwrap();
        let bar = rotation * Vec3::X;
        let yaw = bar.y.atan2(bar.x).to_degrees();
        assert!((20. ..90.).contains(&yaw), "turned {yaw} degrees");
        assert!(bar.z.abs() < 0.02, "tilted: {bar}");
        assert!(origin.distance(Vec3::new(0., 0., 50.)) < 0.5, "{origin}");
    }

    #[test]
    fn a_physbox_without_motion_or_a_compiled_solid_stays_static() {
        for world in [turnstile("32768", true), turnstile("0", false)] {
            let physics = Physics::new(&world);
            assert!(!physics.dynamic.contains_key(&0));
            assert_eq!(physics.entity_colliders[&0].len(), 1);
        }
    }

    #[test]
    #[ignore = "requires an installed owned HL2 copy"]
    fn owned_turnstile_spins_freely_inside_its_frame() {
        // d1_trainstation_01: the frame (func_lod lod_turnstyle_frame) has hub pieces 3.0
        // units from the hinge axis between the arm levels; the 4x4 post's corners reach
        // 2.83. With the prop contact margin the post ground on them and the bars bounced
        // back within 15 degrees.
        let game = source_assets::install::discover().unwrap();
        let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
        let world = source_assets::bsp::Bsp::parse(&data)
            .unwrap()
            .world("d1_trainstation_01")
            .unwrap();
        let mut physics = Physics::new(&world);
        let id = world
            .entities
            .iter()
            .position(|e| e.get("targetname") == Some("turnstyle_1"))
            .unwrap();
        let body = physics.dynamic[&id];
        assert!((physics.bodies[body].mass() - 35.188103).abs() < 0.01);
        physics.bodies[body].set_angvel(vector![0., 0., 2.], true);
        for _ in 0..100 {
            physics.tick(1. / 66.);
        }
        let b = &physics.bodies[body];
        // Only the hinge and rotdamping 0.1 act on it (no air drag: its stored drag
        // fractions are zero): about 1.5 s at nearly 2 rad/s.
        assert!(b.angvel().z > 1.5, "{:?}", b.angvel());
        let at = b.translation() / super::super::SCALE;
        assert!((at - vector![-4086., -1452., 12.]).norm() < 0.5, "{at:?}");
    }

    #[test]
    fn motion_needs_no_flag_damage_or_force() {
        let e = entity;
        assert!(moves(&e(&[]), 0));
        assert!(!moves(&e(&[]), MOTION_DISABLED));
        assert!(!moves(&e(&[("damagetoenablemotion", "5")]), 0));
        assert!(!moves(&e(&[("forcetoenablemotion", "100")]), 0));
    }
}
