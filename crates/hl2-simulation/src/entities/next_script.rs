//! A scripted_sequence's "Next Script" (`m_iszNextScript`; SDK scripted.cpp
//! CAI_ScriptedSequence::Activate, SequenceDone and PostIdleDone).
use super::{number, Scene};
use modkit_core::World;

/// SF_SCRIPT_REPEATABLE: the script may run again (and so may name itself as its next).
const SF_SCRIPT_REPEATABLE: u32 = 4;

impl Scene {
    /// The script named by `m_iszNextScript`: the first entity of that name (Activate's lookup),
    /// when it is a live scripted_sequence.
    fn next_script(&self, world: &World, id: usize) -> Option<usize> {
        let name = self.ent(world, id).get("m_iszNextScript")?.trim();
        if name.is_empty() {
            return None;
        }
        let next = world.entities.iter().position(|e| {
            e.get("targetname")
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })?;
        (world.entities[next].class() == "scripted_sequence" && !self.states[next].killed)
            .then_some(next)
    }

    /// SequenceDone with a next script: no post idle; the next script starts at once on the
    /// actor (a script names itself only when repeatable), and OnPostIdleEndSequence follows
    /// (PostIdleDone). d1_trainstation_01's pacer walks ss_pace_1 and ss_pace_2 in turn, each
    /// the other's next script. The SDK hands the next script this very NPC (SetTarget); ours
    /// begins it as its BeginSequence input does, finding the actor by its m_iszEntity.
    pub(super) fn start_next_script(&mut self, world: &World, id: usize, actor: usize) -> bool {
        let Some(next) = self.next_script(world, id) else {
            return false;
        };
        let flags = number(self.ent(world, id), "spawnflags", 0.) as u32;
        if next == id && flags & SF_SCRIPT_REPEATABLE == 0 {
            return false;
        }
        self.states[actor].script_post_idle = None;
        self.states[id].post_idle_done = Some((self.time, actor));
        self.begin_sequence(world, next);
        true
    }
}
