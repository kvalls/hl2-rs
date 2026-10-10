//! weapon_bugbait, npc_grenade_bugbait and point_bugbait from the singleplayer SDK
//! (weapon_bugbait.cpp, grenade_bugbait.cpp). Antlion reactions belong to step 14: this
//! module only reports where bait landed or was squeezed (`BugBaitEvent`).
//! SDK behavior; retail and native not compared yet.
use crate::{
    entities::Scene,
    gameplay::{activity_sequence, duration, Inventory, Weapon},
    physics::{Physics, ProjectileHull},
    projectiles::{Projectile, ProjectileKind, ProjectileSpawn},
};
use glam::Vec3;
use modkit_core::World;
use serde::Serialize;
use std::collections::BTreeMap;

/// grenade_bugbait.cpp GRENADE_MODEL.
pub const BAIT_MODEL: &str = "models/weapons/w_bugbait.mdl";
/// bugbait_hear_radius / bugbait_distract_time / bugbait_grenade_radius cvar defaults.
pub const HEAR_RADIUS: f32 = 2500.;
pub const DISTRACT_TIME: f32 = 5.;
pub const GRENADE_RADIUS: f32 = 150.;
/// CGrenadeBugBait::Spawn UTIL_SetSize(-2, 2).
const HALF_SIZE: f32 = 2.;
const GRAVITY: f32 = 600.;
/// SF_BUGBAIT_SUPPRESS_CALL / NOT_THROWN / NOT_SQUEEZE.
const SF_SUPPRESS_CALL: u32 = 1;
const SF_NOT_THROWN: u32 = 2;
const SF_NOT_SQUEEZE: u32 = 4;

/// Where bait splatted (thrown) or was squeezed; `call` is false when a point_bugbait
/// with SF_BUGBAIT_SUPPRESS_CALL took it. A thrown call inserts SOUND_BUGBAIT
/// (HEAR_RADIUS, DISTRACT_TIME) and broadcasts the antlion fight goal; a squeeze
/// broadcasts the follow goal. Combine soldiers within GRENADE_RADIUS with a clear line
/// get "HitByBugbait" (listed in `combine`). All of that is step 14.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BugBaitEvent {
    pub position: Vec3,
    pub squeezed: bool,
    pub call: bool,
    pub combine: Vec<usize>,
}

/// CWeaponBugBait m_bDrawBackFinished, m_bRedraw, a pending throw and the event cursor.
#[derive(Clone, Debug, Default, Serialize)]
pub struct BugBaitState {
    draw_back_finished: bool,
    redraw: bool,
    /// EVENT_WEAPON_THROW fired this tick; the host launches it (needs eye/velocity).
    pub pending_throw: bool,
    next_primary: f64,
    next_secondary: f64,
    idle_at: f64,
    cursor: Option<(String, f64, f32)>,
}

/// CGrenadeBugBait::ActivateBugbaitTargets: combine soldiers (thrown only) and
/// point_bugbait sensors (OnBaited). Returns (suppress call, combine soldiers).
pub fn activate_targets(
    world: &World,
    scene: &mut Scene,
    physics: Option<&Physics>,
    origin: Vec3,
    squeezed: bool,
) -> (bool, Vec<usize>) {
    let mut combine = Vec::new();
    let mut suppress = false;
    for (id, entity) in world.entities.iter().enumerate() {
        let state = &scene.states[id];
        if state.killed {
            continue;
        }
        if !squeezed && entity.class() == "npc_combine_s" {
            // UTIL_DistApprox from the world-space center; eye line must be clear.
            let center = state.origin + Vec3::Z * 36.;
            let eye = state.origin + Vec3::Z * 64.;
            if dist_approx(center, origin) < GRENADE_RADIUS
                && physics.is_none_or(|p| p.projectile_ray(origin, eye, &[id]).is_none())
            {
                combine.push(id);
            }
        }
        if entity.class() != "point_bugbait" {
            continue;
        }
        let flags = entity
            .get("spawnflags")
            .and_then(|f| f.parse::<u32>().ok())
            .unwrap_or(0);
        // m_bEnabled is the "Enabled" keyfield; Enable/Disable/Toggle reach the scene
        // state (an Enable input after "Enabled" "0" is not tracked: limitation).
        let enabled = state.enabled && entity.get("Enabled") != Some("0");
        if !enabled
            || squeezed && flags & SF_NOT_SQUEEZE != 0
            || !squeezed && flags & SF_NOT_THROWN != 0
        {
            continue;
        }
        let radius = entity
            .get("radius")
            .and_then(|r| r.parse::<f32>().ok())
            .unwrap_or(0.);
        if radius > state.origin.distance(origin) {
            scene.fire(id, "OnBaited", usize::MAX);
            suppress |= flags & SF_SUPPRESS_CALL != 0;
        }
    }
    (suppress, combine)
}

/// UTIL_DistApprox (mathlib's octagonal distance approximation).
fn dist_approx(a: Vec3, b: Vec3) -> f32 {
    let d = (a - b).abs();
    let (mut big, mut mid, mut small) = (d.x, d.y, d.z);
    if big < mid {
        std::mem::swap(&mut big, &mut mid);
    }
    if mid < small {
        std::mem::swap(&mut mid, &mut small);
    }
    if big < mid {
        std::mem::swap(&mut big, &mut mid);
    }
    big + (mid + small) * 0.5
}

impl Inventory {
    pub(crate) fn bugbait_deploy(&mut self) {
        // CWeaponBugBait::Deploy / Holster.
        self.bugbait = BugBaitState::default();
    }
    /// CWeaponBugBait::ItemPostFrame with Operator_HandleAnimEvent (3900 drawback
    /// finished, 3005 throw).
    pub(crate) fn bugbait_tick(
        &mut self,
        world: &World,
        scene: &mut Scene,
        weapons: &BTreeMap<String, Weapon>,
        primary: bool,
        secondary: bool,
    ) {
        let Some(weapon) = weapons.get("weapon_bugbait").cloned() else {
            return;
        };
        let time = scene.time;
        let elapsed = (time - self.animation_at).max(0.) as f32;
        let previous = self
            .bugbait
            .cursor
            .as_ref()
            .filter(|(a, at, _)| *a == self.animation && *at == self.animation_at)
            .map_or(-f32::EPSILON, |(_, _, e)| *e);
        let events: Vec<i32> = world
            .rigs
            .get(&format!("{}#0", weapon.viewmodel.to_lowercase()))
            .and_then(|rig| rig.clips.get(&self.animation))
            .map(|clip| {
                clip.events_between(previous, elapsed)
                    .into_iter()
                    .map(|e| e.id)
                    .collect()
            })
            .unwrap_or_default();
        self.bugbait.cursor = Some((self.animation.clone(), self.animation_at, elapsed));
        for id in events {
            match id {
                3900 => self.bugbait.draw_back_finished = true,
                3005 => {
                    // ThrowGrenade, then m_bRedraw.
                    self.bugbait.pending_throw = true;
                    self.bugbait.redraw = true;
                }
                _ => {}
            }
        }
        if time < self.owner_attack_until {
            return;
        }
        let send = |inv: &mut Inventory, activity: &str| {
            let sequence = activity_sequence(world, &weapon, activity);
            inv.animate(&sequence, time);
            let end = time + duration(world, &weapon, &sequence);
            inv.bugbait.idle_at = end;
            end
        };
        if self.bugbait.draw_back_finished {
            if !primary {
                let end = send(self, "ACT_VM_THROW");
                self.bugbait.next_primary = end;
                self.bugbait.draw_back_finished = false;
            }
        } else if primary && self.bugbait.next_primary < time {
            // PrimaryAttack: haul back unless a redraw is pending.
            if !self.bugbait.redraw {
                send(self, "ACT_VM_HAULBACK");
                self.bugbait.idle_at = f64::INFINITY;
                self.bugbait.next_primary = f64::INFINITY;
                self.shots += 1;
            }
        } else if secondary && self.bugbait.next_secondary < time {
            // SecondaryAttack: squeeze.
            // EmitSound("Weapon_Bugbait.Splat") at the weapon (the player).
            let at = self.squeeze_origin;
            scene.sounds.push(crate::sounds::SoundRequest {
                origin: Some(at),
                .."Weapon_Bugbait.Splat".into()
            });
            let (suppress, _) = activate_targets(world, scene, None, at, true);
            self.bugbait_events.push(BugBaitEvent {
                position: at,
                squeezed: true,
                call: !suppress,
                combine: Vec::new(),
            });
            let end = send(self, "ACT_VM_SECONDARYATTACK");
            self.bugbait.next_secondary = end;
        }
        // m_bRedraw: once the viewmodel sequence has finished, Reload draws a new one.
        if self.bugbait.redraw
            && f64::from(elapsed) >= duration(world, &weapon, &self.animation)
            && self.bugbait.next_primary <= time
        {
            let end = send(self, "ACT_VM_DRAW");
            self.bugbait.next_primary = end;
            self.bugbait.redraw = false;
        }
        if time > self.bugbait.idle_at {
            self.idle_override = Some(activity_sequence(world, &weapon, "ACT_VM_IDLE"));
        }
    }
    /// CWeaponBugBait::ThrowGrenade: eye + 18 forward + 12 right, player velocity +
    /// 1000 forward, spin (600, RandomInt(-1200, 1200), 0), 0.1 s grace when clear.
    pub fn launch_bugbait(
        &mut self,
        physics: &Physics,
        eye: Vec3,
        forward: Vec3,
        velocity: Vec3,
        time: f64,
    ) {
        if !std::mem::take(&mut self.bugbait.pending_throw) {
            return;
        }
        let forward = forward.normalize_or_zero();
        let right = forward.cross(Vec3::Z).normalize_or(Vec3::Y);
        let grace = if physics.impact_ray(eye, forward, 128.).is_none() {
            0.1
        } else {
            0.
        };
        let spin = (self.random() * 1200.).round();
        self.projectile_spawns.push(ProjectileSpawn {
            kind: ProjectileKind::BugBait,
            position: eye + forward * 18. + right * 12.,
            velocity: velocity + forward * 1000.,
            angular_velocity: Vec3::new(600., spin, 0.),
            at: time,
            damage: 0.,
            radius: 0.,
            mass: 0.,
            // The grace period travels in `lifetime` (as for missiles).
            lifetime: grace,
        });
    }
}

/// npc_grenade_bugbait for one tick: MOVETYPE_FLYGRAVITY (half gravity before and
/// after the move), non-solid during the grace period, BugBaitTouch on the first solid
/// contact. Returns the splat position.
pub(crate) fn bait_tick(
    bait: &mut Projectile,
    physics: &Physics,
    time: f64,
    dt: f32,
) -> Option<Vec3> {
    bait.velocity.z -= 0.5 * GRAVITY * dt;
    let end = bait.position + bait.velocity * dt;
    if time >= bait.solid_at {
        if let Some(hit) = physics.projectile_sweep(
            bait.position,
            end,
            ProjectileHull::Box(Vec3::splat(HALF_SIZE)),
            &[],
        ) {
            bait.position = hit.position;
            return Some(hit.position);
        }
    }
    bait.position = end;
    bait.velocity.z -= 0.5 * GRAVITY * dt;
    bait.angles += bait.angular_velocity * dt;
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projectiles::Projectiles;
    use rapier3d::prelude::*;

    fn bugbait() -> BTreeMap<String, Weapon> {
        // Synthetic definition (not the installed script): no ammo, bucket 5.
        BTreeMap::from([(
            "weapon_bugbait".to_owned(),
            Weapon {
                class: "weapon_bugbait".into(),
                viewmodel: "models/weapons/v_bugbait.mdl".into(),
                magazine: -1,
                default_clip: -1,
                ammo_type: "None".into(),
                slot: 5,
                ..Default::default()
            },
        )])
    }

    #[test]
    fn haul_back_throws_on_release_after_the_drawback_and_squeeze_reports_the_player() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let weapons = bugbait();
        let mut inv = Inventory::default();
        inv.give("weapon_bugbait", &weapons, 0.);
        let mut t = 0;
        let mut step = |inv: &mut Inventory, scene: &mut Scene, primary, secondary| {
            scene.time = f64::from(t) * 0.015;
            t += 1;
            inv.set_attack_input(primary, secondary);
            let feet = Vec3::new(5., 6., 7.);
            inv.tick(&world, scene, &weapons, feet, primary || secondary, 0.015);
        };
        for _ in 0..40 {
            step(&mut inv, &mut scene, false, false);
        }
        step(&mut inv, &mut scene, true, false);
        assert_eq!(inv.animation, "act_vm_haulback");
        // No rig: the drawback-finished event comes from the owned sequence; simulate it.
        inv.bugbait.draw_back_finished = true;
        step(&mut inv, &mut scene, true, false);
        assert_eq!(inv.animation, "act_vm_haulback", "held: keep the drawback");
        step(&mut inv, &mut scene, false, false);
        assert_eq!(inv.animation, "act_vm_throw");
        inv.bugbait.pending_throw = true;
        inv.launch_bugbait(&physics, Vec3::Z * 64., Vec3::X, Vec3::Y * 10., scene.time);
        let spawn = inv.projectile_spawns.pop().unwrap();
        assert_eq!(spawn.kind, ProjectileKind::BugBait);
        assert_eq!(spawn.position, Vec3::new(18., -12., 64.));
        assert_eq!(spawn.velocity, Vec3::new(1000., 10., 0.));
        assert_eq!(spawn.lifetime, 0.1);
        assert_eq!(spawn.angular_velocity.x, 600.);
        // Squeeze: splat sound and a squeezed event at the player.
        for _ in 0..80 {
            step(&mut inv, &mut scene, false, false);
        }
        step(&mut inv, &mut scene, false, true);
        assert!(scene
            .sounds
            .iter()
            .any(|s| s.name == "Weapon_Bugbait.Splat"));
        assert_eq!(
            inv.bugbait_events,
            vec![BugBaitEvent {
                position: Vec3::new(5., 6., 7.),
                squeezed: true,
                call: true,
                combine: vec![]
            }]
        );
    }

    #[test]
    fn thrown_bait_splats_on_the_first_contact_and_fires_point_bugbait() {
        let world = World {
            entities: vec![
                modkit_core::Entity {
                    properties: vec![
                        ("classname".into(), "point_bugbait".into()),
                        ("origin".into(), "300 0 0".into()),
                        ("radius".into(), "100".into()),
                        ("Enabled".into(), "1".into()),
                        ("spawnflags".into(), "1".into()),
                    ],
                },
                modkit_core::Entity {
                    properties: vec![
                        ("classname".into(), "point_bugbait".into()),
                        ("origin".into(), "300 0 0".into()),
                        ("radius".into(), "100".into()),
                        ("spawnflags".into(), "2".into()),
                    ],
                },
            ],
            ..World::default()
        };
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        physics.colliders.insert(
            ColliderBuilder::cuboid(1. / 39.37, 200. / 39.37, 200. / 39.37).translation(vector![
                300. / 39.37,
                0.,
                0.
            ]),
        );
        physics.tick(0.015);
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                kind: ProjectileKind::BugBait,
                position: Vec3::ZERO,
                velocity: Vec3::X * 1000.,
                angular_velocity: Vec3::new(600., 0., 0.),
                at: 0.,
                damage: 0.,
                radius: 0.,
                mass: 0.,
                lifetime: 0.1,
            },
            &mut scene,
        );
        for t in 1..40 {
            scene.time = f64::from(t) * 0.015;
            projectiles.tick(
                &world,
                &mut scene,
                &mut physics,
                Vec3::splat(9999.),
                false,
                0.015,
            );
        }
        assert!(projectiles.active.is_empty());
        assert!(scene
            .sounds
            .iter()
            .any(|s| s.name == "GrenadeBugBait.Splat"));
        let event = &projectiles.bugbait_events[0];
        assert!(!event.squeezed);
        // The first sensor suppresses the call; the second ignores thrown bait.
        assert!(!event.call);
        assert!((event.position.x - 297.).abs() < 0.5, "{}", event.position);
        // Falling 600 u/s^2 for ~0.3 s.
        assert!((event.position.z + 0.5 * 600. * 0.297 * 0.297).abs() < 3.);
    }

    #[test]
    #[ignore = "requires an owned HL2 installation in HL2_ROOT"]
    fn owned_bugbait_script_viewmodel_and_assets_match_the_sdk_expectations() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let weapons = crate::gameplay::definitions(&vfs).unwrap();
        let w = &weapons["weapon_bugbait"];
        println!(
            "bugbait: clip {} ammo {} slot {} sounds {:?}",
            w.magazine, w.ammo_type, w.slot, w.sounds
        );
        let mut world = World::default();
        crate::actors::prepare_weapons(&mut world, &vfs, &weapons).unwrap();
        let rig = &world.rigs[&format!("{}#0", w.viewmodel.to_lowercase())];
        for activity in [
            "ACT_VM_DRAW",
            "ACT_VM_IDLE",
            "ACT_VM_HAULBACK",
            "ACT_VM_THROW",
            "ACT_VM_SECONDARYATTACK",
        ] {
            let sequence = rig
                .lookup_sequence(activity, None, |_| 0)
                .unwrap_or_else(|| panic!("{activity} missing; {:?}", rig.sequences));
            let clip = &rig.clips[sequence];
            println!(
                "{activity}: {sequence} {:.3} s events {:?}",
                clip.duration(),
                clip.events
                    .iter()
                    .map(|e| (e.id, e.cycle))
                    .collect::<Vec<_>>()
            );
        }
        let has = |activity: &str, id: i32| {
            rig.clips[rig.lookup_sequence(activity, None, |_| 0).unwrap()]
                .events
                .iter()
                .any(|e| e.id == id)
        };
        assert!(has("ACT_VM_HAULBACK", 3900), "drawback finished event");
        assert!(has("ACT_VM_THROW", 3005), "throw event");
        assert!(vfs.read(BAIT_MODEL).unwrap().is_some());
        let library = crate::sounds::Library::new(&vfs);
        for cue in ["Weapon_Bugbait.Splat", "GrenadeBugBait.Splat"] {
            assert!(library.all_alternatives(cue).is_ok(), "{cue}");
        }
    }
}
