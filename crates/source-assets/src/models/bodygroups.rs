//! Which classes change bodygroups at runtime, and the body values that gives.
use super::set_body_value;

/// Classes whose AI calls SetBodygroup after spawning, and the groups it changes:
/// CNPC_PoisonZombie's head crab (ReleaseHeadcrab's fRemoveHead, group 1), nest crabs
/// (ZOMBIE_BODYGROUP_NEST_BASE 2..4) and the crab in its hand (ZOMBIE_BODYGROUP_THROW 5).
pub const RUNTIME_BODYGROUPS: &[(&str, &[usize])] = &[
    ("npc_poisonzombie", &[1, 2, 3, 4, 5]),
    // CNPC_Manhack::SpinBlades: the solid blade (MANHACK_BODYGROUP_BLADE 1, also what
    // StartEngine brings out) and the blur (MANHACK_BODYGROUP_BLUR 2) that replaces it at
    // speed. Without group 2 the blur bodies were never built and the blade stayed drawn.
    ("npc_manhack", &[1, 2]),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// manhack.mdl's body parts as (models, base): Body, Blade (none or the blade) and
    /// BladeBlur (none or the blur). SpinBlades shows the blade alone, both, or the blur
    /// alone, so every one of those bodies must be preloaded.
    #[test]
    fn manhack_runtime_bodies_include_the_blur() {
        let parts = [(1, 1), (2, 1), (2, 2)];
        let (_, groups) = RUNTIME_BODYGROUPS
            .iter()
            .find(|(class, _)| *class == "npc_manhack")
            .expect("manhack row");
        let bodies = body_variants(&parts, 0, groups);
        // 1: blade, 3: blade and blur (speed 20-40), 2: blur alone (40 and over).
        assert_eq!(bodies, vec![0, 1, 2, 3]);
    }
}
