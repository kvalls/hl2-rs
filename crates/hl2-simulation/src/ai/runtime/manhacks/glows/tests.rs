use super::{manhack, AiClass, AiRuntime, ModelInstance, Scene, Vec3, World};
use glam::{Mat4, Quat};
use modkit_core::{
    animation::{Attachment, Bone, Clip, Pose, Rig},
    Entity,
};
use std::collections::BTreeMap;

fn instance(id: usize, scale: f32) -> ModelInstance {
    ModelInstance {
        background: false,
        model: "manhack.mdl".into(),
        origin: Vec3::ZERO,
        angles: Vec3::ZERO,
        skin: 0,
        body: 0,
        scale,
        kind: "npc_manhack".into(),
        solid: false,
        solid_mode: None,
        entity: Some(id),
    }
}

fn fixture(count: usize, unrelated: usize) -> (World, Scene, AiRuntime) {
    let pose = |x| Pose {
        position: Vec3::new(x, 0., 0.),
        rotation: Quat::IDENTITY,
    };
    let rig = Rig {
        bones: vec![Bone {
            name: "root".into(),
            parent: None,
            bind: pose(0.),
            inverse_bind: Mat4::IDENTITY,
        }],
        clips: BTreeMap::from([(
            "move".into(),
            Clip {
                events: Vec::new(),
                fps: 1.,
                looping: false,
                frames: vec![vec![pose(0.)], vec![pose(8.)]],
                layer: Default::default(),
            },
        )]),
        attachments: manhack::GLOW_ATTACHMENTS
            .iter()
            .enumerate()
            .map(|(i, name)| Attachment {
                name: (*name).into(),
                bone: 0,
                local: Mat4::from_translation(Vec3::new(0., i as f32 + 1., 2.)),
            })
            .collect(),
        ..Default::default()
    };
    let world = World {
        entities: (0..count)
            .map(|_| Entity {
                properties: vec![("classname".into(), "npc_manhack".into())],
            })
            .collect(),
        model_instances: (0..unrelated)
            .map(|id| instance(count + id, 1.))
            .chain((0..count).rev().map(|id| instance(id, id as f32 + 1.)))
            .collect(),
        rigs: BTreeMap::from([("manhack.mdl#0".into(), rig)]),
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    scene.time = 0.25;
    for (id, state) in scene.states.iter_mut().enumerate() {
        state.animation = "move".into();
        state.animation_started = 0.;
        state.origin = Vec3::new(20. * id as f32, 0., 0.);
        state.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    }
    let ai = AiRuntime::new(&world, None);
    (world, scene, ai)
}

/// Reference calls the single-actor helper, including its original first-match lookup.
fn reference(ai: &AiRuntime, world: &World, scene: &Scene) -> Vec<(Vec3, bool)> {
    let mut result = Vec::new();
    for (&id, actor) in &ai.actors {
        let AiClass::Manhack(m) = &actor.class else {
            continue;
        };
        let state = &scene.states[id];
        if state.visible
            && !state.killed
            && !state.corpse
            && (!m.stunned || manhack::strobe_lit(scene.time, id))
        {
            result.extend(
                scene
                    .attachment_frames(world, id, &manhack::GLOW_ATTACHMENTS)
                    .into_iter()
                    .flatten()
                    .map(|frame| (frame.w_axis.truncate(), m.stunned)),
            );
        }
    }
    result
}

#[test]
fn batch_preserves_actor_order_first_instance_and_scaled_animated_frames() {
    let (mut world, mut scene, ai) = fixture(3, 17);
    world.model_instances.insert(0, instance(1, 7.));
    world.model_instances.push(instance(1, 99.));
    world.model_instances.push(instance(1000, 1.));
    let before: Vec<_> = ai.manhack_glows(&world, &scene).collect();
    assert_eq!(before, reference(&ai, &world, &scene));
    assert_eq!(before.len(), 6);
    assert!(before[0].0.abs_diff_eq(Vec3::new(-1., 2., 2.), 1e-5));
    assert!(before[2].0.abs_diff_eq(Vec3::new(13., 14., 14.), 1e-5));
    scene.time = 0.75;
    let after: Vec<_> = ai.manhack_glows(&world, &scene).collect();
    assert_ne!(before, after);
    assert_eq!(after, reference(&ai, &world, &scene));
}

#[test]
fn batch_preserves_missing_models_rigs_and_attachments() {
    let (mut world, scene, ai) = fixture(3, 0);
    world.model_instances.retain(|i| i.entity != Some(0));
    let broken = world
        .model_instances
        .iter_mut()
        .find(|i| i.entity == Some(1))
        .unwrap();
    broken.model = "missing.mdl".into();
    world.model_instances.push(instance(1, 1.));
    world
        .rigs
        .get_mut("manhack.mdl#0")
        .unwrap()
        .attachments
        .remove(0);
    let actual: Vec<_> = ai.manhack_glows(&world, &scene).collect();
    assert_eq!(actual, reference(&ai, &world, &scene));
    assert_eq!(actual.len(), 1);
    world.rigs.clear();
    assert_eq!(ai.manhack_glows(&world, &scene).count(), 0);
}

#[test]
fn batch_preserves_hidden_killed_corpse_strobe_and_disabled_ai() {
    let (world, mut scene, mut ai) = fixture(5, 0);
    scene.states[0].visible = false;
    scene.states[1].killed = true;
    scene.states[2].corpse = true;
    let AiClass::Manhack(m) = &mut ai.actors.get_mut(&3).unwrap().class else {
        panic!("fixture manhack");
    };
    m.stunned = true;
    assert!(!ai.enabled);
    for step in 0..20 {
        scene.time = f64::from(step) * 0.01;
        let actual: Vec<_> = ai.manhack_glows(&world, &scene).collect();
        assert_eq!(actual, reference(&ai, &world, &scene));
        assert_eq!(
            actual.len(),
            if manhack::strobe_lit(scene.time, 3) {
                4
            } else {
                2
            }
        );
    }
    scene.states[4].visible = false;
    scene.states[3].killed = true;
    assert_eq!(ai.manhack_glows(&world, &scene).count(), 0);
    ai.actors.clear();
    assert_eq!(ai.manhack_glows(&world, &scene).count(), 0);
}

#[test]
#[ignore = "isolated CPU timing; run release with --ignored --nocapture"]
fn benchmark_glow_model_lookup() {
    use std::{hint::black_box, time::Instant};
    for actors in [0, 1, 30] {
        for unrelated in [0, 1000, 10000, 50000] {
            for leading in [true, false] {
                let (mut world, scene, ai) = fixture(actors, unrelated);
                if !leading {
                    world.model_instances.rotate_left(unrelated);
                }
                let batched = || {
                    ai.manhack_glows(black_box(&world), black_box(&scene))
                        .collect::<Vec<_>>()
                };
                let baseline = || reference(black_box(&ai), black_box(&world), black_box(&scene));
                assert_eq!(batched(), baseline());
                for _ in 0..10 {
                    black_box(batched());
                    black_box(baseline());
                }
                let mut old = Vec::new();
                let mut new = Vec::new();
                for pair in 0..8 {
                    for variant in if pair % 2 == 0 {
                        [false, true]
                    } else {
                        [true, false]
                    } {
                        let start = Instant::now();
                        for _ in 0..20 {
                            black_box(if variant { batched() } else { baseline() });
                        }
                        let us = start.elapsed().as_secs_f64() * 1e6 / 20.;
                        if variant {
                            new.push(us);
                        } else {
                            old.push(us);
                        }
                    }
                }
                println!("unrelated={unrelated} actors={actors} leading={leading} baseline_us={old:?} batched_us={new:?}");
            }
        }
    }
}
