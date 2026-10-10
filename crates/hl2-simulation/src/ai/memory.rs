//! CAI_Enemies (ai_memory.cpp): enemy memories with last known/seen positions, seen
//! times, reaction delay, free knowledge and discard. Entities are scene indices;
//! `None` is AI_UNKNOWN_ENEMY (a danger memory without an entity). SDK behavior.
use glam::Vec3;
use serde::Serialize;
use std::collections::BTreeMap;

/// AI_FREE_KNOWLEDGE_DURATION / AI_DEF_ENEMY_DISCARD_TIME.
pub const FREE_KNOWLEDGE_DURATION: f64 = 1.75;
pub const ENEMY_DISCARD_TIME: f64 = 60.;
/// AI_INVALID_TIME.
pub const INVALID_TIME: f64 = -1.;

/// AI_EnemyInfo_t.
#[derive(Clone, Debug, Serialize)]
pub struct EnemyInfo {
    pub last_known: Vec3,
    pub last_seen_position: Vec3,
    pub time_last_seen: f64,
    pub time_first_seen: f64,
    pub time_last_reacquired: f64,
    pub time_valid: f64,
    pub time_last_damage: f64,
    pub time_at_first_hand: f64,
    pub danger: bool,
    pub eluded: bool,
    pub unforgettable: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Enemies {
    pub memories: BTreeMap<Option<usize>, EnemyInfo>,
    pub free_knowledge: f64,
    pub discard_time: f64,
}
impl Default for Enemies {
    fn default() -> Self {
        Self {
            memories: BTreeMap::new(),
            free_knowledge: FREE_KNOWLEDGE_DURATION,
            discard_time: ENEMY_DISCARD_TIME,
        }
    }
}
impl Enemies {
    /// UpdateMemory: returns true for a new memory.
    pub fn update(
        &mut self,
        enemy: Option<usize>,
        position: Vec3,
        reaction_delay: f64,
        first_hand: bool,
        now: f64,
    ) -> bool {
        const DIST_REACQUIRE_SQ: f32 = 240. * 240.;
        const TIME_REACQUIRE: f64 = 4.;
        const MIN_DIST_TIME_REACQUIRE_SQ: f32 = 48. * 48.;
        if let Some(memory) = self.memories.get_mut(&enemy) {
            if first_hand {
                memory.time_last_seen = now;
            }
            memory.eluded = false;
            let delta = (memory.last_known - position).length_squared();
            if delta > DIST_REACQUIRE_SQ
                || delta > MIN_DIST_TIME_REACQUIRE_SQ
                    && now - memory.time_last_seen > TIME_REACQUIRE
            {
                memory.time_last_reacquired = now;
            }
            if delta > 144. {
                memory.last_known = position;
            }
            if first_hand && memory.time_at_first_hand == INVALID_TIME {
                memory.time_at_first_hand = now;
            }
            return false;
        }
        let seen = if first_hand {
            now
        } else {
            // Block free knowledge.
            now - (self.free_knowledge + 0.01)
        };
        self.memories.insert(
            enemy,
            EnemyInfo {
                last_known: position,
                last_seen_position: position,
                time_last_seen: seen,
                time_first_seen: seen,
                time_last_reacquired: seen,
                time_valid: if reaction_delay > 0. {
                    now + reaction_delay
                } else {
                    0.
                },
                time_last_damage: INVALID_TIME,
                time_at_first_hand: if first_hand { now } else { INVALID_TIME },
                danger: enemy.is_none(),
                eluded: false,
                unforgettable: false,
            },
        );
        true
    }
    /// RefreshMemories: discard dead/old memories; within the free-knowledge window the
    /// last known position follows the enemy; a current sighting moves the seen position.
    pub fn refresh(
        &mut self,
        now: f64,
        position_of: impl Fn(usize) -> Option<Vec3>,
        dead: impl Fn(usize) -> bool,
    ) {
        if self.free_knowledge >= self.discard_time {
            self.free_knowledge = self.discard_time - 0.1;
        }
        let (free, discard) = (self.free_knowledge, self.discard_time);
        self.memories.retain(|enemy, memory| match enemy {
            Some(id) if dead(*id) => false,
            None if !memory.danger => false,
            _ => memory.unforgettable || now <= memory.time_last_seen + discard,
        });
        for (enemy, memory) in &mut self.memories {
            let Some(position) = enemy.and_then(&position_of) else {
                continue;
            };
            if now <= memory.time_last_seen + free {
                memory.last_known = position;
            }
            if now <= memory.time_last_seen {
                memory.last_seen_position = position;
            }
        }
    }
    pub fn find(&self, enemy: Option<usize>) -> Option<&EnemyInfo> {
        self.memories.get(&enemy)
    }
    pub fn last_time_seen(&self, enemy: Option<usize>) -> f64 {
        self.find(enemy).map_or(INVALID_TIME, |m| m.time_last_seen)
    }
    pub fn mark_eluded(&mut self, enemy: Option<usize>) {
        if let Some(m) = self.memories.get_mut(&enemy) {
            m.eluded = true;
        }
    }
    pub fn took_damage_from(&mut self, enemy: Option<usize>, now: f64) {
        if let Some(m) = self.memories.get_mut(&enemy) {
            m.time_last_damage = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn memory_follows_cai_enemies() {
        let mut e = Enemies::default();
        assert!(e.update(Some(3), Vec3::ZERO, 0.5, true, 10.));
        assert!(!e.update(Some(3), Vec3::new(10., 0., 0.), 0., true, 11.));
        let m = e.find(Some(3)).unwrap();
        // Moved less than 12 units: the known position stays.
        assert_eq!(m.last_known, Vec3::ZERO);
        assert_eq!(m.time_valid, 10.5);
        // Second-hand news blocks free knowledge.
        assert!(e.update(Some(4), Vec3::ONE, 0., false, 20.));
        assert!((e.last_time_seen(Some(4)) - (20. - 1.76)).abs() < 1e-9);
        // Free knowledge tracks the enemy for 1.75 s after the last sighting.
        e.refresh(12., |_| Some(Vec3::new(500., 0., 0.)), |_| false);
        assert_eq!(e.find(Some(3)).unwrap().last_known, Vec3::new(500., 0., 0.));
        e.refresh(13.5, |_| Some(Vec3::new(900., 0., 0.)), |_| false);
        assert_eq!(e.find(Some(3)).unwrap().last_known, Vec3::new(500., 0., 0.));
        // Discarded after 60 s or when dead.
        e.refresh(80., |_| None, |id| id == 4);
        assert!(e.memories.is_empty());
    }
}
