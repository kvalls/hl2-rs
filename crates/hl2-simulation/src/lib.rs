//! Engine-independent gameplay, entities, NPC and physics simulation shared by HL2 hosts.
//!
//! The Rapier adapter preserves the retained runtime's behavior and limitations;
//! extracting it does not establish equivalence with Valve's VPhysics solver.

pub mod npc_probe;
pub mod physics;

pub mod actors;
pub mod entities;
pub mod footsteps;
pub mod gameplay;
pub mod globals;
pub mod npc;
pub mod player_damage;
pub mod projectiles;
pub mod selection;
pub mod sounds;
pub mod soundscapes;
pub mod suit;
pub mod weapon_crossbow;
pub mod weapon_rpg;

pub mod campaign;
pub mod explosion_particles;
pub mod impacts;
pub mod projectile_visuals;

pub mod attention;
pub mod gestures;
pub mod lipsync;

pub mod monitors;
pub mod tonemap;
