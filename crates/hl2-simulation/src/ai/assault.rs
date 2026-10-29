//! CAI_AssaultBehavior and CAI_AssaultGoal (SDK 2013 ai_behavior_assault.cpp, HL2
//! non-episodic paths): an ai_goal_assault sends its actors to a rally point
//! (assault_rallypoint), where they hold until the assault cue, then to an assault point
//! (assault_assaultpoint), where they fight within its tolerance until it clears or times
//! out, then on to the next assault point. The runtime resolves the map entities and
//! fires the outputs this module queues. SDK behavior; retail not compared beyond the
//! d1_canals_01 barrelpush cop (DESIGN 14a). Not modeled: rally sequences, force
//! crouch, urgent/strict movement styles, hint-group navigation, sentences, the
//! multiple-assault-point hull test, squads.
use super::conditions::{
    Conditions, COND_CAN_MELEE_ATTACK1, COND_CAN_MELEE_ATTACK2, COND_CAN_RANGE_ATTACK1,
    COND_CAN_RANGE_ATTACK2, COND_ENEMY_OCCLUDED, COND_HEAR_BULLET_IMPACT, COND_HEAR_COMBAT,
    COND_HEAR_DANGER, COND_HEAVY_DAMAGE, COND_LIGHT_DAMAGE, COND_LOW_PRIMARY_AMMO, COND_NEW_ENEMY,
    COND_NOT_FACING_ATTACK, COND_NO_PRIMARY_AMMO, COND_PLAYER_PUSHING, COND_PROVOKED,
    COND_SEE_ENEMY, COND_TOO_CLOSE_TO_ATTACK,
};
use super::schedule::{AiNpc, Context, Goal, Motor, Schedule, Schedules, TaskArg, TaskStatus};
use glam::Vec3;
use serde::Serialize;

/// AssaultCue_t.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Cue {
    NoAssault,
    EntityInput,
    PlayerGunfire,
    DontWait,
    Commander,
    None,
}
impl Cue {
    /// The ai_goal_assault "AssaultCue" keyvalue.
    pub fn from_keyvalue(value: i32) -> Self {
        match value {
            1 => Self::EntityInput,
            2 => Self::PlayerGunfire,
            3 => Self::DontWait,
            4 => Self::Commander,
            5 => Self::None,
            _ => Self::NoAssault,
        }
    }
}

/// CUE_POINT_TOLERANCE (3 feet).
pub const CUE_POINT_TOLERANCE: f32 = 36.;
/// ASSAULT_DIVERSION_TIME (seconds since an enemy was seen).
const DIVERSION_TIME: f64 = 4.;

/// An assault_rallypoint's keyvalues.
#[derive(Clone, Debug, Serialize)]
pub struct RallyPoint {
    pub entity: usize,
    pub origin: Vec3,
    pub yaw: f32,
    pub assault_point: String,
    pub delay: f32,
    pub priority: i32,
}
/// An assault_assaultpoint's keyvalues.
#[derive(Clone, Debug, Serialize)]
pub struct AssaultPoint {
    pub entity: usize,
    pub name: String,
    pub origin: Vec3,
    pub yaw: f32,
    pub next: String,
    pub timeout: f32,
    pub clear_on_contact: bool,
    pub allow_diversion: bool,
    pub diversion_radius: f32,
    pub never_timeout: bool,
    pub strict: bool,
    pub tolerance: f32,
    /// SF_ASSAULTPOINT_CLEARONARRIVAL.
    pub clear_on_arrival: bool,
}

/// The assault schedules (the HL2 non-episodic variants of SCHED_HOLD_RALLY_POINT).
const SCHEDULES: &[(&str, &str)] = &[
    (
        "SCHED_MOVE_TO_RALLY_POINT",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_ASSAULT_FAILED_TO_MOVE \
         TASK_GET_PATH_TO_RALLY_POINT 0 TASK_RUN_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 \
         TASK_STOP_MOVING 0 TASK_FACE_RALLY_POINT 0 TASK_HIT_RALLY_POINT 0 \
         TASK_SET_SCHEDULE SCHEDULE:SCHED_HOLD_RALLY_POINT \
         Interrupts COND_HEAR_DANGER COND_PROVOKED COND_NO_PRIMARY_AMMO COND_PLAYER_PUSHING",
    ),
    (
        "SCHED_ASSAULT_FAILED_TO_MOVE",
        "Tasks TASK_ASSAULT_DEFER_SCHEDULE_SELECTION 1 Interrupts",
    ),
    (
        "SCHED_FAIL_MOVE_TO_RALLY_POINT",
        "Tasks TASK_WAIT 1 Interrupts COND_HEAR_DANGER COND_CAN_RANGE_ATTACK1 \
         COND_CAN_MELEE_ATTACK1",
    ),
    (
        "SCHED_HOLD_RALLY_POINT",
        "Tasks TASK_FACE_RALLY_POINT 0 TASK_AWAIT_CUE 0 TASK_WAIT_ASSAULT_DELAY 0 \
         Interrupts COND_NEW_ENEMY COND_CAN_RANGE_ATTACK1 COND_CAN_MELEE_ATTACK1 \
         COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE COND_PLAYER_PUSHING COND_HEAR_DANGER \
         COND_HEAR_BULLET_IMPACT COND_NO_PRIMARY_AMMO COND_TOO_CLOSE_TO_ATTACK",
    ),
    (
        "SCHED_HOLD_ASSAULT_POINT",
        "Tasks TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE TASK_WAIT 3 \
         Interrupts COND_NEW_ENEMY COND_ENEMY_DEAD COND_CAN_RANGE_ATTACK1 \
         COND_CAN_MELEE_ATTACK1 COND_CAN_RANGE_ATTACK2 COND_CAN_MELEE_ATTACK2 \
         COND_TOO_CLOSE_TO_ATTACK COND_LOST_ENEMY COND_HEAR_DANGER COND_HEAR_BULLET_IMPACT \
         COND_NO_PRIMARY_AMMO",
    ),
    (
        "SCHED_MOVE_TO_ASSAULT_POINT",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_ASSAULT_FAILED_TO_MOVE \
         TASK_GATHER_CONDITIONS 0 TASK_GET_PATH_TO_ASSAULT_POINT 0 TASK_RUN_PATH 0 \
         TASK_WAIT_FOR_MOVEMENT 0 TASK_FACE_ASSAULT_POINT 0 TASK_HIT_ASSAULT_POINT 0 \
         Interrupts COND_NO_PRIMARY_AMMO COND_HEAR_DANGER",
    ),
    (
        "SCHED_AT_ASSAULT_POINT",
        "Tasks TASK_FACE_ASSAULT_POINT 0 TASK_HIT_ASSAULT_POINT 0 \
         Interrupts COND_NO_PRIMARY_AMMO COND_HEAR_DANGER",
    ),
    (
        "SCHED_WAIT_AND_CLEAR",
        "Tasks TASK_FACE_ASSAULT_POINT 0 TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE \
         TASK_AWAIT_ASSAULT_TIMEOUT 0 TASK_ANNOUNCE_CLEAR 0 \
         Interrupts COND_NEW_ENEMY COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE COND_CAN_RANGE_ATTACK1 \
         COND_CAN_MELEE_ATTACK1 COND_CAN_RANGE_ATTACK2 COND_CAN_MELEE_ATTACK2 COND_HEAR_DANGER \
         COND_HEAR_BULLET_IMPACT COND_TOO_CLOSE_TO_ATTACK COND_NOT_FACING_ATTACK \
         COND_PLAYER_PUSHING",
    ),
    (
        "SCHED_CLEAR_ASSAULT_POINT",
        "Tasks TASK_ANNOUNCE_CLEAR 0 Interrupts",
    ),
    (
        "SCHED_ASSAULT_MOVE_AWAY",
        "Tasks TASK_MOVE_AWAY_PATH 120 TASK_RUN_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 Interrupts",
    ),
];
pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in SCHEDULES {
        let schedule = Schedule::parse(name, text).expect("assault schedule");
        schedules.0.insert((*name).into(), schedule);
    }
}

/// CAI_AssaultBehavior state of one NPC (enabled by its ai_goal_assault).
#[derive(Clone, Debug, Serialize)]
pub struct Assault {
    /// The ai_goal_assault entity.
    pub goal: usize,
    pub cue: Cue,
    pub received: Cue,
    pub rally: RallyPoint,
    pub point: AssaultPoint,
    /// The assault chain by name (each name's candidate points), for ClearAssaultPoint.
    #[serde(skip)]
    pub chain: Vec<(String, Vec<AssaultPoint>)>,
    pub hit_rally: bool,
    pub hit_assault: bool,
    pub diverting: bool,
    pub last_saw_enemy: f64,
    pub defer_until: f64,
    wait_until: f64,
    /// Outputs to fire: (entity, output name).
    #[serde(skip)]
    pub outputs: Vec<(usize, &'static str)>,
    /// The rally point lock ends (UnlockRallyPoint); the runtime clears its lock table.
    #[serde(skip)]
    pub unlock: bool,
    /// ReceiveAssaultCue's SetCondition(COND_PROVOKED), applied at the next think.
    #[serde(skip)]
    pub provoke: bool,
    /// Facts for TranslateSchedule (refreshed at every selection and task).
    origin: Vec3,
    resting: bool,
    low_ammo: bool,
    seed: u64,
}
impl Assault {
    /// SetParameters + InitializeBehavior (the caller clears the schedule).
    pub fn new(
        goal: usize,
        cue: Cue,
        rally: RallyPoint,
        point: AssaultPoint,
        chain: Vec<(String, Vec<AssaultPoint>)>,
        seed: u64,
    ) -> Self {
        Self {
            goal,
            cue,
            received: Cue::NoAssault,
            rally,
            point,
            chain,
            hit_rally: false,
            hit_assault: false,
            diverting: false,
            last_saw_enemy: 0.,
            defer_until: 0.,
            wait_until: 0.,
            outputs: Vec::new(),
            unlock: false,
            provoke: false,
            origin: Vec3::ZERO,
            resting: false,
            low_ammo: false,
            seed: seed | 1,
        }
    }
    fn random_index(&mut self, len: usize) -> usize {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 33) as usize % len.max(1)
    }
    /// Disable (the goal deactivated, or the assault finished).
    pub fn disable(&mut self) {
        self.cue = Cue::NoAssault;
        self.hit_rally = false;
        self.hit_assault = false;
    }
    pub fn active(&self) -> bool {
        self.cue != Cue::NoAssault
    }
    /// ReceiveAssaultCue (BeginAssault): the caller sets COND_PROVOKED.
    pub fn receive_cue(&mut self, cue: Cue) {
        self.received = cue;
    }
    fn refresh(&mut self, npc: &AiNpc, ctx: &Context) {
        self.origin = ctx.origin;
        self.resting = npc
            .shot_regulator
            .as_ref()
            .is_some_and(|r| r.is_in_rest_interval(ctx.now));
        self.low_ammo =
            npc.conditions.has(COND_NO_PRIMARY_AMMO) || npc.conditions.has(COND_LOW_PRIMARY_AMMO);
    }
    /// AssaultHasBegun.
    fn has_begun(&self) -> bool {
        (self.cue == Cue::DontWait && self.hit_rally) || self.received == self.cue
    }
    /// PollAssaultCue.
    fn poll_cue(&mut self, npc: &AiNpc) -> bool {
        if self.received == Cue::Commander {
            return true;
        }
        match self.cue {
            Cue::NoAssault | Cue::None => false,
            Cue::EntityInput => self.received == Cue::EntityInput,
            Cue::PlayerGunfire => npc.conditions.has(COND_HEAR_COMBAT),
            Cue::DontWait => {
                self.received = Cue::DontWait;
                true
            }
            Cue::Commander => self.received == Cue::Commander,
        }
    }
    /// IsAllowedToDivert.
    fn allowed_to_divert(&self, enemy: Option<Vec3>, now: f64) -> bool {
        let p = &self.point;
        p.allow_diversion
            && (p.diversion_radius == 0.
                || (self.hit_assault
                    && enemy.is_some_and(|e| e.distance(p.origin) <= p.diversion_radius)))
            && self.last_saw_enemy != 0.
            && now - self.last_saw_enemy < DIVERSION_TIME
    }
    /// CanSelectSchedule (IsInterruptable and COND_RECEIVED_ORDERS are the host's).
    pub fn can_select(&mut self, npc: &mut AiNpc, ctx: &Context) -> bool {
        if self.defer_until > ctx.now || !self.active() {
            return false;
        }
        if npc.enemy.is_some() {
            self.last_saw_enemy = ctx.now;
        }
        if self.allowed_to_divert(ctx.enemy.map(|e| e.1), ctx.now) {
            self.diverting = true;
        } else if self.diverting {
            npc.conditions.set(COND_PROVOKED);
            self.diverting = false;
        }
        !self.diverting
    }
    /// EndScheduleSelection (another behavior or the NPC took over).
    pub fn end_selection(&mut self) {
        self.hit_assault = false;
        // Non-episodic rally points are never exclusive.
        self.hit_rally = false;
        self.unlock = true;
    }
    /// ClearAssaultPoint: on to the next assault point, else the assault ends.
    fn clear_point(&mut self) {
        let next = self.point.next.clone();
        if !next.is_empty() {
            let candidates = self
                .chain
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(&next))
                .map(|(_, c)| c.clone())
                .unwrap_or_default();
            if candidates.is_empty() {
                self.cue = Cue::NoAssault;
                return;
            }
            // FindAssaultPoint: one candidate, else a random one other than the current.
            let pick = if candidates.len() == 1 {
                0
            } else {
                let others: Vec<usize> = (0..candidates.len())
                    .filter(|&i| candidates[i].entity != self.point.entity)
                    .collect();
                others[self.random_index(others.len())]
            };
            self.point = candidates[pick].clone();
            self.hit_assault = false;
            return;
        }
        self.cue = Cue::NoAssault;
        self.unlock = true;
        self.outputs.push((self.point.entity, "OnAssaultClear"));
    }
    /// OnScheduleChange: leaving SCHED_WAIT_AND_CLEAR clears a clear-on-contact point
    /// once an enemy is seen.
    pub fn on_schedule_change(&mut self, npc: &AiNpc) {
        if npc.schedule.as_deref() == Some("SCHED_WAIT_AND_CLEAR")
            && self.point.clear_on_contact
            && npc.conditions.has(COND_SEE_ENEMY)
        {
            self.clear_point();
        }
    }
    /// SelectSchedule; None defers to the base CAI_BaseNPC selection.
    pub fn select(&mut self, npc: &mut AiNpc, ctx: &Context) -> Option<&'static str> {
        self.refresh(npc, ctx);
        let c = npc.conditions;
        if !self.point.strict {
            if c.has(COND_PLAYER_PUSHING) {
                return Some("SCHED_ASSAULT_MOVE_AWAY");
            }
            if c.has(COND_HEAR_DANGER) {
                return Some("SCHED_TAKE_COVER_FROM_BEST_SOUND");
            }
        }
        if c.has(COND_CAN_MELEE_ATTACK1) {
            return Some("SCHED_MELEE_ATTACK1");
        }
        if c.has(COND_NO_PRIMARY_AMMO) {
            return Some("SCHED_RELOAD");
        }
        let flat = |a: Vec3, b: Vec3| (a - b).truncate().length();
        if self.hit_rally && !self.hit_assault && !self.has_begun() {
            if flat(ctx.origin, self.rally.origin) > CUE_POINT_TOLERANCE {
                self.hit_rally = false;
                return Some("SCHED_MOVE_TO_RALLY_POINT");
            }
        } else if self.hit_assault && flat(ctx.origin, self.point.origin) > CUE_POINT_TOLERANCE {
            self.hit_assault = false;
        }
        if !self.hit_rally && !self.has_begun() {
            return Some("SCHED_MOVE_TO_RALLY_POINT");
        }
        if !self.hit_assault {
            if self.received == self.cue
                || self.received == Cue::Commander
                || self.cue == Cue::DontWait
            {
                self.unlock = true;
                return Some("SCHED_MOVE_TO_ASSAULT_POINT");
            }
            return Some(if c.has(COND_CAN_RANGE_ATTACK1) {
                "SCHED_RANGE_ATTACK1"
            } else if c.has(COND_LIGHT_DAMAGE) || c.has(COND_HEAVY_DAMAGE) {
                "SCHED_ALERT_FACE"
            } else if npc.enemy.is_some()
                && !c.has(COND_CAN_RANGE_ATTACK2)
                && !c.has(COND_ENEMY_OCCLUDED)
            {
                "SCHED_COMBAT_FACE"
            } else {
                "SCHED_HOLD_RALLY_POINT"
            });
        }
        if self.point.clear_on_arrival {
            return Some("SCHED_CLEAR_ASSAULT_POINT");
        }
        // HasConditionsToInterruptSchedule(SCHED_WAIT_AND_CLEAR).
        let wait_interrupts = [
            COND_NEW_ENEMY,
            COND_LIGHT_DAMAGE,
            COND_HEAVY_DAMAGE,
            COND_CAN_RANGE_ATTACK1,
            COND_CAN_MELEE_ATTACK1,
            COND_CAN_RANGE_ATTACK2,
            COND_CAN_MELEE_ATTACK2,
            COND_HEAR_DANGER,
            COND_HEAR_BULLET_IMPACT,
            COND_TOO_CLOSE_TO_ATTACK,
            COND_NOT_FACING_ATTACK,
            COND_PLAYER_PUSHING,
        ];
        if (npc.enemy.is_none() || c.has(COND_ENEMY_OCCLUDED))
            && !wait_interrupts.iter().any(|w| c.has(*w))
        {
            return Some("SCHED_WAIT_AND_CLEAR");
        }
        if self.point.strict
            && ![
                COND_CAN_RANGE_ATTACK1,
                COND_CAN_RANGE_ATTACK2,
                COND_CAN_MELEE_ATTACK1,
                COND_CAN_MELEE_ATTACK2,
                COND_TOO_CLOSE_TO_ATTACK,
                COND_NOT_FACING_ATTACK,
            ]
            .iter()
            .any(|w| c.has(*w))
        {
            return Some("SCHED_WAIT_AND_CLEAR");
        }
        None
    }
    /// TranslateSchedule.
    pub fn translate(&self, name: &str) -> Option<&'static str> {
        Some(match name {
            "SCHED_ESTABLISH_LINE_OF_FIRE_FALLBACK" => "SCHED_COMBAT_FACE",
            "SCHED_RANGE_ATTACK1" if self.resting => "SCHED_COMBAT_FACE",
            "SCHED_MOVE_TO_WEAPON_RANGE" | "SCHED_CHASE_ENEMY" => {
                if self.hit_assault {
                    "SCHED_WAIT_AND_CLEAR"
                } else {
                    "SCHED_MOVE_TO_ASSAULT_POINT"
                }
            }
            "SCHED_HOLD_RALLY_POINT" if self.low_ammo => "SCHED_RELOAD",
            "SCHED_MOVE_TO_ASSAULT_POINT" if self.origin.distance(self.point.origin) <= 12. => {
                "SCHED_AT_ASSAULT_POINT"
            }
            _ => return None,
        })
    }
    /// BuildScheduleTestBits.
    pub fn custom_interrupts(&self, npc: &AiNpc, now: f64) -> Conditions {
        let mut c = Conditions::default();
        let enemy = npc
            .enemy
            .and_then(|e| npc.enemies.find(Some(e)))
            .map(|m| m.last_known);
        if self.allowed_to_divert(enemy, now)
            && matches!(
                npc.schedule.as_deref(),
                Some(
                    "SCHED_MOVE_TO_ASSAULT_POINT"
                        | "SCHED_MOVE_TO_RALLY_POINT"
                        | "SCHED_HOLD_RALLY_POINT"
                )
            )
        {
            c.set(COND_NEW_ENEMY);
            c.set(COND_SEE_ENEMY);
        }
        c
    }
    fn hit_assault_point(&mut self) {
        self.hit_assault = true;
        self.outputs.push((self.point.entity, "OnArrival"));
    }
    pub fn start_task(
        &mut self,
        npc: &mut AiNpc,
        task: &str,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        self.refresh(npc, ctx);
        use TaskStatus::*;
        Some(match task {
            "TASK_ASSAULT_DEFER_SCHEDULE_SELECTION" => {
                self.defer_until = ctx.now + f64::from(arg.number());
                Complete
            }
            "TASK_GATHER_CONDITIONS" => Complete,
            "TASK_ANNOUNCE_CLEAR" => {
                let p = &self.point;
                if !(!p.clear_on_arrival && p.never_timeout && p.next.is_empty()) {
                    self.clear_point();
                }
                Complete
            }
            "TASK_WAIT_ASSAULT_DELAY" => {
                self.wait_until = ctx.now + f64::from(self.rally.delay);
                return self.run_task(npc, task, arg, ctx, motor);
            }
            "TASK_AWAIT_ASSAULT_TIMEOUT" => {
                self.wait_until = ctx.now + f64::from(self.point.timeout);
                return self.run_task(npc, task, arg, ctx, motor);
            }
            "TASK_GET_PATH_TO_RALLY_POINT" => {
                if motor.path_to(self.rally.origin, true) {
                    Complete
                } else {
                    Failed
                }
            }
            "TASK_GET_PATH_TO_ASSAULT_POINT" => {
                if motor.path_to(self.point.origin, true) {
                    Complete
                } else {
                    Failed
                }
            }
            "TASK_FACE_RALLY_POINT" | "TASK_FACE_ASSAULT_POINT" => {
                if task == "TASK_FACE_ASSAULT_POINT" && npc.conditions.has(COND_CAN_RANGE_ATTACK1) {
                    return Some(Complete);
                }
                npc.set_turn_activity(motor);
                return self.run_task(npc, task, arg, ctx, motor);
            }
            "TASK_HIT_ASSAULT_POINT" => {
                self.hit_assault_point();
                Complete
            }
            "TASK_HIT_RALLY_POINT" => {
                self.hit_rally = true;
                self.outputs.push((self.rally.entity, "OnArrival"));
                Complete
            }
            "TASK_AWAIT_CUE" => {
                if self.poll_cue(npc) {
                    Complete
                } else {
                    if let Some((_, p)) = ctx.enemy {
                        motor.face(Goal::Position(p));
                    }
                    Running
                }
            }
            _ => return None,
        })
    }
    pub fn run_task(
        &mut self,
        npc: &mut AiNpc,
        task: &str,
        _arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        use TaskStatus::*;
        Some(match task {
            "TASK_WAIT_ASSAULT_DELAY" | "TASK_AWAIT_ASSAULT_TIMEOUT" => {
                if self.point.clear_on_contact && npc.conditions.has(COND_SEE_ENEMY) {
                    return Some(Complete);
                }
                if ctx.now >= self.wait_until
                    && (task == "TASK_WAIT_ASSAULT_DELAY" || !self.point.never_timeout)
                {
                    Complete
                } else {
                    Running
                }
            }
            "TASK_FACE_RALLY_POINT" | "TASK_FACE_ASSAULT_POINT" => {
                let yaw = if task == "TASK_FACE_RALLY_POINT" {
                    self.rally.yaw
                } else {
                    self.point.yaw
                };
                let faced = motor.face(Goal::Yaw(yaw));
                if npc.conditions.has(COND_CAN_RANGE_ATTACK1) || faced {
                    Complete
                } else {
                    Running
                }
            }
            "TASK_AWAIT_CUE" => {
                if self.poll_cue(npc) {
                    return Some(Complete);
                }
                if let Some((_, p)) = ctx.enemy {
                    if !npc.conditions.has(COND_ENEMY_OCCLUDED) {
                        motor.face(Goal::Position(p));
                    }
                }
                Running
            }
            "TASK_WAIT_FOR_MOVEMENT"
                if self.point.clear_on_contact && npc.conditions.has(COND_SEE_ENEMY) =>
            {
                self.clear_point();
                Complete
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::schedule::tests::FakeMotor;
    use super::*;

    fn point(entity: usize, name: &str, origin: Vec3, next: &str) -> AssaultPoint {
        AssaultPoint {
            entity,
            name: name.into(),
            origin,
            yaw: 180.,
            next: next.into(),
            timeout: 3.,
            clear_on_contact: false,
            allow_diversion: false,
            diversion_radius: 0.,
            never_timeout: false,
            strict: false,
            tolerance: 36.,
            clear_on_arrival: false,
        }
    }
    fn canals() -> Assault {
        // barrelpush_cop1_assault: rally and assault point at (632, -3400), cue 1.
        let ap = point(
            1133,
            "barrelpush_cop1_assault_ap",
            Vec3::new(632., -3400., 264.),
            "",
        );
        Assault::new(
            1131,
            Cue::from_keyvalue(1),
            RallyPoint {
                entity: 1132,
                origin: Vec3::new(632., -3400., 288.),
                yaw: 180.,
                assault_point: ap.name.clone(),
                delay: 0.,
                priority: 1,
            },
            ap.clone(),
            vec![(ap.name.clone(), vec![ap])],
            9,
        )
    }
    fn ctx(origin: Vec3, enemy: bool) -> Context {
        Context {
            now: 1.,
            origin,
            enemy: enemy.then_some((usize::MAX, Vec3::new(300., -3394., 296.))),
            ..Default::default()
        }
    }

    #[test]
    fn uncued_assault_runs_to_the_rally_point_then_fights_from_it() {
        let mut a = canals();
        let mut npc = AiNpc::new(126);
        npc.enemy = Some(usize::MAX);
        npc.conditions.set(COND_CAN_RANGE_ATTACK1);
        let at_spawn = ctx(Vec3::new(452., -3394., 256.), true);
        assert!(a.can_select(&mut npc, &at_spawn));
        // Even with a shot available, the rally point comes first (it interrupts nothing
        // but danger, provocation, ammo and pushing).
        assert_eq!(
            a.select(&mut npc, &at_spawn),
            Some("SCHED_MOVE_TO_RALLY_POINT")
        );
        let mut motor = FakeMotor::default();
        let arg = TaskArg::Number(0.);
        assert_eq!(
            a.start_task(
                &mut npc,
                "TASK_HIT_RALLY_POINT",
                &arg,
                &at_spawn,
                &mut motor
            ),
            Some(TaskStatus::Complete)
        );
        assert_eq!(a.outputs, [(1132, "OnArrival")]);
        // On the rally point without the cue: shoot from there, never advance or chase.
        let there = ctx(Vec3::new(630., -3400., 256.), true);
        assert_eq!(a.select(&mut npc, &there), Some("SCHED_RANGE_ATTACK1"));
        npc.conditions.clear(COND_CAN_RANGE_ATTACK1);
        assert_eq!(a.select(&mut npc, &there), Some("SCHED_COMBAT_FACE"));
        assert_eq!(
            a.translate("SCHED_CHASE_ENEMY"),
            Some("SCHED_MOVE_TO_ASSAULT_POINT")
        );
        // Pushed off the rally point: back to it.
        let away = ctx(Vec3::new(560., -3400., 256.), true);
        assert_eq!(a.select(&mut npc, &away), Some("SCHED_MOVE_TO_RALLY_POINT"));
    }

    #[test]
    fn cue_advances_to_the_assault_point_which_times_out_and_clears() {
        let mut a = canals();
        a.chain.push((
            "next".into(),
            vec![point(2000, "next", Vec3::new(900., -3400., 264.), "")],
        ));
        a.point.next = "next".into();
        a.hit_rally = true;
        let mut npc = AiNpc::new(126);
        let there = ctx(Vec3::new(630., -3400., 256.), false);
        assert_eq!(a.select(&mut npc, &there), Some("SCHED_HOLD_RALLY_POINT"));
        a.receive_cue(Cue::EntityInput);
        assert_eq!(
            a.select(&mut npc, &there),
            Some("SCHED_MOVE_TO_ASSAULT_POINT")
        );
        assert_eq!(
            a.translate("SCHED_MOVE_TO_ASSAULT_POINT"),
            Some("SCHED_AT_ASSAULT_POINT")
        );
        let mut motor = FakeMotor::default();
        let arg = TaskArg::Number(0.);
        a.start_task(&mut npc, "TASK_HIT_ASSAULT_POINT", &arg, &there, &mut motor);
        assert_eq!(a.select(&mut npc, &there), Some("SCHED_WAIT_AND_CLEAR"));
        // The 3 s vigil ends, the point clears and the next one is assigned.
        assert_eq!(
            a.start_task(
                &mut npc,
                "TASK_AWAIT_ASSAULT_TIMEOUT",
                &arg,
                &there,
                &mut motor
            ),
            Some(TaskStatus::Running)
        );
        let later = Context {
            now: 4.5,
            ..there.clone()
        };
        assert_eq!(
            a.run_task(
                &mut npc,
                "TASK_AWAIT_ASSAULT_TIMEOUT",
                &arg,
                &later,
                &mut motor
            ),
            Some(TaskStatus::Complete)
        );
        a.start_task(&mut npc, "TASK_ANNOUNCE_CLEAR", &arg, &later, &mut motor);
        assert_eq!(a.point.entity, 2000);
        assert!(!a.hit_assault && a.active());
        // The last point clears the assault.
        a.start_task(&mut npc, "TASK_ANNOUNCE_CLEAR", &arg, &later, &mut motor);
        assert!(!a.active());
        assert!(a.outputs.contains(&(2000, "OnAssaultClear")));
    }
}
