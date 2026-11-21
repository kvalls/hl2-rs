//! A scripted_sequence's next script (SDK scripted.cpp SequenceDone, PostIdleDone).
use super::*;
use modkit_core::{
    animation::{Clip, Rig},
    ModelInstance,
};

fn clip(frames: usize) -> Clip {
    Clip {
        layer: Default::default(),
        events: Vec::new(),
        fps: 30.,
        looping: false,
        frames: vec![vec![]; frames],
    }
}

/// d1_trainstation_01's pacer: ss_pace_1 and ss_pace_2 name each other as the next script.
/// When the first ends, its post idle is skipped and the second takes the actor at once; the
/// first's OnPostIdleEndSequence still fires.
#[test]
fn a_finished_script_hands_its_actor_to_the_next_script() {
    let script = |name: &str, next: &str, play: &str| {
        entity(
            "scripted_sequence",
            name,
            &[
                ("m_iszEntity", "actor"),
                ("m_iszPlay", play),
                ("m_iszPostIdle", "post"),
                ("m_iszNextScript", next),
                ("spawnflags", "116"),
            ],
        )
    };
    let mut first = script("first", "second", "play");
    first.properties.extend([
        ("OnEndSequence".into(), "result,SetValue,1,0,-1".into()),
        ("OnPostIdleEndSequence".into(), "result,Add,10,0,-1".into()),
    ]);
    let w = World {
        entities: vec![
            first,
            script("second", "first", "other"),
            entity("npc_citizen", "actor", &[]),
            entity("math_counter", "result", &[]),
        ],
        model_instances: vec![ModelInstance {
            background: false,
            model: "actor.mdl".into(),
            origin: Vec3::ZERO,
            angles: Vec3::ZERO,
            skin: 0,
            body: 0,
            scale: 1.,
            kind: "npc_citizen".into(),
            solid: true,
            solid_mode: None,
            entity: Some(2),
        }],
        rigs: BTreeMap::from([(
            "actor.mdl#0".into(),
            Rig {
                clips: BTreeMap::from([
                    ("play".into(), clip(31)),
                    ("other".into(), clip(31)),
                    ("post".into(), clip(61)),
                ]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut s = Scene::new(&w);
    s.send(0, "BeginSequence", "");
    s.tick(&w, Vec3::ZERO, 0.015);
    assert_eq!(s.states[2].scripted_by, Some(0));
    s.tick(&w, Vec3::ZERO, 1.1);
    s.tick(&w, Vec3::ZERO, 0.015);
    // The second script plays at once: no post idle in between.
    assert_eq!(s.states[2].scripted_by, Some(1));
    assert_eq!(s.states[2].animation, "other");
    assert_eq!(s.states[3].value, 11.);
    // And hands the actor back to the first when it ends.
    s.tick(&w, Vec3::ZERO, 1.1);
    s.tick(&w, Vec3::ZERO, 0.015);
    assert_eq!(s.states[2].scripted_by, Some(0));
    assert_eq!(s.states[2].animation, "play");
}
