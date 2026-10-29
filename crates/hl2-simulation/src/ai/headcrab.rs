//! npc_headcrab (CBaseHeadcrab + CHeadcrab, SDK npc_headcrab.cpp): the classic crab's leap
//! attack (RangeAttack1Conditions, SCHED_HEADCRAB_RANGE_ATTACK1, the JUMP_TELEGRAPH and
//! JUMPATTACK events, JumpAttack, LeapTouch), its sounds and turn rates. Not modeled yet: the
//! fast and black crabs, burrowing, ceiling hanging, canisters, drowning, hopping off NPCs,
//! harassing hops, barnacles, bites on NPCs (the leap's touch tests the player only).
use super::assault::Assault;
use super::conditions::*;
use super::metropolice::CopRequest;
use super::schedule::{
    base_select_schedule, AiNpc, Behavior, Context, Motor, Schedule, Schedules, TaskArg, TaskStatus,
};
use glam::Vec3;
use serde::Serialize;

/// HEADCRAB_MIN_JUMP_DIST / HEADCRAB_MAX_JUMP_DIST.
pub const MIN_JUMP_DIST: f32 = 48.;
pub const MAX_JUMP_DIST: f32 = 256.;
/// m_flFieldOfView (CBaseHeadcrab::Spawn).
pub const FIELD_OF_VIEW: f32 = 0.5;
/// HEADCRAB_IGNORE_WORLD_COLLISION_TIME: the leap's touch ignores solids this long.
pub const IGNORE_WORLD_COLLISION_TIME: f64 = 0.5;
/// A leap that ran into something solid delays the next one this long (RunTask).
pub const FAILED_ATTACK_DELAY: f64 = 1.2;
/// SetViewOffset(Vector(6, 0, 11)).
pub const EYE_HEIGHT: f32 = 11.;
pub const AE_JUMPATTACK: &str = "AE_HEADCRAB_JUMPATTACK";
pub const AE_JUMP_TELEGRAPH: &str = "AE_HEADCRAB_JUMP_TELEGRAPH";
/// Every sound cue this class emits (the host preloads them).
pub const SOUNDS: &[&str] = &[
    "NPC_HeadCrab.Idle",
    "NPC_HeadCrab.Alert",
    "NPC_HeadCrab.Pain",
    "NPC_HeadCrab.Die",
    "NPC_Headcrab.Attack",
    "NPC_HeadCrab.Bite",
    "NPC_BlackHeadcrab.Telegraph",
    "NPC_BlackHeadcrab.Attack",
    "NPC_BlackHeadcrab.Bite",
    "NPC_BlackHeadcrab.Pain",
    "NPC_BlackHeadcrab.Die",
];

/// CBaseHeadcrab::JumpAttack's velocity toward `target` (not a random hop): the arc's
/// height is the target's height above the origin clamped to 16..120 (400 when thrown),
/// plus 8 under 32 so the impact comes on the way down; the horizontal speed reaches the
/// target in that flight time; the whole capped at 650 u/s (1000 thrown).
pub fn jump_velocity(origin: Vec3, target: Vec3, gravity: f32, thrown: bool) -> Vec3 {
    let gravity = gravity.max(1.);
    let mut height = target.z - origin.z;
    if height < 16. {
        height = 16.;
    } else {
        height = height.min(if thrown { 400. } else { 120. });
    }
    let additional = if height < 32. { 8. } else { 0. };
    height += additional;
    let speed = (2. * gravity * height).sqrt();
    let time = speed / gravity + (2. * additional / gravity).sqrt();
    let mut velocity = (target - origin) / time;
    velocity.z = speed;
    let max = if thrown { 1000. } else { 650. };
    let length = velocity.length();
    if length > max {
        velocity *= max / length;
    }
    velocity
}
/// JumpAttack(bRandomJump): a hop along the facing (yaw in degrees) at 350 u/s.
pub fn random_hop(yaw: f32) -> Vec3 {
    let y = yaw.to_radians();
    Vec3::new(y.cos(), y.sin(), 1.) * 350.
}
/// The leap's facts for RangeAttack1Conditions.
#[derive(Clone, Copy, Debug)]
pub struct LeapCheck {
    /// m_flNextAttack has passed.
    pub ready: bool,
    pub on_ground: bool,
    /// 2D facing dot toward the enemy.
    pub dot: f32,
    /// EnemyDistance.
    pub distance: f32,
    /// The trace from its eyes to the enemy's eyes reaches the enemy.
    pub clear: bool,
    /// The enemy's eyes are more than 36 above its origin and a short hull trace toward
    /// them hits something else.
    pub high_blocked: bool,
}
/// CBaseHeadcrab::RangeAttack1Conditions (not burrowed). None is COND_NONE.
pub fn range_condition(check: LeapCheck) -> Option<Cond> {
    if !check.ready || !check.on_ground {
        return None;
    }
    if check.dot < 0.65 {
        return Some(COND_NOT_FACING_ATTACK);
    }
    if check.distance < MIN_JUMP_DIST {
        return Some(COND_TOO_CLOSE_TO_ATTACK);
    }
    if check.distance > MAX_JUMP_DIST {
        return Some(COND_TOO_FAR_TO_ATTACK);
    }
    if !check.clear {
        return None;
    }
    if check.high_blocked {
        return Some(COND_TOO_CLOSE_TO_ATTACK);
    }
    Some(COND_CAN_RANGE_ATTACK1)
}
/// CHeadcrab::MaxYawSpeed; CBlackHeadcrab::MaxYawSpeed (`black`: 10 moving, else 30).
pub fn max_yaw_speed(activity: &str, black: bool) -> f32 {
    match (activity, black) {
        ("ACT_RUN" | "ACT_WALK", false) => 20.,
        ("ACT_RUN" | "ACT_WALK", true) => 10.,
        ("ACT_TURN_LEFT" | "ACT_TURN_RIGHT", false) => 15.,
        _ => 30.,
    }
}
/// CBlackHeadcrab::TouchDamage (HL2, not episodic): the bite's damage, cut to leave one
/// health when it would kill, then DMG_POISON of all but one health; nothing below two.
/// Returns (bite, poison).
pub fn poison_bite(bite: f32, health: f32) -> Option<(f32, f32)> {
    if health <= 1. {
        return None;
    }
    let first = if bite >= health { health - 1. } else { bite };
    let left = health - first;
    Some((first, if left > 1. { left - 1. } else { 0. }))
}
const SCHEDULES: &[(&str, &str)] = &[(
    "SCHED_HEADCRAB_RANGE_ATTACK1",
    "Tasks TASK_STOP_MOVING 0 TASK_FACE_ENEMY 0 TASK_RANGE_ATTACK1 0 \
     TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE TASK_FACE_IDEAL 0 TASK_WAIT_RANDOM 0.5 Interrupts \
     COND_ENEMY_OCCLUDED COND_NO_PRIMARY_AMMO",
)];
pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in SCHEDULES {
        let schedule = Schedule::parse(name, text).expect("headcrab schedule");
        schedules.0.insert((*name).into(), schedule);
    }
}
pub fn is_headcrab_class(class: &str) -> bool {
    matches!(
        class,
        "npc_headcrab" | "npc_headcrab_black" | "npc_headcrab_poison"
    )
}
/// An npc_headcrab under the AI.
#[derive(Clone, Debug, Serialize)]
pub struct Headcrab {
    #[serde(skip)]
    pub requests: Vec<CopRequest>,
    pub assault: Option<Assault>,
    pub script_goal: Option<(Vec3, bool)>,
    /// m_flNextAttack.
    pub next_attack: f64,
    /// m_bMidJump.
    pub mid_jump: bool,
    /// m_bCommittedToJump with m_vecCommittedJumpPos (the telegraph's enemy eyes).
    pub committed_jump: Option<Vec3>,
    /// m_bAttackFailed.
    pub attack_failed: bool,
    /// LeapTouch is set (until it touches something or lands).
    pub leap_touch: bool,
    /// m_flIgnoreWorldCollisionTime.
    pub ignore_world_until: f64,
    /// Bites this crab landed on the player (diagnostics).
    pub bites: u32,
    /// The last RangeAttack1Conditions inputs: (distance, dot, condition) (diagnostics).
    pub last_check: Option<(f32, f32, Option<i32>)>,
    /// COND_SEE_ENEMY as gathered this think. MaintainSchedule re-gathers conditions when a
    /// schedule ends within one think (SetSchedule clears them); ours does not, so
    /// SelectFailSchedule reads this copy.
    pub sees_enemy: bool,
    /// CBlackHeadcrab (npc_headcrab_black / npc_headcrab_poison): poison bites, its sounds
    /// and yaw speeds.
    pub black: bool,
}
impl Headcrab {
    pub fn for_class(class: &str) -> Self {
        Self {
            black: matches!(class, "npc_headcrab_black" | "npc_headcrab_poison"),
            ..Self::new()
        }
    }
    /// The class's sound for a cue (Telegraph, Attack, Bite, Pain, Die).
    pub fn sound(&self, cue: &str) -> &'static str {
        match (self.black, cue) {
            (true, "Telegraph") => "NPC_BlackHeadcrab.Telegraph",
            (true, "Attack") => "NPC_BlackHeadcrab.Attack",
            (true, "Bite") => "NPC_BlackHeadcrab.Bite",
            (true, "Pain") => "NPC_BlackHeadcrab.Pain",
            (true, _) => "NPC_BlackHeadcrab.Die",
            // CHeadcrab::TelegraphSound is the alert; its attack cue is spelled Headcrab.
            (false, "Telegraph") => "NPC_HeadCrab.Alert",
            (false, "Attack") => "NPC_Headcrab.Attack",
            (false, "Bite") => "NPC_HeadCrab.Bite",
            (false, "Pain") => "NPC_HeadCrab.Pain",
            (false, _) => "NPC_HeadCrab.Die",
        }
    }
    pub fn new() -> Self {
        Self {
            requests: Vec::new(),
            assault: None,
            script_goal: None,
            next_attack: 0.,
            mid_jump: false,
            committed_jump: None,
            attack_failed: false,
            leap_touch: false,
            ignore_world_until: 0.,
            bites: 0,
            last_check: None,
            sees_enemy: false,
            black: false,
        }
    }
}
impl Default for Headcrab {
    fn default() -> Self {
        Self::new()
    }
}
impl Behavior for Headcrab {
    /// CBaseHeadcrab::SelectSchedule's special states (hidden, ceiling, burrowed, water,
    /// NPC heads, barnacles) are not modeled: CAI_BaseNPC::SelectSchedule (unarmed).
    fn select_schedule(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        base_select_schedule(npc, ctx, false)
    }
    fn translate_schedule(&mut self, name: &str, _npc: &AiNpc, _ctx: &Context) -> String {
        match name {
            "SCHED_RANGE_ATTACK1" => "SCHED_HEADCRAB_RANGE_ATTACK1",
            "SCHED_FAIL_TAKE_COVER" => "SCHED_ALERT_FACE",
            other => other,
        }
        .into()
    }
    /// CBaseHeadcrab::SelectFailSchedule: a back-away that found no spot leaps at a visible
    /// enemy instead (hops when not firmly on the ground are not modeled).
    fn select_fail_schedule(&mut self, npc: &AiNpc, failed: &str) -> String {
        let task = npc
            .last_failed_task
            .as_deref()
            .and_then(|t| t.rsplit('/').next());
        if failed == "SCHED_BACK_AWAY_FROM_ENEMY"
            && task == Some("TASK_FIND_BACKAWAY_FROM_SAVEPOSITION")
            && (self.sees_enemy || npc.conditions.has(COND_SEE_ENEMY))
        {
            return "SCHED_RANGE_ATTACK1".into();
        }
        npc.fail_schedule
            .clone()
            .unwrap_or_else(|| "SCHED_FAIL".into())
    }
    fn run_task(
        &mut self,
        npc: &mut AiNpc,
        task: &str,
        _arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        (task == "TASK_RANGE_ATTACK1").then(|| {
            if !motor.activity_finished() {
                return TaskStatus::Running;
            }
            // The leap is over: no touch, ACT_IDLE, and a failed leap delays the next.
            self.mid_jump = false;
            self.leap_touch = false;
            npc.activity = "ACT_IDLE".into();
            motor.set_activity("ACT_IDLE");
            if std::mem::take(&mut self.attack_failed) {
                self.next_attack = ctx.now + FAILED_ATTACK_DELAY;
            }
            TaskStatus::Complete
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn leaps_follow_jump_attack() {
        let g = 600.;
        // A target 200 ahead at the same height: the 16 + 8 unit arc.
        let v = jump_velocity(Vec3::ZERO, Vec3::new(200., 0., 0.), g, false);
        let speed = (2. * g * 24.).sqrt();
        let time = speed / g + (2. * 8. / g).sqrt();
        assert!((v.z - speed).abs() < 1e-3);
        assert!((v.x - 200. / time).abs() < 1e-2, "{v}");
        // Higher targets arc up to 120 units; long ones are capped at 650 u/s.
        let high = jump_velocity(Vec3::ZERO, Vec3::new(100., 0., 300.), g, false);
        assert!((high.z - (2. * g * 120f32).sqrt()).abs() < 1e-2);
        let far = jump_velocity(Vec3::ZERO, Vec3::new(2000., 0., 0.), g, false);
        assert!((far.length() - 650.).abs() < 1e-2);
        let hop = random_hop(90.);
        assert!(hop.distance(Vec3::new(0., 350., 350.)) < 1e-2, "{hop}");
    }
    #[test]
    fn leap_conditions_follow_range_attack1_conditions() {
        let ok = LeapCheck {
            ready: true,
            on_ground: true,
            dot: 0.9,
            distance: 150.,
            clear: true,
            high_blocked: false,
        };
        assert_eq!(range_condition(ok), Some(COND_CAN_RANGE_ATTACK1));
        assert_eq!(range_condition(LeapCheck { ready: false, ..ok }), None);
        assert_eq!(
            range_condition(LeapCheck {
                on_ground: false,
                ..ok
            }),
            None
        );
        assert_eq!(
            range_condition(LeapCheck { dot: 0.6, ..ok }),
            Some(COND_NOT_FACING_ATTACK)
        );
        assert_eq!(
            range_condition(LeapCheck {
                distance: 40.,
                ..ok
            }),
            Some(COND_TOO_CLOSE_TO_ATTACK)
        );
        assert_eq!(
            range_condition(LeapCheck {
                distance: 300.,
                ..ok
            }),
            Some(COND_TOO_FAR_TO_ATTACK)
        );
        assert_eq!(range_condition(LeapCheck { clear: false, ..ok }), None);
        assert_eq!(
            range_condition(LeapCheck {
                high_blocked: true,
                ..ok
            }),
            Some(COND_TOO_CLOSE_TO_ATTACK)
        );
        assert_eq!(max_yaw_speed("ACT_RUN", false), 20.);
        assert_eq!(max_yaw_speed("ACT_IDLE", false), 30.);
        assert_eq!(max_yaw_speed("ACT_RUN", true), 10.);
        assert_eq!(max_yaw_speed("ACT_TURN_LEFT", true), 30.);
        // Poison bites leave one health: 5 then 94 of 100; a 3-health victim 2 then 0;
        // a 5-damage bite on 5 health is cut to 4; nothing at one health.
        assert_eq!(poison_bite(5., 100.), Some((5., 94.)));
        assert_eq!(poison_bite(5., 3.), Some((2., 0.)));
        assert_eq!(poison_bite(5., 5.), Some((4., 0.)));
        assert_eq!(poison_bite(5., 1.), None);
        let black = Headcrab::for_class("npc_headcrab_poison");
        assert!(black.black && is_headcrab_class("npc_headcrab_black"));
        assert_eq!(black.sound("Bite"), "NPC_BlackHeadcrab.Bite");
        assert_eq!(Headcrab::new().sound("Telegraph"), "NPC_HeadCrab.Alert");
        assert_eq!(Headcrab::new().sound("Attack"), "NPC_Headcrab.Attack");
        let mut schedules = Schedules::default();
        add_schedules(&mut schedules);
        assert!(schedules.0.contains_key("SCHED_HEADCRAB_RANGE_ATTACK1"));
        let mut crab = Headcrab::new();
        let mut npc = AiNpc::new(1);
        npc.last_failed_task =
            Some("SCHED_BACK_AWAY_FROM_ENEMY/TASK_FIND_BACKAWAY_FROM_SAVEPOSITION".into());
        npc.conditions.set(COND_SEE_ENEMY);
        assert_eq!(
            crab.select_fail_schedule(&npc, "SCHED_BACK_AWAY_FROM_ENEMY"),
            "SCHED_RANGE_ATTACK1"
        );
        npc.conditions.clear(COND_SEE_ENEMY);
        assert_eq!(
            crab.select_fail_schedule(&npc, "SCHED_BACK_AWAY_FROM_ENEMY"),
            "SCHED_FAIL"
        );
        assert_eq!(
            crab.translate_schedule("SCHED_RANGE_ATTACK1", &npc, &Context::default()),
            "SCHED_HEADCRAB_RANGE_ATTACK1"
        );
    }
    #[test]
    fn crabs_seek_line_of_sight_with_their_innate_leap() {
        use super::super::state::NpcState;
        let ctx = Context {
            innate_range: Some((MIN_JUMP_DIST, MAX_JUMP_DIST)),
            ..Default::default()
        };
        let mut crab = Headcrab::new();
        let pick = |crab: &mut Headcrab, conds: &[Cond]| {
            let mut npc = AiNpc::new(1);
            npc.state = NpcState::Combat;
            for &c in conds {
                npc.conditions.set(c);
            }
            crab.select_schedule(&mut npc, &ctx)
        };
        // SelectCombatSchedule: an occluded enemy gets line of fire, a visible one too far
        // to leap at is approached, and a visible one in range before the attack delay is
        // over makes the crab take cover.
        assert_eq!(
            pick(&mut crab, &[COND_ENEMY_OCCLUDED]),
            "SCHED_ESTABLISH_LINE_OF_FIRE"
        );
        assert_eq!(
            pick(&mut crab, &[COND_SEE_ENEMY, COND_TOO_FAR_TO_ATTACK]),
            "SCHED_MOVE_TO_WEAPON_RANGE"
        );
        assert_eq!(
            pick(&mut crab, &[COND_SEE_ENEMY]),
            "SCHED_TAKE_COVER_FROM_ENEMY"
        );
    }
    #[test]
    #[ignore = "requires owned installed HL2"]
    fn owned_headcrab_leap_carries_its_events() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let wanted = ["jumpattack_broadcast".to_owned(), "run1".to_owned()].into();
        let rig =
            source_assets::animation::load(&vfs, "models/headcrabclassic.mdl", &wanted).unwrap();
        let sequence = rig
            .lookup_sequence("ACT_RANGE_ATTACK1", None, |_| 0)
            .unwrap();
        let events: Vec<_> = rig.clips[sequence]
            .events
            .iter()
            .map(|e| (e.name.to_uppercase(), e.cycle))
            .collect();
        println!("{sequence} {events:?}");
        assert!(events.iter().any(|(n, _)| n == AE_JUMP_TELEGRAPH));
        assert!(events.iter().any(|(n, _)| n == AE_JUMPATTACK));
        assert!(rig.lookup_sequence("ACT_RUN", None, |_| 0).is_some());
        assert_eq!(
            source_assets::models::read_eye_position(&vfs, "models/headcrabclassic.mdl")
                .unwrap()
                .z,
            16.
        );
        // The black crab's leap and run.
        let path = "models/headcrabblack.mdl";
        let names = source_assets::animation::sequence_names(&vfs, path).unwrap();
        let rig =
            source_assets::animation::load(&vfs, path, &names.iter().cloned().collect()).unwrap();
        let leap = rig
            .lookup_sequence("ACT_RANGE_ATTACK1", None, |_| 0)
            .unwrap();
        let events: Vec<_> = rig.clips[leap]
            .events
            .iter()
            .map(|e| (e.name.to_uppercase(), e.cycle))
            .collect();
        println!("black {names:?} {leap} {events:?}");
        assert!(events.iter().any(|(n, _)| n == AE_JUMP_TELEGRAPH));
        assert!(events.iter().any(|(n, _)| n == AE_JUMPATTACK));
        assert!(rig.lookup_sequence("ACT_RUN", None, |_| 0).is_some());
    }
}
