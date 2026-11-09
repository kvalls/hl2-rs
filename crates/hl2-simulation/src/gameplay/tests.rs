mod shotgun;
#[test]
fn an_empty_frag_switches_to_the_heaviest_weapon_with_ammo() {
    let mk = |class: &str, ammo: &str, magazine: i32, weight: i32, no_to: bool| Weapon {
        class: class.into(),
        ammo_type: ammo.into(),
        magazine,
        ammo_max: 10,
        weight,
        no_autoswitch_to: no_to,
        ..Default::default()
    };
    let weapons: BTreeMap<String, Weapon> = [
        mk("weapon_frag", "grenade", -1, 1, false),
        mk("weapon_pistol", "Pistol", 18, 2, false),
        mk("weapon_smg1", "SMG1", 45, 3, false),
        mk("weapon_rpg", "RPG_Round", -1, 0, true),
        mk("weapon_crowbar", "None", -1, 0, false),
    ]
    .into_iter()
    .map(|w| (w.class.clone(), w))
    .collect();
    let mut inv = Inventory::default();
    for (class, clip) in [
        ("weapon_frag", -1),
        ("weapon_pistol", 0),
        ("weapon_smg1", 0),
        ("weapon_rpg", -1),
        ("weapon_crowbar", -1),
    ] {
        inv.owned.insert(class.into(), clip);
    }
    inv.active = "weapon_frag".into();
    inv.set_ammo("Pistol", 5);
    inv.set_ammo("RPG_Round", 3);
    // SMG1 has neither clip nor reserve; the RPG never takes auto-switches.
    assert!(!inv.has_any_ammo(&weapons["weapon_frag"]));
    assert_eq!(
        inv.next_best_weapon(&weapons).as_deref(),
        Some("weapon_pistol")
    );
    inv.set_ammo("Pistol", 0);
    assert_eq!(
        inv.next_best_weapon(&weapons).as_deref(),
        Some("weapon_crowbar")
    );
}
#[test]
fn frag_launches_follow_weapon_frag_math() {
    use super::*;
    let world = World::default();
    let physics = Physics::new(&world);
    let eye = Vec3::new(0., 0., 64.);
    let (source, velocity, spin, _) = frag_launch(
        &physics,
        FragThrow::Throw,
        eye,
        Vec3::X,
        Vec3::Y * 10.,
        Vec3::ZERO,
        None,
        0.5,
    );
    // eye + 18 forward + 8 right (right of +X is -Y); forward.z + 0.1 at 1200 u/s.
    assert_eq!(source, Vec3::new(18., -8., 64.));
    assert_eq!(velocity, Vec3::new(1200., 10., 120.));
    assert_eq!(spin, Vec3::new(600., 600., 0.));
    let (source, velocity, _, _) = frag_launch(
        &physics,
        FragThrow::Lob,
        eye,
        Vec3::X,
        Vec3::ZERO,
        Vec3::ZERO,
        None,
        0.,
    );
    assert_eq!(source, Vec3::new(18., -8., 56.));
    assert_eq!(velocity, Vec3::new(350., 0., 50.));
    let (source, velocity, spin, _) = frag_launch(
        &physics,
        FragThrow::Roll,
        eye,
        Vec3::new(0.6, 0., -0.8),
        Vec3::ZERO,
        Vec3::ZERO,
        Some(Vec3::Z),
        0.,
    );
    assert!((source - Vec3::new(18., 0., 4.)).length() < 1e-4);
    assert!((velocity - Vec3::new(700., 0., 0.)).length() < 1e-3);
    assert_eq!(spin, Vec3::new(0., 0., 720.));
}
#[test]
fn trace_bleed_marks_the_wall_behind_bleeding_npcs() {
    use rapier3d::prelude::*;
    const S: f32 = 1. / 39.37;
    let mut world = World::default();
    let npc = |class: &str| modkit_core::Entity {
        properties: vec![("classname".into(), class.into())],
    };
    world.entities.push(npc("npc_metropolice"));
    world.entities.push(npc("npc_cscanner"));
    // Synthetic wall 100 units behind the hit point (not game data).
    let mut physics = Physics::default();
    physics.colliders.insert(
        ColliderBuilder::cuboid(5. * S, 200. * S, 200. * S).translation(vector![100. * S, 0., 0.]),
    );
    physics.refresh_queries();
    let hit = crate::physics::RayHit {
        entity: 0,
        position: Vec3::ZERO,
        normal: -Vec3::X,
        hitgroup: 0,
        bone: None,
    };
    let mut inv = Inventory::default();
    let count = |inv: &mut Inventory, id: usize, damage: f32| {
        inv.blood.clear();
        let mut blood = std::mem::take(&mut inv.blood);
        trace_bleed(
            &physics,
            &world,
            id,
            &hit,
            Vec3::X,
            damage,
            &mut || inv.random(),
            &mut blood,
        );
        inv.blood = blood;
        inv.blood.len()
    };
    // 1 / 2 / 4 traces by damage, all reaching the wall within 172 units.
    assert_eq!(count(&mut inv, 0, 5.), 1);
    assert_eq!(count(&mut inv, 0, 12.), 2);
    assert_eq!(count(&mut inv, 0, 40.), 4);
    assert!(inv
        .blood
        .iter()
        .all(|(b, c)| (b.position.x - 95.).abs() < 1. && *c == BloodColor::Red));
    // Machines do not bleed.
    assert_eq!(count(&mut inv, 1, 40.), 0);
    assert_eq!(blood_color("npc_headcrab"), Some(BloodColor::Yellow));
}
#[test]
fn breaking_an_explosive_prop_queues_its_explosion() {
    // Synthetic: prop_physics with health 10 and ExplodeDamage 100 (radius 0 ->
    // magnitude x 2.5), and one with only health (no explosion).
    use crate::projectiles::{Damage, DamageTarget};
    let prop = |extra: &[(&str, &str)]| modkit_core::Entity {
        properties: [("classname", "prop_physics"), ("health", "10")]
            .iter()
            .chain(extra)
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    };
    let world = World {
        entities: vec![
            prop(&[("ExplodeDamage", "100.7"), ("origin", "5 0 0")]),
            prop(&[]),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let hit = |id| Damage {
        target: DamageTarget::Entity(id),
        amount: 20.,
        kind: crate::player_damage::DMG_BULLET,
        dissolve: false,
        direction: Vec3::X,
        origin: Vec3::ZERO,
        attacker: Attacker::Player,
    };
    Inventory::default().apply_projectile_damage(
        vec![hit(0), hit(1)],
        &world,
        &mut scene,
        &mut physics,
    );
    assert!(scene.states[0].killed && scene.states[1].killed);
    assert_eq!(scene.prop_explosions.len(), 1);
    let (_, magnitude, radius, _, attacker) = scene.prop_explosions[0];
    assert_eq!((magnitude, radius), (100., 250.));
    // The shooter is the explosion's attacker (m_hLastAttacker).
    assert_eq!(attacker, Attacker::Player);
}
#[test]
fn burn_damage_scorches_a_zombie_down_to_fifty() {
    // OnTakeDamage_Alive: Scorch(8, 50) per accepted DMG_BURN hit.
    use crate::projectiles::{Damage, DamageTarget};
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_zombie".into()),
                ("rendercolor".into(), "255 200 40".into()),
            ],
        }],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    scene.states[0].value = 1000.;
    let mut physics = Physics::new(&world);
    let burn = Damage {
        target: DamageTarget::Entity(0),
        amount: 1.,
        kind: crate::player_damage::DMG_BURN | crate::fire::DMG_DIRECT,
        dissolve: false,
        direction: Vec3::ZERO,
        origin: Vec3::ZERO,
        attacker: Attacker::None,
    };
    Inventory::default().apply_projectile_damage(vec![burn], &world, &mut scene, &mut physics);
    assert_eq!(scene.render_colors[&0], [247, 192, 40]);
    for _ in 0..40 {
        Inventory::default().apply_projectile_damage(vec![burn], &world, &mut scene, &mut physics);
    }
    assert_eq!(scene.render_colors[&0], [47, 48, 40]);
}
#[test]
fn flames_and_blasts_reach_the_npcs_pain_sounds_but_only_characters_hurt_its_ai() {
    // OnTakeDamage_Alive: PainSound for any damage; COND_LIGHT/HEAVY_DAMAGE and the
    // attacker's memory only for an FL_NPC/FL_CLIENT attacker (a flame is its own).
    use crate::projectiles::{Damage, DamageTarget};
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![("classname".into(), "npc_metropolice".into())],
        }],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    scene.states[0].value = 40.;
    let mut physics = Physics::new(&world);
    let hit = |amount: f32, kind: u32, attacker: Attacker| Damage {
        target: DamageTarget::Entity(0),
        amount,
        kind,
        dissolve: false,
        direction: Vec3::ZERO,
        origin: Vec3::ZERO,
        attacker,
    };
    let flame = hit(1., crate::player_damage::DMG_BURN, Attacker::None);
    let blast = hit(25., crate::player_damage::DMG_BLAST, Attacker::Player);
    Inventory::default().apply_projectile_damage(
        vec![flame, blast],
        &world,
        &mut scene,
        &mut physics,
    );
    let reports: Vec<_> = scene
        .npc_damage
        .iter()
        .map(|r| (r.victim, r.by_player, r.damage))
        .collect();
    assert_eq!(reports, [(0, false, 0.), (0, true, 25.)]);
    assert_eq!(scene.states[0].value, 14.);
}
#[test]
fn a_gas_can_burst_ignites_only_npcs_allowed_to_burn_within_its_radius() {
    // CBreakableProp::Break (explode_fire): Ignite(30) on the combat characters within
    // m_explodeRadius that pass the damage filter; AllowedToIgnite classes only.
    let npc = |class: &str, origin: &str| modkit_core::Entity {
        properties: [("classname", class), ("origin", origin)]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    };
    let world = World {
        entities: vec![
            npc("npc_zombie", "50 0 0"),
            npc("npc_metropolice", "0 60 0"),
            npc("npc_citizen", "-40 0 0"),
            npc("npc_headcrab_black", "0 -30 0"),
            // 120 units away: its hull's nearest point is 107 from the burst.
            npc("npc_zombie", "120 0 0"),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    scene.states[0].value = 50.;
    let ignited = scene.ignite_near(&world, Vec3::ZERO, 100., Attacker::Player);
    assert_eq!(ignited, [0, 1, 3]);
    assert!(scene.fires.burning(0) && !scene.fires.burning(2) && !scene.fires.burning(4));
    assert_eq!(scene.fires.flames[&0].until, 30.);
    // CNPC_BaseZombie::Ignite: health capped to about ten seconds of burning.
    assert!(scene.states[0].value <= 60. && scene.states[0].value >= 40.);
    // Already burning: once; a zero radius ignites nothing.
    assert!(scene
        .ignite_near(&world, Vec3::ZERO, 100., Attacker::Player)
        .is_empty());
    assert!(scene
        .ignite_near(&world, Vec3::new(120., 0., 0.), 0., Attacker::Player)
        .is_empty());
    // A designer's Ignite input burns any NPC.
    assert!(scene.ignite_npc(&world, 2, 30., true));
}
#[test]
fn breakable_damage_applies_prop_data_modifiers() {
    use crate::player_damage::*;
    // Wooden.Large (owned propdata.txt): bullets 0.75, club 2, explosive 1.5.
    let wood = modkit_core::PropData {
        health: 50.,
        dmg_bullets: 0.75,
        dmg_club: 2.,
        dmg_explosive: 1.5,
        ..Default::default()
    };
    assert_eq!(breakable_damage(8., DMG_BULLET, Some(&wood)), 6.);
    assert_eq!(
        breakable_damage(8., DMG_BULLET | DMG_BUCKSHOT, Some(&wood)),
        12.
    );
    assert_eq!(breakable_damage(10., DMG_CLUB, Some(&wood)), 20.);
    assert_eq!(breakable_damage(10., DMG_BLAST, Some(&wood)), 15.);
    assert_eq!(breakable_damage(10., DMG_CRUSH, Some(&wood)), 10.);
    assert_eq!(
        breakable_damage(1., DMG_SLASH | DMG_CRUSH, Some(&wood)),
        10.
    );
    assert_eq!(breakable_damage(10., DMG_POISON, Some(&wood)), 0.);
    // Without prop data the CBreakableProp defaults (1) apply; brush breakables
    // use the func_breakdmg_* convars.
    assert_eq!(breakable_damage(8., DMG_BULLET, None), 8.);
    assert_eq!(
        breakable_damage(8., DMG_BULLET, Some(&FUNC_BREAKABLE_DAMAGE)),
        4.
    );
}
#[test]
fn fall_damage_and_armor_exceptions_follow_sdk() {
    use super::*;
    assert_eq!(fall_damage(500.), 0.);
    assert!((fall_damage(922.5) - 100.).abs() < 1e-3);
    assert!((fall_damage(693.) - 42.05).abs() < 0.05);
    let mut inv = Inventory {
        health: 100.,
        armor: 50.,
        ..Default::default()
    };
    // Falls ignore armor; other damage spends one armor point per point absorbed.
    use crate::player_damage::{DamageInfo, PlayerDamage, DMG_BULLET, DMG_FALL};
    let mut damage = PlayerDamage::default();
    let mut hit = |inv: &mut Inventory, amount, kind| {
        damage.take(
            inv,
            DamageInfo {
                amount,
                kind,
                inflictor: glam::Vec3::ZERO,
            },
        )
    };
    assert_eq!(hit(&mut inv, 30., DMG_FALL), 30.);
    assert_eq!(inv.armor, 50.);
    assert_eq!(hit(&mut inv, 30., DMG_BULLET), 6.);
    assert_eq!(inv.armor, 26.);
    assert_eq!(hit(&mut inv, 500., DMG_FALL), 64.);
    assert_eq!(hit(&mut inv, 10., DMG_BULLET), 0.);
}
#[test]
fn map_clock_transfer_preserves_remaining_cooldown_reload_and_charge() {
    use super::*;
    let mut inv = Inventory {
        health: 42.,
        suit: true,
        next_attack: 101.5,
        owner_attack_until: 102.,
        soonest_attack: 100.25,
        last_shot: 99.,
        animation_at: 98.,
        ar2_charge_until: Some(100.5),
        reload: Some(Reload {
            weapon: "weapon_smg1".into(),
            at: 103.,
            phase: ReloadPhase::Magazine,
        }),
        ..Default::default()
    };
    inv.next_secondary.insert("weapon_smg1".into(), 104.);
    inv.empty_fire.insert(
        "weapon_smg1".into(),
        EmptyFire {
            latched: true,
            next_sound: 100.4,
        },
    );
    inv.rebase_clock(100., 0.);
    assert_eq!(
        (inv.next_attack, inv.owner_attack_until, inv.soonest_attack),
        (1.5, 2., 0.25)
    );
    assert_eq!((inv.last_shot, inv.animation_at), (-1., -2.));
    assert_eq!(inv.ar2_charge_until, Some(0.5));
    assert_eq!(inv.reload.as_ref().unwrap().at, 3.);
    assert_eq!(inv.next_secondary["weapon_smg1"], 4.);
    assert!((inv.empty_fire["weapon_smg1"].next_sound - 0.4).abs() < 1e-9);
    assert!(inv.suit && inv.health == 42.);
}

use super::*;
fn melee_target(origin: Vec3) -> (Physics, Scene) {
    use rapier3d::prelude::*;
    let mut physics = Physics::default();
    physics.colliders.insert(
        ColliderBuilder::cuboid(5. / 39.37, 5. / 39.37, 5. / 39.37)
            .translation(vector![
                origin.x / 39.37,
                origin.y / 39.37,
                origin.z / 39.37
            ])
            .user_data(1),
    );
    physics.tick(0.015);
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "prop_physics".into()),
                (
                    "origin".into(),
                    format!("{} {} {}", origin.x, origin.y, origin.z),
                ),
            ],
        }],
        ..Default::default()
    };
    (physics, Scene::new(&world))
}
#[test]
fn melee_hull_reaches_offset_target_and_rejects_side_origin() {
    let (physics, mut scene) = melee_target(Vec3::new(40., 20., 0.));
    assert!(physics.impact_ray(Vec3::ZERO, Vec3::X, 75.).is_none());
    assert_eq!(
        melee_trace(&physics, &scene, Vec3::ZERO, Vec3::X)
            .unwrap()
            .entity,
        0
    );
    scene.states[0].origin = Vec3::Y * 40.;
    assert!(melee_trace(&physics, &scene, Vec3::ZERO, Vec3::X).is_none());
}
#[test]
fn melee_full_range_ray_precedes_hull_facing_filter() {
    let (physics, mut scene) = melee_target(Vec3::X * 70.);
    scene.states[0].origin = -Vec3::X * 70.;
    assert_eq!(
        melee_trace(&physics, &scene, Vec3::ZERO, Vec3::X)
            .unwrap()
            .entity,
        0
    );
}
#[test]
fn melee_facing_uses_live_rigid_body_origin() {
    use rapier3d::prelude::*;
    let (mut physics, mut scene) = melee_target(Vec3::new(40., 20., 0.));
    scene.states[0].origin = -Vec3::X * 40.;
    let body = physics
        .bodies
        .insert(RigidBodyBuilder::fixed().translation(vector![40. / 39.37, 20. / 39.37, 0.]));
    physics.dynamic.insert(0, body);
    assert!(melee_trace(&physics, &scene, Vec3::ZERO, Vec3::X).is_some());
}
pub(super) fn definitions() -> BTreeMap<String, Weapon> {
    [
        ("weapon_crowbar", "None", -1, 0, 1),
        ("weapon_pistol", "Pistol", 18, 150, 1),
        ("weapon_357", "357", 6, 12, 1),
        ("weapon_smg1", "SMG1", 45, 225, 1),
        ("weapon_ar2", "AR2", 30, 60, 1),
        ("weapon_shotgun", "Buckshot", 6, 30, 7),
    ]
    .into_iter()
    .map(|(class, ammo, clip, ammo_max, pellets)| {
        (
            class.into(),
            Weapon {
                name: class.into(),
                ammo_type: ammo.into(),
                magazine: clip,
                default_clip: clip,
                ammo_max,
                secondary_ammo_type: match class {
                    "weapon_smg1" => "SMG1_Grenade",
                    "weapon_ar2" => "AR2AltFire",
                    _ => "None",
                }
                .into(),
                secondary_ammo_max: if matches!(class, "weapon_smg1" | "weapon_ar2") {
                    3
                } else {
                    0
                },
                pellets,
                ..Default::default()
            },
        )
    })
    .collect()
}
#[test]
fn health_and_battery_pickups_clamp_and_leave_full_or_suitless_items() {
    let world = World {
        entities: [
            "item_healthkit",
            "item_healthvial",
            "item_battery",
            "item_healthkit",
            "item_battery",
        ]
        .into_iter()
        .map(|class| modkit_core::Entity {
            properties: vec![("classname".into(), class.into())],
        })
        .collect(),
        ..Default::default()
    };
    let mut inv = Inventory {
        health: 65.,
        armor: 95.,
        suit: true,
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    inv.tick(&world, &mut scene, &definitions(), Vec3::ZERO, false, 0.015);
    assert_eq!((inv.health, inv.armor), (100., 100.));
    assert!(scene.states[..3].iter().all(|state| state.killed));
    assert!(scene.states[3..].iter().all(|state| !state.killed));
    assert_eq!(
        scene.sounds,
        ["HealthKit.Touch", "HealthVial.Touch", "ItemBattery.Touch"]
    );
    inv.suit = false;
    inv.armor = 95.;
    let mut scene = Scene::new(&world);
    inv.tick(&world, &mut scene, &definitions(), Vec3::ZERO, false, 0.015);
    assert!(scene.states.iter().all(|state| !state.killed));
    assert!(scene.sounds.is_empty());
}
pub(super) fn shotgun_definitions() -> BTreeMap<String, Weapon> {
    let mut defs = definitions();
    let w = defs.get_mut("weapon_shotgun").unwrap();
    w.viewmodel = "models/weapons/v_shotgun.mdl".into();
    w.sounds = [
        ("single_shot", "Weapon_Shotgun.Single"),
        ("double_shot", "Weapon_Shotgun.Double"),
        ("special1", "Weapon_Shotgun.Special1"),
        ("empty", "Weapon_Shotgun.Empty"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    defs
}
pub(super) fn impact_wall() -> World {
    let mut world = World::default();
    world.brushes.push(modkit_core::Brush {
        contents: 1,
        planes: [
            (Vec3::X, 101.),
            (-Vec3::X, -100.),
            (Vec3::Y, 50.),
            (-Vec3::Y, 50.),
            (Vec3::Z, 50.),
            (-Vec3::Z, 50.),
        ]
        .into_iter()
        .map(|(normal, distance)| modkit_core::Plane { normal, distance })
        .collect(),
    });
    world
}
#[test]
fn npc_bullets_hit_the_player_hull_before_the_wall_else_the_wall() {
    // Wall face at x = 100 (z -50..50); the shooter at x = 0 fires along +X.
    let world = impact_wall();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    physics.refresh_queries();
    let shot = crate::npc_weapons::NpcShot {
        attacker: 99,
        weapon: "weapon_pistol".into(),
        origin: Vec3::new(0., 0., 10.),
        directions: vec![Vec3::X],
        damage: 3.,
        ammo_type: "Pistol".into(),
        tracer: true,
        missile: false,
    };
    let mut uniform = || 0.;
    let mut out = NpcBullets::default();
    npc_fire_bullets(
        &world,
        &mut scene,
        &mut physics,
        &shot,
        Some(PlayerTarget {
            feet: Vec3::new(60., 0., -10.),
            crouched: false,
            yaw: 180.,
        }),
        &mut uniform,
        &mut out,
    );
    assert_eq!(out.player_hits.len(), 1);
    assert!(out.impacts.is_empty());
    assert!((out.tracers[0].2.x - 44.).abs() < 1e-3, "{:?}", out.tracers);
    // A player behind the wall (or ducked below the line) is not hit.
    let mut out = NpcBullets::default();
    npc_fire_bullets(
        &world,
        &mut scene,
        &mut physics,
        &shot,
        Some(PlayerTarget {
            feet: Vec3::new(150., 0., -10.),
            crouched: false,
            yaw: 180.,
        }),
        &mut uniform,
        &mut out,
    );
    assert!(out.player_hits.is_empty());
    assert_eq!(out.impacts.len(), 1);
    assert!((out.impacts[0].0.position.x - 100.).abs() < 0.5);
}
#[test]
fn npc_bullets_need_a_player_hitbox_inside_the_hull() {
    // One "head" hitbox on a root bone: a ray through the hull below it misses.
    let bone = modkit_core::animation::Bone {
        name: "root".into(),
        parent: None,
        bind: modkit_core::animation::Pose {
            position: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
        },
        inverse_bind: glam::Mat4::IDENTITY,
    };
    let rig = modkit_core::animation::Rig {
        bones: vec![bone],
        hitboxes: vec![modkit_core::animation::Hitbox {
            bone: 0,
            group: 0,
            mins: Vec3::new(-4., -4., 56.),
            maxs: Vec3::new(4., 4., 66.),
        }],
        ..Default::default()
    };
    let world = World {
        rigs: [(format!("{PLAYER_MODEL}#0"), rig)].into(),
        ..Default::default()
    };
    let target = PlayerTarget {
        feet: Vec3::new(100., 0., 0.),
        crouched: false,
        yaw: 90.,
    };
    let head = player_hitbox_distance(&world, target, 0., Vec3::new(0., 0., 60.), Vec3::X);
    assert!((head.unwrap().unwrap() - 96.).abs() < 1e-3);
    let chest = player_hitbox_distance(&world, target, 0., Vec3::new(0., 0., 40.), Vec3::X);
    assert_eq!(chest, Some(None));
    // Without the rig the hull stands in.
    assert_eq!(
        player_hitbox_distance(&World::default(), target, 0., Vec3::ZERO, Vec3::X),
        None
    );
}
#[test]
fn pellets_after_the_kill_add_to_the_death_force_like_multidamage() {
    // FireBullets traces all pellets first and applies one multidamage: every pellet
    // on the NPC adds its force (Buckshot BULLET_IMPULSE(400, 1200) each), and the
    // force point is the last trace attack.
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_metropolice".into()),
                ("origin".into(), "80 0 0".into()),
            ],
        }],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    scene.states[0].value = 1.;
    let mut uniform = || 0.5;
    let mut blood = Vec::new();
    let mut pellet = |scene: &mut Scene, z: f32| {
        let hit = crate::physics::RayHit {
            entity: 0,
            position: Vec3::new(70., 0., z),
            normal: -Vec3::X,
            hitgroup: crate::skill::HITGROUP_CHEST,
            bone: None,
        };
        apply_hit(
            &world,
            scene,
            &mut physics,
            8.,
            "Buckshot",
            &hit,
            Vec3::X,
            false,
            HitSource {
                player: true,
                npc: None,
                origin: Vec3::ZERO,
            },
            &mut uniform,
            &mut blood,
        )
    };
    assert!(pellet(&mut scene, 50.));
    assert!(!pellet(&mut scene, 40.));
    assert!(!pellet(&mut scene, 30.));
    let (force, at) = scene.npc_death_forces[&0];
    assert!(
        (force.x - 3. * bullet_impulse("Buckshot")).abs() < 1.,
        "{force}"
    );
    assert_eq!(at, Some(Vec3::new(70., 0., 50.)));
    // The corpse was not damaged again.
    assert!(scene.states[0].killed);
}
#[test]
fn a_shotgun_batch_that_kills_gives_the_corpse_every_pellets_force() {
    // A 40 HP cop takes seven 8-damage chest pellets in one shot: it dies on the fifth,
    // and ApplyMultiDamage's single TakeDamage carries all seven pellets' force (native
    // front-facing shotgun kill at 40 HP: 96-99 units; per-pellet deaths gave ~25).
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_metropolice".into()),
                ("origin".into(), "80 0 0".into()),
            ],
        }],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    scene.states[0].value = 40.;
    let weapon = Weapon {
        class: "weapon_shotgun".into(),
        ammo_type: "Buckshot".into(),
        damage: 8.,
        ..Default::default()
    };
    let batch = (0..7)
        .map(|i| {
            let hit = crate::physics::RayHit {
                entity: 0,
                position: Vec3::new(70., 0., 40. + i as f32),
                normal: -Vec3::X,
                hitgroup: crate::skill::HITGROUP_CHEST,
                bone: None,
            };
            (hit, Vec3::X)
        })
        .collect();
    let mut inv = Inventory::default();
    inv.hit_batch(
        &world,
        &mut scene,
        &mut physics,
        &weapon,
        batch,
        false,
        Vec3::ZERO,
    );
    assert!(scene.states[0].killed);
    let (force, at) = scene.npc_death_forces[&0];
    assert!(
        (force.x - 7. * bullet_impulse("Buckshot")).abs() < 1.,
        "{force}"
    );
    assert_eq!(at, Some(Vec3::new(70., 0., 46.)));
}
#[test]
fn a_world_pellet_between_a_targets_pellets_keeps_their_multidamage_batch() {
    // Four chest pellets, one into the wall behind (CWorld has no m_takedamage, so
    // TraceAttack adds no multidamage), three more: the 40 HP cop dies once, from one
    // ApplyMultiDamage carrying all seven pellets' force. Splitting the batch at the wall
    // left the cop at 8 HP and gave the corpse only the last three pellets.
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_metropolice".into()),
                ("origin".into(), "80 0 0".into()),
            ],
        }],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    scene.states[0].value = 40.;
    let weapon = Weapon {
        class: "weapon_shotgun".into(),
        ammo_type: "Buckshot".into(),
        damage: 8.,
        ..Default::default()
    };
    let mut inv = Inventory::default();
    let mut pending = Vec::new();
    for i in 0..8 {
        let hit = crate::physics::RayHit {
            entity: if i == 4 { usize::MAX } else { 0 },
            position: Vec3::new(if i == 4 { 300. } else { 70. }, 0., 40. + i as f32),
            normal: -Vec3::X,
            hitgroup: if i == 4 {
                0
            } else {
                crate::skill::HITGROUP_CHEST
            },
            bone: None,
        };
        inv.pellet_hit(
            &world,
            &mut scene,
            &mut physics,
            &weapon,
            &mut pending,
            (hit, Vec3::X, false, Vec3::ZERO),
        );
        assert!(
            !scene.states[0].killed,
            "pellet {i} applied the batch early"
        );
    }
    inv.hit_batch(
        &world,
        &mut scene,
        &mut physics,
        &weapon,
        pending,
        false,
        Vec3::ZERO,
    );
    assert!(scene.states[0].killed);
    let (force, at) = scene.npc_death_forces[&0];
    assert!(
        (force.x - 7. * bullet_impulse("Buckshot")).abs() < 1.,
        "{force}"
    );
    assert_eq!(at, Some(Vec3::new(70., 0., 47.)));
}
#[test]
fn zombies_halve_body_bullets_and_release_their_crabs() {
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_zombie".into()),
                ("origin".into(), "80 0 0".into()),
            ],
        }],
        ..Default::default()
    };
    let mut physics = Physics::new(&world);
    let mut uniform = || 0.5;
    let mut blood = Vec::new();
    let player = HitSource {
        player: true,
        npc: None,
        origin: Vec3::ZERO,
    };
    let shoot = |scene: &mut Scene,
                 physics: &mut Physics,
                 damage: f32,
                 ammo: &str,
                 hitgroup: i32,
                 blood: &mut Vec<_>,
                 uniform: &mut dyn FnMut() -> f32| {
        let hit = crate::physics::RayHit {
            entity: 0,
            position: Vec3::new(70., 0., 50.),
            normal: -Vec3::X,
            hitgroup,
            bone: None,
        };
        apply_hit(
            &world,
            scene,
            physics,
            damage,
            ammo,
            &hit,
            Vec3::X,
            false,
            player,
            uniform,
            blood,
        )
    };
    // 50 HP: a pistol body shot deals 5 x 0.5, a headshot 5 x 2.
    let mut scene = Scene::new(&world);
    shoot(
        &mut scene,
        &mut physics,
        5.,
        "Pistol",
        crate::skill::HITGROUP_CHEST,
        &mut blood,
        &mut uniform,
    );
    assert_eq!(scene.states[0].value, 47.5);
    shoot(
        &mut scene,
        &mut physics,
        5.,
        "Pistol",
        crate::skill::HITGROUP_HEAD,
        &mut blood,
        &mut uniform,
    );
    assert_eq!(scene.states[0].value, 37.5);
    // Buckshot: x 0.625, no halving (8 -> 5 a pellet).
    shoot(
        &mut scene,
        &mut physics,
        8.,
        "Buckshot",
        crate::skill::HITGROUP_CHEST,
        &mut blood,
        &mut uniform,
    );
    assert_eq!(scene.states[0].value, 32.5);
    // A body-shot kill lets the crab go alive.
    scene.states[0].value = 1.;
    assert!(shoot(
        &mut scene,
        &mut physics,
        5.,
        "Pistol",
        crate::skill::HITGROUP_CHEST,
        &mut blood,
        &mut uniform
    ));
    assert!(scene
        .crab_releases
        .last()
        .is_some_and(|r| r.live && r.entity == 0));
    // A weak headshot kill (10 of 50: under a quarter) keeps the crab on the corpse.
    let mut scene = Scene::new(&world);
    scene.states[0].value = 1.;
    shoot(
        &mut scene,
        &mut physics,
        5.,
        "Pistol",
        crate::skill::HITGROUP_HEAD,
        &mut blood,
        &mut uniform,
    );
    assert!(scene.crab_releases.is_empty());
    // A strong one (357: 40 x 2 of 50) throws it off as a ragdoll.
    let mut scene = Scene::new(&world);
    scene.states[0].value = 1.;
    shoot(
        &mut scene,
        &mut physics,
        40.,
        "357",
        crate::skill::HITGROUP_HEAD,
        &mut blood,
        &mut uniform,
    );
    assert!(scene.crab_releases.last().is_some_and(|r| !r.live));
}
#[test]
fn player_headshots_sneak_attack_unaware_humanoids_like_native() {
    // Native (oracle T163611Z, cop #257 at ~80 units): chest 40 -> 35, then a
    // headshot 35 -> -70 (full health x sk_npc_head 3).
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_metropolice".into()),
                ("origin".into(), "80 0 0".into()),
            ],
        }],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut uniform = || 0.5;
    let mut blood = Vec::new();
    let player = HitSource {
        player: true,
        npc: None,
        origin: Vec3::ZERO,
    };
    let mut shoot = |scene: &mut Scene, hitgroup: i32, attacker: HitSource| {
        let hit = crate::physics::RayHit {
            entity: 0,
            position: Vec3::new(70., 0., 50.),
            normal: -Vec3::X,
            hitgroup,
            bone: None,
        };
        apply_hit(
            &world,
            scene,
            &mut physics,
            5.,
            "Pistol",
            &hit,
            Vec3::X,
            false,
            attacker,
            &mut uniform,
            &mut blood,
        )
    };
    shoot(&mut scene, crate::skill::HITGROUP_CHEST, player);
    assert_eq!(scene.states[0].value, 35.);
    let killed = shoot(&mut scene, crate::skill::HITGROUP_HEAD, player);
    assert!(killed);
    assert_eq!(scene.states[0].value, -70.);
    // An NPC that knows the player, or a shot from beyond 360 units: 5 x 3.
    let mut scene = Scene::new(&world);
    scene.npc_knows_player.insert(0);
    shoot(&mut scene, crate::skill::HITGROUP_HEAD, player);
    assert_eq!(scene.states[0].value, 25.);
    let mut scene = Scene::new(&world);
    let far = HitSource {
        player: true,
        npc: None,
        origin: Vec3::new(-400., 0., 0.),
    };
    shoot(&mut scene, crate::skill::HITGROUP_HEAD, far);
    assert_eq!(scene.states[0].value, 25.);
}
#[test]
fn reload_transfers_only_available_ammunition() {
    let defs = definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_pistol", &defs, 0.);
    inv.owned.insert("weapon_pistol".into(), 12);
    inv.pistol_ammo = 3;
    inv.reload(&defs, &mut scene, &world);
    scene.time = 2.;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    assert_eq!(inv.owned["weapon_pistol"], 15);
    assert_eq!(inv.pistol_ammo, 0);
}
#[test]
fn revolver_reload_waits_and_preserves_other_ammunition() {
    let defs = definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_357", &defs, 0.);
    inv.owned.insert("weapon_357".into(), 1);
    inv.give_ammo("357", 4, &defs);
    inv.pistol_ammo = 20;
    inv.give_ammo("SMG1", 90, &defs);
    inv.reload(&defs, &mut scene, &world);
    scene.time = 3.6;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    assert_eq!(inv.owned["weapon_357"], 1);
    scene.time = 3.7;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    assert_eq!(inv.owned["weapon_357"], 5);
    assert_eq!(inv.reserve_for("weapon_357", &defs), 0);
    assert_eq!(inv.pistol_ammo, 20);
    assert_eq!(inv.reserve_for("weapon_smg1", &defs), 90);
}
#[test]
fn switching_cancels_reload_without_refilling_either_weapon() {
    let defs = definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_pistol", &defs, 0.);
    inv.owned.insert("weapon_pistol".into(), 1);
    inv.pistol_ammo = 20;
    inv.reload(&defs, &mut scene, &world);
    inv.give("weapon_ar2", &defs, 0.3);
    inv.owned.insert("weapon_ar2".into(), 3);
    inv.give_ammo("AR2", 10, &defs);
    scene.time = 4.;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    assert_eq!(inv.owned["weapon_ar2"], 3);
    assert_eq!(inv.reserve_for("weapon_ar2", &defs), 10);
    assert_eq!(inv.owned["weapon_pistol"], 1);
    assert_eq!(inv.pistol_ammo, 20);
    assert!(!inv.is_reloading());
}
#[test]
fn shotgun_inserts_one_shell_and_retains_it_when_interrupted() {
    let defs = definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut inv = Inventory::default();
    let mut physics = Physics::new(&world);
    inv.give("weapon_shotgun", &defs, 0.);
    inv.owned.insert("weapon_shotgun".into(), 0);
    inv.give_ammo("Buckshot", 3, &defs);
    inv.reload(&defs, &mut scene, &world);
    scene.time = 0.49;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    assert_eq!(inv.owned["weapon_shotgun"], 0);
    scene.time = 0.5;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    assert_eq!(inv.owned["weapon_shotgun"], 1);
    assert_eq!(inv.reserve_for("weapon_shotgun", &defs), 2);
    assert_eq!(inv.animation, "reload2");
    scene.time = 0.6;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert!(inv.delayed_attack);
    assert_eq!(inv.shots, 0);
    scene.time = 0.9;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 1);
    assert_eq!(inv.bullets, 7);
    assert_eq!(inv.owned["weapon_shotgun"], 0);
    assert_eq!(inv.reserve_for("weapon_shotgun", &defs), 2);
}
#[test]
fn automatic_cadence_catches_up_without_an_idle_burst() {
    let defs = definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut inv = Inventory::default();
    let mut physics = Physics::new(&world);
    inv.give("weapon_smg1", &defs, 0.);
    scene.time = 1.;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    // After three float32 0.075 intervals, rather than just before the rounded deadline.
    scene.time = 1.226;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 4);
    assert_eq!(inv.owned["weapon_smg1"], 41);
    scene.time = 10.;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    scene.time = 20.;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 5);
    assert_eq!(inv.animation, "fire01");
}
fn automatic_definitions() -> BTreeMap<String, Weapon> {
    let mut defs = definitions();
    for (class, sound) in [
        ("weapon_smg1", "Weapon_SMG1.Empty"),
        ("weapon_ar2", "Weapon_AR2.Empty"),
    ] {
        defs.get_mut(class)
            .unwrap()
            .sounds
            .insert("empty".into(), sound.into());
    }
    defs
}
#[test]
fn automatic_empty_trigger_clicks_before_reloading_while_held() {
    let defs = automatic_definitions();
    let world = World::default();
    for class in ["weapon_smg1", "weapon_ar2"] {
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give(class, &defs, 0.);
        inv.owned.insert(class.into(), 0);
        inv.give_ammo(&defs[class].ammo_type, 5, &defs);
        let draw_deadline = inv.next_attack;
        let draw_animation = inv.animation.clone();

        scene.time = 1.;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert!(!inv.is_reloading());
        assert_eq!(
            scene.sounds,
            [crate::sounds::SoundRequest::from(
                defs[class].sounds["empty"].clone()
            )]
        );
        assert_eq!(inv.next_attack, draw_deadline);
        assert_eq!(inv.animation, draw_animation);
        assert_eq!(inv.animation_at, 0.);

        scene.time = 1.015;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert!(inv.is_reloading());
        let reload_animation = if class == "weapon_ar2" {
            "ir_reload"
        } else {
            "reload"
        };
        assert_eq!(inv.animation, reload_animation);
        let completion = 1.015 + fallback_duration(class, reload_animation);
        assert_eq!(inv.next_attack, completion);
        assert_eq!(inv.shots, 0);

        scene.time = completion - 0.001;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        assert_eq!(inv.owned[class], 0);
        assert_eq!(inv.reserve_for(class, &defs), 5);
        scene.time = completion;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert!(!inv.is_reloading());
        assert_eq!(inv.owned[class], 4);
        assert_eq!(inv.reserve_for(class, &defs), 0);
        assert_eq!(inv.shots, 1);
    }
}
#[test]
fn automatic_empty_sound_throttle_survives_trigger_release() {
    let defs = automatic_definitions();
    let world = World::default();
    for class in ["weapon_smg1", "weapon_ar2"] {
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give(class, &defs, 0.);
        inv.owned.insert(class.into(), 0);
        let draw_deadline = inv.next_attack;
        let draw_animation = inv.animation.clone();
        for (time, clicks) in [(1., 1), (1.1, 1), (1.3, 1), (1.5, 1), (1.5001, 2)] {
            scene.time = time;
            inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
            inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
            inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
            assert_eq!(scene.sounds.len(), clicks);
            assert_eq!(inv.next_attack, draw_deadline);
            assert_eq!(inv.animation, draw_animation);
            assert_eq!(inv.shots, 0);
            assert!(!inv.is_reloading());
        }
    }
}
#[test]
fn automatic_idle_reload_requires_deadline_to_have_elapsed() {
    let defs = definitions();
    let world = World::default();
    for class in ["weapon_smg1", "weapon_ar2"] {
        let mut scene = Scene::new(&world);
        let mut inv = Inventory::default();
        inv.give(class, &defs, 0.);
        inv.owned.insert(class.into(), 0);
        inv.give_ammo(&defs[class].ammo_type, 5, &defs);
        scene.time = inv.next_attack;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert!(!inv.is_reloading());
        scene.time += 0.015;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert!(inv.is_reloading());
        assert_eq!(inv.animation_at, scene.time);
    }
}
#[test]
fn automatic_empty_sound_is_per_weapon_and_survives_switches() {
    let defs = automatic_definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_smg1", &defs, 0.);
    inv.owned.insert("weapon_smg1".into(), 0);
    inv.owned.insert("weapon_ar2".into(), 0);
    scene.time = 1.;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    // Keep draw deadlines out of this sound-state comparison. Ordinary
    // switching still sets its recorded draw duration.
    inv.give("weapon_ar2", &defs, 0.);
    scene.time = 1.1;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(scene.sounds, ["Weapon_SMG1.Empty", "Weapon_AR2.Empty"]);
    inv.give("weapon_smg1", &defs, 0.);
    scene.time = 1.2;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(scene.sounds.len(), 2);
}
#[test]
fn revolver_cooldown_and_circular_spread_bounds() {
    let defs = definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut inv = Inventory::default();
    let mut physics = Physics::new(&world);
    inv.give("weapon_357", &defs, 0.);
    for time in [1., 1.1, 1.74, 1.75] {
        scene.time = time;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    }
    assert_eq!(inv.shots, 2);
    for direction in [Vec3::X, Vec3::Z] {
        for _ in 0..1000 {
            let spread = inv.spread(direction, cone(10.));
            assert!(spread.is_finite());
            assert!(spread.angle_between(direction) <= cone(10.).atan() + 1e-5);
        }
    }
}
#[test]
fn full_reserve_leaves_pickup_available_and_types_independent() {
    let defs = definitions();
    let mut world = World::default();
    world.entities.push(modkit_core::Entity {
        properties: vec![
            ("classname".into(), "item_ammo_ar2".into()),
            ("origin".into(), "0 0 24".into()),
        ],
    });
    let mut scene = Scene::new(&world);
    let mut inv = Inventory::default();
    assert_eq!(inv.give_ammo("AR2", 100, &defs), 60);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    assert!(!scene.states[0].killed);
    inv.set_ammo("AR2", 55);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    assert!(scene.states[0].killed);
    assert_eq!(inv.reserve_for("weapon_ar2", &defs), 60);
    assert_eq!(inv.reserve_for("weapon_pistol", &defs), 0);
}

#[test]
fn a_dropped_weapon_keeps_its_clip_when_equipped() {
    // An NPC's SMG1 drop carries 25 rounds (Weapon_Drop); Weapon_Equip keeps them.
    let defs = definitions();
    let mut world = World::default();
    world.entities.push(modkit_core::Entity {
        properties: vec![
            ("classname".into(), "weapon_smg1".into()),
            ("origin".into(), "0 0 24".into()),
            ("ammo".into(), "25".into()),
        ],
    });
    let mut scene = Scene::new(&world);
    let mut inv = Inventory::default();
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    assert!(scene.states[0].killed);
    assert_eq!(inv.owned.get("weapon_smg1"), Some(&25));
}
#[test]
fn secondary_ammo_pickups_clamp_independently_and_full_pickups_remain() {
    let defs = definitions();
    let mut inv = Inventory::default();
    assert_eq!(inv.give_ammo("smg1_grenade", 10, &defs), 3);
    assert_eq!(inv.secondary_for("weapon_smg1", &defs), 3);
    assert_eq!(inv.secondary_for("weapon_ar2", &defs), 0);
    assert_eq!(inv.reserve_for("weapon_smg1", &defs), 0);
    let mut world = World::default();
    world.entities.push(modkit_core::Entity {
        properties: vec![
            ("classname".into(), "item_ammo_smg1_grenade".into()),
            ("origin".into(), "0 0 24".into()),
        ],
    });
    let mut scene = Scene::new(&world);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    assert!(!scene.states[0].killed);
    inv.set_ammo("SMG1_Grenade", 2);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    assert!(scene.states[0].killed);
    assert_eq!(inv.secondary_for("weapon_smg1", &defs), 3);
}

#[test]
fn developer_loadout_grants_native_secondary_quantities_and_draw_blocks_launch() {
    let mut defs = definitions();
    // Preserve the native fixed GiveAmmo amounts even if custom carry limits are larger.
    defs.get_mut("weapon_ar2").unwrap().secondary_ammo_max = 8;
    defs.get_mut("weapon_smg1").unwrap().secondary_ammo_max = 7;
    let mut inv = Inventory::default();
    inv.refill_ammo(&defs);
    assert_eq!(inv.secondary_for("weapon_smg1", &defs), 3);
    assert_eq!(inv.secondary_for("weapon_ar2", &defs), 5);
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    for class in ["weapon_smg1", "weapon_ar2"] {
        inv.give(class, &defs, 0.);
        let ammo = inv.secondary_for(class, &defs);
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.secondary_for(class, &defs), ammo);
        assert_eq!(inv.shots, 0);
    }
    assert_eq!(inv.reserve_for("weapon_smg1", &defs), 225);
    assert_eq!(inv.reserve_for("weapon_ar2", &defs), 60);
}

fn projectile_definitions() -> BTreeMap<String, Weapon> {
    let mut defs = automatic_definitions();
    let smg = defs.get_mut("weapon_smg1").unwrap();
    smg.secondary_damage = 100.;
    smg.secondary_radius = 250.;
    smg.sounds
        .insert("double_shot".into(), "Weapon_SMG1.Double".into());
    let ar2 = defs.get_mut("weapon_ar2").unwrap();
    ar2.secondary_radius = 10.;
    ar2.secondary_mass = 150.;
    ar2.secondary_lifetime = 2.;
    ar2.sounds
        .insert("special1".into(), "Weapon_CombineGuard.Special1".into());
    ar2.sounds
        .insert("double_shot".into(), "Weapon_IRifle.Single".into());
    defs
}
#[test]
fn smg_grenade_has_separate_ammo_and_deadlines_without_hitscan_damage() {
    let defs = projectile_definitions();
    let world = impact_wall();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_smg1", &defs, 0.);
    inv.refill_ammo(&defs);
    scene.time = 1.;
    inv.secondary_attack(
        &defs,
        &world,
        &mut scene,
        &mut physics,
        Vec3::Z * 64.,
        Vec3::X,
    );
    assert_eq!(inv.owned["weapon_smg1"], 45);
    assert_eq!(inv.secondary_for("weapon_smg1", &defs), 2);
    assert_eq!(inv.shots, 1);
    assert_eq!(inv.bullets, 0);
    assert!(inv.impacts.is_empty());
    assert_eq!(inv.animation, "altfire");
    assert_eq!(scene.sounds, ["Weapon_SMG1.Double"]);
    assert_eq!(inv.projectile_spawns.len(), 1);
    assert_eq!(inv.projectile_spawns[0].velocity, Vec3::X * 1000.);
    for time in [1.015, 1.49, 1.5, 1.999] {
        scene.time = time;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    }
    assert_eq!(inv.projectile_spawns.len(), 1);
    scene.time = 1.5;
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.owned["weapon_smg1"], 44);
    scene.time = 2.;
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.projectile_spawns.len(), 2);
    assert_eq!(inv.secondary_for("weapon_smg1", &defs), 1);
}
#[test]
fn smg_grenade_cancels_unfinished_magazine_reload_and_keeps_reserves() {
    let defs = projectile_definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_smg1", &defs, 0.);
    inv.owned.insert("weapon_smg1".into(), 0);
    inv.give_ammo("SMG1", 45, &defs);
    inv.give_ammo("SMG1_Grenade", 1, &defs);
    scene.time = 1.;
    inv.reload(&defs, &mut scene, &world);
    assert!(inv.is_reloading());
    scene.time = 1.1;
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert!(!inv.is_reloading());
    assert_eq!(inv.owned["weapon_smg1"], 0);
    assert_eq!(inv.reserve_for("weapon_smg1", &defs), 45);
    assert_eq!(inv.secondary_for("weapon_smg1", &defs), 0);
    scene.time = 2.5;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    assert_eq!(inv.owned["weapon_smg1"], 0);
    assert_eq!(inv.reserve_for("weapon_smg1", &defs), 45);
}
#[test]
fn empty_projectile_secondary_uses_its_own_half_second_deadline() {
    let defs = projectile_definitions();
    let world = World::default();
    for class in ["weapon_smg1", "weapon_ar2"] {
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give(class, &defs, 0.);
        let primary_deadline = inv.next_attack;
        scene.time = 1.;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        scene.time = 1.49;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(scene.sounds.len(), 1);
        assert_eq!(inv.animation, "dryfire");
        assert_eq!(inv.next_attack, primary_deadline);
        assert_eq!(inv.next_secondary[class], 1.5);
        assert!(inv.projectile_spawns.is_empty());
        assert!(inv.can_holster());
    }
}
#[test]
fn ar2_charge_uses_current_aim_after_strict_deadline_and_vetoes_holster_reload() {
    let defs = projectile_definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_ar2", &defs, 0.);
    inv.refill_ammo(&defs);
    scene.time = 1.;
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.animation, "shake");
    assert_eq!(inv.charge_until(), Some(1.5));
    assert_eq!(inv.secondary_for("weapon_ar2", &defs), 3);
    assert!(!inv.can_holster());
    inv.give("weapon_pistol", &defs, 1.1);
    assert!(inv.owned.contains_key("weapon_pistol"));
    assert_eq!(inv.active, "weapon_ar2");
    inv.owned.insert("weapon_ar2".into(), 20);
    inv.reload(&defs, &mut scene, &world);
    assert!(!inv.is_reloading());
    scene.time = 1.5;
    inv.advance_projectile_fire(&world, &mut scene, &defs, Vec3::Y * 10., Vec3::Y);
    assert!(inv.projectile_spawns.is_empty());
    scene.time = 1.515;
    inv.set_attack_input(false, false);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
    inv.advance_projectile_fire(&world, &mut scene, &defs, Vec3::Y * 10., Vec3::Y);
    assert!(inv.can_holster());
    assert_eq!(inv.projectile_spawns.len(), 1);
    assert_eq!(inv.projectile_spawns[0].position, Vec3::Y * 10.);
    assert_eq!(inv.projectile_spawns[0].velocity, Vec3::Y * 1000.);
    assert_eq!(inv.projectile_spawns[0].radius, 10.);
    assert_eq!(inv.projectile_spawns[0].lifetime, 2.);
    assert_eq!(inv.projectile_spawns[0].mass, 150.);
    assert_eq!(inv.secondary_for("weapon_ar2", &defs), 2);
    assert_eq!(inv.owned["weapon_ar2"], 20);
    assert_eq!(inv.shots, 1);
    assert_eq!(inv.bullets, 0);
    assert_eq!(inv.animation, "ir_fire2");
    assert_eq!(
        scene.sounds,
        ["Weapon_CombineGuard.Special1", "Weapon_IRifle.Single"]
    );
    inv.advance_projectile_fire(&world, &mut scene, &defs, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.projectile_spawns.len(), 1);
    scene.time = 2.015;
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.owned["weapon_ar2"], 20);
    scene.time = inv.owner_attack_until;
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert!(inv.owned["weapon_ar2"] < 20);
}
#[test]
fn ar2_does_not_cancel_magazine_reload_for_a_secondary_charge() {
    let defs = projectile_definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_ar2", &defs, 0.);
    inv.refill_ammo(&defs);
    inv.owned.insert("weapon_ar2".into(), 10);
    scene.time = 1.;
    inv.reload(&defs, &mut scene, &world);
    scene.time = 1.1;
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert!(inv.is_reloading());
    assert_eq!(inv.charge_until(), None);
    assert_eq!(inv.secondary_for("weapon_ar2", &defs), 3);
    assert!(inv.projectile_spawns.is_empty());
}
#[test]
fn projectile_damage_applies_blast_armor_and_lethal_outputs_once() {
    use crate::projectiles::{Damage, DamageTarget};
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_metropolice".into()),
                ("health".into(), "90".into()),
            ],
        }],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut inv = Inventory {
        armor: 50.,
        ..Default::default()
    };
    inv.apply_projectile_damage(
        vec![Damage {
            target: DamageTarget::Player,
            amount: 100.,
            kind: crate::player_damage::DMG_BLAST,
            dissolve: false,
            direction: Vec3::X,
            origin: Vec3::X,
            attacker: Attacker::None,
        }],
        &world,
        &mut scene,
        &mut physics,
    );
    let blast = scene.player_damage.pop().unwrap();
    assert_eq!(
        (blast.amount, blast.kind),
        (100., crate::player_damage::DMG_BLAST)
    );
    crate::player_damage::PlayerDamage::default().take(&mut inv, blast);
    assert_eq!(inv.armor, 0.);
    assert_eq!(inv.health, 50.);
    let dissolve = Damage {
        target: DamageTarget::Entity(0),
        amount: 0.,
        kind: crate::player_damage::DMG_DISSOLVE,
        dissolve: true,
        direction: Vec3::X,
        origin: Vec3::ZERO,
        attacker: Attacker::Player,
    };
    inv.apply_projectile_damage(vec![dissolve, dissolve], &world, &mut scene, &mut physics);
    assert!(scene.states[0].killed);
    assert!(!scene.states[0].visible);
    assert_eq!((inv.hits, inv.kills), (1, 1));
}

#[test]
fn animation_sounds_suppress_only_the_same_symbolic_engine_sound() {
    let mut weapon = Weapon {
        viewmodel: "test".into(),
        ..Default::default()
    };
    weapon
        .sounds
        .insert("reload".into(), "Weapon_AR2.Reload".into());
    let mut world = World::default();
    let mut rig = modkit_core::animation::Rig::default();
    rig.clips.insert(
        "ir_reload".into(),
        modkit_core::animation::Clip {
            layer: Default::default(),
            fps: 30.,
            looping: false,
            frames: Vec::new(),
            events: vec![modkit_core::animation::ClipEvent {
                cycle: 0.5,
                id: 5004,
                flags: 0,
                name: String::new(),
                options: "Weapon_AR2.Reload_Rotate".into(),
            }],
        },
    );
    world.rigs.insert("test#0".into(), rig);
    let mut scene = Scene::new(&world);
    play_sound(&mut scene, &weapon, "reload", &world, "ir_reload");
    assert_eq!(scene.sounds, ["Weapon_AR2.Reload"]);
    scene.sounds.clear();
    world
        .rigs
        .get_mut("test#0")
        .unwrap()
        .clips
        .get_mut("ir_reload")
        .unwrap()
        .events[0]
        .options = "Weapon_AR2.Reload".into();
    play_sound(&mut scene, &weapon, "reload", &world, "ir_reload");
    assert!(scene.sounds.is_empty());
}
#[test]
#[ignore = "requires an owned HL2 installation in HL2_ROOT"]
fn owned_crowbar_attack_transition_is_smooth_without_twitch_or_hop() {
    let vfs = source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
    let weapons = crate::gameplay::definitions(&vfs).unwrap();
    let w = &weapons["weapon_crowbar"];
    let mut world = World::default();
    crate::actors::prepare_weapons(&mut world, &vfs, &weapons).unwrap();
    let rig = &world.rigs[&format!("{}#0", w.viewmodel.to_lowercase())];

    let mut inv = Inventory {
        active: "weapon_crowbar".into(),
        ..Default::default()
    };
    inv.animate("misscenter1", 0.0);

    let mut prev_hand: Option<Vec3> = None;
    let mut prev_tip: Option<Vec3> = None;
    let mut max_hand_step_delta = 0.0f32;
    let mut max_tip_step_delta = 0.0f32;

    // Sample in 20ms steps across the 0.667s end of swing and through the 0.2s fadeout window to 1.0s.
    // During the transition (t >= 0.66s), the ending attack sequence smoothly cross-fades into idle.
    for step in 33..=50 {
        let t = step as f64 * 0.02;
        let matrices = inv.viewmodel_matrices(rig, t);
        let hand = matrices[8].transform_point3(Vec3::ZERO);
        let tip = matrices[12].transform_point3(Vec3::ZERO);
        if let (Some(ph), Some(pt)) = (prev_hand, prev_tip) {
            let dh = (hand - ph).length();
            let dt = (tip - pt).length();
            if dh > max_hand_step_delta {
                max_hand_step_delta = dh;
            }
            if dt > max_tip_step_delta {
                max_tip_step_delta = dt;
            }
        }
        prev_hand = Some(hand);
        prev_tip = Some(tip);
    }

    // Without blending, hand.y jumped by >10.6 units across a single 20ms step (0.66s to 0.68s, dh = 10.96).
    // With smoothstep cross-fade into idle, consecutive 20ms step deltas remain continuous and smooth (< 2.0 units).
    assert!(
        max_hand_step_delta < 2.0,
        "hand step delta spiked: {max_hand_step_delta:.2}"
    );
    assert!(
        max_tip_step_delta < 2.0,
        "tip step delta spiked: {max_tip_step_delta:.2}"
    );
}

#[test]
fn friends_refuse_the_damage_of_attackers_that_like_them() {
    use crate::projectiles::{Damage, DamageTarget};
    let entity = |class: &str| modkit_core::Entity {
        properties: vec![("classname".into(), class.into())],
    };
    // 0 citizen (rebel: PlayerAlly), 1 combine soldier, 2 citizen, 3 metrocop.
    let world = World {
        entities: vec![
            entity("npc_citizen"),
            entity("npc_combine_s"),
            entity("npc_citizen"),
            entity("npc_metropolice"),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    assert!(damage_refused(&world, &scene, 0, Attacker::Player));
    assert!(damage_refused(&world, &scene, 0, Attacker::Entity(2)));
    assert!(!damage_refused(&world, &scene, 0, Attacker::Entity(1)));
    assert!(!damage_refused(&world, &scene, 0, Attacker::Entity(0)));
    assert!(!damage_refused(&world, &scene, 0, Attacker::None));
    assert!(!damage_refused(&world, &scene, 3, Attacker::Player));
    scene.player_hurts_friendlies = true;
    assert!(!damage_refused(&world, &scene, 0, Attacker::Player));
    assert!(damage_refused(&world, &scene, 0, Attacker::Entity(2)));
    scene.player_hurts_friendlies = false;
    // A dead attacker's handle is gone: its late grenade is nobody's.
    scene.states[2].killed = true;
    assert!(!damage_refused(&world, &scene, 0, Attacker::Entity(2)));
    // The player's frag blast leaves the citizen untouched; a Combine one hurts it.
    let mut physics = Physics::new(&world);
    let blast = |attacker| Damage {
        target: DamageTarget::Entity(0),
        amount: 30.,
        kind: crate::player_damage::DMG_BLAST,
        dissolve: false,
        direction: Vec3::X,
        origin: Vec3::ZERO,
        attacker,
    };
    let mut inventory = Inventory::default();
    inventory.apply_projectile_damage(
        vec![blast(Attacker::Player)],
        &world,
        &mut scene,
        &mut physics,
    );
    assert_eq!(scene.states[0].value, 0.);
    assert_eq!(inventory.hits, 0);
    inventory.apply_projectile_damage(
        vec![blast(Attacker::Entity(1))],
        &world,
        &mut scene,
        &mut physics,
    );
    assert!(scene.states[0].value > 0. && inventory.hits == 1);
}

#[test]
fn exploding_props_credit_their_last_attacker_or_recent_physics_attacker() {
    use crate::projectiles::{Damage, DamageTarget};
    let barrel = |name: &str| modkit_core::Entity {
        properties: [
            ("classname", "prop_physics"),
            ("health", "10"),
            ("ExplodeDamage", "100"),
            ("targetname", name),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
    };
    let world = World {
        entities: vec![
            barrel("a"),
            barrel("b"),
            barrel("c"),
            modkit_core::Entity {
                properties: vec![("classname".into(), "npc_combine_s".into())],
            },
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let hit = |id, amount, attacker| Damage {
        target: DamageTarget::Entity(id),
        amount,
        kind: crate::player_damage::DMG_CRUSH,
        dissolve: false,
        direction: Vec3::X,
        origin: Vec3::ZERO,
        attacker,
    };
    // a: the player shot it, then it broke on the world: the player's explosion.
    // b: the player dropped it 1.5 s before it broke on the world (within 2 s).
    // c: an NPC shot it last, after the player's drop.
    scene.prop_physgun_drop(&world, 1);
    scene.prop_physgun_drop(&world, 2);
    assert_eq!(scene.physics_attacker(1, 1.), Attacker::Player);
    scene.time = 1.5;
    assert_eq!(scene.physics_attacker(1, 1.), Attacker::None);
    let mut inventory = Inventory::default();
    inventory.apply_projectile_damage(
        vec![
            hit(0, 5., Attacker::Player),
            hit(0, 20., Attacker::None),
            hit(1, 20., Attacker::None),
            hit(2, 5., Attacker::Entity(3)),
            hit(2, 20., Attacker::None),
        ],
        &world,
        &mut scene,
        &mut physics,
    );
    let attackers: Vec<_> = scene.prop_explosions.iter().map(|e| e.4).collect();
    assert_eq!(
        attackers,
        [Attacker::Player, Attacker::Player, Attacker::Entity(3)]
    );
}
