//! Step 14 NPC AI core reconstructed from the Source SDK 2013 AI (ai_basenpc*.cpp,
//! ai_default.cpp, ai_memory.cpp, ai_senses.cpp, soundent.cpp, ai_squad.cpp,
//! hl2_gamerules.cpp). Engine-free; hosts supply world queries and the motor through
//! traits. Every behavior here is "SDK behavior, retail not compared".
pub mod conditions;
pub mod default_schedules;
pub mod memory;
pub mod relationships;
pub mod schedule;
pub mod state;
