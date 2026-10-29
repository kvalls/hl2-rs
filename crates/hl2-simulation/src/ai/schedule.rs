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
    /// CAI_TacticalServices::FindCoverPos(threat, threat eye, min, max) from the NPC: a
    /// graph node the threat's eye cannot see. None when there is none (or no graph).
    fn find_cover(
        &mut self,
        _threat: Vec3,
        _threat_eye: Vec3,
        _min: f32,
        _max: f32,
    ) -> Option<Vec3> {
        None
    }
    /// CAI_TacticalServices::FindLos(threat, threat eye, min, max): a graph node in range
    /// of the threat from which the weapon sees the threat's eye.
    fn find_los(&mut self, _threat: Vec3, _threat_eye: Vec3, _min: f32, _max: f32) -> Option<Vec3> {
        None
    }
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
    /// The enemy's EyePosition (its feet plus view offset), with `enemy`.
    pub enemy_eye: Option<Vec3>,
    /// The enemy's last known position (enemy memory), with `enemy`.
    pub enemy_lkp: Option<Vec3>,
    /// The active weapon's (min, max) range (MIN/MAX of m_fMin/MaxRange1/2).
    pub weapon_range: Option<(f32, f32)>,
    /// The active weapon's GetMinBurst, GetMaxBurst and GetFireRate.
    pub weapon_burst: Option<(u32, u32, f32)>,
    /// The best sound's volume (CSound::Volume), with `best_sound`.
    pub best_sound_volume: f32,
    /// m_flDistTooFar (1024; SF_NPC_LONG_RANGE: 1e9).
    pub dist_too_far: f32,
    /// Whether the enemy has me in its view cone (FInViewCone), with `enemy`.
    pub enemy_sees_me: Option<bool>,
    /// Off the ground (falling: FL_ONGROUND clear).
    pub airborne: bool,
}

/// A class's schedule hooks (SelectSchedule, TranslateSchedule, StartTask/RunTask).
pub trait Behavior {
    fn select_schedule(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        default_select_schedule(npc, ctx)
    }
    /// TranslateSchedule (the class reads the NPC's conditions and enemy, like the SDK's
    /// member function).
    fn translate_schedule(&mut self, name: &str, _npc: &AiNpc, _ctx: &Context) -> String {
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

/// CAI_BaseNPC::SelectSchedule for a class that inherits it: the idle/alert/dead/script
/// branches of `default_select_schedule` and, in combat, `base_combat_schedule`.
pub fn base_select_schedule(npc: &mut AiNpc, ctx: &Context, armed: bool) -> String {
    if npc.state == NpcState::Combat {
        return base_combat_schedule(npc, ctx, armed);
    }
    default_select_schedule(npc, ctx)
}
/// CAI_BaseNPC::SelectCombatSchedule for an NPC with a gun (`armed`: GetActiveWeapon)
/// and no innate attacks (the humanoids; innate classes come with DESIGN 14n step 4).
/// Not modeled: forced interactions, flinches, D_FR enemies (FearSound/RUN_FROM_ENEMY)
/// and squad attack slots (no squads: every slot is free).
pub fn base_combat_schedule(npc: &mut AiNpc, ctx: &Context, armed: bool) -> String {
    let c = npc.conditions;
    if c.has(COND_NEW_ENEMY)
        && npc
            .enemy
            .and_then(|e| npc.enemies.find(Some(e)))
            .is_some_and(|m| ctx.now - m.time_first_seen < 2.)
    {
        return "SCHED_WAKE_ANGRY".into();
    }
    if c.has(COND_ENEMY_DEAD) {
        // SetEnemy(NULL) + ChooseEnemy happened in the runtime's gather; another enemy
        // reselects, none goes alert.
        npc.conditions.clear(COND_ENEMY_DEAD);
        if npc.enemy.is_some() {
            return base_combat_schedule(npc, ctx, armed);
        }
        npc.state = NpcState::Alert;
        return default_select_schedule(npc, ctx);
    }
    if c.has(COND_LOW_PRIMARY_AMMO) || c.has(COND_NO_PRIMARY_AMMO) {
        return "SCHED_HIDE_AND_RELOAD".into();
    }
    if !c.has(COND_SEE_ENEMY) {
        return if !c.has(COND_ENEMY_OCCLUDED) {
            "SCHED_COMBAT_FACE"
        } else if armed {
            "SCHED_ESTABLISH_LINE_OF_FIRE"
        } else {
            "SCHED_TAKE_COVER_FROM_ENEMY"
        }
        .into();
    }
    if c.has(COND_TOO_CLOSE_TO_ATTACK) {
        return "SCHED_BACK_AWAY_FROM_ENEMY".into();
    }
    if c.has(COND_WEAPON_PLAYER_IN_SPREAD)
        || c.has(COND_WEAPON_BLOCKED_BY_FRIEND)
        || c.has(COND_WEAPON_SIGHT_OCCLUDED)
    {
        return "SCHED_ESTABLISH_LINE_OF_FIRE".into();
    }
    if npc
        .shot_regulator
        .as_ref()
        .is_some_and(|r| r.is_in_rest_interval(ctx.now))
        && c.has(COND_CAN_RANGE_ATTACK1)
    {
        return "SCHED_COMBAT_FACE".into();
    }
    for (cond, schedule) in [
        (COND_CAN_RANGE_ATTACK1, "SCHED_RANGE_ATTACK1"),
        (COND_CAN_RANGE_ATTACK2, "SCHED_RANGE_ATTACK2"),
        (COND_CAN_MELEE_ATTACK1, "SCHED_MELEE_ATTACK1"),
        (COND_CAN_MELEE_ATTACK2, "SCHED_MELEE_ATTACK2"),
        (COND_NOT_FACING_ATTACK, "SCHED_COMBAT_FACE"),
    ] {
        if c.has(cond) {
            return schedule.into();
        }
    }
    if armed {
        "SCHED_MOVE_TO_WEAPON_RANGE"
    } else {
        "SCHED_TAKE_COVER_FROM_ENEMY"
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
    /// Diagnostics: the last task that failed, as "SCHEDULE/TASK".
    pub last_failed_task: Option<String>,
    /// CAI_ShotRegulator of the active gun (bits_CAP_USE_SHOT_REGULATOR humanoids).
    pub shot_regulator: Option<crate::npc_weapons::ShotRegulator>,
    /// bits_MEMORY_HAD_LOS: the enemy was in line of sight at the last think.
    pub had_enemy_los: bool,
    /// GOALTYPE_ENEMY navigation: the chase path's goal position and the earliest next
    /// repath (UpdateEnemyPos).
    pub enemy_goal: Option<Vec3>,
    enemy_repath_at: f64,
    /// m_UnreachableEnts: entities a path request failed to reach, until the expiry or
    /// until they move more than 120 units from where they were.
    pub unreachable: Vec<Unreachable>,
    /// ForceChooseNewEnemy (RememberUnreachable of the enemy): ChooseEnemy runs again.
    pub force_choose_enemy: bool,
}

/// UnreachableEnt_t.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Unreachable {
    pub entity: usize,
    pub expires: f64,
    pub location: Vec3,
}
/// RememberUnreachable's default duration (NPC_UNREACHABLE_TIMEOUT).
pub const UNREACHABLE_TIME: f64 = 3.;
/// IsUnreachable: an entity that moved this far is tested again.
const UNREACHABLE_DIST_TOLERANCE: f32 = 120.;

/// WorldSpaceCenter to the enemy's WorldSpaceCenter (both standing 72-unit hulls: the
/// centers sit 36 units above the origins, so the offsets cancel).
fn enemy_center_distance(ctx: &Context) -> Option<f32> {
    ctx.enemy.map(|(_, p)| p.distance(ctx.origin))
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
    /// GetEnemyLKP: the enemy's last known position from memory (which follows the enemy
    /// while seen and for AI_FREE_KNOWLEDGE_DURATION after), else its current position.
    /// The face and attack tasks aim here; before, they tracked an unseen enemy forever.
    fn enemy_position(&self, ctx: &Context) -> Option<Vec3> {
        self.enemy
            .and_then(|e| self.enemies.find(Some(e)))
            .map(|m| m.last_known)
            .or(ctx.enemy.map(|e| e.1))
    }
    /// CAI_BaseNPC::RememberUnreachable(entity, duration -1): 3 s from now, at its
    /// current position; the enemy itself also forces a new ChooseEnemy.
    pub fn remember_unreachable(&mut self, entity: usize, position: Vec3, now: f64) {
        if self.enemy == Some(entity) {
            self.force_choose_enemy = true;
        }
        let expires = now + UNREACHABLE_TIME;
        match self.unreachable.iter_mut().find(|u| u.entity == entity) {
            Some(u) => {
                u.expires = expires;
                u.location = position;
            }
            None => self.unreachable.push(Unreachable {
                entity,
                expires,
                location: position,
            }),
        }
    }
    /// CAI_BaseNPC::IsUnreachable: remembered, not expired and within 120 units of where
    /// it was (an expired or moved record is dropped).
    pub fn is_unreachable(&mut self, entity: usize, position: Vec3, now: f64) -> bool {
        let Some(index) = self.unreachable.iter().position(|u| u.entity == entity) else {
            return false;
        };
        let u = self.unreachable[index];
        if now > u.expires || position.distance(u.location) > UNREACHABLE_DIST_TOLERANCE {
            self.unreachable.swap_remove(index);
            return false;
        }
        true
    }
    /// The read-only IsUnreachable (BestEnemy's reachability), without dropping records.
    pub fn unreachable_now(&self, entity: usize, position: Vec3, now: f64) -> bool {
        self.unreachable.iter().any(|u| {
            u.entity == entity
                && now <= u.expires
                && position.distance(u.location) <= UNREACHABLE_DIST_TOLERANCE
        })
    }
    /// TASK_GET_PATH_TO_ENEMY: unreachable fails at once; a failed path request
    /// remembers the enemy as unreachable (RememberUnreachable).
    fn path_to_enemy(&mut self, ctx: &Context, motor: &mut dyn Motor) -> TaskStatus {
        let Some((enemy, position)) = ctx.enemy else {
            return TaskStatus::Failed;
        };
        if self.is_unreachable(enemy, position, ctx.now) {
            return TaskStatus::Failed;
        }
        if motor.path_to(position, true) {
            self.enemy_goal = Some(position);
            TaskStatus::Complete
        } else {
            self.remember_unreachable(enemy, position, ctx.now);
            TaskStatus::Failed
        }
    }
    /// TASK_GET_PATH_TO_ENEMY_LKP: the same to the enemy's last known position.
    fn path_to_enemy_lkp(&mut self, ctx: &Context, motor: &mut dyn Motor) -> TaskStatus {
        let Some((enemy, position)) = ctx.enemy else {
            return TaskStatus::Failed;
        };
        if self.is_unreachable(enemy, position, ctx.now) {
            return TaskStatus::Failed;
        }
        if motor.path_to(ctx.enemy_lkp.unwrap_or(position), true) {
            TaskStatus::Complete
        } else {
            self.remember_unreachable(enemy, position, ctx.now);
            TaskStatus::Failed
        }
    }
    /// SetSchedule from outside the selection (scripted schedules).
    pub fn force_schedule(&mut self, name: &str) {
        self.set_schedule(name.into());
    }
    /// ACT_IDLE as the ideal activity: CAI_BaseNPC::SetTurnActivity when a face task
    /// starts unfaced (turn activities ACT_90_LEFT/RIGHT, ACT_180_LEFT are not modeled)
    /// and GetStoppedActivity when movement stops (no arrival activities modeled). It
    /// also ends a finished gesture such as a harass wave.
    pub fn set_turn_activity(&mut self, motor: &mut dyn Motor) {
        if self.activity != "ACT_IDLE" {
            self.activity = "ACT_IDLE".into();
            motor.set_activity("ACT_IDLE");
        }
    }
    /// CAI_BaseHumanoid::StartTaskRangeAttack1: fail in the rest interval, else one
    /// shot (OnRangeAttack1 + ACT_RANGE_ATTACK1 restarted) or the angry idle mid-burst.
    fn start_regulated_attack(
        &mut self,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        let regulator = self.shot_regulator.as_mut()?;
        if regulator.is_in_rest_interval(ctx.now) {
            return Some(TaskStatus::Failed);
        }
        let activity = if regulator.should_shoot(ctx.now) {
            regulator.on_range_attack1(ctx.now, enemy_center_distance(ctx));
            "ACT_RANGE_ATTACK1"
        } else {
            "ACT_IDLE_ANGRY"
        };
        self.activity = activity.into();
        motor.set_activity(activity);
        Some(TaskStatus::Running)
    }
    /// CAI_BaseHumanoid::RunTaskRangeAttack1: keep facing the enemy; when the shot's
    /// activity ends, shoot again within the burst, else complete at the rest.
    fn run_regulated_attack(&mut self, ctx: &Context, motor: &mut dyn Motor) -> Option<TaskStatus> {
        self.shot_regulator.as_ref()?;
        if let Some(p) = self.enemy_position(ctx) {
            motor.face(Goal::Position(p));
        }
        if !motor.activity_finished() {
            return Some(TaskStatus::Running);
        }
        if self.enemy.is_none() {
            return Some(TaskStatus::Complete);
        }
        let regulator = self.shot_regulator.as_mut()?;
        if !regulator.is_in_rest_interval(ctx.now) {
            if regulator.should_shoot(ctx.now) {
                regulator.on_range_attack1(ctx.now, enemy_center_distance(ctx));
                self.activity = "ACT_RANGE_ATTACK1".into();
                motor.set_activity("ACT_RANGE_ATTACK1");
            }
            return Some(TaskStatus::Running);
        }
        Some(TaskStatus::Complete)
    }
    /// CAI_BaseNPC::UpdateEnemyPos while following a chase path: when the enemy's last
    /// known position strays from the goal by more than GetGoalRepathTolerance (120,
    /// scaled down by the seconds to reach it, x1.2 for non-player targets), refind the
    /// path (at most every 0.1 s). None: nothing to do; Some(false): no route. The
    /// smoothed-velocity term uses the metrocop run speed (we keep no NPC velocity).
    fn repath_to_enemy(&mut self, ctx: &Context, motor: &mut dyn Motor) -> Option<bool> {
        const RUN_SPEED: f32 = 190.;
        let goal = self.enemy_goal?;
        let (enemy, lkp) = ctx.enemy?;
        if ctx.now < self.enemy_repath_at
            || matches!(motor.movement(), MoveStatus::Arrived | MoveStatus::Idle)
        {
            return None;
        }
        let seconds = (ctx.origin - lkp).length() / RUN_SPEED;
        let mut tolerance = (120. * seconds).clamp(0., 120.);
        if enemy != super::metropolice::PLAYER {
            tolerance *= 1.2;
        }
        if goal.distance(lkp) <= tolerance {
            return None;
        }
        self.enemy_repath_at = ctx.now + 0.1;
        if !motor.path_to(lkp, true) {
            return Some(false);
        }
        self.enemy_goal = Some(lkp);
        Some(true)
    }
    fn set_schedule(&mut self, name: String) {
        self.enemy_goal = None;
        self.schedule = Some(name);
        self.task = 0;
        self.task_started = false;
        self.fail_schedule = None;
        // CAI_BaseNPC::SetSchedule: m_Conditions.ClearAll() except COND_IN_PVS (so a
        // condition raised while selecting, e.g. a baton change, does not interrupt it).
        let in_pvs = self.conditions.has(COND_IN_PVS);
        self.conditions = Conditions::default();
        if in_pvs {
            self.conditions.set(COND_IN_PVS);
        }
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
            let resting = self
                .shot_regulator
                .as_ref()
                .is_some_and(|r| r.is_in_rest_interval(ctx.now));
            let valid = current.is_some_and(|s| {
                let mut interrupts = s.interrupts.union(custom);
                // CAI_BaseHumanoid::BuildScheduleTestBits: COND_CAN_RANGE_ATTACK1 does not
                // interrupt while the shot regulator rests.
                if resting {
                    interrupts.clear(COND_CAN_RANGE_ATTACK1);
                }
                !failed
                    && !self.conditions.has(COND_SCHEDULE_DONE)
                    && !self.conditions.intersects(interrupts)
            });
            if !valid {
                let next = if failed {
                    let name = self.schedule.clone().unwrap_or_default();
                    behavior.select_fail_schedule(self, &name)
                } else {
                    // GetNewSchedule: SelectIdealState is applied by the caller each think.
                    behavior.select_schedule(self, ctx)
                };
                let next = behavior.translate_schedule(&next, self, ctx);
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
                    self.last_failed_task = Some(format!(
                        "{}/{task}",
                        self.schedule.as_deref().unwrap_or_default()
                    ));
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
        if task == "TASK_RANGE_ATTACK1" {
            if let Some(status) = self.start_regulated_attack(ctx, motor) {
                return status;
            }
        }
        let status = self.start_generic_task(behavior, task, arg, ctx, motor);
        // StartTask TASK_FACE_TARGET always calls SetTurnActivity; the other face tasks
        // when not facing yet.
        if task == "TASK_FACE_TARGET" && status != TaskStatus::Failed
            || task.starts_with("TASK_FACE_") && status == TaskStatus::Running
        {
            self.set_turn_activity(motor);
        }
        status
    }
    fn start_generic_task(
        &mut self,
        behavior: &mut dyn Behavior,
        task: &str,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> TaskStatus {
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
            // SetWait(4): after 4 s of falling the SDK slams the ground entity (done here
            // by completing).
            "TASK_FALL_TO_GROUND" => {
                self.wait_until = ctx.now + 4.;
                self.run_task(behavior, task, arg, ctx, motor)
            }
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
                    // SetIdealActivity(GetStoppedActivity()): ACT_IDLE (no arrival
                    // activities are modeled).
                    self.set_turn_activity(motor);
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
                    let s = behavior.translate_schedule(s, self, ctx);
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
                // GetEnemy()->GetAbsOrigin(), not the LKP.
                self.save_position = ctx.enemy.map(|e| e.1);
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
            "TASK_GET_CHASE_PATH_TO_ENEMY" => {
                // The enemy itself when it is within flTaskData of its last known
                // position (chained TASK_GET_PATH_TO_ENEMY), else the LKP.
                let Some((_, p)) = ctx.enemy else {
                    return Failed;
                };
                let lkp = ctx.enemy_lkp.unwrap_or(p);
                if p.distance_squared(lkp) < arg.number().powi(2) {
                    self.path_to_enemy(ctx, motor)
                } else {
                    self.path_to_enemy_lkp(ctx, motor)
                }
            }
            "TASK_GET_PATH_TO_ENEMY" => self.path_to_enemy(ctx, motor),
            "TASK_GET_PATH_TO_RANGE_ENEMY_LKP_LOS" => match ctx.enemy {
                Some((_, p)) if motor.path_to(p, true) => Complete,
                _ => Failed,
            },
            "TASK_GET_PATH_TO_ENEMY_LOS" | "TASK_GET_PATH_TO_ENEMY_LKP_LOS" => {
                // FindLos from the enemy's origin (LKP for the _LKP_ task) and eye, within
                // the weapon's range (2000 without one) capped by m_flDistTooFar; the
                // lateral LOS probe is not modeled.
                let Some((_, origin)) = ctx.enemy else {
                    return Failed;
                };
                let (min, max) = ctx.weapon_range.unwrap_or((0., 2000.));
                let max = max.min(if ctx.dist_too_far > 0. {
                    ctx.dist_too_far
                } else {
                    1024.
                });
                let threat = if task == "TASK_GET_PATH_TO_ENEMY_LOS" {
                    origin
                } else {
                    ctx.enemy_lkp.unwrap_or(origin)
                };
                let eye = threat + (ctx.enemy_eye.unwrap_or(origin + Vec3::Z * 64.) - origin);
                match motor.find_los(threat, eye, min, max) {
                    Some(goal) if motor.path_to(goal, true) => Complete,
                    _ => Failed,
                }
            }
            "TASK_GET_PATH_TO_ENEMY_LKP" => self.path_to_enemy_lkp(ctx, motor),
            "TASK_FIND_COVER_FROM_ENEMY" | "TASK_FIND_COVER_FROM_ORIGIN" => {
                // FindCoverFromEnemy -> FindCoverPos(enemy origin, enemy eye, 0,
                // CoverRadius 1024) (self when there is no enemy; FROM_ORIGIN: from the
                // NPC's own origin and eye). FindLateralCover is not modeled.
                let (threat, eye) = match (task, ctx.enemy) {
                    ("TASK_FIND_COVER_FROM_ENEMY", Some((_, p))) => {
                        (p, ctx.enemy_eye.unwrap_or(p + Vec3::Z * 64.))
                    }
                    _ => (ctx.origin, ctx.origin + Vec3::Z * 64.),
                };
                match motor.find_cover(threat, eye, 0., 1024.) {
                    Some(goal) if motor.path_to(goal, true) => {
                        // m_flMoveWaitFinished = now + flTaskData.
                        Complete
                    }
                    _ => Failed,
                }
            }
            "TASK_FIND_COVER_FROM_BEST_SOUND" => {
                // FindCoverPos(sound react origin, same, MIN(volume, 120), 1024).
                match ctx.best_sound {
                    Some(sound) => {
                        let min = ctx.best_sound_volume.min(120.);
                        match motor.find_cover(sound, sound, min, 1024.) {
                            Some(goal) if motor.path_to(goal, true) => Complete,
                            _ => Failed,
                        }
                    }
                    None => Failed,
                }
            }
            "TASK_RUN_PATH_TIMED" => {
                self.activity = "ACT_RUN".into();
                motor.set_activity("ACT_RUN");
                self.wait_until = ctx.now + f64::from(arg.number());
                Running
            }
            "TASK_WAIT_FACE_ENEMY_RANDOM" => {
                self.wait_until = ctx.now + f64::from(ctx.random * arg.number());
                self.run_task(behavior, task, arg, ctx, motor)
            }
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
            // FL_ONGROUND.
            "TASK_FALL_TO_GROUND" => {
                if !ctx.airborne || ctx.now >= self.wait_until {
                    Complete
                } else {
                    Running
                }
            }
            "TASK_WAIT" | "TASK_WAIT_RANDOM" => {
                if ctx.now >= self.wait_until {
                    Complete
                } else {
                    Running
                }
            }
            "TASK_RUN_PATH_TIMED" => {
                if ctx.now >= self.wait_until
                    || matches!(motor.movement(), MoveStatus::Idle | MoveStatus::Arrived)
                {
                    Complete
                } else {
                    Running
                }
            }
            "TASK_WAIT_FACE_ENEMY" | "TASK_WAIT_FACE_ENEMY_RANDOM" => {
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
                    // SetIdealActivity(GetStoppedActivity()): ACT_IDLE (no arrival
                    // activities are modeled).
                    self.set_turn_activity(motor);
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
            "TASK_WAIT_FOR_MOVEMENT" if self.repath_to_enemy(ctx, motor) == Some(false) => Failed,
            "TASK_WAIT_FOR_MOVEMENT" => match motor.movement() {
                MoveStatus::Moving => Running,
                MoveStatus::Failed => Failed,
                MoveStatus::Arrived | MoveStatus::Idle => {
                    // The goal ended: SetIdealActivity(GetStoppedActivity()).
                    self.set_turn_activity(motor);
                    Complete
                }
            },
            "TASK_RANGE_ATTACK1" if self.shot_regulator.is_some() => {
                self.run_regulated_attack(ctx, motor).unwrap_or(Complete)
            }
            "TASK_RELOAD" => {
                if motor.activity_finished() {
                    // RunAttackTask: a reload ends with GetShotRegulator()->Reset(false),
                    // a fresh burst after a rest.
                    if let Some(regulator) = self.shot_regulator.as_mut() {
                        regulator.reset(false, ctx.now);
                    }
                    Complete
                } else {
                    Running
                }
            }
            "TASK_PLAY_SEQUENCE" | "TASK_RANGE_ATTACK1" | "TASK_RANGE_ATTACK2"
            | "TASK_MELEE_ATTACK1" | "TASK_MELEE_ATTACK2" | "TASK_BIG_FLINCH"
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
        /// path_to fails (no route) and counts the requests.
        pub no_route: bool,
        pub path_requests: u32,
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
            self.path_requests += 1;
            if self.no_route {
                return false;
            }
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
    fn unreachable_enemies_skip_path_requests_until_they_expire_or_move() {
        // TASK_GET_CHASE_PATH_TO_ENEMY 300 against an enemy no route reaches.
        let mut npc = AiNpc::new(1);
        npc.enemy = Some(7);
        let mut motor = FakeMotor {
            no_route: true,
            ..Default::default()
        };
        let arg = TaskArg::parse("300");
        let ctx = |now: f64, enemy: Vec3| Context {
            now,
            enemy: Some((7, enemy)),
            enemy_lkp: Some(enemy),
            ..Default::default()
        };
        let chase = |npc: &mut AiNpc, motor: &mut FakeMotor, c: &Context| {
            npc.start_task(&mut Base, "TASK_GET_CHASE_PATH_TO_ENEMY", &arg, c, motor)
        };
        let at = Vec3::new(500., 0., 200.);
        assert_eq!(
            chase(&mut npc, &mut motor, &ctx(1., at)),
            TaskStatus::Failed
        );
        assert_eq!(motor.path_requests, 1);
        // RememberUnreachable(GetEnemy()): ForceChooseNewEnemy, 3 s at that position.
        assert!(npc.force_choose_enemy);
        assert!(npc.unreachable_now(7, at, 3.9));
        // Within the 3 s and 120 units: fails without another path request.
        let near = at + Vec3::X * 100.;
        assert_eq!(
            chase(&mut npc, &mut motor, &ctx(2., near)),
            TaskStatus::Failed
        );
        assert_eq!(motor.path_requests, 1);
        // Moved more than 120 units: tested (and remembered) again.
        let moved = at + Vec3::X * 130.;
        assert_eq!(
            chase(&mut npc, &mut motor, &ctx(2.5, moved)),
            TaskStatus::Failed
        );
        assert_eq!(motor.path_requests, 2);
        // Expired: tested again; a route now completes and drops nothing else.
        motor.no_route = false;
        assert_eq!(
            chase(&mut npc, &mut motor, &ctx(6., moved)),
            TaskStatus::Complete
        );
        assert_eq!(motor.path_requests, 3);
        assert!(npc.unreachable.is_empty());
        // The enemy within 300 of its LKP: the enemy itself (GOALTYPE_ENEMY).
        assert_eq!(npc.enemy_goal, Some(moved));
        // Farther from its LKP: the LKP, without the enemy goal.
        npc.enemy_goal = None;
        let lkp_ctx = Context {
            enemy_lkp: Some(Vec3::ZERO),
            ..ctx(7., moved)
        };
        assert_eq!(chase(&mut npc, &mut motor, &lkp_ctx), TaskStatus::Complete);
        assert_eq!(motor.path, Some(Vec3::ZERO));
        assert_eq!(npc.enemy_goal, None);
    }

    #[test]
    fn chase_paths_follow_the_moving_enemy() {
        let mut npc = AiNpc::new(1);
        npc.enemy_goal = Some(Vec3::new(300., 0., 0.));
        let mut motor = FakeMotor {
            status: Some(MoveStatus::Moving),
            ..Default::default()
        };
        let ctx = |now: f64, enemy: Vec3| Context {
            now,
            enemy: Some((super::super::metropolice::PLAYER, enemy)),
            ..Default::default()
        };
        // Moved 50 of a 120-unit tolerance: keep the path.
        assert_eq!(
            npc.repath_to_enemy(&ctx(1., Vec3::new(300., 50., 0.)), &mut motor),
            None
        );
        // Moved 200: refind the path to the new position, then wait 0.1 s.
        let moved = Vec3::new(300., 200., 0.);
        assert_eq!(npc.repath_to_enemy(&ctx(1., moved), &mut motor), Some(true));
        assert_eq!(motor.path, Some(moved));
        let further = Vec3::new(300., 400., 0.);
        assert_eq!(npc.repath_to_enemy(&ctx(1.05, further), &mut motor), None);
        assert_eq!(
            npc.repath_to_enemy(&ctx(1.2, further), &mut motor),
            Some(true)
        );
        // Close by, the tolerance shrinks with the time to reach the enemy: 36 units
        // away (0.19 s at run speed) a 30-unit move already repaths.
        npc.enemy_goal = Some(Vec3::new(20., 0., 0.));
        assert_eq!(
            npc.repath_to_enemy(&ctx(2., Vec3::new(20., 30., 0.)), &mut motor),
            Some(true)
        );
    }
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
