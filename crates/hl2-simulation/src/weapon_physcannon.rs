//! weapon_physcannon (gravity gun) from the singleplayer SDK (weapon_physcannon.cpp:
//! CWeaponPhysCannon, CGrabController; player_pickup.cpp launch velocity) acting on the
//! existing Rapier prop bodies. The mega cannon, ragdolls and NPC grabs are out of scope.
//! SDK behavior; VPhysics' shadow controller lives in vphysics.dll (not in the SDK), so
//! the hold uses a velocity-to-target approximation ("native fit needed").
use crate::{
    entities::Scene,
    gameplay::{activity_sequence, duration, play_sound, Inventory, Weapon},
    grab::{body_mass, center, GrabController, HOLD_MAX_ERROR, SCALE},
    physics::Physics,
};
use glam::Vec3;
use modkit_core::World;
use rapier3d::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;

/// physcannon_* and player cvar defaults (SDK ConVars; skill.cfg does not set them).
pub const MIN_FORCE: f32 = 700.;
pub const MAX_FORCE: f32 = 1500.;
pub const MAX_MASS: f32 = 250.;
pub const TRACE_LENGTH: f32 = 250.;
pub const PULL_FORCE: f32 = 4000.;
pub const CONE: f32 = 0.97;

/// What the viewmodel/elements are doing (EFFECT_* and OpenElements/CloseElements).
#[derive(Clone, Debug, Default, Serialize)]
pub struct PhyscannonState {
    /// m_bActive with the held entity id.
    pub held: Option<usize>,
    /// m_bOpen and the "active" pose parameter (UTIL_Approach 0.1 per frame).
    pub open: bool,
    pub element_position: f32,
    element_destination: f32,
    next_primary: f64,
    next_secondary: f64,
    idle_at: f64,
    check_suppress: f64,
    element_debounce: f64,
    change_state: ElementChange,
    attack2_debounce: bool,
    deny_played: bool,
    /// IN_ATTACK2 last frame (m_afButtonPressed).
    secondary_held: bool,
    /// The tick's buttons from Inventory::tick.
    buttons: (bool, bool),
    grab: Option<GrabController>,
    /// Launch/punt effect positions this frame (EFFECT_LAUNCH), host-drawn.
    pub launches: Vec<Vec3>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
enum ElementChange {
    #[default]
    None,
    Open,
    Closed,
}
/// Pickup_DefaultPhysGunLaunchVelocity: max force up to 100 kg, then a spline down to
/// the min force at 600 kg (masses capped at 1000).
pub fn launch_speed(mass: f32) -> f32 {
    if mass <= 100. {
        return MAX_FORCE;
    }
    let mass = mass.min(1000.);
    let t = ((mass - 100.) / 500.).clamp(0., 1.);
    MAX_FORCE + (MIN_FORCE - MAX_FORCE) * modkit_core::animation::simple_spline(t)
}
/// PuntVPhysics for one body: ApplyForceCenter(forward x 15000) and
/// ApplyForceOffset(forward x min(mass, 250) x 600) at the hit point (both impulses in
/// kg in/s). Downward punts reflect with z x -0.65. Returns (linear dv, offset impulse).
pub fn punt_impulse(forward: Vec3, mass: f32) -> (Vec3, Vec3) {
    let mut forward = forward;
    if forward.z < 0. {
        forward.z *= -0.65;
    }
    (
        forward * 15000. / mass.max(1e-3),
        forward * mass.min(MAX_MASS) * 600.,
    )
}

/// CBasePlayer::CanPickupObject with physcannon_maxmass for our Rapier props: a dynamic
/// body (VPhysics object) of at most 250 kg. Hinges, NO_PLAYER_PICKUP and the
/// prop spawnflag overrides are not modeled.
pub fn can_pickup(physics: &Physics, id: usize) -> bool {
    body_mass(physics, id).is_some_and(|m| m <= MAX_MASS)
}

impl Inventory {
    pub(crate) fn physcannon_deploy(&mut self) {
        let held = self.physcannon.held;
        self.physcannon = PhyscannonState {
            held,
            ..Default::default()
        };
    }
    /// CWeaponPhysCannon::CanHolster: not while holding.
    pub fn physcannon_can_holster(&self) -> bool {
        !(self.active == "weapon_physcannon" && self.physcannon.held.is_some())
    }
    pub(crate) fn physcannon_buttons(&mut self, primary: bool, secondary: bool) {
        self.physcannon.buttons = (primary, secondary);
    }
    fn cannon_send(&mut self, world: &World, weapon: &Weapon, activity: &str, time: f64) {
        let sequence = activity_sequence(world, weapon, activity);
        self.physcannon.idle_at = time + duration(world, weapon, &sequence);
        self.animate(&sequence, time);
    }
    /// The gravity gun's frame (ItemPreFrame then ItemPostFrame/ItemBusyFrame); call in
    /// the weapons stage with the eye, aim, player feet and crouch. Physics changes
    /// (punt/pull impulses, the hold velocity) take effect in the rigid-body step.
    #[allow(clippy::too_many_arguments)]
    pub fn physcannon_frame(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        eye: Vec3,
        forward: Vec3,
        feet: Vec3,
        crouched: bool,
    ) {
        self.physcannon.launches.clear();
        if self.active != "weapon_physcannon" || self.health <= 0. {
            return;
        }
        let Some(weapon) = weapons.get("weapon_physcannon").cloned() else {
            return;
        };
        let time = scene.time;
        let forward = forward.normalize_or_zero();
        let s = &mut self.physcannon;
        // ItemPreFrame: elements approach their destination; update a held object.
        s.element_position += (s.element_destination - s.element_position).clamp(-0.1, 0.1);
        if s.held.is_some() && !self.update_held(physics, eye, forward, feet, crouched) {
            self.detach(physics, scene, &weapon, world, true, false);
        }
        if time < self.owner_attack_until {
            return;
        }
        let (primary, secondary) = self.physcannon.buttons;
        let pressed2 = secondary && !self.physcannon.secondary_held;
        self.physcannon.secondary_held = secondary;
        // ItemPostFrame.
        if self.physcannon.held.is_none() {
            self.check_for_target(physics, eye, forward, time);
            let s = &mut self.physcannon;
            if s.element_debounce < time && s.change_state != ElementChange::None {
                match s.change_state {
                    ElementChange::Open => self.open_elements(scene, &weapon, world, time),
                    _ => self.close_elements(scene, &weapon, world, time),
                }
                self.physcannon.change_state = ElementChange::None;
            }
        }
        if secondary && !self.physcannon.attack2_debounce {
            self.cannon_secondary(weapons, world, scene, physics, eye, forward, pressed2);
        } else {
            self.physcannon.deny_played = false;
        }
        if !secondary {
            self.physcannon.attack2_debounce = false;
        }
        if primary {
            self.cannon_primary(&weapon, world, scene, physics, eye, forward, feet);
        } else if time > self.physcannon.idle_at {
            // WeaponIdle: shake (ACT_VM_RELOAD) while holding, else idle.
            let activity = if self.physcannon.held.is_some() {
                "ACT_VM_RELOAD"
            } else {
                "ACT_VM_IDLE"
            };
            self.cannon_send(world, &weapon, activity, time);
        }
    }
    fn open_elements(&mut self, scene: &mut Scene, weapon: &Weapon, world: &World, time: f64) {
        if self.physcannon.open {
            return;
        }
        play_sound(scene, weapon, "special2", world, "");
        self.physcannon.element_position = self.physcannon.element_position.max(0.);
        self.physcannon.element_destination = 1.;
        self.cannon_send(world, weapon, "ACT_VM_IDLE", time);
        self.physcannon.open = true;
    }
    fn close_elements(&mut self, scene: &mut Scene, weapon: &Weapon, world: &World, time: f64) {
        if !self.physcannon.open {
            return;
        }
        play_sound(scene, weapon, "melee_hit", world, "");
        self.physcannon.element_position = self.physcannon.element_position.min(1.);
        self.physcannon.element_destination = 0.;
        self.cannon_send(world, weapon, "ACT_VM_IDLE", time);
        self.physcannon.open = false;
    }
    /// FindObjectTrace: a line to TraceLength x 4, then a 4-unit hull when it hit no
    /// entity. Returns (entity or world, fraction of the 1000 units, end).
    fn find_trace(physics: &Physics, eye: Vec3, forward: Vec3) -> Option<(usize, f32, Vec3)> {
        let length = TRACE_LENGTH * 4.;
        let line = physics.impact_ray(eye, forward, length);
        let hit = match line {
            Some(hit) if hit.entity != usize::MAX => Some(hit),
            _ => physics
                .impact_hull(eye, forward, length, Vec3::splat(4.))
                .or(line),
        }?;
        Some((
            hit.entity,
            eye.distance(hit.position) / length,
            hit.position,
        ))
    }
    /// CheckForTarget: open the elements while aiming at a pickable object in range.
    fn check_for_target(&mut self, physics: &Physics, eye: Vec3, forward: Vec3, time: f64) {
        let s = &mut self.physcannon;
        if s.check_suppress > time || s.held.is_some() {
            return;
        }
        if let Some((entity, fraction, _)) = Self::find_trace(physics, eye, forward) {
            if fraction * TRACE_LENGTH * 4. <= TRACE_LENGTH && can_pickup(physics, entity) {
                s.change_state = ElementChange::None;
                s.change_state = ElementChange::Open;
                s.element_debounce = time.min(s.element_debounce);
                return;
            }
        }
        if s.element_debounce < time && s.change_state == ElementChange::None {
            s.change_state = ElementChange::Closed;
            s.element_debounce = time + 0.5;
        }
    }
    /// FindObjectInCone: the nearest dynamic body within 251 units whose center is
    /// inside the 0.97 cone and visible.
    fn find_in_cone(physics: &Physics, eye: Vec3, forward: Vec3) -> Option<usize> {
        let mut nearest = TRACE_LENGTH + 1.;
        let mut best = None;
        for &id in physics.dynamic.keys() {
            let Some(c) = center(physics, id) else {
                continue;
            };
            let los = c - eye;
            let distance = los.length();
            if distance >= nearest || los.normalize_or_zero().dot(forward) <= CONE {
                continue;
            }
            if physics
                .impact_ray(eye, los / distance.max(1e-6), distance + 1.)
                .is_some_and(|hit| hit.entity == id)
            {
                nearest = distance;
                best = Some(id);
            }
        }
        best
    }
    /// SecondaryAttack: drop on the press while holding; otherwise FindObject (attach
    /// within 250 units, pull farther objects, deny sound for unpickable ones).
    #[allow(clippy::too_many_arguments)]
    fn cannon_secondary(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        eye: Vec3,
        forward: Vec3,
        pressed: bool,
    ) {
        let time = scene.time;
        let weapon = weapons["weapon_physcannon"].clone();
        if self.physcannon.next_secondary > time {
            return;
        }
        if self.physcannon.held.is_some() && pressed {
            self.physcannon.next_primary = time + 0.5;
            self.physcannon.next_secondary = time + 0.5;
            self.detach(physics, scene, &weapon, world, true, false);
            self.cannon_send(world, &weapon, "ACT_VM_PRIMARYATTACK", time);
            return;
        }
        if self.physcannon.held.is_some() {
            return;
        }
        let trace = Self::find_trace(physics, eye, forward);
        let (mut entity, mut attach, mut pull) = (None, false, false);
        if let Some((id, fraction, _)) = trace.filter(|(id, _, _)| *id != usize::MAX) {
            entity = Some(id);
            attach = fraction <= 0.25;
            pull = !attach;
        }
        if !attach && !pull {
            if let Some(id) = Self::find_in_cone(physics, eye, forward) {
                entity = Some(id);
                // Within TraceLength x 4 of the eye (always true inside the cone search).
                attach = true;
            }
        }
        let Some(id) = entity.filter(|id| can_pickup(physics, *id)) else {
            if entity.is_some() || attach || pull {
                // OBJECT_NOT_FOUND with the deny sound once per press.
                if !self.physcannon.deny_played {
                    self.physcannon.deny_played = true;
                    play_sound(scene, &weapon, "special3", world, "");
                }
            }
            self.physcannon.next_secondary = time + 0.1;
            self.close_elements(scene, &weapon, world, time);
            return;
        };
        if attach {
            self.attach(physics, scene, &weapon, world, id, forward);
            play_sound(scene, &weapon, "special1", world, "");
            self.cannon_send(world, &weapon, "ACT_VM_PRIMARYATTACK", time);
            self.physcannon.next_secondary = time + 0.5;
            self.physcannon.attack2_debounce = true;
        } else {
            // Pull: ApplyForceCenter(pullDir x 4000, scaled by (mass + 0.5) / 50 under
            // 50 kg), an impulse in kg in/s.
            if let (Some(c), Some(mass)) = (center(physics, id), body_mass(physics, id)) {
                let mut pull = (eye - c).normalize_or_zero() * PULL_FORCE;
                if mass < 50. {
                    pull *= (mass + 0.5) / 50.;
                }
                if let Some(body) = physics.bodies.get_mut(physics.dynamic[&id]) {
                    let j = pull * SCALE;
                    body.apply_impulse(vector![j.x, j.y, j.z], true);
                }
            }
            self.physcannon.next_secondary = time + 0.1;
            self.close_elements(scene, &weapon, world, time);
        }
    }
    /// AttachObject + CGrabController::AttachEntity: mass 1 while held (saved), no
    /// gravity (shadow control), the eye-space rotation kept.
    #[allow(clippy::too_many_arguments)]
    fn attach(
        &mut self,
        physics: &mut Physics,
        scene: &mut Scene,
        weapon: &Weapon,
        world: &World,
        id: usize,
        forward: Vec3,
    ) {
        let time = scene.time;
        let Some(grab) = GrabController::attach(physics, id, forward, false, None) else {
            return;
        };
        self.physcannon.held = Some(id);
        self.physcannon.grab = Some(grab);
        self.physcannon.next_secondary = time + 0.4;
        self.open_elements(scene, weapon, world, time);
    }
    /// DetachObject + DetachEntity: restore gravity, clear (launch) or clamp (drop) the
    /// velocity; MELEE_MISS on a plain drop.
    fn detach(
        &mut self,
        physics: &mut Physics,
        scene: &mut Scene,
        weapon: &Weapon,
        world: &World,
        sound: bool,
        launched: bool,
    ) {
        if self.physcannon.held.take().is_none() {
            return;
        }
        if let Some(grab) = self.physcannon.grab.take() {
            grab.detach(physics, launched);
        }
        if sound {
            play_sound(scene, weapon, "melee_miss", world, "");
        }
    }
    /// CGrabController::UpdateObject + Simulate: the target in front of the eye (pitch
    /// clamped to +/-75, radius from the player and object extents, pulled in by a
    /// brush trace, pushed off the player's axis), velocity toward it (max 1000 u/s,
    /// 3600 deg/s), and the error accumulator that drops it past 12 units.
    fn update_held(
        &mut self,
        physics: &mut Physics,
        eye: Vec3,
        forward: Vec3,
        feet: Vec3,
        crouched: bool,
    ) -> bool {
        let Some(grab) = self.physcannon.grab.as_mut() else {
            return false;
        };
        grab.update(physics, eye, forward, feet, crouched, HOLD_MAX_ERROR)
    }
    /// PrimaryAttack: launch a held object (within 250 units of the player center) at
    /// Pickup_DefaultPhysGunLaunchVelocity, or punt what the 8-unit hull/line trace hits.
    #[allow(clippy::too_many_arguments)]
    fn cannon_primary(
        &mut self,
        weapon: &Weapon,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        eye: Vec3,
        forward: Vec3,
        feet: Vec3,
    ) {
        let time = scene.time;
        if self.physcannon.next_primary > time {
            return;
        }
        if let Some(id) = self.physcannon.held {
            let held_center = center(physics, id).unwrap_or(eye);
            if held_center.distance(feet + Vec3::Z * 36.) > TRACE_LENGTH {
                self.dry_fire(scene, weapon, world, time);
                return;
            }
            let mass = self.physcannon.grab.as_ref().map_or(1., |g| g.saved_mass);
            self.detach(physics, scene, weapon, world, false, true);
            if let Some(body) = physics
                .dynamic
                .get(&id)
                .and_then(|h| physics.bodies.get_mut(*h))
            {
                // AddVelocity(forward x force); the random launch spin is not modeled.
                let v = forward * launch_speed(mass) * SCALE;
                body.set_linvel(*body.linvel() + vector![v.x, v.y, v.z], true);
            }
            self.physcannon.next_primary = time + 0.5;
            self.physcannon.next_secondary = time + 0.5;
            self.physcannon.launches.push(held_center);
            self.physcannon.change_state = ElementChange::Closed;
            self.physcannon.element_debounce = time + 0.1;
            self.physcannon.check_suppress = time + 0.25;
            self.primary_fire_effect(scene, weapon, world);
            self.cannon_send(world, weapon, "ACT_VM_SECONDARYATTACK", time);
            return;
        }
        self.physcannon.next_primary = time + 0.5;
        let hit = physics
            .impact_hull(eye, forward, TRACE_LENGTH, Vec3::splat(8.))
            .filter(|h| h.entity != usize::MAX && physics.dynamic.contains_key(&h.entity))
            .or_else(|| physics.impact_ray(eye, forward, TRACE_LENGTH))
            .filter(|h| h.entity != usize::MAX);
        let Some(hit) = hit else {
            self.dry_fire(scene, weapon, world, time);
            return;
        };
        let id = hit.entity;
        if let Some(handle) = physics.dynamic.get(&id).copied() {
            // PuntVPhysics (single-object props: ratio 1).
            let mass = body_mass(physics, id).unwrap_or(1.);
            let (dv, offset) = punt_impulse(forward, mass);
            if let Some(body) = physics.bodies.get_mut(handle) {
                let j = dv * mass * SCALE;
                body.apply_impulse(vector![j.x, j.y, j.z], true);
                let k = offset * SCALE;
                let p = hit.position * SCALE;
                body.apply_impulse_at_point(vector![k.x, k.y, k.z], point![p.x, p.y, p.z], true);
            }
        } else {
            let class = world.entities.get(id).map_or("", |e| e.class());
            if !matches!(class, "func_breakable" | "func_physbox") && !class.starts_with("prop_") {
                // Non-VPhysics without damage, or NPCs (headcrab/antlion ragdoll punts
                // are out of scope): dry fire.
                self.dry_fire(scene, weapon, world, time);
                return;
            }
            // PuntNonVPhysics: 1 damage (glass 15 is not distinguished), DMG_CRUSH.
            scene.entity_damage.push(crate::projectiles::Damage {
                target: crate::projectiles::DamageTarget::Entity(id),
                amount: 1.,
                dissolve: false,
                direction: forward,
                origin: hit.position,
            });
        }
        self.physcannon.launches.push(hit.position);
        self.primary_fire_effect(scene, weapon, world);
        self.cannon_send(world, weapon, "ACT_VM_SECONDARYATTACK", time);
        self.physcannon.change_state = ElementChange::Closed;
        self.physcannon.element_debounce = time + 0.5;
        self.physcannon.check_suppress = time + 0.25;
        self.physcannon.next_secondary = time + 0.5;
    }
    fn dry_fire(&mut self, scene: &mut Scene, weapon: &Weapon, world: &World, time: f64) {
        self.cannon_send(world, weapon, "ACT_VM_PRIMARYATTACK", time);
        play_sound(scene, weapon, "empty", world, "");
    }
    /// PrimaryFireEffect: WeaponSound(SINGLE) and the white 32-alpha 0.1 s FFADE_IN
    /// (the view punch is not modeled).
    fn primary_fire_effect(&mut self, scene: &mut Scene, weapon: &Weapon, world: &World) {
        scene
            .screen_fades
            .push(crate::player_damage::ScreenFade::new(
                [245, 245, 255, 32],
                0.1,
                0.,
                0x0001,
            ));
        play_sound(scene, weapon, "single_shot", world, "");
    }
    /// ForceDrop on holster (only reachable without a held object, see can_holster).
    pub(crate) fn physcannon_holster(&mut self) {
        self.physcannon.held = None;
        self.physcannon.grab = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cannon() -> BTreeMap<String, Weapon> {
        // Synthetic definition (not the installed script).
        let sounds = [
            ("single_shot", "Weapon_PhysCannon.Launch"),
            ("special1", "Weapon_PhysCannon.Pickup"),
            ("special2", "Weapon_PhysCannon.OpenClaws"),
            ("special3", "Weapon_PhysCannon.TooHeavy"),
            ("melee_hit", "Weapon_PhysCannon.CloseClaws"),
            ("melee_miss", "Weapon_PhysCannon.Drop"),
            ("empty", "Weapon_PhysCannon.DryFire"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        BTreeMap::from([(
            "weapon_physcannon".to_owned(),
            Weapon {
                class: "weapon_physcannon".into(),
                viewmodel: "models/weapons/v_physcannon.mdl".into(),
                magazine: -1,
                default_clip: -1,
                ammo_type: "None".into(),
                sounds,
                ..Default::default()
            },
        )])
    }
    /// A 10 kg cube prop (entity 0) at `at` and a floor far below.
    fn prop(at: Vec3, mass: f32) -> Physics {
        let mut physics = Physics::default();
        let body = physics.bodies.insert(
            RigidBodyBuilder::dynamic()
                .gravity_scale(0.)
                .translation(vector![at.x * SCALE, at.y * SCALE, at.z * SCALE]),
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
    fn frame(
        inv: &mut Inventory,
        scene: &mut Scene,
        physics: &mut Physics,
        primary: bool,
        secondary: bool,
    ) {
        let world = World::default();
        let weapons = cannon();
        inv.set_attack_input(primary, secondary);
        inv.tick(
            &world,
            scene,
            &weapons,
            Vec3::ZERO,
            primary || secondary,
            0.015,
        );
        inv.physcannon_frame(
            &weapons,
            &world,
            scene,
            physics,
            Vec3::Z * 64.,
            Vec3::X,
            Vec3::ZERO,
            false,
        );
    }

    #[test]
    fn launch_speed_follows_the_physgun_spline() {
        assert_eq!(launch_speed(10.), 1500.);
        assert_eq!(launch_speed(100.), 1500.);
        assert!((launch_speed(350.) - 1100.).abs() < 1e-3);
        assert_eq!(launch_speed(600.), 700.);
        assert_eq!(launch_speed(5000.), 700.);
        let (dv, offset) = punt_impulse(Vec3::new(0.6, 0., -0.8), 10.);
        assert!((dv - Vec3::new(900., 0., 780.)).length() < 1e-2);
        assert!((offset - Vec3::new(3600., 0., 3120.)).length() < 1e-1);
    }

    #[test]
    fn punt_pushes_a_prop_forward_with_the_sdk_impulse() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = prop(Vec3::new(100., 0., 64.), 10.);
        let mut inv = Inventory::default();
        inv.give("weapon_physcannon", &cannon(), 0.);
        for t in 0..40 {
            scene.time = f64::from(t) * 0.015;
            frame(&mut inv, &mut scene, &mut physics, false, false);
        }
        scene.time += 0.015;
        frame(&mut inv, &mut scene, &mut physics, true, false);
        let body = &physics.bodies[physics.dynamic[&0]];
        // 15000/10 + 600 = 2100 in/s along X (the offset impulse at the face adds spin).
        let v = body.linvel().x / SCALE;
        assert!((v - 2100.).abs() < 5., "{v}");
        assert!(scene
            .sounds
            .iter()
            .any(|s| s.name == "Weapon_PhysCannon.Launch"));
        assert_eq!(scene.screen_fades.len(), 1);
    }

    #[test]
    fn pickup_holds_in_front_then_launches_at_1500_and_heavy_props_are_denied() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = prop(Vec3::new(150., 0., 64.), 10.);
        let mut inv = Inventory::default();
        inv.give("weapon_physcannon", &cannon(), 0.);
        let mut t = 0;
        let mut run = |inv: &mut Inventory,
                       scene: &mut Scene,
                       physics: &mut Physics,
                       n,
                       primary,
                       secondary| {
            for _ in 0..n {
                t += 1;
                scene.time = f64::from(t) * 0.015;
                frame(inv, scene, physics, primary, secondary);
                physics.tick(0.015);
            }
        };
        run(&mut inv, &mut scene, &mut physics, 40, false, false);
        run(&mut inv, &mut scene, &mut physics, 2, false, true);
        assert_eq!(inv.physcannon.held, Some(0));
        assert!(scene
            .sounds
            .iter()
            .any(|s| s.name == "Weapon_PhysCannon.Pickup"));
        assert!(!inv.physcannon_can_holster());
        run(&mut inv, &mut scene, &mut physics, 60, false, false);
        let c = center(&physics, 0).unwrap();
        // Held at 24 + 2r - r from the eye, r = player radius + 8 (cube half extent).
        let expected = 24. + crate::grab::PLAYER_RADIUS + 8.;
        assert!((c - Vec3::new(expected, 0., 64.)).length() < 2., "{c}");
        run(&mut inv, &mut scene, &mut physics, 1, true, false);
        assert_eq!(inv.physcannon.held, None);
        let v = physics.bodies[physics.dynamic[&0]].linvel().x / SCALE;
        assert!((v - 1500.).abs() < 20., "{v}");
        // A 300 kg prop: deny sound, no pickup.
        let mut heavy = prop(Vec3::new(150., 0., 64.), 300.);
        let mut inv = Inventory::default();
        let mut scene = Scene::new(&world);
        inv.give("weapon_physcannon", &cannon(), 0.);
        for i in 0..44 {
            scene.time = f64::from(i) * 0.015;
            frame(&mut inv, &mut scene, &mut heavy, false, i >= 40);
        }
        assert_eq!(inv.physcannon.held, None);
        assert_eq!(
            scene
                .sounds
                .iter()
                .filter(|s| s.name == "Weapon_PhysCannon.TooHeavy")
                .count(),
            1
        );
    }

    #[test]
    fn far_objects_are_pulled_toward_the_eye() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = prop(Vec3::new(600., 0., 64.), 10.);
        let mut inv = Inventory::default();
        inv.give("weapon_physcannon", &cannon(), 0.);
        for t in 0..40 {
            scene.time = f64::from(t) * 0.015;
            frame(&mut inv, &mut scene, &mut physics, false, t == 39);
        }
        // 4000 x (10.5 / 50) kg in/s on 10 kg = 84 in/s toward the player.
        let v = physics.bodies[physics.dynamic[&0]].linvel().x / SCALE;
        assert!((v + 84.).abs() < 1., "{v}");
        assert_eq!(inv.physcannon.held, None);
    }

    #[test]
    #[ignore = "requires an owned HL2 installation in HL2_ROOT"]
    fn owned_physcannon_script_and_viewmodel_match_the_sdk_expectations() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let weapons = crate::gameplay::definitions(&vfs).unwrap();
        let w = &weapons["weapon_physcannon"];
        println!(
            "physcannon: slot {} pos {} sounds {:?}",
            w.slot, w.slot_position, w.sounds
        );
        for key in [
            "single_shot",
            "special1",
            "special2",
            "special3",
            "melee_hit",
            "melee_miss",
            "empty",
        ] {
            assert!(w.sounds.contains_key(key), "{key}");
        }
        let mut world = World::default();
        crate::actors::prepare_weapons(&mut world, &vfs, &weapons).unwrap();
        let rig = &world.rigs[&format!("{}#0", w.viewmodel.to_lowercase())];
        for activity in [
            "ACT_VM_DRAW",
            "ACT_VM_IDLE",
            "ACT_VM_PRIMARYATTACK",
            "ACT_VM_SECONDARYATTACK",
            "ACT_VM_RELOAD",
        ] {
            let sequence = rig
                .lookup_sequence(activity, None, |_| 0)
                .unwrap_or_else(|| panic!("{activity} missing; {:?}", rig.sequences));
            println!(
                "{activity}: {sequence} {:.3} s",
                rig.clips[sequence].duration()
            );
        }
        println!(
            "pose parameters {:?}",
            rig.pose_parameters
                .iter()
                .map(|p| &p.name)
                .collect::<Vec<_>>()
        );
        assert!(rig
            .pose_parameters
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case("active")));
    }
}
