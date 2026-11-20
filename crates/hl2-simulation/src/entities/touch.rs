//! Per-entity trigger touches (SDK triggers.cpp CBaseTrigger::StartTouch/EndTouch).
//!
//! A trigger keeps the entities touching it. OnStartTouch fires for every entity that starts
//! touching and passes the trigger's filters, OnStartTouchAll when the list goes from empty
//! to one; OnEndTouch fires for every entity that leaves or is removed, OnEndTouchAll when
//! the list empties. A disabled trigger ends the touches it holds (EndTouch fires even
//! while disabled: the SDK's commented-out check), while a removed trigger frees its touch
//! links without its own outputs (physics_main_shared.cpp PhysicsRemoveTouchedList).
//! NPCs and VPhysics objects refresh their touches only when they move (PhysicsTouchTriggers
//! after a step), the player every tick; a trigger that becomes active (the map's start,
//! Enable) touches everything inside it at once (PostClientActive, Enable).

use super::{is_vphysics_class, number, Scene, PLAYER};
use crate::npc_makers::is_npc_class;
use glam::Vec3;
use modkit_core::{Entity, World};

/// The NPCs and VPhysics objects that may touch triggers, gathered once per tick on the
/// first trigger that accepts them (the lists keep their capacity between ticks).
#[derive(Clone, Debug, Default)]
pub(super) struct Candidates {
    fresh: bool,
    npcs: Vec<usize>,
    objects: Vec<usize>,
}

/// PassesTriggerFilters' spawnflags (clients 1, NPCs 2, physics objects 8, everything 0x40)
/// and filtername, for trigger_once and trigger_multiple; other classes touch the player.
struct Rules<'a> {
    clients: bool,
    npcs: bool,
    objects: bool,
    filter: Option<&'a str>,
}

fn rules(e: &Entity) -> Rules<'_> {
    let gated = matches!(e.class(), "trigger_once" | "trigger_multiple");
    let flags = number(e, "spawnflags", 1.) as u32;
    Rules {
        clients: !gated || flags & (1 | 0x40) != 0,
        npcs: gated && flags & (2 | 0x40) != 0,
        objects: gated && flags & (8 | 0x40) != 0,
        filter: gated
            .then(|| e.get("filtername").map(str::trim).filter(|f| !f.is_empty()))
            .flatten(),
    }
}

impl Scene {
    /// One trigger's tick: its touches, then OnTrigger (trigger_once and trigger_multiple,
    /// once per `wait`), trigger_hurt's touchers and the touch-driven level change.
    pub(super) fn tick_trigger(&mut self, world: &World, id: usize) {
        let e = self.ent(world, id);
        let was_touched = !self.states[id].touchers.is_empty();
        let enabled = self.states[id].enabled;
        let activator = self.update_touches(world, id);
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
                    is_npc_class(self.ent(world, n).class())
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

    fn refresh_candidates(&mut self, world: &World) {
        let mut candidates = std::mem::take(&mut self.touch_candidates);
        candidates.npcs.clear();
        candidates.objects.clear();
        for n in 0..self.states.len() {
            if self.states[n].killed || !self.states[n].visible {
                continue;
            }
            let class = self.ent(world, n).class();
            if is_npc_class(class) {
                candidates.npcs.push(n);
            } else if is_vphysics_class(class) {
                candidates.objects.push(n);
            }
        }
        candidates.fresh = true;
        self.touch_candidates = candidates;
    }

    /// After the trigger pass: every entity's origin, to tell next tick who moved.
    pub(super) fn remember_touch_origins(&mut self) {
        self.touch_candidates.fresh = false;
        self.touch_origins.clear();
        self.touch_origins
            .extend(self.states.iter().map(|state| state.origin));
    }

    /// An NPC or object that has not moved since the last trigger pass: its touches are not
    /// re-tested (the player's are, every tick).
    fn resting(&self, other: usize) -> bool {
        other != PLAYER
            && self
                .states
                .get(other)
                .is_some_and(|s| self.touch_origins.get(other) == Some(&s.origin))
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

    /// One trigger's touches this tick. Returns the OnTrigger activator: the player (its
    /// touches are tested every tick) first, else the longest-touching NPC or object that
    /// moved or entered this tick (CTriggerMultiple::MultiTouch runs from Touch, which an
    /// NPC's or object's PhysicsTouchTriggers calls only when it moves: d1_trainstation_05's
    /// NPC-only gate trigger opens its gate once natively, not every `wait`).
    pub(super) fn update_touches(&mut self, world: &World, id: usize) -> Option<usize> {
        let enabled = self.states[id].enabled;
        let armed = self.states[id].touch_armed;
        let held = std::mem::take(&mut self.states[id].touchers);
        let mut left = Vec::new();
        for &other in &held {
            let alive = other == PLAYER || self.states.get(other).is_some_and(|s| !s.killed);
            let kept = alive && armed && self.resting(other);
            if !(enabled && (kept || self.still_touching(world, id, other))) {
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
            self.states[id].touch_armed = false;
            return None;
        }
        let e = self.ent(world, id);
        let rules = rules(e);
        if (rules.npcs || rules.objects) && !self.touch_candidates.fresh {
            self.refresh_candidates(world);
        }
        let entered = self.entering(world, id, armed, &rules);
        for &other in &entered {
            self.states[id].touchers.push(other);
            self.fire(id, "OnStartTouch", other);
            if self.states[id].touchers.len() == 1 {
                self.diagnostics.trigger_entries += 1;
                self.fire(id, "OnStartTouchAll", other);
            }
        }
        self.states[id].touch_armed = true;
        let touchers = &self.states[id].touchers;
        touchers.contains(&PLAYER).then_some(PLAYER).or_else(|| {
            touchers
                .iter()
                .copied()
                .find(|&other| entered.contains(&other) || !self.resting(other))
        })
    }

    /// Entities that start touching trigger `id` this tick and pass its filters; once
    /// `armed`, only NPCs and objects that moved.
    fn entering(&self, world: &World, id: usize, armed: bool, rules: &Rules) -> Vec<usize> {
        let held = &self.states[id].touchers;
        // Already touching first, then the overlap, then the filter entity.
        let starts = |other: usize, overlaps: &dyn Fn() -> bool| {
            !held.contains(&other)
                && overlaps()
                && rules
                    .filter
                    .is_none_or(|f| self.passes_filter(world, f, other))
        };
        let mut entering = Vec::new();
        if rules.clients && starts(PLAYER, &|| self.player_inside(world, id, self.player_feet)) {
            entering.push(PLAYER);
        }
        let alive = |n: usize| {
            !self.states[n].killed && self.states[n].visible && !(armed && self.resting(n))
        };
        let origin = |n: usize| self.states[n].origin;
        let candidates = &self.touch_candidates;
        if rules.npcs {
            for &n in &candidates.npcs {
                if alive(n) && starts(n, &|| self.player_inside(world, id, origin(n))) {
                    entering.push(n);
                }
            }
        }
        if rules.objects {
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
