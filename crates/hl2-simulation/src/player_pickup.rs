//! +USE carry of small physics props: CPlayerPickupController (SDK
//! weapon_physcannon.cpp), CHL2_Player::PickupObject / CBasePlayer::CanPickupObject
//! (35 kg, 128 units) and CBasePlayer::FindUseEntity's traces. SDK behavior, not
//! compared with retail; the hold shares the gravity gun's CGrabController port.
use crate::{
    entities::Scene,
    gameplay::{Inventory, Weapon},
    grab::{self, GrabController, DOT_30DEGREE, HOLD_MAX_ERROR, SCALE},
    physics::{is_projectile_entity, Physics},
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
    // A thrown frag (CBaseGrenade::Use -> PickupObject) or a prop_physics.
    if physics.no_pickup.contains(&id) {
        return false;
    }
    let grenade = is_projectile_entity(id) && physics.dynamic.contains_key(&id);
    if !grenade
        && !world
            .entities
            .get(id)
            .is_some_and(|e| e.class().starts_with("prop_physics"))
    {
        return false;
    }
    let size = grab::half_extents(physics, id) * 2.;
    grab::body_mass(physics, id).is_some_and(|m| m <= MAX_MASS) && size.max_element() <= MAX_SIZE
}

/// Distance from `point` to the infinite line through `start` along `direction`
/// (CalcDistanceToLine).
fn distance_to_line(point: Vec3, start: Vec3, direction: Vec3) -> f32 {
    let along = (point - start).dot(direction);
    (point - (start + direction * along)).length()
}

/// FindUseEntity for physics props: the 1024-unit eye line (returned at once), then
/// 16-unit hulls over 72 units at the down tangents (the last usable hit within
/// PLAYER_USE_RADIUS of the player, vertical distance measured to the hull's height
/// interval). Then the PLAYER_USE_RADIUS sphere for FCAP_USE_IN_RADIUS entities (thrown
/// grenades): within the 0.8 view cone, closer to the view line than the trace pick,
/// and not occluded. Traces use COLLISION_GROUP_NONE, so they hit grenades too.
pub fn find_use_prop(
    world: &World,
    physics: &Physics,
    eye: Vec3,
    forward: Vec3,
    up: Vec3,
    feet: Vec3,
    height: f32,
) -> Option<usize> {
    let mut nearest = None;
    for i in 0..=TANGENTS.len() {
        let hit = if i == 0 {
            physics.pick_ray(eye, forward, 1024.)
        } else {
            let down = (forward - up * TANGENTS[i - 1]).normalize_or_zero();
            physics.pick_hull(eye, down, 72., Vec3::splat(16.))
        };
        let Some(hit) = hit.filter(|hit| can_pickup(world, physics, hit.entity)) else {
            continue;
        };
        let mut delta = hit.position - eye;
        let below = feet.z - hit.position.z;
        let above = hit.position.z - (feet.z + height);
        delta.z = below.max(above).max(0.);
        if delta.length() < USE_RADIUS {
            if i == 0 {
                return Some(hit.entity);
            }
            nearest = Some(hit.entity);
        }
    }
    let closest = |id: usize| {
        physics
            .entity_closest_point(id, eye)
            .or(grab::center(physics, id))
    };
    let mut nearest_distance = nearest
        .and_then(closest)
        .map_or(f32::MAX, |point| distance_to_line(point, eye, forward));
    for &id in physics.dynamic.keys() {
        if !is_projectile_entity(id) || !can_pickup(world, physics, id) {
            continue;
        }
        let Some(point) = closest(id) else {
            continue;
        };
        if point.distance(eye) > USE_RADIUS || (point - eye).normalize_or_zero().dot(forward) < 0.8
        {
            continue;
        }
        let distance = distance_to_line(point, eye, forward);
        if distance >= nearest_distance {
            continue;
        }
        let to = point - eye;
        let length = to.length();
        if length < 1e-3
            || physics
                .pick_ray(eye, to / length, length)
                .is_none_or(|hit| hit.entity == id || hit.position.distance(point) < 0.1)
        {
            nearest = Some(id);
            nearest_distance = distance;
        }
    }
    nearest
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
            self.carry_shutdown(weapons, physics, scene, world, false);
            return true;
        }
        let right = forward.cross(Vec3::Z).normalize_or(Vec3::Y);
        let up = right.cross(forward).normalize_or(Vec3::Z);
        let height = if crouched { 36. } else { 72. };
        let Some(id) = find_use_prop(world, physics, eye, forward, up, feet, height) else {
            return false;
        };
        scene.prop_player_used(world, id);
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
        let Some(mut grab) = GrabController::attach(physics, id, forward, true, Some(DOT_30DEGREE))
        else {
            return true;
        };
        grab.carry_angles = crate::grab::CarryAngles::for_entity(world, id);
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
        scene.prop_physgun_pickup(world, id, crate::entities::PhysGunPickup::Player);
        true
    }
    /// CBasePlayer::PostThink -> CPlayerPickupController::Use(USE_SET) every tick:
    /// attack held throws (player_throwforce x mass factor), attack2 held or a hold
    /// error above 12 drops, otherwise UpdateObject.
    #[allow(clippy::too_many_arguments)]
    pub fn carry_tick(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        world: &World,
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
            self.carry_shutdown(weapons, physics, scene, world, false);
            return;
        }
        if primary {
            self.carry_shutdown(weapons, physics, scene, world, true);
            // OnPhysGunDrop(THROWN_BY_PLAYER): m_bThrownByPlayer.
            physics.thrown_by_player.insert(id);
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
            self.carry_shutdown(weapons, physics, scene, world, false);
        }
    }
    /// Shutdown: DetachEntity (clear the velocity when a dropped object touches
    /// something, else clamp it), then Deploy the holstered weapon again.
    fn carry_shutdown(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        physics: &mut Physics,
        scene: &mut Scene,
        world: &World,
        thrown: bool,
    ) {
        let Some(grab) = self.carry.take() else {
            return;
        };
        let touching = !thrown && physics.entity_in_contact(grab.entity);
        grab.detach(physics, touching);
        scene.prop_physgun_drop(world, grab.entity);
        self.redeploy(weapons, scene.time);
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
        // EnableSprint(false): hl2_normspeed while carrying.
        assert_eq!(inv.player_max_speed(), Some(190.));
        assert!(inv.viewmodel_effects(0.).sprites.is_empty());
        // Held: carried toward 24 + 2r in front of the eye, gravity off.
        for t in 0..60 {
            scene.time = f64::from(t) * 0.015;
            inv.carry_tick(
                &weapons,
                &world,
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
            &world,
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

    #[test]
    fn use_picks_up_a_resting_thrown_frag_until_it_detonates() {
        use crate::projectiles::{frag_key, ProjectileKind, ProjectileSpawn, Projectiles};
        // Synthetic solid written for this test (not game data).
        let mut world = World::default();
        world.model_physics.insert(
            frag_key(),
            modkit_core::PhysicsSolid {
                mass: 1.,
                ..Default::default()
            },
        );
        let weapons = pistol();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        physics.colliders.insert(
            ColliderBuilder::cuboid(2000. * SCALE, 2000. * SCALE, 8. * SCALE).translation(vector![
                0.,
                0.,
                -8. * SCALE
            ]),
        );
        physics.tick(0.015);
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                angles: None,
                kind: ProjectileKind::FragGrenade,
                position: Vec3::new(40., 0., 6.),
                velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                at: 0.,
                damage: 125.,
                radius: 250.,
                mass: 0.,
                lifetime: 3.,
            },
            &mut scene,
        );
        let step = |scene: &mut Scene, physics: &mut Physics, p: &mut Projectiles, t: u32| {
            scene.time = f64::from(t) * 0.015;
            p.tick(&world, scene, physics, Vec3::splat(9999.), false, 0.015);
            physics.tick(0.015);
        };
        for t in 0..40 {
            step(&mut scene, &mut physics, &mut projectiles, t);
        }
        let frag = crate::physics::projectile_entity(projectiles.active[0].id);
        assert!(can_pickup(&world, &physics, frag));
        // Ordinary traces (bullets, player movement) still skip the grenade.
        assert!(physics
            .impact_ray(Vec3::new(40., 0., 40.), -Vec3::Z, 60.)
            .is_none_or(|hit| hit.entity != frag));
        let mut inv = Inventory::default();
        inv.give("weapon_pistol", &weapons, 0.);
        // Looking down at the grenade from standing height (FindUseEntity's eye line).
        let eye = Vec3::Z * 64.;
        let forward = (Vec3::new(40., 0., 4.) - eye).normalize();
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
        assert_eq!(inv.carry.as_ref().unwrap().entity, frag);
        // Held: lifted off the floor in front of the eye; the fuse keeps running and
        // the detonation ends the carry (the entity is gone).
        for t in 40..120 {
            inv.carry_tick(
                &weapons,
                &world,
                &mut scene,
                &mut physics,
                eye,
                forward,
                Vec3::ZERO,
                false,
                false,
                false,
            );
            step(&mut scene, &mut physics, &mut projectiles, t);
        }
        assert!(
            projectiles.active[0].position.z > 20.,
            "{}",
            projectiles.active[0].position
        );
        for t in 120..230 {
            inv.carry_tick(
                &weapons,
                &world,
                &mut scene,
                &mut physics,
                eye,
                forward,
                Vec3::ZERO,
                false,
                false,
                false,
            );
            step(&mut scene, &mut physics, &mut projectiles, t);
        }
        assert!(projectiles.active.is_empty());
        inv.carry_tick(
            &weapons,
            &world,
            &mut scene,
            &mut physics,
            eye,
            forward,
            Vec3::ZERO,
            false,
            false,
            false,
        );
        assert!(!inv.carrying());
        assert!(!physics.dynamic.contains_key(&frag));
    }

    #[test]
    fn prevent_pickup_spawnflag_blocks_use_and_gravity_gun_pickup() {
        // SF_PHYSPROP_PREVENT_PICKUP (0x200) on entity 0 (synthetic).
        let world = world();
        let mut physics = prop(Vec3::new(60., 0., 50.), 5.);
        assert!(can_pickup(&world, &physics, 0));
        assert!(crate::weapon_physcannon::can_pickup(&physics, 0));
        physics.no_pickup.insert(0);
        assert!(!can_pickup(&world, &physics, 0));
        assert!(!crate::weapon_physcannon::can_pickup(&physics, 0));
    }
}
