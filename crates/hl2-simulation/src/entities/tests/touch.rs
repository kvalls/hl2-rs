//! Per-entity trigger touches (SDK CBaseTrigger::StartTouch/EndTouch).
use super::*;

const FAR: Vec3 = Vec3::new(500., 0., 0.);

/// A 16-unit trigger cube at the origin whose touch outputs go to the targets st, sta,
/// et and eta far in the future, so each tick's fired outputs stay queued.
fn touch_world(class: &str, spawnflags: &str, npcs: usize) -> World {
    let output = |target: &str| format!("{target},Trigger,,1000,-1");
    let mut entities = vec![entity(
        class,
        "zone",
        &[
            ("model", "*1"),
            ("spawnflags", spawnflags),
            ("OnStartTouch", &output("st")),
            ("OnStartTouchAll", &output("sta")),
            ("OnEndTouch", &output("et")),
            ("OnEndTouchAll", &output("eta")),
        ],
    )];
    for n in 0..npcs {
        let origin = format!("{} 0 0", 500 + 100 * n);
        entities.push(entity("npc_citizen", "walker", &[("origin", &origin)]));
    }
    World {
        entities,
        brush_models: vec![modkit_core::BrushModel {
            id: 1,
            brushes: vec![modkit_core::Brush {
                planes: [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
                    .into_iter()
                    .map(|normal| modkit_core::Plane {
                        normal,
                        distance: 8.,
                    })
                    .collect(),
                contents: 1,
            }],
            mins: Vec3::splat(-8.),
            maxs: Vec3::splat(8.),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// One tick with the player's feet at `feet`; the touch outputs it fired as
/// (target, activator), in firing order.
fn touch_tick(scene: &mut Scene, world: &World, feet: Vec3) -> Vec<(String, usize)> {
    scene.tick(world, feet, 0.015);
    let mut fired: Vec<_> = std::mem::take(&mut scene.queue)
        .into_iter()
        .filter(|p| p.caller == 0)
        .map(|p| (p.sequence, p.target, p.activator))
        .collect();
    fired.sort_by_key(|f| f.0);
    fired.into_iter().map(|(_, t, a)| (t, a)).collect()
}

fn fired(list: &[(&str, usize)]) -> Vec<(String, usize)> {
    list.iter().map(|&(t, a)| (t.to_owned(), a)).collect()
}

#[test]
fn every_entity_starts_and_ends_its_own_touch() {
    // NPCs only (2): the first arrival also fires the All output, the last departure too.
    let world = touch_world("trigger_multiple", "2", 2);
    let mut scene = Scene::new(&world);
    assert!(touch_tick(&mut scene, &world, FAR).is_empty());
    scene.states[1].origin = Vec3::ZERO;
    let first = touch_tick(&mut scene, &world, FAR);
    assert_eq!(first, fired(&[("st", 1), ("sta", 1)]));
    assert_eq!(scene.diagnostics.trigger_entries, 1);
    scene.states[2].origin = Vec3::new(4., 0., 0.);
    assert_eq!(touch_tick(&mut scene, &world, FAR), fired(&[("st", 2)]));
    assert!(touch_tick(&mut scene, &world, FAR).is_empty());
    scene.states[1].origin = FAR;
    assert_eq!(touch_tick(&mut scene, &world, FAR), fired(&[("et", 1)]));
    // A killed toucher ends its touch like one walking out (PhysicsRemoveTouchedList).
    scene.states[2].killed = true;
    let last = touch_tick(&mut scene, &world, FAR);
    assert_eq!(last, fired(&[("et", 2), ("eta", 2)]));
    assert_eq!(scene.diagnostics.trigger_entries, 1);
}

#[test]
fn an_npc_starts_touching_while_the_player_is_inside() {
    // Clients and NPCs (1 | 2): the player inside does not hide an arriving NPC.
    let world = touch_world("trigger_multiple", "3", 1);
    let mut scene = Scene::new(&world);
    let inside = Vec3::new(0., 0., -30.);
    let player = touch_tick(&mut scene, &world, inside);
    assert_eq!(player, fired(&[("st", PLAYER), ("sta", PLAYER)]));
    scene.states[1].origin = Vec3::ZERO;
    assert_eq!(touch_tick(&mut scene, &world, inside), fired(&[("st", 1)]));
    assert_eq!(
        touch_tick(&mut scene, &world, FAR),
        fired(&[("et", PLAYER)])
    );
    scene.states[1].origin = FAR;
    let last = touch_tick(&mut scene, &world, FAR);
    assert_eq!(last, fired(&[("et", 1), ("eta", 1)]));
}

#[test]
fn a_disabled_trigger_ends_its_touches_and_an_enabled_one_restarts_them() {
    let world = touch_world("trigger_multiple", "1", 0);
    let mut scene = Scene::new(&world);
    let inside = Vec3::new(0., 0., -30.);
    touch_tick(&mut scene, &world, inside);
    // Inputs arrive after this tick's touches: the next tick sees the new state.
    let two_ticks = |scene: &mut Scene| {
        let mut fired = touch_tick(scene, &world, inside);
        fired.extend(touch_tick(scene, &world, inside));
        fired
    };
    scene.send(0, "Disable", "");
    let ended = two_ticks(&mut scene);
    assert_eq!(ended, fired(&[("et", PLAYER), ("eta", PLAYER)]));
    assert!(two_ticks(&mut scene).is_empty());
    scene.send(0, "Enable", "");
    let started = two_ticks(&mut scene);
    assert_eq!(started, fired(&[("st", PLAYER), ("sta", PLAYER)]));
}

#[test]
fn a_spent_trigger_once_frees_its_touches_without_outputs() {
    let world = touch_world("trigger_once", "1", 0);
    let mut scene = Scene::new(&world);
    let inside = Vec3::new(0., 0., -30.);
    let started = touch_tick(&mut scene, &world, inside);
    assert_eq!(started, fired(&[("st", PLAYER), ("sta", PLAYER)]));
    assert!(!scene.states[0].enabled);
    assert!(touch_tick(&mut scene, &world, FAR).is_empty());
}

#[test]
fn npcs_touch_when_they_move_and_a_new_trigger_touches_what_stands_inside() {
    // A trigger's first pass (PostClientActive) touches an NPC already standing inside.
    let world = touch_world("trigger_multiple", "2", 2);
    let mut scene = Scene::new(&world);
    scene.states[1].origin = Vec3::ZERO;
    assert_eq!(
        touch_tick(&mut scene, &world, FAR),
        fired(&[("st", 1), ("sta", 1)])
    );
    // A maker's NPC appears inside the armed trigger: no touch until its first step.
    scene.states[2].killed = true;
    scene.states[2].origin = Vec3::new(4., 0., 0.);
    touch_tick(&mut scene, &world, FAR);
    scene.states[2].killed = false;
    assert!(touch_tick(&mut scene, &world, FAR).is_empty());
    scene.states[2].origin = Vec3::new(5., 0., 0.);
    assert_eq!(touch_tick(&mut scene, &world, FAR), fired(&[("st", 2)]));
}

#[test]
fn template_makers_never_touch() {
    let mut world = touch_world("trigger_multiple", "2", 0);
    world.entities.push(entity(
        "npc_template_maker",
        "maker",
        &[("origin", "0 0 0")],
    ));
    let mut scene = Scene::new(&world);
    assert!(touch_tick(&mut scene, &world, FAR).is_empty());
}
