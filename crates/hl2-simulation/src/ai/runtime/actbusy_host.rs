//! The runtime's side of CAI_ActBusyBehavior for every class that hosts it: finding hints
//! for a behavior about to search, hint locks, goal outputs and removing an NPC that left.
use std::collections::BTreeMap;

use glam::Vec3;
use modkit_core::World;

use super::super::actbusy::{
    find_exit_hint, find_hint, named_hint, ActBusy, BusyAnim, HintCandidate,
};
use super::super::humanoid::{Humanoid, Kind};
use super::{AiClass, NpcState};
use crate::entities::Scene;

impl AiClass {
    /// The hosted CAI_ActBusyBehavior: metrocops and player companions so far.
    pub fn actbusy_mut(&mut self) -> Option<&mut ActBusy> {
        match self {
            Self::Metropolice(cop) => Some(&mut cop.actbusy),
            Self::Humanoid(h) if hosts_actbusy(h) => Some(&mut h.actbusy),
            _ => None,
        }
    }
}
fn hosts_actbusy(h: &Humanoid) -> bool {
    matches!(h.kind, Kind::Companion { .. })
}

/// What FindActBusyHintNode and SelectScheduleForLeaving search.
pub(super) struct HintSearch<'a> {
    pub hints: &'a [HintCandidate],
    pub anims: &'a [BusyAnim],
    pub locks: &'a BTreeMap<usize, usize>,
}
impl HintSearch<'_> {
    /// Offers a hint to a behavior about to search this think.
    pub fn offer(
        &self,
        busy: &mut ActBusy,
        (world, id): (&World, usize),
        (origin, now): (Vec3, f64),
        state: NpcState,
        roll: f32,
    ) {
        busy.candidate = None;
        if !busy.wants_hint(now) {
            return;
        }
        let group = world
            .entities
            .get(id)
            .and_then(|e| e.get("hintgroup"))
            .unwrap_or("")
            .trim();
        let state = match state {
            NpcState::Combat => 3,
            NpcState::Alert => 2,
            _ => 1,
        };
        let free = |hint: &usize| self.locks.get(hint).is_none_or(|&holder| holder == id);
        busy.candidate = if busy.leaving {
            find_exit_hint(self.hints, group, state, roll)
        } else if let Some(hint) = busy.forced_hint.filter(free) {
            named_hint(self.hints, self.anims, hint)
        } else {
            find_hint(
                self.hints,
                self.anims,
                group,
                origin,
                busy.search_range,
                state,
                |hint| self.locks.get(&hint).is_some_and(|&holder| holder != id),
                busy.use_nearest,
                roll,
            )
        };
    }
}

/// After a think: CAI_Hint::Lock/Unlock (exit points are never locked), goal outputs, and
/// the SUB_Remove of an NPC that reached its exit (makers see it through DeathNotice).
pub(super) fn settle(
    busy: &mut ActBusy,
    id: usize,
    locks: &mut BTreeMap<usize, usize>,
    scene: &mut Scene,
) {
    for hint in busy.released.drain(..) {
        if locks.get(&hint) == Some(&id) {
            locks.remove(&hint);
        }
    }
    if let Some(hint) = busy.hint.as_ref().filter(|h| !h.exit) {
        locks.insert(hint.entity, id);
    }
    let outputs = std::mem::take(&mut busy.outputs);
    if let Some(goal) = busy.goal {
        for output in outputs {
            scene.fire(goal, output, id);
        }
    }
    if std::mem::take(&mut busy.remove) {
        scene.send(id, "Kill", "");
    }
}
