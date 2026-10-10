//! AI conditions (ai_condition.h SCOND_t) as a bitset. Shared conditions keep the SDK
//! order and names; classes add their own after `LAST_SHARED_CONDITION`.
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Cond(pub u8);

pub const COND_NONE: Cond = Cond(0);
pub const COND_IN_PVS: Cond = Cond(1);
pub const COND_IDLE_INTERRUPT: Cond = Cond(2);
pub const COND_LOW_PRIMARY_AMMO: Cond = Cond(3);
pub const COND_NO_PRIMARY_AMMO: Cond = Cond(4);
pub const COND_NO_SECONDARY_AMMO: Cond = Cond(5);
pub const COND_NO_WEAPON: Cond = Cond(6);
pub const COND_SEE_HATE: Cond = Cond(7);
pub const COND_SEE_FEAR: Cond = Cond(8);
pub const COND_SEE_DISLIKE: Cond = Cond(9);
pub const COND_SEE_ENEMY: Cond = Cond(10);
pub const COND_LOST_ENEMY: Cond = Cond(11);
pub const COND_ENEMY_WENT_NULL: Cond = Cond(12);
pub const COND_ENEMY_OCCLUDED: Cond = Cond(13);
pub const COND_TARGET_OCCLUDED: Cond = Cond(14);
pub const COND_HAVE_ENEMY_LOS: Cond = Cond(15);
pub const COND_HAVE_TARGET_LOS: Cond = Cond(16);
pub const COND_LIGHT_DAMAGE: Cond = Cond(17);
pub const COND_HEAVY_DAMAGE: Cond = Cond(18);
pub const COND_PHYSICS_DAMAGE: Cond = Cond(19);
pub const COND_REPEATED_DAMAGE: Cond = Cond(20);
pub const COND_CAN_RANGE_ATTACK1: Cond = Cond(21);
pub const COND_CAN_RANGE_ATTACK2: Cond = Cond(22);
pub const COND_CAN_MELEE_ATTACK1: Cond = Cond(23);
pub const COND_CAN_MELEE_ATTACK2: Cond = Cond(24);
pub const COND_PROVOKED: Cond = Cond(25);
pub const COND_NEW_ENEMY: Cond = Cond(26);
pub const COND_ENEMY_TOO_FAR: Cond = Cond(27);
pub const COND_ENEMY_FACING_ME: Cond = Cond(28);
pub const COND_BEHIND_ENEMY: Cond = Cond(29);
pub const COND_ENEMY_DEAD: Cond = Cond(30);
pub const COND_ENEMY_UNREACHABLE: Cond = Cond(31);
pub const COND_SEE_PLAYER: Cond = Cond(32);
pub const COND_LOST_PLAYER: Cond = Cond(33);
pub const COND_SEE_NEMESIS: Cond = Cond(34);
pub const COND_TASK_FAILED: Cond = Cond(35);
pub const COND_SCHEDULE_DONE: Cond = Cond(36);
pub const COND_SMELL: Cond = Cond(37);
pub const COND_TOO_CLOSE_TO_ATTACK: Cond = Cond(38);
pub const COND_TOO_FAR_TO_ATTACK: Cond = Cond(39);
pub const COND_NOT_FACING_ATTACK: Cond = Cond(40);
pub const COND_WEAPON_HAS_LOS: Cond = Cond(41);
pub const COND_WEAPON_BLOCKED_BY_FRIEND: Cond = Cond(42);
pub const COND_WEAPON_PLAYER_IN_SPREAD: Cond = Cond(43);
pub const COND_WEAPON_PLAYER_NEAR_TARGET: Cond = Cond(44);
pub const COND_WEAPON_SIGHT_OCCLUDED: Cond = Cond(45);
pub const COND_BETTER_WEAPON_AVAILABLE: Cond = Cond(46);
pub const COND_HEALTH_ITEM_AVAILABLE: Cond = Cond(47);
pub const COND_GIVE_WAY: Cond = Cond(48);
pub const COND_WAY_CLEAR: Cond = Cond(49);
pub const COND_HEAR_DANGER: Cond = Cond(50);
pub const COND_HEAR_THUMPER: Cond = Cond(51);
pub const COND_HEAR_BUGBAIT: Cond = Cond(52);
pub const COND_HEAR_COMBAT: Cond = Cond(53);
pub const COND_HEAR_WORLD: Cond = Cond(54);
pub const COND_HEAR_PLAYER: Cond = Cond(55);
pub const COND_HEAR_BULLET_IMPACT: Cond = Cond(56);
pub const COND_HEAR_PHYSICS_DANGER: Cond = Cond(57);
pub const COND_HEAR_MOVE_AWAY: Cond = Cond(58);
pub const COND_HEAR_SPOOKY: Cond = Cond(59);
pub const COND_NO_HEAR_DANGER: Cond = Cond(60);
pub const COND_FLOATING_OFF_GROUND: Cond = Cond(61);
pub const COND_MOBBED_BY_ENEMIES: Cond = Cond(62);
pub const COND_RECEIVED_ORDERS: Cond = Cond(63);
pub const COND_PLAYER_ADDED_TO_SQUAD: Cond = Cond(64);
pub const COND_PLAYER_REMOVED_FROM_SQUAD: Cond = Cond(65);
pub const COND_PLAYER_PUSHING: Cond = Cond(66);
pub const COND_NPC_FREEZE: Cond = Cond(67);
pub const COND_NPC_UNFREEZE: Cond = Cond(68);
pub const COND_TALKER_RESPOND_TO_QUESTION: Cond = Cond(69);
pub const COND_NO_CUSTOM_INTERRUPTS: Cond = Cond(70);
/// First class-specific condition (LAST_SHARED_CONDITION).
pub const LAST_SHARED_CONDITION: u8 = 71;
const NAMES: [&str; 71] = [
    "COND_NONE",
    "COND_IN_PVS",
    "COND_IDLE_INTERRUPT",
    "COND_LOW_PRIMARY_AMMO",
    "COND_NO_PRIMARY_AMMO",
    "COND_NO_SECONDARY_AMMO",
    "COND_NO_WEAPON",
    "COND_SEE_HATE",
    "COND_SEE_FEAR",
    "COND_SEE_DISLIKE",
    "COND_SEE_ENEMY",
    "COND_LOST_ENEMY",
    "COND_ENEMY_WENT_NULL",
    "COND_ENEMY_OCCLUDED",
    "COND_TARGET_OCCLUDED",
    "COND_HAVE_ENEMY_LOS",
    "COND_HAVE_TARGET_LOS",
    "COND_LIGHT_DAMAGE",
    "COND_HEAVY_DAMAGE",
    "COND_PHYSICS_DAMAGE",
    "COND_REPEATED_DAMAGE",
    "COND_CAN_RANGE_ATTACK1",
    "COND_CAN_RANGE_ATTACK2",
    "COND_CAN_MELEE_ATTACK1",
    "COND_CAN_MELEE_ATTACK2",
    "COND_PROVOKED",
    "COND_NEW_ENEMY",
    "COND_ENEMY_TOO_FAR",
    "COND_ENEMY_FACING_ME",
    "COND_BEHIND_ENEMY",
    "COND_ENEMY_DEAD",
    "COND_ENEMY_UNREACHABLE",
    "COND_SEE_PLAYER",
    "COND_LOST_PLAYER",
    "COND_SEE_NEMESIS",
    "COND_TASK_FAILED",
    "COND_SCHEDULE_DONE",
    "COND_SMELL",
    "COND_TOO_CLOSE_TO_ATTACK",
    "COND_TOO_FAR_TO_ATTACK",
    "COND_NOT_FACING_ATTACK",
    "COND_WEAPON_HAS_LOS",
    "COND_WEAPON_BLOCKED_BY_FRIEND",
    "COND_WEAPON_PLAYER_IN_SPREAD",
    "COND_WEAPON_PLAYER_NEAR_TARGET",
    "COND_WEAPON_SIGHT_OCCLUDED",
    "COND_BETTER_WEAPON_AVAILABLE",
    "COND_HEALTH_ITEM_AVAILABLE",
    "COND_GIVE_WAY",
    "COND_WAY_CLEAR",
    "COND_HEAR_DANGER",
    "COND_HEAR_THUMPER",
    "COND_HEAR_BUGBAIT",
    "COND_HEAR_COMBAT",
    "COND_HEAR_WORLD",
    "COND_HEAR_PLAYER",
    "COND_HEAR_BULLET_IMPACT",
    "COND_HEAR_PHYSICS_DANGER",
    "COND_HEAR_MOVE_AWAY",
    "COND_HEAR_SPOOKY",
    "COND_NO_HEAR_DANGER",
    "COND_FLOATING_OFF_GROUND",
    "COND_MOBBED_BY_ENEMIES",
    "COND_RECEIVED_ORDERS",
    "COND_PLAYER_ADDED_TO_SQUAD",
    "COND_PLAYER_REMOVED_FROM_SQUAD",
    "COND_PLAYER_PUSHING",
    "COND_NPC_FREEZE",
    "COND_NPC_UNFREEZE",
    "COND_TALKER_RESPOND_TO_QUESTION",
    "COND_NO_CUSTOM_INTERRUPTS",
];

impl Cond {
    pub fn name(self) -> &'static str {
        NAMES
            .get(usize::from(self.0))
            .copied()
            .unwrap_or("COND_CLASS_SPECIFIC")
    }
    pub fn from_name(name: &str) -> Option<Self> {
        NAMES
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .map(|i| Cond(i as u8))
    }
}

/// CAI_ScheduleBits: up to 128 conditions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Conditions(pub u128);
impl Conditions {
    pub fn of(conditions: &[Cond]) -> Self {
        let mut set = Self::default();
        for c in conditions {
            set.set(*c);
        }
        set
    }
    pub fn set(&mut self, c: Cond) {
        if c != COND_NONE {
            self.0 |= 1u128 << c.0;
        }
    }
    pub fn clear(&mut self, c: Cond) {
        self.0 &= !(1u128 << c.0);
    }
    pub fn has(&self, c: Cond) -> bool {
        self.0 & (1u128 << c.0) != 0
    }
    pub fn intersects(&self, other: Conditions) -> bool {
        self.0 & other.0 != 0
    }
    pub fn union(self, other: Conditions) -> Self {
        Self(self.0 | other.0)
    }
    pub fn names(&self) -> Vec<&'static str> {
        (0..128u8)
            .filter(|i| self.0 & (1u128 << i) != 0)
            .map(|i| Cond(i).name())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conditions_keep_sdk_names_and_set_semantics() {
        assert_eq!(COND_SEE_ENEMY.name(), "COND_SEE_ENEMY");
        assert_eq!(Cond::from_name("cond_hear_danger"), Some(COND_HEAR_DANGER));
        let mut set = Conditions::of(&[COND_NEW_ENEMY, COND_SEE_ENEMY, COND_NONE]);
        assert!(set.has(COND_SEE_ENEMY) && !set.has(COND_NONE));
        set.clear(COND_NEW_ENEMY);
        assert_eq!(set.names(), vec!["COND_SEE_ENEMY"]);
        assert!(set.intersects(Conditions::of(&[COND_SEE_ENEMY, COND_LIGHT_DAMAGE])));
        assert!(LAST_SHARED_CONDITION < 128);
    }
}
