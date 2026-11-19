//! Per-entity trigger touches (SDK triggers.cpp CBaseTrigger::StartTouch/EndTouch).
//!
//! A trigger keeps the entities touching it. OnStartTouch fires for every entity that starts
//! touching and passes the trigger's filters, OnStartTouchAll when the list goes from empty
//! to one; OnEndTouch fires for every entity that leaves or is removed, OnEndTouchAll when
//! the list empties. A disabled trigger ends the touches it holds (EndTouch fires even
//! while disabled: the SDK's commented-out check), while a removed trigger frees its touch
//! links without its own outputs (physics_main_shared.cpp PhysicsRemoveTouchedList).

use super::{is_vphysics_class, number, Scene, PLAYER};
use glam::Vec3;
use modkit_core::World;

/// The NPCs and VPhysics objects that may touch triggers this tick, gathered once per tick
/// on the first trigger that accepts them.
#[derive(Default)]
pub(super) struct Candidates {
    npcs: Vec<usize>,
    objects: Vec<usize>,
}

impl Scene {
    /// One trigger's tick: its touches, then OnTrigger (trigger_once and trigger_multiple,
    /// once per `wait`), trigger_hurt's touchers and the touch-driven level change.
    pub(super) fn tick_trigger(
        &mut self,
        world: &World,
        id: usize,
        candidates: &mut Option<Candidates>,
    ) {
        let e = self.ent(world, id);
        let was_touched = !self.states[id].touchers.is_empty();
        let enabled = self.states[id].enabled;
        let activator = self.update_touches(world, id, candidates);
        let gated = matches!(e.class(), "trigger_once" | "trigger_multiple");
        let due = self.time - self.states[id].last_trigger >= number(e, "wait", 0.2) as f64;
        if let Some(activator) = activator.filter(|_| gated && due) {
            self.fire(id, "OnTrigger", activator);
            self.states[id].last_trigger = self.time;
            if e.class() == "trigger_once" {
                // SUB_Remove: a removed trigger frees its touches without its own outputs.
                self.states[id].enabled = false;
                self.states[id].touchers.clear();
            }
        }
        if enabled && e.class() == "trigger_hurt" {
            // Every touching player/NPC, before the trigger filters (Touch).
            let mut touching: Vec<usize> = (0..self.states.len())
                .filter(|&n| {
                    self.ent(world, n).class().starts_with("npc_")
                        && !self.states[n].killed
                        && self.player_inside(world, id, self.states[n].origin)
                })
                .collect();
            if self.player_inside(world, id, self.player_feet) {
                touching.insert(0, PLAYER);
            }
            self.trigger_hurt(world, id, touching);
        }
        // SF_CHANGELEVEL_NOTOUCH (0x2) leaves the ChangeLevel input available, but must not
        // change maps when the player overlaps the brush.
        if activator.is_some()
            && !was_touched
            && e.class() == "trigger_changelevel"
            && number(e, "spawnflags", 0.) as u32 & 2 == 0
        {
            self.send(id, "ChangeLevel", "");
        }
    }

    fn touch_candidates(&self, world: &World) -> Candidates {
        let mut candidates = Candidates::default();
        for n in 0..self.states.len() {
            if self.states[n].killed || !self.states[n].visible {
                continue;
            }
            let class = self.ent(world, n).class();
            if class.starts_with("npc_") {
                candidates.npcs.push(n);
            } else if is_vphysics_class(class) {
                candidates.objects.push(n);
            }
        }
        candidates
    }

    /// Whether `other` still overlaps trigger `id`: the player's hull at the feet, an NPC's
    /// hull at its origin, a VPhysics object's origin (the small-object test).
    fn still_touching(&self, world: &World, id: usize, other: usize) -> bool {
        if other == PLAYER {
            return self.player_inside(world, id, self.player_feet);
        }
        let Some(state) = self.states.get(other) else {
            return false;
        };
        if state.killed || !state.visible {
            return false;
        }
        if is_vphysics_class(self.ent(world, other).class()) {
            self.touches(world, id, state.origin, Vec3::splat(2.))
        } else {
            self.player_inside(world, id, state.origin)
        }
    }

    /// One trigger's touches this tick. trigger_once and trigger_multiple honor their
    /// spawnflags (clients 1, NPCs 2, physics objects 8, everything 0x40) and filtername
    /// (PassesTriggerFilters, checked when a touch starts); other trigger classes keep
    /// player-only touches. Returns the OnTrigger activator while anything touches an
    /// enabled trigger: the player first, else the longest-touching entity.
    pub(super) fn update_touches(
        &mut self,
        world: &World,
        id: usize,
        candidates: &mut Option<Candidates>,
    ) -> Option<usize> {
        let enabled = self.states[id].enabled;
        let held = std::mem::take(&mut self.states[id].touchers);
        let mut left = Vec::new();
        for &other in &held {
            if !(enabled && self.still_touching(world, id, other)) {
                left.push(other);
            }
        }
        let mut touchers = held;
        touchers.retain(|other| !left.contains(other));
        self.states[id].touchers = touchers;
        for &other in &left {
            self.fire(id, "OnEndTouch", other);
        }
        if let Some(&last) = left.last().filter(|_| self.states[id].touchers.is_empty()) {
            self.fire(id, "OnEndTouchAll", last);
        }
        if !enabled {
            return None;
        }
        for other in self.entering(world, id, candidates) {
            self.states[id].touchers.push(other);
            self.fire(id, "OnStartTouch", other);
            if self.states[id].touchers.len() == 1 {
                self.diagnostics.trigger_entries += 1;
                self.fire(id, "OnStartTouchAll", other);
            }
        }
        let touchers = &self.states[id].touchers;
        touchers
            .contains(&PLAYER)
            .then_some(PLAYER)
            .or_else(|| touchers.first().copied())
    }

    /// Entities that start touching trigger `id` this tick and pass its filters.
    fn entering(
        &self,
        world: &World,
        id: usize,
        candidates: &mut Option<Candidates>,
    ) -> Vec<usize> {
        let e = self.ent(world, id);
        let gated = matches!(e.class(), "trigger_once" | "trigger_multiple");
        let flags = number(e, "spawnflags", 1.) as u32;
        let clients = !gated || flags & (1 | 0x40) != 0;
        let npcs = gated && flags & (2 | 0x40) != 0;
        let objects = gated && flags & (8 | 0x40) != 0;
        let filter = gated
            .then(|| e.get("filtername").map(str::trim).filter(|f| !f.is_empty()))
            .flatten();
        let held = &self.states[id].touchers;
        // Already touching first, then the overlap, then the filter entity.
        let starts = |other: usize, overlaps: &dyn Fn() -> bool| {
            !held.contains(&other)
                && overlaps()
                && filter.is_none_or(|f| self.passes_filter(world, f, other))
        };
        let mut entering = Vec::new();
        if clients && starts(PLAYER, &|| self.player_inside(world, id, self.player_feet)) {
            entering.push(PLAYER);
        }
        if !(npcs || objects) {
            return entering;
        }
        let candidates = candidates.get_or_insert_with(|| self.touch_candidates(world));
        let alive = |n: usize| !self.states[n].killed && self.states[n].visible;
        let origin = |n: usize| self.states[n].origin;
        if npcs {
            for &n in &candidates.npcs {
                if alive(n) && starts(n, &|| self.player_inside(world, id, origin(n))) {
                    entering.push(n);
                }
            }
        }
        if objects {
            for &n in &candidates.objects {
                let small = Vec3::splat(2.);
                if alive(n) && starts(n, &|| self.touches(world, id, origin(n), small)) {
                    entering.push(n);
                }
            }
        }
        entering
    }
}
