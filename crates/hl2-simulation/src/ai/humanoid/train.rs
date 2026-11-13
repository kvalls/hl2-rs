//! Citizens riding a func_tracktrain (CNPC_Citizen, SDK npc_citizen17.cpp; retail server.dll
//! holds the same schedule text and sequence names): d1_trainstation_01's passengers.
use super::Humanoid;
use crate::ai::schedule::{Motor, TaskStatus};
use source_assets::models::ai_activities::TRAIN_RIDE_SEQUENCES;

/// SCHED_CITIZEN_SIT_ON_TRAIN as CNPC_Citizen defines it.
pub(super) const SIT_ON_TRAIN: (&str, &str) = (
    "SCHED_CITIZEN_SIT_ON_TRAIN",
    "Tasks TASK_CIT_SIT_ON_TRAIN 0 TASK_WAIT_RANDOM 1 TASK_CIT_LEAVE_TRAIN 0 Interrupts",
);

/// A citizen spawned on a train: CNPC_Citizen::Spawn removes bits_CAP_MOVE_GROUND and sets
/// MOVETYPE_NONE when its move parent is a func_tracktrain.
#[derive(Clone, Debug, serde::Serialize)]
pub struct TrainRide {
    /// citizen_train_2 sits (NameMatches); every other rider stands.
    pub sits: bool,
    /// MOVETYPE_NONE: SelectSchedule returns SCHED_CITIZEN_SIT_ON_TRAIN.
    pub movetype_none: bool,
    /// The move parent is still a func_tracktrain.
    pub parented: bool,
    /// The first think checked the parent (Spawn's test).
    started: bool,
}

impl TrainRide {
    /// A citizen with a map-spawn parent; the first think keeps it only on a train.
    pub fn at_spawn(e: &modkit_core::Entity) -> Option<Self> {
        (e.class() == "npc_citizen" && e.get("parentname").is_some_and(|p| !p.is_empty())).then(
            || Self {
                sits: e
                    .get("targetname")
                    .is_some_and(|n| n.eq_ignore_ascii_case("citizen_train_2")),
                movetype_none: true,
                parented: true,
                started: false,
            },
        )
    }
    /// Sit_Idle, Sit_Exit, Stand, Stand_Exit (the loader's TRAIN_RIDE_SEQUENCES order).
    fn sequence(&self, leave: bool) -> &'static str {
        TRAIN_RIDE_SEQUENCES[usize::from(!self.sits) * 2 + usize::from(leave)]
    }
}

impl Humanoid {
    /// The runtime's per-think state: GetActiveWeapon and the train parent.
    pub(crate) fn refresh(&mut self, armed: bool, on_train: bool) {
        self.armed = armed;
        if let Some(train) = &mut self.train {
            if !train.started && !on_train {
                self.train = None;
                return;
            }
            train.started = true;
            train.parented = on_train;
        }
    }
    /// CNPC_Citizen::SelectSchedule: a citizen that cannot move rides the train.
    pub(super) fn train_schedule(&self) -> Option<String> {
        self.train
            .as_ref()
            .filter(|t| t.movetype_none)
            .map(|_| SIT_ON_TRAIN.0.to_owned())
    }
    /// TASK_CIT_SIT_ON_TRAIN plays the sit or stand sequence until the parent is gone (then
    /// MOVETYPE_STEP and the ground capability return); TASK_CIT_LEAVE_TRAIN plays the exit
    /// sequence to its end (SetupVPhysicsHull is not modeled).
    pub(super) fn train_task(
        &mut self,
        run: bool,
        task: &str,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        let train = self.train.as_mut()?;
        let leave = match task {
            "TASK_CIT_SIT_ON_TRAIN" => false,
            "TASK_CIT_LEAVE_TRAIN" => true,
            _ => return None,
        };
        if !run {
            motor.set_activity(train.sequence(leave));
            return Some(TaskStatus::Running);
        }
        Some(match leave {
            false if !train.parented => {
                train.movetype_none = false;
                TaskStatus::Complete
            }
            true if motor.activity_finished() => TaskStatus::Complete,
            _ => TaskStatus::Running,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::Kind;
    use super::*;
    use crate::ai::bird::tests::FakeMotor;

    fn citizen(name: &str, parent: &str) -> modkit_core::Entity {
        modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_citizen".into()),
                ("targetname".into(), name.into()),
                ("parentname".into(), parent.into()),
            ],
        }
    }

    #[test]
    fn train_riders_sit_or_stand_until_the_parent_is_cleared() {
        assert!(TrainRide::at_spawn(&citizen("citizen_train_1", "")).is_none());
        let mut h = Humanoid::new(Kind::Companion { citizen: true }, true, 1);
        h.train = TrainRide::at_spawn(&citizen("citizen_train_2", "intro_train_2"));
        h.refresh(true, true);
        assert_eq!(h.train_schedule().as_deref(), Some(SIT_ON_TRAIN.0));
        let mut motor = FakeMotor::default();
        let start = h.train_task(false, "TASK_CIT_SIT_ON_TRAIN", &mut motor);
        assert_eq!(start, Some(TaskStatus::Running));
        assert_eq!(motor.activity, "d1_t01_TrainRide_Sit_Idle");
        let run = h.train_task(true, "TASK_CIT_SIT_ON_TRAIN", &mut motor);
        assert_eq!(run, Some(TaskStatus::Running));
        // relay_train_reached_station: SetParent with no target.
        h.refresh(true, false);
        let run = h.train_task(true, "TASK_CIT_SIT_ON_TRAIN", &mut motor);
        assert_eq!(run, Some(TaskStatus::Complete));
        assert_eq!(h.train_schedule(), None);
        h.train_task(false, "TASK_CIT_LEAVE_TRAIN", &mut motor);
        assert_eq!(motor.activity, "d1_t01_TrainRide_Sit_Exit");
        motor.finished = true;
        let run = h.train_task(true, "TASK_CIT_LEAVE_TRAIN", &mut motor);
        assert_eq!(run, Some(TaskStatus::Complete));
        // A parent that is not a train at the first think: an ordinary citizen.
        let mut h = Humanoid::new(Kind::Companion { citizen: true }, false, 1);
        h.train = TrainRide::at_spawn(&citizen("citizen_train_1", "crate"));
        h.refresh(false, false);
        assert!(h.train.is_none());
    }
}
