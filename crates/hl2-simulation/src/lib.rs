//! Engine-independent gameplay, entities, NPC and physics simulation shared by HL2 hosts.
//!
//! The Rapier adapter preserves the retained runtime's behavior and limitations;
//! extracting it does not establish equivalence with Valve's VPhysics solver.

pub mod drag;
pub mod fire;
pub mod npc_probe;
pub mod physics;

pub mod actors;
pub mod ai;
pub mod citizens;
pub mod entities;
pub mod flashlight;
pub mod footsteps;
pub mod gameplay;
pub mod gibs;
pub mod globals;
pub mod grab;
pub mod impact_damage;
pub mod muzzle_effects;
pub mod npc;
mod npc_acttables;
pub mod npc_makers;
pub mod npc_weapons;
pub mod phys_explosion;
pub mod player_damage;
pub mod player_pickup;
pub mod projectiles;
pub mod ragdolls;
pub mod selection;
pub mod skill;
pub mod sounds;
pub mod soundscapes;
pub mod suit;
pub mod view_punch;
pub mod viewmodel_effects;
pub mod weapon_bugbait;
pub mod weapon_crossbow;
pub mod weapon_physcannon;
pub mod weapon_rpg;

pub mod campaign;
pub mod captions;
pub mod commentary;
pub mod explosion_particles;
pub mod impacts;
pub mod pcf_runtime;
pub mod projectile_visuals;

pub mod attention;
pub mod gestures;
pub mod lipsync;

pub mod color_correction;
pub mod monitors;
pub mod motion_blur;
pub mod shadows;
pub mod tonemap;
pub mod view_control;
