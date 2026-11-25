//! Authored root motion on actor sequences (Scene::ai_auto_movement, PR #5's eb51f6c).
use super::*;

#[test]
fn ai_auto_movement_rotates_root_motion_by_yaw_and_clamps_on_finish() {
    use modkit_core::{
        animation::{Clip, ClipLayer, MovementRecord, Pose, Rig, RootMotion, Sequence},
        ModelInstance,
    };

    let delta = Vec3::new(-24.69, -1.14, 0.);
    let dist = delta.length();
    let motion = RootMotion {
        fps: 30.,
        frame_count: 31,
        records: vec![MovementRecord {
            end_frame: 25,
            motion_flags: 0xc0,
            v0: dist,
            v1: dist,
            end_yaw_degrees: 0.,
            direction: delta.normalize(),
            cumulative_position: delta,
        }],
    };
    let entry_clip = Clip {
        events: Vec::new(),
        fps: 30.,
        looping: false,
        frames: vec![
            vec![Pose {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
            }];
            31
        ],
        layer: ClipLayer {
            root_motion: Some(motion),
            ..Default::default()
        },
    };
    let w = World {
        entities: vec![entity("npc_citizen", "citizen", &[("origin", "0 0 0")])],
        model_instances: vec![ModelInstance {
            background: false,
            model: "female_ss.mdl".into(),
            origin: Vec3::ZERO,
            angles: Vec3::new(0., 180., 0.),
            skin: 0,
            body: 0,
            scale: 1.,
            kind: "npc_citizen".into(),
            solid: false,
            solid_mode: None,
            entity: Some(0),
        }],
        rigs: BTreeMap::from([(
            "female_ss.mdl#0".into(),
            Rig {
                clips: BTreeMap::from([("sit_entry".to_string(), entry_clip)]),
                sequences: vec![Sequence::new("sit_entry", "ACT_BUSY_SIT_CHAIR_ENTRY", 1, 0)],
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut scene = Scene::new(&w);
    scene.states[0].origin = Vec3::new(-2237., -1472., 19.);
    scene.ai_set_yaw(0, 180.);
    assert!(scene.ai_set_activity(&w, 0, "ACT_BUSY_SIT_CHAIR_ENTRY"));
    scene.tick(&w, Vec3::ZERO, 0.5);
    assert!(scene.ai_auto_movement(&w, 0, false));
    assert!(scene.states[0].origin.x > -2237.);
    assert!(scene.ai_auto_movement(&w, 0, true));
    let expected = Vec3::new(-2237. + 24.69, -1472. + 1.14, 19.);
    assert!(
        scene.states[0].origin.distance(expected) < 1e-3,
        "origin {} expected {}",
        scene.states[0].origin,
        expected
    );
}
