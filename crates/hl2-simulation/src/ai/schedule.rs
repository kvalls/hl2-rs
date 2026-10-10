//! Schedules and tasks (ai_schedule.cpp, ai_basenpc_schedule.cpp): schedule
//! definitions (the default table plus class schedules parsed from the SDK's
//! "Tasks"/"Interrupts" text format), the MaintainSchedule loop (interrupts, task
//! completion/failure, fail schedules, SelectSchedule/TranslateSchedule through a class
//! trait) and the generic tasks. Movement, facing and activities go through `Motor`,
//! which the host implements over npc.rs. SDK behavior, retail not compared.
use super::conditions::*;
use super::state::NpcState;
use glam::Vec3;
use serde::Serialize;
use std::collections::BTreeMap;

/// A task argument as the SDK schedule text spells it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum TaskArg {
    Number(f32),
    Schedule(String),
    Activity(String),
    Task(String),
    State(String),
}
impl TaskArg {
    pub fn parse(text: &str) -> Self {
        let (kind, value) = text.split_once(':').unwrap_or(("", text));
        match kind.to_ascii_uppercase().as_str() {
            "SCHEDULE" => Self::Schedule(value.into()),
            "ACTIVITY" => Self::Activity(value.into()),
            "TASK" => Self::Task(value.into()),
            "NPC_STATE" | "STATE" => Self::State(value.into()),
            _ => Self::Number(text.parse().unwrap_or(0.)),
        }
    }
    pub fn number(&self) -> f32 {
        match self {
            Self::Number(n) => *n,
            _ => 0.,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Schedule {
    pub name: String,
    pub tasks: Vec<(String, TaskArg)>,
    pub interrupts: Conditions,
}
impl Schedule {
    /// Parse the SDK text format ("Tasks" pairs then "Interrupts" condition names).
    /// Unknown condition names are reported.
    pub fn parse(name: &str, text: &str) -> Result<Self, String> {
        let mut tokens = text.split_whitespace();
        let mut tasks = Vec::new();
        let mut interrupts = Conditions::default();
        let mut section = "";
        while let Some(token) = tokens.next() {
            match token {
                "Tasks" | "Interrupts" => section = token,
                _ if section == "Tasks" => {
                    let arg = tokens
                        .next()
                        .ok_or_else(|| format!("{name}: {token} without argument"))?;
                    tasks.push((token.to_owned(), TaskArg::parse(arg)));
                }
                _ if section == "Interrupts" => {
                    interrupts.set(
                        Cond::from_name(token).ok_or_else(|| format!("{name}: unknown {token}"))?,
                    );
                }
                _ => return Err(format!("{name}: {token} outside a section")),
            }
        }
        Ok(Self {
            name: name.into(),
            tasks,
            interrupts,
        })
    }
}

/// All known schedules by name.
#[derive(Clone, Debug)]
pub struct Schedules(pub BTreeMap<String, Schedule>);
impl Default for Schedules {
    fn default() -> Self {
        let mut map = BTreeMap::new();
        for (name, tasks, interrupts) in super::default_schedules::DEFAULT_SCHEDULES {
            let mut set = Conditions::default();
            for c in *interrupts {
                if let Some(c) = Cond::from_name(c) {
                    set.set(c);
                }
            }
            map.insert(
                (*name).to_owned(),
                Schedule {
                    name: (*name).to_owned(),
                    tasks: tasks
                        .iter()
                        .map(|(t, a)| ((*t).to_owned(), TaskArg::parse(a)))
                        .collect(),
                    interrupts: set,
                },
            );
        }
        Self(map)
    }
}

/// Where a face/path task points.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Goal {
    Position(Vec3),
    /// A yaw in degrees (TASK_FACE_IDEAL, turns).
    Yaw(f32),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum MoveStatus {
    Idle,
    Moving,
    Arrived,
    Failed,
}
/// The motor/navigator the host provides (npc.rs Controller, animation state).
pub trait Motor {
    fn set_activity(&mut self, activity: &str);
    /// The current activity's sequence has finished (m_fSequenceFinished).
    fn activity_finished(&self) -> bool;
    /// Stop; true once stopped.
    fn stop_moving(&mut self) -> bool;
    /// Turn toward the goal; true when facing within tolerance.
    fn face(&mut self, goal: Goal) -> bool;
    /// Build a route; false when there is none.
    fn path_to(&mut self, goal: Vec3, run: bool) -> bool;
    fn movement(&self) -> MoveStatus;
}

/// Per-think facts the class and the generic tasks read.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub now: f64,
    pub origin: Vec3,
    pub enemy: Option<(usize, Vec3)>,
    pub target: Option<Vec3>,
    pub best_sound: Option<Vec3>,
    /// The local player's position (TASK_FACE_PLAYER), when there is one.
    pub player: Option<Vec3>,
    /// The NPC's yaw in degrees.
    pub yaw: f32,
    /// A uniform [0, 1) draw for this think (random waits, sentence choice).
    pub random: f32,
}

/// A class's schedule hooks (SelectSchedule, TranslateSchedule, StartTask/RunTask).
pub trait Behavior {
    fn select_schedule(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        default_select_schedule(npc, ctx)
    }
    fn translate_schedule(&mut self, name: &str) -> String {
        name.to_owned()
    }
    /// BuildScheduleTestBits: class interrupts added to the current schedule's.
    fn custom_interrupts(&self, _npc: &AiNpc) -> Conditions {
        Conditions::default()
    }
    fn select_fail_schedule(&mut self, npc: &AiNpc, _failed: &str) -> String {
        npc.fail_schedule
            .clone()
            .unwrap_or_else(|| "SCHED_FAIL".into())
    }
    /// Start a class task; None when the class does not know it.
    fn start_task(
        &mut self,
        _npc: &mut AiNpc,
        _task: &str,
        _arg: &TaskArg,
        _ctx: &Context,
        _motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        None
    }
    fn run_task(
        &mut self,
        _npc: &mut AiNpc,
        _task: &str,
        _arg: &TaskArg,
        _ctx: &Context,
        _motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        None
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum TaskStatus {
    Running,
    Complete,
    Failed,
}

/// The base CAI_BaseNPC::SelectSchedule (idle/alert/combat/dead), without flinch,
/// interactions, weapons/shot regulator and squad attack slots, which classes add.
pub fn default_select_schedule(npc: &AiNpc, ctx: &Context) -> String {
    let c = &npc.conditions;
    let hearing = [
        COND_HEAR_DANGER,
        COND_HEAR_COMBAT,
        COND_HEAR_WORLD,
        COND_HEAR_BULLET_IMPACT,
        COND_HEAR_PLAYER,
    ];
    match npc.state {
        NpcState::Idle => {
            if hearing.iter().any(|h| c.has(*h)) {
                "SCHED_ALERT_FACE_BESTSOUND"
            } else {
                "SCHED_IDLE_STAND"
            }
        }
        NpcState::Alert => {
            if hearing.iter().any(|h| c.has(*h)) {
                "SCHED_ALERT_FACE_BESTSOUND"
            } else if ctx.now - npc.enemies.last_time_seen(None)
                < super::state::TIME_CARE_ABOUT_DAMAGE
            {
                "SCHED_ALERT_FACE"
            } else {
                "SCHED_ALERT_STAND"
            }
        }
        NpcState::Combat => {
            if c.has(COND_NEW_ENEMY)
                && npc
                    .enemy
                    .and_then(|e| npc.enemies.find(Some(e)))
                    .is_some_and(|m| ctx.now - m.time_first_seen < 2.)
            {
                "SCHED_WAKE_ANGRY"
            } else if !c.has(COND_SEE_ENEMY) {
                if !c.has(COND_ENEMY_OCCLUDED) {
                    "SCHED_COMBAT_FACE"
                } else {
                    "SCHED_CHASE_ENEMY"
                }
            } else if c.has(COND_CAN_RANGE_ATTACK1) {
                "SCHED_RANGE_ATTACK1"
            } else if c.has(COND_CAN_MELEE_ATTACK1) {
                "SCHED_MELEE_ATTACK1"
            } else if c.has(COND_NOT_FACING_ATTACK) {
                "SCHED_COMBAT_FACE"
            } else {
                "SCHED_CHASE_ENEMY"
            }
        }
        NpcState::Dead => "SCHED_DIE",
        NpcState::Script => "SCHED_AISCRIPT",
        _ => "SCHED_IDLE_STAND",
    }
    .into()
}

/// The schedule-relevant part of CAI_BaseNPC.
#[derive(Clone, Debug, Default, Serialize)]
pub struct AiNpc {
    pub entity: usize,
    pub state: NpcState,
    pub ideal_state: NpcState,
    pub conditions: Conditions,
    pub enemy: Option<usize>,
    #[serde(skip)]
    pub enemies: super::memory::Enemies,
    pub schedule: Option<String>,
    pub task: usize,
    task_started: bool,
    pub fail_schedule: Option<String>,
    wait_until: f64,
    pub save_position: Option<Vec3>,
    pub last_position: Option<Vec3>,
    pub activity: String,
    /// Diagnostics: tasks no implementation knew (failed).
    pub unsupported_tasks: BTreeMap<String, u32>,
    pub schedule_changes: u32,
}

/// Up to this many tasks may complete in one think (MaintainSchedule's loop limit).
const MAX_TASKS_PER_THINK: usize = 8;

impl AiNpc {
    pub fn new(entity: usize) -> Self {
        Self {
            entity,
            state: NpcState::Idle,
            ideal_state: NpcState::Idle,
            ..Default::default()
        }
    }
    /// The enemy's current position, else its last known position from memory.
    fn enemy_position(&self, ctx: &Context) -> Option<Vec3> {
        ctx.enemy.map(|e| e.1).or_else(|| {
            self.enemy
                .and_then(|e| self.enemies.find(Some(e)))
                .map(|m| m.last_known)
        })
    }
    fn set_schedule(&mut self, name: String) {
        self.schedule = Some(name);
        self.task = 0;
        self.task_started = false;
        self.fail_schedule = None;
        self.conditions.clear(COND_TASK_FAILED);
        self.conditions.clear(COND_SCHEDULE_DONE);
        self.schedule_changes += 1;
    }
    /// CAI_BaseNPC::MaintainSchedule for one think.
    pub fn maintain(
        &mut self,
        schedules: &Schedules,
        behavior: &mut dyn Behavior,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) {
        for _ in 0..MAX_TASKS_PER_THINK {
            let current = self.schedule.as_ref().and_then(|n| schedules.0.get(n));
            let failed = self.conditions.has(COND_TASK_FAILED);
            let custom = behavior.custom_interrupts(self);
            let valid = current.is_some_and(|s| {
                !failed
                    && !self.conditions.has(COND_SCHEDULE_DONE)
                    && !self.conditions.intersects(s.interrupts.union(custom))
            });
            if !valid {
                let next = if failed {
                    let name = self.schedule.clone().unwrap_or_default();
                    behavior.select_fail_schedule(self, &name)
                } else {
                    // GetNewSchedule: SelectIdealState is applied by the caller each think.
                    behavior.select_schedule(self, ctx)
                };
                let next = behavior.translate_schedule(&next);
                let next = if schedules.0.contains_key(&next) {
                    next
                } else {
                    *self
                        .unsupported_tasks
                        .entry(format!("schedule {next}"))
                        .or_default() += 1;
                    "SCHED_IDLE_STAND".into()
                };
                self.set_schedule(next);
            }
            let Some(schedule) = self
                .schedule
                .as_ref()
                .and_then(|n| schedules.0.get(n))
                .cloned()
            else {
                return;
            };
            let Some((task, arg)) = schedule.tasks.get(self.task).cloned() else {
                self.conditions.set(COND_SCHEDULE_DONE);
                continue;
            };
            let status = if !self.task_started {
                self.task_started = true;
                self.start_task(behavior, &task, &arg, ctx, motor)
            } else {
                self.run_task(behavior, &task, &arg, ctx, motor)
            };
            match status {
                TaskStatus::Running => return,
                TaskStatus::Complete => {
                    self.task += 1;
                    self.task_started = false;
                    if self.task >= schedule.tasks.len() {
                        self.conditions.set(COND_SCHEDULE_DONE);
                    }
                }
                TaskStatus::Failed => {
                    self.conditions.set(COND_TASK_FAILED);
                }
            }
        }
    }
    fn start_task(
        &mut self,
        behavior: &mut dyn Behavior,
        task: &str,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> TaskStatus {
        if let Some(status) = behavior.start_task(self, task, arg, ctx, motor) {
            return status;
        }
        use TaskStatus::*;
        let face = |motor: &mut dyn Motor, goal: Option<Vec3>| match goal {
            Some(p) => {
                if motor.face(Goal::Position(p)) {
                    Complete
                } else {
                    Running
                }
            }
            None => Failed,
        };
        match task {
            "TASK_WAIT" | "TASK_WAIT_FACE_ENEMY" => {
                self.wait_until = ctx.now + f64::from(arg.number());
                self.run_task(behavior, task, arg, ctx, motor)
            }
            "TASK_WAIT_RANDOM" => {
                // RandomFloat(0, flTaskData).
                self.wait_until = ctx.now + f64::from(ctx.random * arg.number());
                self.run_task(behavior, task, arg, ctx, motor)
            }
            "TASK_WAIT_INDEFINITE" => Running,
            "TASK_WAIT_PVS"
            | "TASK_SOUND_WAKE"
            | "TASK_SOUND_DIE"
            | "TASK_ANNOUNCE_ATTACK"
            | "TASK_REMEMBER"
            | "TASK_SET_ROUTE_SEARCH_TIME"
            | "TASK_SET_TOLERANCE_DISTANCE"
            | "TASK_IGNORE_OLD_ENEMIES"
            | "TASK_DEFER_DODGE"
            | "TASK_SPEAK_SENTENCE" => Complete,
            "TASK_SET_ACTIVITY" => {
                if let TaskArg::Activity(a) = arg {
                    self.activity = a.clone();
                    motor.set_activity(a);
                }
                Complete
            }
            "TASK_STOP_MOVING" => {
                if motor.stop_moving() {
                    Complete
                } else {
                    Running
                }
            }
            "TASK_SET_FAIL_SCHEDULE" => {
                if let TaskArg::Schedule(s) = arg {
                    self.fail_schedule = Some(s.clone());
                }
                Complete
            }
            "TASK_SET_SCHEDULE" => {
                if let TaskArg::Schedule(s) = arg {
                    let s = behavior.translate_schedule(s);
                    self.set_schedule(s);
                    // The new schedule starts on the next loop iteration.
                    self.task_started = false;
                    return Running;
                }
                Failed
            }
            "TASK_SUGGEST_STATE" => {
                if let TaskArg::State(s) = arg {
                    self.ideal_state = match s.as_str() {
                        "NPC_STATE_IDLE" | "IDLE" => NpcState::Idle,
                        "NPC_STATE_ALERT" | "ALERT" => NpcState::Alert,
                        "NPC_STATE_COMBAT" | "COMBAT" => NpcState::Combat,
                        _ => self.ideal_state,
                    };
                }
                Complete
            }
            "TASK_FACE_ENEMY" => face(motor, self.enemy_position(ctx)),
            "TASK_FACE_PLAYER" => face(motor, ctx.player),
            "TASK_FACE_TARGET" => face(motor, ctx.target),
            "TASK_FACE_SAVEPOSITION" => face(motor, self.save_position),
            "TASK_FACE_IDEAL" | "TASK_FACE_REASONABLE" => {
                face(motor, self.enemy_position(ctx).or(ctx.best_sound)).max_complete()
            }
            "TASK_TURN_LEFT" | "TASK_TURN_RIGHT" => {
                let sign = if task == "TASK_TURN_LEFT" { 1. } else { -1. };
                if motor.face(Goal::Yaw(sign * arg.number())) {
                    Complete
                } else {
                    Running
                }
            }
            "TASK_STORE_LASTPOSITION" => {
                self.last_position = Some(ctx.origin);
                Complete
            }
            "TASK_CLEAR_LASTPOSITION" => {
                self.last_position = None;
                Complete
            }
            "TASK_STORE_ENEMY_POSITION_IN_SAVEPOSITION" => {
                self.save_position = self.enemy_position(ctx);
                Complete
            }
            "TASK_STORE_BESTSOUND_REACTORIGIN_IN_SAVEPOSITION" => {
                self.save_position = ctx.best_sound;
                if ctx.best_sound.is_some() {
                    Complete
                } else {
                    Failed
                }
            }
            "TASK_GET_PATH_TO_ENEMY_LOS"
            | "TASK_GET_CHASE_PATH_TO_ENEMY"
            | "TASK_GET_PATH_TO_RANGE_ENEMY_LKP_LOS" => match ctx.enemy {
                Some((_, p)) if motor.path_to(p, true) => Complete,
                _ => Failed,
            },
            "TASK_GET_PATH_TO_TARGET" => match ctx.target {
                Some(p) if motor.path_to(p, true) => Complete,
                _ => Failed,
            },
            "TASK_GET_PATH_TO_LASTPOSITION" => match self.last_position {
                Some(p) if motor.path_to(p, true) => Complete,
                _ => Failed,
            },
            "TASK_GET_PATH_TO_BESTSOUND" => match ctx.best_sound {
                Some(p) if motor.path_to(p, false) => Complete,
                _ => Failed,
            },
            "TASK_RUN_PATH"
            | "TASK_WALK_PATH"
            | "TASK_RUN_PATH_FLEE"
            | "TASK_ITEM_RUN_PATH"
            | "TASK_WEAPON_RUN_PATH" => {
                let activity = if task == "TASK_WALK_PATH" {
                    "ACT_WALK"
                } else {
                    "ACT_RUN"
                };
                self.activity = activity.into();
                motor.set_activity(activity);
                Complete
            }
            "TASK_WAIT_FOR_MOVEMENT"
            | "TASK_PLAY_SEQUENCE"
            | "TASK_RANGE_ATTACK1"
            | "TASK_RANGE_ATTACK2"
            | "TASK_MELEE_ATTACK1"
            | "TASK_MELEE_ATTACK2"
            | "TASK_RELOAD"
            | "TASK_BIG_FLINCH"
            | "TASK_SMALL_FLINCH" => {
                let activity = match task {
                    "TASK_RANGE_ATTACK1" => Some("ACT_RANGE_ATTACK1"),
                    "TASK_RANGE_ATTACK2" => Some("ACT_RANGE_ATTACK2"),
                    "TASK_MELEE_ATTACK1" => Some("ACT_MELEE_ATTACK1"),
                    "TASK_MELEE_ATTACK2" => Some("ACT_MELEE_ATTACK2"),
                    "TASK_RELOAD" => Some("ACT_RELOAD"),
                    "TASK_BIG_FLINCH" => Some("ACT_BIG_FLINCH"),
                    "TASK_SMALL_FLINCH" => Some("ACT_SMALL_FLINCH"),
                    _ => match arg {
                        TaskArg::Activity(a) => Some(a.as_str()),
                        _ => None,
                    },
                };
                if let Some(a) = activity {
                    self.activity = a.into();
                    motor.set_activity(a);
                }
                Running
            }
            _ => {
                *self.unsupported_tasks.entry(task.to_owned()).or_default() += 1;
                Failed
            }
        }
    }
    fn run_task(
        &mut self,
        behavior: &mut dyn Behavior,
        task: &str,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> TaskStatus {
        if let Some(status) = behavior.run_task(self, task, arg, ctx, motor) {
            return status;
        }
        use TaskStatus::*;
        match task {
            "TASK_WAIT" | "TASK_WAIT_RANDOM" => {
                if ctx.now >= self.wait_until {
                    Complete
                } else {
                    Running
                }
            }
            "TASK_WAIT_FACE_ENEMY" => {
                if let Some((_, p)) = ctx.enemy {
                    motor.face(Goal::Position(p));
                }
                if ctx.now >= self.wait_until {
                    Complete
                } else {
                    Running
                }
            }
            "TASK_WAIT_INDEFINITE" => Running,
            "TASK_STOP_MOVING" => {
                if motor.stop_moving() {
                    Complete
                } else {
                    Running
                }
            }
            "TASK_FACE_ENEMY"
            | "TASK_FACE_TARGET"
            | "TASK_FACE_PLAYER"
            | "TASK_FACE_SAVEPOSITION"
            | "TASK_FACE_IDEAL"
            | "TASK_FACE_REASONABLE" => {
                let goal = match task {
                    "TASK_FACE_TARGET" => ctx.target,
                    "TASK_FACE_PLAYER" => ctx.player,
                    "TASK_FACE_SAVEPOSITION" => self.save_position,
                    _ => self.enemy_position(ctx).or(ctx.best_sound),
                };
                match goal {
                    Some(p) if motor.face(Goal::Position(p)) => Complete,
                    Some(_) => Running,
                    None => Complete,
                }
            }
            "TASK_TURN_LEFT" | "TASK_TURN_RIGHT" => {
                let sign = if task == "TASK_TURN_LEFT" { 1. } else { -1. };
                if motor.face(Goal::Yaw(sign * arg.number())) {
                    Complete
                } else {
                    Running
                }
            }
            "TASK_WAIT_FOR_MOVEMENT" => match motor.movement() {
                MoveStatus::Moving => Running,
                MoveStatus::Failed => Failed,
                MoveStatus::Arrived | MoveStatus::Idle => Complete,
            },
            "TASK_PLAY_SEQUENCE" | "TASK_RANGE_ATTACK1" | "TASK_RANGE_ATTACK2"
            | "TASK_MELEE_ATTACK1" | "TASK_MELEE_ATTACK2" | "TASK_RELOAD" | "TASK_BIG_FLINCH"
            | "TASK_SMALL_FLINCH" => {
                if motor.activity_finished() {
                    Complete
                } else {
                    Running
                }
            }
            _ => Running,
        }
    }
}
trait MaxComplete {
    fn max_complete(self) -> Self;
}
impl MaxComplete for TaskStatus {
    /// Facing an absent ideal yaw completes (the motor keeps its yaw).
    fn max_complete(self) -> Self {
        if self == TaskStatus::Failed {
            TaskStatus::Complete
        } else {
            self
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[derive(Default)]
    pub struct FakeMotor {
        pub activity: String,
        pub finished: bool,
        pub facing_ticks: u32,
        pub path: Option<Vec3>,
        pub status: Option<MoveStatus>,
        pub activities: Vec<String>,
    }
    impl Motor for FakeMotor {
        fn set_activity(&mut self, activity: &str) {
            self.activity = activity.into();
            self.activities.push(activity.into());
        }
        fn activity_finished(&self) -> bool {
            self.finished
        }
        fn stop_moving(&mut self) -> bool {
            true
        }
        fn face(&mut self, _goal: Goal) -> bool {
            self.facing_ticks += 1;
            self.facing_ticks >= 3
        }
        fn path_to(&mut self, goal: Vec3, _run: bool) -> bool {
            self.path = Some(goal);
            true
        }
        fn movement(&self) -> MoveStatus {
            self.status.unwrap_or(MoveStatus::Arrived)
        }
    }
    struct Base;
    impl Behavior for Base {}

    #[test]
    fn default_table_parses_and_text_format_round_trips() {
        let s = Schedules::default();
        assert!(s.0.len() >= 80);
        let idle = &s.0["SCHED_IDLE_STAND"];
        assert_eq!(idle.tasks[0].0, "TASK_STOP_MOVING");
        assert_eq!(idle.tasks[2], ("TASK_WAIT".into(), TaskArg::Number(5.)));
        assert!(idle.interrupts.has(COND_NEW_ENEMY) && idle.interrupts.has(COND_PROVOKED));
        let parsed = Schedule::parse(
            "SCHED_TEST",
            "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_FAIL TASK_WAIT 0.5 Interrupts COND_SEE_ENEMY",
        )
        .unwrap();
        assert_eq!(parsed.tasks[0].1, TaskArg::Schedule("SCHED_FAIL".into()));
        assert!(parsed.interrupts.has(COND_SEE_ENEMY));
        assert!(Schedule::parse("X", "Interrupts COND_BOGUS").is_err());
    }

    #[test]
    fn idle_stand_waits_then_is_interrupted_by_a_new_enemy() {
        let schedules = Schedules::default();
        let mut npc = AiNpc::new(1);
        let mut motor = FakeMotor::default();
        let mut ctx = Context::default();
        let mut base = Base;
        // 15 ms thinks.
        for t in 0..10 {
            ctx.now = f64::from(t) * 0.015;
            npc.maintain(&schedules, &mut base, &ctx, &mut motor);
        }
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_IDLE_STAND"));
        assert_eq!(motor.activity, "ACT_IDLE");
        assert_eq!(npc.task, 2, "waiting in TASK_WAIT 5");
        // COND_NEW_ENEMY interrupts; in combat the base picks SCHED_WAKE_ANGRY or faces.
        npc.conditions.set(COND_NEW_ENEMY);
        npc.state = NpcState::Combat;
        npc.enemy = Some(7);
        npc.enemies
            .update(Some(7), Vec3::X * 100., 0., true, ctx.now);
        ctx.enemy = Some((7, Vec3::X * 100.));
        ctx.now += 0.015;
        npc.maintain(&schedules, &mut base, &ctx, &mut motor);
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_WAKE_ANGRY"));
        assert!(npc.schedule_changes >= 2);
    }

    #[test]
    fn task_failure_runs_the_fail_schedule() {
        let mut schedules = Schedules::default();
        schedules.0.insert(
            "SCHED_TEST_CHASE".into(),
            Schedule::parse("SCHED_TEST_CHASE", "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_ALERT_STAND TASK_GET_PATH_TO_TARGET 0 Interrupts").unwrap(),
        );
        struct Chase;
        impl Behavior for Chase {
            fn select_schedule(&mut self, _: &mut AiNpc, _: &Context) -> String {
                "SCHED_TEST_CHASE".into()
            }
        }
        let mut npc = AiNpc::new(1);
        let mut motor = FakeMotor::default();
        // No target: TASK_GET_PATH_TO_TARGET fails, the fail schedule takes over.
        npc.maintain(&schedules, &mut Chase, &Context::default(), &mut motor);
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_ALERT_STAND"));
        let ctx = Context {
            target: Some(Vec3::Y * 64.),
            ..Default::default()
        };
        let mut npc = AiNpc::new(1);
        npc.maintain(&schedules, &mut Chase, &ctx, &mut motor);
        assert_eq!(motor.path, Some(Vec3::Y * 64.));
    }
}
