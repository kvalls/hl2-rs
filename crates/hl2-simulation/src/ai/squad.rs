//! CAI_Squad (ai_squad.cpp, ai_squadslot.h): members by name, the leader (first
//! member), per-enemy strategy slots (PER_ENEMY_SQUADSLOTS is defined in HL2), shared
//! enemy memory and SquadNewEnemy. SDK behavior, retail not compared.
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

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Squad {
    pub name: String,
    pub members: Vec<usize>,
    /// Occupied slots per enemy (None for no enemy).
    slots: BTreeMap<Option<usize>, BTreeSet<i32>>,
}
impl Squad {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }
    pub fn add(&mut self, member: usize) {
        if !self.members.contains(&member) {
            self.members.push(member);
        }
    }
    pub fn remove(&mut self, member: usize) {
        self.members.retain(|m| *m != member);
    }
    pub fn leader(&self) -> Option<usize> {
        self.members.first().copied()
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
    fn two_attack_slots_per_enemy() {
        let mut squad = Squad::new("cops");
        for m in [3, 4, 5] {
            squad.add(m);
        }
        assert_eq!(squad.leader(), Some(3));
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
