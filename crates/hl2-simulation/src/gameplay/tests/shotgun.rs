//! The shotgun's primary and secondary fire, pump and latches.
use super::super::*;
use super::{definitions, impact_wall, shotgun_definitions};

#[test]
fn shotgun_primary_has_seven_impacts_with_an_exact_first_pellet() {
    let defs = definitions();
    let mut world = World::default();
    world.brushes.push(modkit_core::Brush {
        contents: 1,
        planes: vec![
            modkit_core::Plane {
                normal: Vec3::X,
                distance: 101.,
            },
            modkit_core::Plane {
                normal: -Vec3::X,
                distance: -100.,
            },
            modkit_core::Plane {
                normal: Vec3::Y,
                distance: 50.,
            },
            modkit_core::Plane {
                normal: -Vec3::Y,
                distance: 50.,
            },
            modkit_core::Plane {
                normal: Vec3::Z,
                distance: 50.,
            },
            modkit_core::Plane {
                normal: -Vec3::Z,
                distance: 50.,
            },
        ],
    });
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    physics.tick(0.015);
    let mut inv = Inventory::default();
    inv.give("weapon_shotgun", &defs, 0.);
    scene.time = 1.;
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 1);
    assert_eq!(inv.bullets, 7);
    assert_eq!(inv.impacts.len(), 7);
    assert!(inv.impacts[0].0.position.distance(Vec3::X * 100.) < 0.001);
    assert!(inv.impacts[1..]
        .iter()
        .any(|(hit, _, _)| hit.position.y.abs() > 0.1));
    assert_eq!(inv.owned["weapon_shotgun"], 5);
}

#[test]
fn shotgun_fire_is_blocked_until_fire_and_pump_animations_finish() {
    let defs = definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_shotgun", &defs, 0.);
    scene.time = 1.;
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    scene.time = 1.334;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    assert_eq!(inv.animation, "pump");
    scene.time = 1.8;
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 1);
    scene.time = 1.868;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 2);
    assert_eq!(inv.owned["weapon_shotgun"], 4);
}

#[test]
fn shotgun_secondary_consumes_two_shells_and_spreads_all_twelve_pellets() {
    let mut defs = shotgun_definitions();
    // The secondary count is fixed independently of the primary pellet cvar.
    defs.get_mut("weapon_shotgun").unwrap().pellets = 3;
    let world = impact_wall();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    physics.tick(0.015);
    let mut inv = Inventory::default();
    inv.give("weapon_shotgun", &defs, 0.);
    inv.give_ammo("Buckshot", 10, &defs);
    scene.time = 1.;
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.owned["weapon_shotgun"], 4);
    assert_eq!(inv.reserve_for("weapon_shotgun", &defs), 10);
    assert_eq!(inv.shots, 1);
    assert_eq!(inv.bullets, 12);
    assert_eq!(inv.impacts.len(), 12);
    assert!(inv.impacts[0].0.position.distance(Vec3::X * 100.) > 0.1);
    assert_eq!(inv.animation, "altfire");
    assert_eq!(scene.sounds, ["Weapon_Shotgun.Double"]);
    assert!(inv.shotgun_need_pump);
    assert_eq!(inv.next_attack, 1.5);
}

#[test]
fn shotgun_secondary_with_one_shell_uses_primary_and_preserves_primary_latch() {
    let defs = shotgun_definitions();
    let world = impact_wall();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    physics.tick(0.015);
    let mut inv = Inventory::default();
    inv.give("weapon_shotgun", &defs, 0.);
    inv.owned.insert("weapon_shotgun".into(), 1);
    inv.delayed_attack = true;
    inv.delayed_secondary_attack = true;
    scene.time = 1.;
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.owned["weapon_shotgun"], 0);
    assert_eq!(inv.shots, 1);
    assert_eq!(inv.bullets, 7);
    assert!(inv.impacts[0].0.position.distance(Vec3::X * 100.) < 0.001);
    assert_eq!(inv.animation, "fire01");
    assert_eq!(scene.sounds, ["Weapon_Shotgun.Single"]);
    assert!(!inv.shotgun_need_pump);
    assert!(inv.delayed_attack);
    assert!(!inv.delayed_secondary_attack);
    assert!((inv.next_attack - (1. + 10. / 30.)).abs() < 1e-8);
}

#[test]
fn shotgun_secondary_interrupt_waits_for_two_shells_and_retains_insertion_deadline() {
    let defs = shotgun_definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_shotgun", &defs, 0.);
    inv.owned.insert("weapon_shotgun".into(), 0);
    inv.give_ammo("Buckshot", 4, &defs);
    inv.reload(&defs, &mut scene, &world);
    for time in [0.5, 0.6, 0.9] {
        scene.time = time;
        inv.set_attack_input(false, true);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert!(inv.is_reloading());
        assert!(!inv.delayed_secondary_attack);
        assert_eq!(inv.shots, 0);
    }
    assert_eq!(inv.owned["weapon_shotgun"], 2);
    let insertion_end = inv.next_attack;
    scene.time = 1.;
    inv.set_attack_input(false, true);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert!(!inv.is_reloading());
    assert!(inv.delayed_secondary_attack);
    assert!(!inv.delayed_attack);
    assert_eq!(inv.next_attack, insertion_end);
    assert_eq!(inv.shots, 0);
    // Released buttons do not lose the queued shot or cancel its deadline.
    scene.time = insertion_end - 0.001;
    inv.set_attack_input(false, false);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 0);
    scene.time = insertion_end;
    inv.set_attack_input(false, false);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 1);
    assert_eq!(inv.bullets, 12);
    assert_eq!(inv.owned["weapon_shotgun"], 0);
    assert_eq!(inv.reserve_for("weapon_shotgun", &defs), 2);
    assert!(!inv.delayed_secondary_attack);
    assert!(!inv.shotgun_need_pump);
}

#[test]
fn shotgun_secondary_and_pump_wait_for_loaded_clip_durations() {
    let defs = shotgun_definitions();
    let mut world = World::default();
    let mut rig = modkit_core::animation::Rig::default();
    // Different fixture durations prove that loaded model clips override fallbacks.
    for (name, frame_count) in [("altfire", 26), ("pump", 11)] {
        rig.clips.insert(
            name.into(),
            modkit_core::animation::Clip {
                layer: Default::default(),
                fps: 25.,
                looping: false,
                frames: vec![Vec::new(); frame_count],
                events: Vec::new(),
            },
        );
    }
    world
        .rigs
        .insert("models/weapons/v_shotgun.mdl#0".into(), rig);
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_shotgun", &defs, 0.);
    scene.time = 1.;
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.next_attack, 2.);
    scene.time = 1.999;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.animation, "altfire");
    assert_eq!(inv.shots, 1);
    scene.time = 2.;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    assert_eq!(inv.animation, "pump");
    let pump_end = inv.next_attack;
    assert!((pump_end - 2.4).abs() < 1e-6);
    scene.time = pump_end - 0.001;
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 1);
    scene.time = pump_end;
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 2);
    assert_eq!(inv.owned["weapon_shotgun"], 2);
    assert_eq!(
        scene.sounds,
        [
            "Weapon_Shotgun.Double",
            "Weapon_Shotgun.Special1",
            "Weapon_Shotgun.Double"
        ]
    );
}

#[test]
fn shotgun_both_buttons_latch_primary_but_dispatch_secondary_first() {
    let defs = shotgun_definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_shotgun", &defs, 0.);
    inv.owned.insert("weapon_shotgun".into(), 3);
    inv.give_ammo("Buckshot", 3, &defs);
    scene.time = 1.;
    inv.reload(&defs, &mut scene, &world);
    scene.time = 1.1;
    inv.set_attack_input(true, true);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    assert!(inv.delayed_attack);
    assert!(!inv.delayed_secondary_attack);
    assert!(!inv.is_reloading());
    assert_eq!(inv.next_attack, 1.5);
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 0);
    scene.time = 1.5;
    inv.set_attack_input(true, true);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    // The host dispatches secondary first even though reload latched primary.
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 1);
    assert_eq!(inv.bullets, 12);
    assert_eq!(inv.owned["weapon_shotgun"], 1);
    assert!(inv.delayed_attack);
    scene.time = 2.;
    inv.set_attack_input(false, false);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.animation, "pump");
    assert_eq!(inv.shots, 1);
    scene.time = inv.next_attack;
    inv.set_attack_input(false, false);
    inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
    inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 2);
    assert_eq!(inv.bullets, 19);
    assert_eq!(inv.owned["weapon_shotgun"], 0);
    assert!(!inv.delayed_attack);
}

#[test]
fn shotgun_empty_secondary_dry_fires_and_switching_clears_both_latches() {
    let defs = shotgun_definitions();
    let world = World::default();
    let mut scene = Scene::new(&world);
    let mut physics = Physics::new(&world);
    let mut inv = Inventory::default();
    inv.give("weapon_shotgun", &defs, 0.);
    inv.owned.insert("weapon_shotgun".into(), 0);
    inv.delayed_secondary_attack = true;
    scene.time = 1.;
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.shots, 0);
    assert_eq!(inv.animation, "dryfire");
    assert_eq!(scene.sounds, ["Weapon_Shotgun.Empty"]);
    assert!(!inv.delayed_secondary_attack);
    inv.delayed_attack = true;
    inv.delayed_secondary_attack = true;
    inv.set_attack_input(false, true);
    inv.give("weapon_pistol", &defs, 1.1);
    assert!(!inv.delayed_attack);
    assert!(!inv.delayed_secondary_attack);
    assert!(inv.attack_input.is_none());
    scene.time = 3.;
    inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
    assert_eq!(inv.owned["weapon_pistol"], 18);
    assert_eq!(inv.shots, 0);
}
