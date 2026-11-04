//! TASK_ANNOUNCE_ATTACK for the Combine.
use super::super::metropolice::{CopRequest, PLAYER};
use super::super::schedule::{AiNpc, Context, Motor, TaskArg, TaskStatus};
use super::Humanoid;

impl Humanoid {
    /// TASK_ANNOUNCE_ATTACK for the Combine: the announcement and its 2 s wait in ACT_IDLE.
    pub(super) fn combine_announce(
        &mut self,
        npc: &mut AiNpc,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> TaskStatus {
        let grenade = matches!(arg, TaskArg::Number(n) if *n as i32 != 1);
        let announce = grenade
            || (ctx.enemy.is_some_and(|(e, _)| e == PLAYER)
                && ctx.enemy_sees_me == Some(false)
                && ctx.now - self.last_attack > 3.);
        if !announce {
            return TaskStatus::Complete;
        }
        if grenade {
            self.requests
                .push(CopRequest::Sentence("COMBINE_THROW_GRENADE"));
        } else {
            self.last_attack = ctx.now;
            self.requests.push(CopRequest::Sentence("COMBINE_ANNOUNCE"));
        }
        self.announce_until = ctx.now + 2.;
        npc.activity = "ACT_IDLE".into();
        motor.set_activity("ACT_IDLE");
        TaskStatus::Running
    }
}
