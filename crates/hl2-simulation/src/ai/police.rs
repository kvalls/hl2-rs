//! ai_goal_police and CAI_PolicingBehavior (hl2/ai_goal_police.cpp,
//! hl2/ai_behavior_police.cpp): the metrocop holding a post, warning a target who comes
//! within twice the police radius (move-along sentences, harass gestures, warning-level
//! outputs), turning hostile with the baton after POLICE_MAX_WARNINGS or when the target
//! crosses the radius, and returning to the post. SDK behavior, retail not compared.
use super::conditions::*;
use super::schedule::{
    AiNpc, Behavior, Context, Goal, Motor, Schedule, Schedules, TaskArg, TaskStatus,
};
use super::state::NpcState;
use glam::Vec3;
use serde::Serialize;

/// ai_behavior_police.h.
pub const PATROL_RADIUS_RATIO: f32 = 2.;
pub const POLICE_MAX_WARNINGS: u32 = 4;
/// ai_goal_police.h spawnflags.
pub const SF_KNOCKOUT_BEHIND: u32 = 1 << 1;
pub const SF_DO_NOT_LEAVE_POST: u32 = 1 << 2;
/// Class conditions after the shared ones.
pub const COND_POLICE_TARGET_TOO_CLOSE_HARASS: Cond = Cond(LAST_SHARED_CONDITION);
pub const COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS: Cond = Cond(LAST_SHARED_CONDITION + 1);

/// The behavior's schedules in the SDK text format (ai_behavior_police.cpp).
const SCHEDULES: &[(&str, &str)] = &[
    (
        "SCHED_POLICE_WARN_TARGET",
        "Tasks TASK_STOP_MOVING 0 TASK_FACE_TARGET 0 TASK_POLICE_ANNOUNCE_HARASS 0 \
         TASK_PLAY_SEQUENCE ACTIVITY:ACT_POLICE_HARASS1 Interrupts COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS",
    ),
    (
        "SCHED_POLICE_HARASS_TARGET",
        "Tasks TASK_STOP_MOVING 0 TASK_FACE_TARGET 0 TASK_POLICE_GET_PATH_TO_HARASS_GOAL 64 \
         TASK_WAIT_FOR_MOVEMENT 0 TASK_POLICE_ANNOUNCE_HARASS 0 TASK_PLAY_SEQUENCE ACTIVITY:ACT_POLICE_HARASS1 \
         Interrupts COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS",
    ),
    (
        "SCHED_POLICE_SUPPRESS_TARGET",
        "Tasks TASK_STOP_MOVING 0 TASK_FACE_TARGET 0 TASK_POLICE_ANNOUNCE_HARASS 0 \
         TASK_PLAY_SEQUENCE ACTIVITY:ACT_POLICE_HARASS1 Interrupts",
    ),
    (
        "SCHED_POLICE_RETURN_FROM_HARASS",
        "Tasks TASK_STOP_MOVING 0 TASK_POLICE_GET_PATH_TO_POLICE_GOAL 16 TASK_WALK_PATH 0 \
         TASK_WAIT_FOR_MOVEMENT 0 TASK_STOP_MOVING 0 Interrupts COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS",
    ),
    (
        "SCHED_POLICE_TRACK_TARGET",
        "Tasks TASK_FACE_TARGET 0 Interrupts COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS",
    ),
    (
        "SCHED_POLICE_FACE_ALONG_GOAL",
        "Tasks TASK_WAIT_RANDOM 2 TASK_POLICE_FACE_ALONG_GOAL 0 Interrupts COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS",
    ),
];
/// Add the policing schedules to a registry (the condition names are class-specific,
/// so the parser sees them under their numeric bits).
pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in SCHEDULES {
        let text = text
            .replace("COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS", "")
            .replace("Interrupts", "Interrupts ");
        let mut schedule = Schedule::parse(name, &text).expect("police schedule");
        if SCHEDULES
            .iter()
            .any(|(n, t)| n == name && t.contains("COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS"))
        {
            schedule
                .interrupts
                .set(COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS);
        }
        schedules.0.insert((*name).into(), schedule);
    }
}

/// ai_goal_police keyvalues and state.
#[derive(Clone, Debug, Serialize)]
pub struct PoliceGoal {
    pub entity: usize,
    pub origin: Vec3,
    pub yaw: f32,
    /// PoliceRadius.
    pub radius: f32,
    /// PoliceTarget (resolved by the host).
    pub target: Option<usize>,
    pub spawnflags: u32,
    /// EnableKnockOut / DisableKnockOut.
    pub override_knockout: bool,
}
impl PoliceGoal {
    pub fn remain_at_post(&self) -> bool {
        self.spawnflags & SF_DO_NOT_LEAVE_POST != 0
    }
    /// CAI_PoliceGoal::ShouldKnockOutTarget: a visible target behind the goal's plane.
    pub fn should_knock_out(&self, target: Vec3, visible: bool) -> bool {
        if self.override_knockout {
            return true;
        }
        if self.spawnflags & SF_KNOCKOUT_BEHIND == 0 || !visible {
            return false;
        }
        let yaw = self.yaw.to_radians();
        Vec3::new(yaw.cos(), yaw.sin(), 0.).dot((target - self.origin).normalize_or_zero()) < 0.
    }
}

/// CAI_PolicingBehavior state for one metrocop.
#[derive(Clone, Debug, Serialize)]
pub struct Policing {
    pub goal: PoliceGoal,
    pub warnings: u32,
    pub next_harass: f64,
    pub aggressive_until: f64,
    pub hostile: bool,
    /// HostSetBatonState (the stunstick on/off request for the metrocop).
    pub baton: bool,
    /// ai_goal_police outputs to fire this think (OnFirstWarning ... OnSupressingTarget).
    pub outputs: Vec<&'static str>,
    /// Sentences to speak (HostSpeakSentence), e.g. "METROPOLICE_MOVE_ALONG_A".
    pub sentences: Vec<String>,
    /// The harass gesture for ACT_POLICE_HARASS1 by warning count (NPC_TranslateActivity).
    pub harass_activity: &'static str,
    /// The target's world-space center and whether the cop sees it (set each think).
    pub target_center: Option<Vec3>,
    pub target_visible: bool,
}
impl Policing {
    pub fn new(goal: PoliceGoal) -> Self {
        Self {
            goal,
            warnings: 0,
            next_harass: 0.,
            aggressive_until: 0.,
            hostile: false,
            baton: false,
            outputs: Vec::new(),
            sentences: Vec::new(),
            harass_activity: "ACT_POLICE_HARASS1",
            target_center: None,
            target_visible: false,
        }
    }
    /// GatherConditions: HARASS within twice the radius (and 32 units of height),
    /// SUPPRESS within the radius or for a knock-out.
    pub fn gather_conditions(&self, npc: &mut AiNpc) {
        npc.conditions.clear(COND_POLICE_TARGET_TOO_CLOSE_HARASS);
        npc.conditions.clear(COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS);
        let Some(target) = self.target_center else {
            return;
        };
        if self.goal.should_knock_out(target, self.target_visible) {
            npc.conditions.set(COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS);
        }
        let center = self.goal.origin + Vec3::Z * 36.;
        let distance = (center - target).truncate().length_squared();
        let radius = self.goal.radius * PATROL_RADIUS_RATIO;
        if distance < radius * radius && (center.z - target.z).abs() < 32. {
            npc.conditions.set(COND_POLICE_TARGET_TOO_CLOSE_HARASS);
            if distance < self.goal.radius * self.goal.radius {
                npc.conditions.set(COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS);
            }
        }
    }
    fn maintain_goal_position(&self, origin: Vec3) -> bool {
        (origin.z - self.goal.origin.z).abs() > 64.
            || (origin - self.goal.origin).truncate().length() > 16.
    }
    fn become_hostile(&mut self, npc: &mut AiNpc, ctx: &Context) {
        self.hostile = true;
        npc.enemy = self.goal.target;
        npc.state = NpcState::Combat;
        if let Some(target) = self.goal.target {
            npc.enemies.update(
                Some(target),
                self.target_center.unwrap_or_default(),
                0.,
                true,
                ctx.now,
            );
        }
        self.baton = true;
    }
    fn select_suppress(&mut self, npc: &mut AiNpc, ctx: &Context) -> &'static str {
        self.aggressive_until = ctx.now + 4.;
        if !self.hostile {
            self.become_hostile(npc, ctx);
            self.warnings = POLICE_MAX_WARNINGS;
            return "SCHED_COMBAT_FACE";
        }
        if self.goal.remain_at_post() {
            if self.maintain_goal_position(ctx.origin) {
                return "SCHED_CHASE_ENEMY";
            }
            return if self.next_harass < ctx.now {
                "SCHED_POLICE_WARN_TARGET"
            } else {
                "SCHED_COMBAT_FACE"
            };
        }
        "SCHED_CHASE_ENEMY"
    }
    fn select_harass(&mut self, npc: &mut AiNpc, ctx: &Context) -> Option<&'static str> {
        self.aggressive_until = ctx.now + 4.;
        if self.maintain_goal_position(ctx.origin) {
            return Some("SCHED_POLICE_RETURN_FROM_HARASS");
        }
        if self.next_harass >= ctx.now {
            return None;
        }
        match self.warnings {
            0 => self.outputs.push("OnFirstWarning"),
            1 => self.outputs.push("OnSecondWarning"),
            _ => {}
        }
        if self.warnings < POLICE_MAX_WARNINGS {
            self.warnings += 1;
        }
        if self.warnings >= POLICE_MAX_WARNINGS {
            if !self.hostile {
                self.become_hostile(npc, ctx);
                self.outputs.push("OnSupressingTarget");
                return Some("SCHED_COMBAT_FACE");
            }
            if !self.goal.remain_at_post() {
                return Some("SCHED_CHASE_ENEMY");
            }
        }
        if self.warnings == POLICE_MAX_WARNINGS - 1 {
            self.outputs.push("OnLastWarning");
            self.baton = true;
            if !self.goal.remain_at_post() {
                return Some("SCHED_POLICE_HARASS_TARGET");
            }
        }
        Some("SCHED_POLICE_WARN_TARGET")
    }
    /// AnnouncePolicing: MOVE_ALONG_A/B/C by warning count, then A or B at random.
    fn announce(&mut self, random: f32) {
        const WARNINGS: [&str; 3] = [
            "METROPOLICE_MOVE_ALONG_A",
            "METROPOLICE_MOVE_ALONG_B",
            "METROPOLICE_MOVE_ALONG_C",
        ];
        let index = if (1..=3).contains(&self.warnings) {
            self.warnings as usize - 1
        } else {
            usize::from(random >= 0.5)
        };
        self.sentences.push(WARNINGS[index].into());
    }
}
impl Behavior for Policing {
    fn select_schedule(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        self.select(npc, ctx).into()
    }
    fn translate_schedule(&mut self, name: &str) -> String {
        name.to_owned()
    }
    fn start_task(
        &mut self,
        _npc: &mut AiNpc,
        task: &str,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        Some(match task {
            "TASK_POLICE_ANNOUNCE_HARASS" => {
                self.announce(ctx.random);
                // Randomly say this again in 4 to 6 s (RandomInt).
                self.next_harass = ctx.now + 4. + (ctx.random * 3.).floor().min(2.) as f64;
                // NPC_TranslateActivity: HARASS1 on the first warning, HARASS2 after.
                self.harass_activity = if self.warnings == 1 {
                    "ACT_POLICE_HARASS1"
                } else {
                    "ACT_POLICE_HARASS2"
                };
                TaskStatus::Complete
            }
            "TASK_POLICE_GET_PATH_TO_POLICE_GOAL" => {
                if motor.path_to(self.goal.origin, false) {
                    TaskStatus::Complete
                } else {
                    TaskStatus::Failed
                }
            }
            "TASK_POLICE_GET_PATH_TO_HARASS_GOAL" => {
                let Some(target) = self.target_center else {
                    return Some(TaskStatus::Failed);
                };
                let center = ctx.origin + Vec3::Z * 36.;
                let distance = center.distance(target);
                if distance < arg.number() {
                    return Some(TaskStatus::Complete);
                }
                let direction = (target - center) / distance;
                let mut harass = ctx.origin + direction * (distance - arg.number());
                // A point on the policing radius along the same ray, when that is nearer the post.
                if let Some(t) =
                    ray_sphere_far(ctx.origin, direction, self.goal.origin, self.goal.radius)
                {
                    let on_radius = self.goal.origin + direction * t;
                    if (self.goal.origin - harass).truncate().length()
                        > (self.goal.origin - on_radius).truncate().length()
                    {
                        harass = on_radius;
                    }
                }
                if motor.path_to(harass, false) {
                    motor.set_activity("ACT_WALK_ANGRY");
                    TaskStatus::Complete
                } else {
                    TaskStatus::Failed
                }
            }
            "TASK_POLICE_FACE_ALONG_GOAL" => {
                if motor.face(Goal::Yaw(self.goal.yaw)) {
                    TaskStatus::Complete
                } else {
                    TaskStatus::Running
                }
            }
            "TASK_PLAY_SEQUENCE" if matches!(arg, TaskArg::Activity(a) if a == "ACT_POLICE_HARASS1") =>
            {
                motor.set_activity(self.harass_activity);
                TaskStatus::Running
            }
            _ => return None,
        })
    }
    fn run_task(
        &mut self,
        _npc: &mut AiNpc,
        task: &str,
        _arg: &TaskArg,
        _ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        match task {
            "TASK_POLICE_FACE_ALONG_GOAL" => Some(if motor.face(Goal::Yaw(self.goal.yaw)) {
                TaskStatus::Complete
            } else {
                TaskStatus::Running
            }),
            _ => None,
        }
    }
}
impl Policing {
    /// CAI_PolicingBehavior::SelectSchedule.
    pub fn select(&mut self, npc: &mut AiNpc, ctx: &Context) -> &'static str {
        if self.goal.target.is_none() {
            return "SCHED_IDLE_STAND";
        }
        if self.aggressive_until >= ctx.now && npc.conditions.has(COND_CAN_MELEE_ATTACK1) {
            return "SCHED_MELEE_ATTACK1";
        }
        if npc.conditions.has(COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS) {
            return self.select_suppress(npc, ctx);
        }
        if npc.conditions.has(COND_POLICE_TARGET_TOO_CLOSE_HARASS) {
            if let Some(s) = self.select_harass(npc, ctx) {
                return s;
            }
        }
        if self.aggressive_until < ctx.now {
            if npc.enemy.is_some() {
                npc.enemy = None;
                npc.state = NpcState::Alert;
            }
            self.baton = false;
            self.hostile = false;
        }
        if self.maintain_goal_position(ctx.origin) {
            return "SCHED_POLICE_RETURN_FROM_HARASS";
        }
        if self.baton {
            return "SCHED_POLICE_TRACK_TARGET";
        }
        if angle_diff(ctx.yaw, self.goal.yaw).abs() > 15. {
            return "SCHED_POLICE_FACE_ALONG_GOAL";
        }
        "SCHED_IDLE_STAND"
    }
    /// One think: gather the police conditions, then maintain the schedule with this
    /// behavior (state changes from selection are applied to `npc`).
    pub fn think(
        &mut self,
        npc: &mut AiNpc,
        schedules: &Schedules,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) {
        self.gather_conditions(npc);
        npc.maintain(schedules, self, ctx, motor);
    }
}
/// IntersectInfiniteRayWithSphere: the larger root, if any.
fn ray_sphere_far(origin: Vec3, direction: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let m = origin - center;
    let b = m.dot(direction);
    let c = m.length_squared() - radius * radius;
    let disc = b * b - c;
    (disc >= 0.).then(|| -b + disc.sqrt())
}
fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + 180.).rem_euclid(360.) - 180.
}

#[cfg(test)]
mod tests {
    use super::super::schedule::tests::FakeMotor;
    use super::*;

    fn cop() -> (Policing, AiNpc, Schedules) {
        let mut schedules = Schedules::default();
        add_schedules(&mut schedules);
        let goal = PoliceGoal {
            entity: 10,
            origin: Vec3::ZERO,
            yaw: 0.,
            radius: 64.,
            target: Some(0),
            spawnflags: SF_DO_NOT_LEAVE_POST,
            override_knockout: false,
        };
        (Policing::new(goal), AiNpc::new(1), schedules)
    }

    #[test]
    fn warnings_escalate_to_the_baton_and_hostility_at_the_post() {
        let (mut police, mut npc, schedules) = cop();
        let mut motor = FakeMotor {
            finished: true,
            ..Default::default()
        };
        let mut ctx = Context::default();
        // The player stands 100 units away: inside 2 x 64, outside 64.
        police.target_center = Some(Vec3::new(100., 0., 36.));
        police.target_visible = true;
        ctx.target = police.target_center;
        let mut warned_at = vec![];
        for t in 0..2000 {
            ctx.now = f64::from(t) * 0.015;
            let before = police.sentences.len();
            police.think(&mut npc, &schedules, &ctx, &mut motor);
            if police.sentences.len() > before {
                warned_at.push((ctx.now, police.warnings));
            }
        }
        // MOVE_ALONG_A, B, C every 4-6 s, the baton with the last warning (3 of 4).
        assert_eq!(
            &police.sentences[..3],
            [
                "METROPOLICE_MOVE_ALONG_A",
                "METROPOLICE_MOVE_ALONG_B",
                "METROPOLICE_MOVE_ALONG_C"
            ]
        );
        assert_eq!(
            &police.outputs[..3],
            ["OnFirstWarning", "OnSecondWarning", "OnLastWarning"]
        );
        assert!(police.baton);
        for pair in warned_at.windows(2) {
            let gap = pair[1].0 - pair[0].0;
            assert!((3.9..6.2).contains(&gap), "{warned_at:?}");
        }
        // The fourth warning makes the cop hostile toward the target.
        assert!(
            police.outputs.contains(&"OnSupressingTarget"),
            "{:?}",
            police.outputs
        );
        assert!(police.hostile && npc.enemy == Some(0) && npc.state == NpcState::Combat);
    }

    #[test]
    fn crossing_the_radius_suppresses_at_once() {
        let (mut police, mut npc, schedules) = cop();
        let mut motor = FakeMotor::default();
        police.target_center = Some(Vec3::new(40., 0., 36.));
        let ctx = Context {
            target: police.target_center,
            ..Default::default()
        };
        police.think(&mut npc, &schedules, &ctx, &mut motor);
        assert!(npc.conditions.has(COND_POLICE_TARGET_TOO_CLOSE_SUPPRESS));
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_COMBAT_FACE"));
        assert!(police.hostile && police.baton && police.warnings == POLICE_MAX_WARNINGS);
    }

    #[test]
    fn knock_out_behind_the_post_and_return_to_post() {
        let (mut police, mut npc, schedules) = cop();
        police.goal.spawnflags |= SF_KNOCKOUT_BEHIND;
        assert!(police.goal.should_knock_out(Vec3::new(-500., 0., 0.), true));
        assert!(!police
            .goal
            .should_knock_out(Vec3::new(-500., 0., 0.), false));
        assert!(!police.goal.should_knock_out(Vec3::new(500., 0., 0.), true));
        // Away from the post with nobody near: walk back.
        police.target_center = Some(Vec3::new(5000., 0., 36.));
        let ctx = Context {
            origin: Vec3::new(200., 0., 0.),
            ..Default::default()
        };
        let mut motor = FakeMotor {
            status: Some(super::super::schedule::MoveStatus::Moving),
            ..Default::default()
        };
        police.think(&mut npc, &schedules, &ctx, &mut motor);
        assert_eq!(
            npc.schedule.as_deref(),
            Some("SCHED_POLICE_RETURN_FROM_HARASS")
        );
        assert_eq!(motor.path, Some(Vec3::ZERO));
        assert_eq!(motor.activity, "ACT_WALK");
    }
}
