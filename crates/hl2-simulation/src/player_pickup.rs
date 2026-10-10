//! +USE carry of small physics props: CPlayerPickupController (SDK
//! weapon_physcannon.cpp), CHL2_Player::PickupObject / CBasePlayer::CanPickupObject
//! (35 kg, 128 units) and CBasePlayer::FindUseEntity's traces. SDK behavior, not
//! compared with retail; the hold shares the gravity gun's CGrabController port.
use crate::{
    entities::Scene,
    gameplay::{Inventory, Weapon},
    grab::{self, GrabController, DOT_30DEGREE, HOLD_MAX_ERROR, SCALE},
    physics::Physics,
};
use glam::Vec3;
use modkit_core::World;
use rapier3d::prelude::*;

fn vector(v: Vec3) -> Vector<Real> {
    vector![v.x, v.y, v.z]
}
use std::collections::BTreeMap;

/// CHL2_Player::PickupObject -> CanPickupObject(pObject, 35, 128).
pub const MAX_MASS: f32 = 35.;
pub const MAX_SIZE: f32 = 128.;
/// player_throwforce (weapon_physcannon.cpp ConVar default).
pub const THROW_FORCE: f32 = 1000.;
/// PLAYER_USE_RADIUS (baseplayer_shared.h).
pub const USE_RADIUS: f32 = 80.;
/// FindUseEntity's down tangents after the forward line (45, 30, 20, 15, 10, -10, -15).
const TANGENTS: [f32; 7] = [
    1.,
    0.577_350_26,
    0.363_970_23,
    0.267_949_2,
    0.176_326_98,
    -0.176_326_98,
    -0.267_949_2,
];

/// CBasePlayer::CanPickupObject for our Rapier props: a dynamic (VPhysics, motion
/// enabled) prop_physics body of at most 35 kg whose collision box is at most 128 on
/// every axis (world AABB here; the SDK uses the OBB size). Hinges and
/// FVPHYSICS_NO_PLAYER_PICKUP are not modeled.
pub fn can_pickup(world: &World, physics: &Physics, id: usize) -> bool {
    if !world
        .entities
        .get(id)
        .is_some_and(|e| e.class().starts_with("prop_physics"))
    {
        return false;
    }
    let size = grab::half_extents(physics, id) * 2.;
    grab::body_mass(physics, id).is_some_and(|m| m <= MAX_MASS) && size.max_element() <= MAX_SIZE
}

/// FindUseEntity for physics props: the 1024-unit eye line, then 16-unit hulls over 72
/// units at the down tangents; the first hit usable prop within PLAYER_USE_RADIUS of
/// the player (vertical distance measured to the hull's height interval) wins.
pub fn find_use_prop(
    world: &World,
    physics: &Physics,
    eye: Vec3,
    forward: Vec3,
    up: Vec3,
    feet: Vec3,
    height: f32,
) -> Option<usize> {
    let mut traces = vec![physics.impact_ray(eye, forward, 1024.)];
    for tangent in TANGENTS {
        let down = (forward - up * tangent).normalize_or_zero();
        traces.push(physics.impact_hull(eye, down, 72., Vec3::splat(16.)));
    }
    traces.into_iter().flatten().find_map(|hit| {
        if !can_pickup(world, physics, hit.entity) {
            return None;
        }
        let mut delta = hit.position - eye;
        let below = feet.z - hit.position.z;
        let above = hit.position.z - (feet.z + height);
        delta.z = below.max(above).max(0.);
        (delta.length() < USE_RADIUS).then_some(hit.entity)
    })
}

impl Inventory {
    /// True while a prop is carried with +USE (the weapon is holstered).
    pub fn carrying(&self) -> bool {
        self.carry.is_some()
    }
    /// +USE press. While carrying: ClearUseEntity -> Use(USE_OFF) -> Shutdown (drop).
    /// Otherwise PickupObject on the prop FindUseEntity selects. Returns true when the
    /// press was consumed (doors and buttons are not tried then).
    #[allow(clippy::too_many_arguments)]
    pub fn player_use(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        eye: Vec3,
        forward: Vec3,
        feet: Vec3,
        crouched: bool,
    ) -> bool {
        if self.carry.is_some() {
            self.carry_shutdown(weapons, physics, scene.time, false);
            return true;
        }
        let right = forward.cross(Vec3::Z).normalize_or(Vec3::Y);
        let up = right.cross(forward).normalize_or(Vec3::Z);
        let height = if crouched { 36. } else { 72. };
        let Some(id) = find_use_prop(world, physics, eye, forward, up, feet, height) else {
            return false;
        };
        // Can't pick up what you're standing on.
        if physics
            .impact_ray(feet + Vec3::Z * 2., -Vec3::Z, 4.)
            .is_some_and(|hit| hit.entity == id)
        {
            return true;
        }
        // Init: the active weapon must holster (CanHolster; a gravity gun that holds
        // something refuses).
        if !self.can_holster() || self.physcannon.held.is_some() {
            return true;
        }
        let Some(grab) = GrabController::attach(physics, id, forward, true, Some(DOT_30DEGREE))
        else {
            return true;
        };
        self.holster_hooks(weapons, scene.time);
        // AttachEntity: the object's impact sound against the player as feedback
        // (PhysicsImpactSound volume 1, speed 64; the player material is "player").
        if let Some(name) = physics
            .prop_materials
            .get(&id)
            .and_then(|prop| world.surface_materials.get(prop))
            .and_then(|surface| {
                source_assets::surfaceprops::impact_sound(
                    surface,
                    world.surface_materials.get("player"),
                    64.,
                )
            })
        {
            scene.sounds.push(crate::sounds::SoundRequest {
                origin: grab::center(physics, id),
                volume: Some(1.),
                ..name.into()
            });
        }
        self.carry = Some(grab);
        true
    }
    /// CBasePlayer::PostThink -> CPlayerPickupController::Use(USE_SET) every tick:
    /// attack held throws (player_throwforce x mass factor), attack2 held or a hold
    /// error above 12 drops, otherwise UpdateObject.
    #[allow(clippy::too_many_arguments)]
    pub fn carry_tick(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        scene: &mut Scene,
        physics: &mut Physics,
        eye: Vec3,
        forward: Vec3,
        feet: Vec3,
        crouched: bool,
        primary: bool,
        secondary: bool,
    ) {
        let Some(grab) = self.carry.as_ref() else {
            return;
        };
        let id = grab.entity;
        if secondary || grab.error > HOLD_MAX_ERROR || !physics.dynamic.contains_key(&id) {
            self.carry_shutdown(weapons, physics, scene.time, false);
            return;
        }
        if primary {
            self.carry_shutdown(weapons, physics, scene.time, true);
            let Some(mass) = grab::body_mass(physics, id) else {
                return;
            };
            // massFactor = RemapVal(clamp(mass, 0.5, 15), 0.5, 15, 0.5, 4).
            let factor = 0.5 + (mass.clamp(0.5, 15.) - 0.5) / 14.5 * 3.5;
            let impulse = forward.normalize_or_zero() * THROW_FORCE * factor;
            // ApplyTorqueCenter(RandomAngularImpulse(-10, 10) x massFactor). The torque's
            // inertia scaling is inside vphysics.dll; approximated as deg/s per kg.
            let spin = Vec3::new(self.random(), self.random(), self.random()) * 10. * factor
                / mass.max(0.5);
            if let Some(body) = physics
                .dynamic
                .get(&id)
                .and_then(|h| physics.bodies.get_mut(*h))
            {
                let j = impulse * SCALE;
                body.apply_impulse(vector![j.x, j.y, j.z], true);
                let w = *body.angvel() + vector(spin * (std::f32::consts::PI / 180.));
                body.set_angvel(w, true);
            }
            return;
        }
        let keep = self
            .carry
            .as_mut()
            .is_some_and(|grab| grab.update(physics, eye, forward, feet, crouched, HOLD_MAX_ERROR));
        if !keep {
            self.carry_shutdown(weapons, physics, scene.time, false);
        }
    }
    /// Shutdown: DetachEntity (clear the velocity when a dropped object touches
    /// something, else clamp it), then Deploy the holstered weapon again.
    fn carry_shutdown(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        physics: &mut Physics,
        time: f64,
        thrown: bool,
    ) {
        let Some(grab) = self.carry.take() else {
            return;
        };
        let touching = !thrown && physics.entity_in_contact(grab.entity);
        grab.detach(physics, touching);
        self.redeploy(weapons, time);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        World {
            entities: vec![modkit_core::Entity {
                properties: vec![("classname".into(), "prop_physics".into())],
            }],
            ..Default::default()
        }
    }
    /// A floating 16-unit cube prop (entity 0) of `mass` kg at `at` (synthetic).
    fn prop(at: Vec3, mass: f32) -> Physics {
        let mut physics = Physics::default();
        let body = physics.bodies.insert(
            RigidBodyBuilder::dynamic()
                .gravity_scale(0.)
                .translation(vector(at * SCALE)),
        );
        let (bodies, colliders) = (&mut physics.bodies, &mut physics.colliders);
        colliders.insert_with_parent(
            ColliderBuilder::cuboid(8. * SCALE, 8. * SCALE, 8. * SCALE)
                .mass(mass)
                .user_data(1),
            body,
            bodies,
        );
        physics.dynamic.insert(0, body);
        physics.tick(0.015);
        physics
    }
    fn pistol() -> BTreeMap<String, Weapon> {
        BTreeMap::from([(
            "weapon_pistol".to_owned(),
            Weapon {
                class: "weapon_pistol".into(),
                magazine: 18,
                default_clip: 18,
                ..Default::default()
            },
        )])
    }

    #[test]
    fn use_picks_up_a_small_prop_holds_it_and_throws_with_the_mass_factor() {
        let world = world();
        let weapons = pistol();
        let mut scene = Scene::new(&world);
        let mut physics = prop(Vec3::new(60., 0., 50.), 5.);
        let mut inv = Inventory::default();
        inv.give("weapon_pistol", &weapons, 0.);
        let eye = Vec3::Z * 64.;
        let forward = Vec3::new(1., 0., -0.2).normalize();
        assert!(inv.player_use(
            &weapons,
            &world,
            &mut scene,
            &mut physics,
            eye,
            forward,
            Vec3::ZERO,
            false
        ));
        assert!(inv.carrying());
        // Held: carried toward 24 + 2r in front of the eye, gravity off.
        for t in 0..60 {
            scene.time = f64::from(t) * 0.015;
            inv.carry_tick(
                &weapons,
                &mut scene,
                &mut physics,
                eye,
                forward,
                Vec3::ZERO,
                false,
                false,
                false,
            );
            physics.tick(0.015);
        }
        let held = grab::center(&physics, 0).unwrap();
        assert!(held.x > 40. && held.x < 90., "{held}");
        assert!(inv.carrying());
        // Attack throws: 1000 x RemapVal(5, 0.5, 15, 0.5, 4) / 5 kg along the aim.
        inv.carry_tick(
            &weapons,
            &mut scene,
            &mut physics,
            eye,
            forward,
            Vec3::ZERO,
            false,
            true,
            false,
        );
        assert!(!inv.carrying());
        let factor = 0.5 + 4.5 / 14.5 * 3.5;
        let body = &physics.bodies[physics.dynamic[&0]];
        let v = Vec3::new(body.linvel().x, body.linvel().y, body.linvel().z) / SCALE;
        let throw = 1000. * factor / 5.;
        assert!(v.dot(forward) > throw - 10., "{v} vs {throw}");
        assert!(v.dot(forward) < throw + 300., "{v} vs {throw}");
        // The pistol was redeployed.
        assert_eq!(inv.active, "weapon_pistol");
        assert_eq!(inv.animation, "draw");
    }

    #[test]
    fn heavy_or_large_props_are_not_carried_and_use_again_drops() {
        let world = world();
        let weapons = pistol();
        let mut scene = Scene::new(&world);
        let eye = Vec3::Z * 64.;
        let mut heavy = prop(Vec3::new(50., 0., 64.), 40.);
        let mut inv = Inventory::default();
        assert!(!inv.player_use(
            &weapons,
            &world,
            &mut scene,
            &mut heavy,
            eye,
            Vec3::X,
            Vec3::ZERO,
            false
        ));
        assert!(!inv.carrying());
        // Out of PLAYER_USE_RADIUS: not found.
        let mut far = prop(Vec3::new(200., 0., 64.), 5.);
        assert!(!inv.player_use(
            &weapons,
            &world,
            &mut scene,
            &mut far,
            eye,
            Vec3::X,
            Vec3::ZERO,
            false
        ));
        let mut light = prop(Vec3::new(50., 0., 64.), 5.);
        assert!(inv.player_use(
            &weapons,
            &world,
            &mut scene,
            &mut light,
            eye,
            Vec3::X,
            Vec3::ZERO,
            false
        ));
        assert!(inv.carrying());
        // +USE again: Use(USE_OFF) -> Shutdown, gravity restored.
        assert!(inv.player_use(
            &weapons,
            &world,
            &mut scene,
            &mut light,
            eye,
            Vec3::X,
            Vec3::ZERO,
            false
        ));
        assert!(!inv.carrying());
        assert_eq!(light.bodies[light.dynamic[&0]].gravity_scale(), 0.);
    }
}
