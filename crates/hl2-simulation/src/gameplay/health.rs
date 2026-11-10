//! An NPC's health and max health before and after its first damage.

use crate::entities::Scene;

/// An NPC's health before its first damage: the class's Spawn value from skill.cfg
/// (`Skill::npc_health`), else the map's health keyvalue, else 40.
pub fn npc_spawn_health(scene: &Scene, e: &modkit_core::Entity) -> f32 {
    scene.skill.npc_entity_health(e).unwrap_or_else(|| {
        e.get("health")
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| *v > 0.)
            .unwrap_or(40.)
    })
}
/// An NPC's m_iMaxHealth: what was recorded when it changed (zombie torsos), else its spawn health.
pub fn npc_max_health(scene: &Scene, e: &modkit_core::Entity, id: usize) -> f32 {
    scene
        .npc_max_health
        .get(&id)
        .copied()
        .unwrap_or_else(|| npc_spawn_health(scene, e))
}
/// An NPC's health now: before its first damage (`hits::ensure_health`) that is its spawn health.
pub fn npc_health(scene: &Scene, e: &modkit_core::Entity, id: usize) -> f32 {
    let state = &scene.states[id];
    if state.value <= 0. && !state.killed {
        return npc_spawn_health(scene, e);
    }
    state.value
}
