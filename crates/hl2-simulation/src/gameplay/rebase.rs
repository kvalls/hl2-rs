//! Level-transition clock rebasing for player inventory and weapon states.
use super::{FragState, Inventory};

impl FragState {
    /// Shift frag animation cursor timestamp across changelevel and drop old-world throws.
    pub(crate) fn rebase_clock(&mut self, delta: f64) {
        if let Some((_, at, _)) = &mut self.cursor {
            *at += delta;
        }
        self.pending.clear();
    }
}

impl Inventory {
    /// Retain remaining weapon deadlines when the destination map resets scene time.
    /// Transient hit/spawn queues, held objects, and world-space effects belong to the old world.
    pub fn rebase_clock(&mut self, old: f64, new: f64) {
        let delta = new - old;
        self.animation_at += delta;
        self.next_attack += delta;
        self.owner_attack_until += delta;
        self.soonest_attack += delta;
        self.last_shot += delta;
        for time in self.next_secondary.values_mut() {
            *time += delta;
        }
        for empty in self.empty_fire.values_mut() {
            empty.next_sound += delta;
        }
        if let Some(time) = &mut self.ar2_charge_until {
            *time += delta;
        }
        if let Some(reload) = &mut self.reload {
            reload.at += delta;
        }
        self.physcannon.rebase_clock(delta);
        self.crossbow.rebase_clock(delta);
        self.fov.rebase_clock(delta);
        self.bugbait.rebase_clock(delta);
        self.rpg.rebase_clock(delta);
        self.frag.rebase_clock(delta);
        self.carry = None;
        self.bugbait_events.clear();
        self.tracer_requests.clear();
        self.ar2_impacts.clear();
        self.deferred_sounds.clear();
        self.impacts.clear();
        self.blood.clear();
        self.projectile_spawns.clear();
        self.attack_input = None;
        self.holding_attack = false;
    }
}
