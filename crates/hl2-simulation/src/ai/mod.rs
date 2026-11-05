//! Step 14 NPC AI core reconstructed from the Source SDK 2013 AI (ai_basenpc*.cpp,
//! ai_default.cpp, ai_memory.cpp, ai_senses.cpp, soundent.cpp, ai_squad.cpp,
//! hl2_gamerules.cpp). Engine-free; hosts supply world queries and the motor through
//! traits. Every behavior here is "SDK behavior, retail not compared".
pub mod actbusy;
pub mod ally_speech;
pub mod assault;
pub mod barnacle;
pub mod bird;
pub mod conditions;
pub mod default_schedules;
pub mod efficiency;
pub mod enemy_choice;
pub mod headcrab;
pub mod host;
pub mod humanoid;
pub mod memory;
pub mod metropolice;
pub mod moveshoot;
pub mod police;
pub mod relationships;
pub mod runtime;
pub mod schedule;
pub mod senses;
pub mod spawn;
pub mod squad;
pub mod state;
pub mod talker;
pub mod zombie;

/// The next number in [0, 1) of a class's own dice.
pub(crate) fn roll(seed: &mut u64) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed >> 40) as f32 / (1u32 << 24) as f32
}
