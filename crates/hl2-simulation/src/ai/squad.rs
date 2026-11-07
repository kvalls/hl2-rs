//! CAI_Squad (ai_squad.cpp, ai_squadslot.h): members in join order, the leader (the
//! first member once there are two), per-enemy strategy slots (PER_ENEMY_SQUADSLOTS is
//! defined in HL2), the squad sound gate (FOkToMakeSound/JustMadeSound) and
//! SquadNewEnemy's per-member test. The runtime (`ai::runtime`) owns the squads by name
//! (CAI_SquadManager), joins, deaths and enemy-memory sharing. Silent members
//! (npc_enemyfinder) are not AI actors here, so every member counts. SDK behavior,
//! retail not compared.
use std::collections::{BTreeMap, BTreeSet};

/// SQUAD_SLOT_t shared slots; classes add theirs after LAST_SHARED_SQUADSLOT.
pub const SLOT_ATTACK1: i32 = 0;
pub const SLOT_ATTACK2: i32 = 1;
pub const SLOT_INVESTIGATE_SOUND: i32 = 2;
pub const SLOT_EXCLUSIVE_HANDSIGN: i32 = 3;
pub const SLOT_EXCLUSIVE_RELOAD: i32 = 4;
pub const SLOT_PICKUP_WEAPON1: i32 = 5;
pub const SLOT_PICKUP_WEAPON2: i32 = 6;
pub const SLOT_SPECIAL_ATTACK: i32 = 7;
pub const LAST_SHARED_SQUADSLOT: i32 = 8;
/// MAX_SQUAD_MEMBERS.
pub const MAX_SQUAD_MEMBERS: usize = 16;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Squad {
    pub name: String,
    pub members: Vec<usize>,
    /// Occupied slots per enemy (None for no enemy).
    #[serde(serialize_with = "slots_as_pairs")]
    slots: BTreeMap<Option<usize>, BTreeSet<i32>>,
    /// m_flSquadSoundWaitTime / m_nSquadSoundPriority.
    pub sound_wait: f64,
    pub sound_priority: i32,
}
/// Reports are JSON, whose keys are strings, and an enemy is not one: a squad holding a slot would
/// otherwise take the whole AI block out of every report.
fn slots_as_pairs<S: serde::Serializer>(
    slots: &BTreeMap<Option<usize>, BTreeSet<i32>>,
    out: S,
) -> Result<S::Ok, S::Error> {
    out.collect_seq(slots.iter())
}

impl Squad {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }
    /// AddToSquad's list part: a full squad replaces its last member.
    pub fn add(&mut self, member: usize) {
        if !self.members.contains(&member) {
            if self.members.len() == MAX_SQUAD_MEMBERS {
                self.members.pop();
            }
            self.members.push(member);
        }
    }
    pub fn remove(&mut self, member: usize) {
        self.members.retain(|m| *m != member);
    }
    /// GetLeader: the first member, only while the squad has more than one.
    pub fn leader(&self) -> Option<usize> {
        (self.members.len() > 1).then(|| self.members[0])
    }
    /// NumMembers.
    pub fn num_members(&self) -> usize {
        self.members.len()
    }
    /// FOkToMakeSound: while the last squad sound's wait runs, only a higher priority.
    pub fn ok_to_make_sound(&self, priority: i32, now: f64) -> bool {
        now > self.sound_wait || priority > self.sound_priority
    }
    /// JustMadeSound(priority, until).
    pub fn just_made_sound(&mut self, priority: i32, until: f64) {
        self.sound_wait = until;
        self.sound_priority = priority;
    }
    pub fn is_slot_occupied(&self, enemy: Option<usize>, slot: i32) -> bool {
        self.slots.get(&enemy).is_some_and(|s| s.contains(&slot))
    }
    /// OccupyStrategySlotRange: keep a slot already in range, else take the first free
    /// one (vacating the previous). `current` is the member's m_iMySquadSlot.
    pub fn occupy_range(
        &mut self,
        enemy: Option<usize>,
        start: i32,
        end: i32,
        current: &mut Option<i32>,
    ) -> bool {
        if current.is_some_and(|s| (start..=end).contains(&s)) {
            return true;
        }
        for slot in start..=end {
            if !self.is_slot_occupied(enemy, slot) {
                if let Some(previous) = current.take() {
                    self.vacate(enemy, previous);
                }
                self.slots.entry(enemy).or_default().insert(slot);
                *current = Some(slot);
                return true;
            }
        }
        false
    }
    pub fn vacate(&mut self, enemy: Option<usize>, slot: i32) {
        if let Some(s) = self.slots.get_mut(&enemy) {
            s.remove(&slot);
        }
    }
    /// The members other than the updater, who receive UpdateEnemyMemory.
    pub fn others(&self, updater: usize) -> impl Iterator<Item = usize> + '_ {
        self.members.iter().copied().filter(move |m| *m != updater)
    }
}
/// SquadNewEnemy's per-member test: take the enemy without one, or when the current
/// enemy is unseen and older than 3 s.
pub fn should_take_squad_enemy(
    current: Option<usize>,
    new: usize,
    sees_enemy: bool,
    since_seen: f64,
) -> bool {
    match current {
        None => true,
        Some(c) => c != new && !sees_enemy && since_seen > 3.,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_squad_holding_a_slot_still_goes_into_a_report() {
        let mut squad = Squad::new("cops");
        let mut held = None;
        assert!(squad.occupy_range(Some(7), SLOT_ATTACK1, SLOT_ATTACK2, &mut held));
        let report = serde_json::to_value(&squad).expect("a report needs string keys");
        assert_eq!(report["slots"], serde_json::json!([[7, [SLOT_ATTACK1]]]));
    }
    #[test]
    fn two_attack_slots_per_enemy() {
        let mut squad = Squad::new("cops");
        for m in [3, 4, 5] {
            squad.add(m);
        }
        assert_eq!(squad.leader(), Some(3));
        // A lone member is no leader (GetLeader needs two).
        let mut lone = Squad::new("one");
        lone.add(7);
        assert_eq!((lone.leader(), lone.num_members()), (None, 1));
        // The squad sound gate.
        lone.just_made_sound(1, 10.);
        assert!(!lone.ok_to_make_sound(1, 9.) && lone.ok_to_make_sound(2, 9.));
        assert!(lone.ok_to_make_sound(0, 10.5));
        let (mut a, mut b, mut c) = (None, None, None);
        assert!(squad.occupy_range(Some(0), SLOT_ATTACK1, SLOT_ATTACK2, &mut a));
        assert!(squad.occupy_range(Some(0), SLOT_ATTACK1, SLOT_ATTACK2, &mut b));
        assert!(!squad.occupy_range(Some(0), SLOT_ATTACK1, SLOT_ATTACK2, &mut c));
        // Slots are per enemy.
        assert!(squad.occupy_range(Some(9), SLOT_ATTACK1, SLOT_ATTACK2, &mut c));
        assert_eq!((a, b, c), (Some(0), Some(1), Some(0)));
        squad.vacate(Some(0), 0);
        let mut d = None;
        assert!(squad.occupy_range(Some(0), SLOT_ATTACK1, SLOT_ATTACK2, &mut d));
        assert_eq!(squad.others(4).collect::<Vec<_>>(), vec![3, 5]);
        assert!(should_take_squad_enemy(None, 1, false, 0.));
        assert!(!should_take_squad_enemy(Some(2), 1, true, 10.));
        assert!(should_take_squad_enemy(Some(2), 1, false, 3.5));
    }
}
