//! The behaviors a humanoid hosts: selection and task dispatch to the running one.
use super::Humanoid;
use crate::ai::schedule::{
    base_select_schedule, AiNpc, Behavior, Context, Motor, TaskArg, TaskStatus,
};

impl Humanoid {
    /// BehaviorSelectSchedule: the assault behavior when it can select (its base
    /// selection is CAI_BaseNPC's).
    pub(super) fn behavior_select(&mut self, npc: &mut AiNpc, ctx: &Context) -> Option<String> {
        let assault = self.assault.as_mut()?;
        if !assault.can_select(npc, ctx) {
            return None;
        }
        self.running_assault = true;
        Some(
            assault
                .select(npc, ctx)
                .map(String::from)
                .unwrap_or_else(|| base_select_schedule(npc, ctx, self.armed)),
        )
    }
    /// The running behavior's RunTask (`run`) or StartTask.
    pub(super) fn behavior_task(
        &mut self,
        run: bool,
        npc: &mut AiNpc,
        task: &str,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        if self.running_actbusy {
            return if run {
                self.actbusy.run_task(npc, task, arg, ctx, motor)
            } else {
                self.actbusy.start_task(npc, task, arg, ctx, motor)
            };
        }
        let running = self.running_assault;
        let assault = self.assault.as_mut().filter(|_| running)?;
        if run {
            assault.run_task(npc, task, arg, ctx, motor)
        } else {
            assault.start_task(npc, task, arg, ctx, motor)
        }
    }
}
