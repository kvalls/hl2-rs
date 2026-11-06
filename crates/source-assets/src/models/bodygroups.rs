//! Which classes change bodygroups at runtime, and the body values that gives.
use super::set_body_value;

/// Classes whose AI calls SetBodygroup after spawning, and the groups it changes:
/// CNPC_PoisonZombie's head crab (ReleaseHeadcrab's fRemoveHead, group 1), nest crabs
/// (ZOMBIE_BODYGROUP_NEST_BASE 2..4) and the crab in its hand (ZOMBIE_BODYGROUP_THROW 5).
pub const RUNTIME_BODYGROUPS: &[(&str, &[usize])] = &[
    ("npc_poisonzombie", &[1, 2, 3, 4, 5]),
    // The blade CNPC_Manhack::StartEngine brings out.
    ("npc_manhack", &[1]),
];
/// Every body value reachable from `body` by setting `groups` (each to any submodel).
pub fn body_variants(parts: &[(usize, i32)], body: i32, groups: &[usize]) -> Vec<i32> {
    let mut bodies = vec![body];
    for &group in groups {
        let Some(&part) = parts.get(group) else {
            continue;
        };
        bodies = bodies
            .iter()
            .flat_map(|&b| (0..part.0 as i32).map(move |v| set_body_value(part, b, v)))
            .collect();
    }
    bodies.sort_unstable();
    bodies.dedup();
    bodies
}
