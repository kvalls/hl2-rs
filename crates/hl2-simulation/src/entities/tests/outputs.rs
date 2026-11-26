//! Outputs that carry a value (COutputInt, COutputFloat).
use super::*;

/// A valued output (the queue goal's OnQueueMoved with its new count) hands the value to the
/// connections that have no parameter of their own; an authored parameter is kept.
#[test]
fn a_valued_output_passes_its_value_to_connections_without_a_parameter() {
    let world = World {
        entities: vec![
            entity(
                "ai_goal_actbusy_queue",
                "queue",
                &[
                    ("OnQueueMoved", "counter,SetValue,,0,-1"),
                    ("OnQueueMoved", "relay,Trigger,authored,0,-1"),
                ],
            ),
            entity("math_counter", "counter", &[]),
            entity("logic_relay", "relay", &[]),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    scene.queue.clear();
    scene.fire_value(0, "OnQueueMoved", 3., 0);
    let mut sent: Vec<_> = scene
        .queue
        .iter()
        .map(|p| (p.target.clone(), p.parameter.clone()))
        .collect();
    sent.sort();
    assert_eq!(
        sent,
        [
            ("counter".to_owned(), "3".to_owned()),
            ("relay".to_owned(), "authored".to_owned())
        ]
    );
}
