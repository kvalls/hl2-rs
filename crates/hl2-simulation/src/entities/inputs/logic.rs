//! Logic entities: `logic_relay`, `logic_branch`, `logic_case` and `math_counter`.

use super::{Delivery, Scene};
use crate::entities::number;
use std::collections::BTreeSet;

/// Trigger, CancelPending.
pub(super) fn relay(scene: &mut Scene, d: &Delivery) -> bool {
    let (id, p) = (d.id, d.pending);
    match d.input {
        "trigger" => {
            if scene.states[id].enabled {
                scene.fire(id, "OnTrigger", p.activator);
                if number(d.entity, "spawnflags", 0.) as u32 & 1 != 0 {
                    scene.states[id].enabled = false;
                }
            }
        }
        // CLogicRelay::InputCancelPending: g_EventQueue.CancelEvents(this).
        "cancelpending" => scene.queue.retain(|q| q.caller != id),
        _ => return false,
    }
    true
}

/// SetValue, SetValueTest, Test.
pub(super) fn branch(scene: &mut Scene, d: &Delivery) -> bool {
    let (input, id, p) = (d.input, d.id, d.pending);
    if !matches!(input, "setvalue" | "setvaluetest" | "test") {
        return false;
    }
    if input != "test" {
        scene.states[id].value = d.value;
    }
    if input != "setvalue" {
        scene.fire(
            id,
            if scene.states[id].value != 0. {
                "OnTrue"
            } else {
                "OnFalse"
            },
            p.activator,
        );
    }
    true
}

/// InValue, PickRandom, PickRandomShuffle.
pub(super) fn case(scene: &mut Scene, d: &Delivery) -> bool {
    let (input, id, p) = (d.input, d.id, d.pending);
    match input {
        "invalue" => {
            let matched = (1..=16)
                .find(|i| d.entity.get(&format!("Case{i:02}")) == Some(p.parameter.as_str()));
            scene.fire(
                id,
                &matched.map_or("OnDefault".into(), |i| format!("OnCase{i:02}")),
                p.activator,
            );
        }
        "pickrandom" | "pickrandomshuffle" => {
            let cases = scene.states[id]
                .outputs
                .iter()
                .filter_map(|o| {
                    o.name
                        .strip_prefix("OnCase")
                        .and_then(|n| n.parse::<usize>().ok())
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if !cases.is_empty() {
                scene.random = scene
                    .random
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1);
                let chosen = if input == "pickrandomshuffle" {
                    let state = &mut scene.states[id];
                    let new_batch = state.shuffle.is_empty();
                    if new_batch {
                        state.shuffle = cases;
                    }
                    let candidates: Vec<_> = state
                        .shuffle
                        .iter()
                        .enumerate()
                        .filter(|(_, case)| {
                            !new_batch
                                || state.shuffle.len() == 1
                                || Some(**case) != state.last_shuffle
                        })
                        .map(|(index, _)| index)
                        .collect();
                    let index = candidates[(scene.random >> 32) as usize % candidates.len()];
                    let case = state.shuffle.swap_remove(index);
                    state.last_shuffle = Some(case);
                    case
                } else {
                    cases[(scene.random >> 32) as usize % cases.len()]
                };
                scene.fire(id, &format!("OnCase{chosen:02}"), p.activator);
            }
        }
        _ => return false,
    }
    true
}

/// Add, Subtract, SetValue, SetValueNoFire.
pub(super) fn counter(scene: &mut Scene, d: &Delivery) -> bool {
    let (input, id, p, value) = (d.input, d.id, d.pending, d.value);
    if !matches!(input, "add" | "subtract" | "setvalue" | "setvaluenofire") {
        return false;
    }
    let min = number(d.entity, "min", 0.);
    let max = number(d.entity, "max", 0.);
    let old = scene.states[id].value;
    let mut next = match input {
        "add" => old + value,
        "subtract" => old - value,
        _ => value,
    };
    if max > min {
        next = next.clamp(min, max);
    }
    scene.states[id].value = next;
    if input != "setvaluenofire" {
        scene.fire_value(id, "OutValue", next, p.activator);
        if max > min && next >= max && old < max {
            scene.fire(id, "OnHitMax", p.activator);
        }
        if max > min && next <= min && old > min {
            scene.fire(id, "OnHitMin", p.activator);
        }
    }
    true
}
