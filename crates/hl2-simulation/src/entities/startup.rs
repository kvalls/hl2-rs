//! The last steps of a scene's construction at map start.
use super::{number, weapon_idle_activity, Scene};
use modkit_core::World;

impl Scene {
    /// The weapon users' first idle.
    pub(super) fn start_weapon_idles(&mut self, world: &World) {
        // Metrocops without a DefaultAnim start on their (weapon-translated) idle activity,
        // as do other weapon users given a weapon (a citizen's SMG1: ACT_IDLE_SMG1).
        for id in 0..world.entities.len() {
            let e = &world.entities[id];
            if e.get("DefaultAnim").is_some() || !crate::npc_weapons::uses_weapons(e.class()) {
                continue;
            }
            let weapon = e.get("additionalequipment").unwrap_or("");
            if e.class() == "npc_metropolice" || !matches!(weapon.trim(), "" | "0") {
                self.animate_activity_if_present(world, id, weapon_idle_activity(weapon));
            }
        }
    }
    /// Timers' first delays and logic_auto's outputs.
    pub(super) fn start_logic(&mut self, world: &World, new_game: bool) {
        // CTimerEntity::Spawn -> Enable -> ResetTimer: random timers draw their first delay.
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() == "logic_timer" && number(e, "UseRandomTime", 0.) != 0. {
                self.states[id].timer_at = self.time + self.timer_interval(e);
            }
        }
        for id in 0..world.entities.len() {
            if world.entities[id].class() == "logic_auto" {
                self.fire(id, "OnMapSpawn", usize::MAX);
                if new_game {
                    self.fire(id, "OnNewGame", usize::MAX);
                }
            }
        }
    }
}
