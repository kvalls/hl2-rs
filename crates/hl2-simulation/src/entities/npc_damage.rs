//! The hit reports the damage paths leave for the AI.
use glam::Vec3;

/// A hit on an NPC for its AI's OnTakeDamage_Alive: who, whether the player dealt it, how much
/// (0 from no character), the damage force in kg in/s (the entity damage path carries only its
/// direction) and the damage's kind bits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NpcDamage {
    pub victim: usize,
    pub by_player: bool,
    pub damage: f32,
    pub force: Vec3,
    pub kind: u32,
}
impl NpcDamage {
    pub fn new(victim: usize, by_player: bool, damage: f32, force: Vec3, kind: u32) -> Self {
        Self {
            victim,
            by_player,
            damage,
            force,
            kind,
        }
    }
    /// A bullet's or a club's.
    pub fn hit(victim: usize, by_player: bool, damage: f32, force: Vec3, club: bool) -> Self {
        use crate::player_damage::{DMG_BULLET, DMG_CLUB};
        let kind = if club { DMG_CLUB } else { DMG_BULLET };
        Self::new(victim, by_player, damage, force, kind)
    }
}
