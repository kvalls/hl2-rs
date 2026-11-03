//! npc_zombie (CNPC_BaseZombie + CZombie, SDK npc_basezombie.cpp / npc_zombie.cpp):
//! the classic zombie's schedules (chase, claw melee, wander after losing the enemy),
//! its claw animation events and its sounds. Not modeled yet: headcrab release, torso
//! split, physics-object swats, door bashing, burning, the moan envelope (idle moans
//! play the plain Zombie.Idle script), slumped zombies, the fast zombie and the poison
//! zombie's headcrab throws, breathing loops and idle cadence (its idles use the classic
//! moan clock outside combat).
use super::assault::Assault;
use super::conditions::{
    Cond, COND_CAN_MELEE_ATTACK1, COND_ENEMY_DEAD, COND_ENEMY_UNREACHABLE, COND_LOST_ENEMY,
    COND_NOT_FACING_ATTACK, COND_TOO_FAR_TO_ATTACK,
};
use super::metropolice::CopRequest;
use super::schedule::{
    default_select_schedule, AiNpc, Behavior, Context, Motor, Schedule, Schedules, TaskArg,
    TaskStatus,
};
use super::state::NpcState;
use glam::Vec3;
use serde::Serialize;

/// ZOMBIE_MELEE_REACH (GetClawAttackRange).
pub const CLAW_REACH: f32 = 55.;
/// The zombie classes under this AI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Kind {
    /// npc_zombie (CZombie).
    Classic,
    /// npc_poisonzombie (CNPC_PoisonZombie): melee only here.
    Poison,
}
impl Kind {
    pub fn for_class(class: &str) -> Option<Self> {
        match class {
            "npc_zombie" => Some(Self::Classic),
            "npc_poisonzombie" => Some(Self::Poison),
            _ => None,
        }
    }
    /// GetClawAttackRange (CNPC_PoisonZombie: 70).
    pub fn claw_reach(self) -> f32 {
        match self {
            Self::Classic => CLAW_REACH,
            Self::Poison => 70.,
        }
    }
    /// The class's own cue for a CZombie cue name (CNPC_PoisonZombie's overrides).
    pub fn cue(self, classic: &'static str) -> &'static str {
        match (self, classic) {
            (Self::Classic, c) => c,
            (Self::Poison, "Zombie.FootstepLeft") => "NPC_PoisonZombie.FootstepLeft",
            (Self::Poison, "Zombie.FootstepRight") => "NPC_PoisonZombie.FootstepRight",
            (Self::Poison, "Zombie.Attack") => "NPC_PoisonZombie.Attack",
            (Self::Poison, "Zombie.Alert") => "NPC_PoisonZombie.Alert",
            (Self::Poison, "Zombie.Idle") => "NPC_PoisonZombie.Idle",
            (Self::Poison, "Zombie.Pain") => "NPC_PoisonZombie.Pain",
            (Self::Poison, "Zombie.Die") => "NPC_PoisonZombie.Die",
            (Self::Poison, c) => c,
        }
    }
}
/// m_flFieldOfView (CZombie::Spawn).
pub const FIELD_OF_VIEW: f32 = 0.2;

/// Every sound cue this class emits (the host preloads them; a cue that is not
/// preloaded is dropped).
pub const SOUNDS: &[&str] = &[
    "Zombie.FootstepLeft",
    "Zombie.FootstepRight",
    "Zombie.ScuffLeft",
    "Zombie.ScuffRight",
    "Zombie.Attack",
    "Zombie.AttackHit",
    "Zombie.AttackMiss",
    "Zombie.Alert",
    "Zombie.Idle",
    "Zombie.Pain",
    "Zombie.Die",
    "NPC_BaseZombie.PoundDoor",
    "NPC_PoisonZombie.FootstepLeft",
    "NPC_PoisonZombie.FootstepRight",
    "NPC_PoisonZombie.Attack",
    "NPC_PoisonZombie.Alert",
    "NPC_PoisonZombie.Idle",
    "NPC_PoisonZombie.Pain",
    "NPC_PoisonZombie.Die",
];
/// CNPC_BaseZombie's schedules (SDK text, the non-episodic SCHED_ZOMBIE_CHASE_ENEMY;
/// COND_ZOMBIE_CAN_SWAT_ATTACK, COND_ZOMBIE_RELEASECRAB and the obstruction condition
/// are not modeled and left out of the interrupts).
const ZOMBIE_SCHEDULES: &[(&str, &str)] = &[
    (
        "SCHED_ZOMBIE_CHASE_ENEMY",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_CHASE_ENEMY_FAILED \
         TASK_SET_TOLERANCE_DISTANCE 24 TASK_GET_CHASE_PATH_TO_ENEMY 600 TASK_RUN_PATH 0 \
         TASK_WAIT_FOR_MOVEMENT 0 TASK_FACE_ENEMY 0 Interrupts COND_NEW_ENEMY \
         COND_ENEMY_DEAD COND_ENEMY_UNREACHABLE COND_CAN_RANGE_ATTACK1 \
         COND_CAN_MELEE_ATTACK1 COND_CAN_RANGE_ATTACK2 COND_CAN_MELEE_ATTACK2 \
         COND_TOO_CLOSE_TO_ATTACK COND_TASK_FAILED",
    ),
    (
        "SCHED_ZOMBIE_MELEE_ATTACK1",
        "Tasks TASK_STOP_MOVING 0 TASK_FACE_ENEMY 0 TASK_ANNOUNCE_ATTACK 1 \
         TASK_MELEE_ATTACK1 0 TASK_SET_SCHEDULE SCHEDULE:SCHED_ZOMBIE_POST_MELEE_WAIT \
         Interrupts COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE",
    ),
    (
        "SCHED_ZOMBIE_POST_MELEE_WAIT",
        "Tasks TASK_ZOMBIE_WAIT_POST_MELEE 0 Interrupts",
    ),
    (
        "SCHED_ZOMBIE_WANDER_MEDIUM",
        "Tasks TASK_STOP_MOVING 0 TASK_WANDER 480384 TASK_WALK_PATH 0 \
         TASK_WAIT_FOR_MOVEMENT 0 TASK_STOP_MOVING 0 TASK_WAIT_PVS 0 \
         TASK_SET_SCHEDULE SCHEDULE:SCHED_ZOMBIE_WANDER_MEDIUM Interrupts COND_NEW_ENEMY \
         COND_SEE_ENEMY COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE",
    ),
    (
        "SCHED_ZOMBIE_WANDER_STANDOFF",
        "Tasks TASK_STOP_MOVING 0 TASK_WANDER 480384 TASK_WALK_PATH 0 \
         TASK_WAIT_FOR_MOVEMENT 0 TASK_STOP_MOVING 0 TASK_WAIT_PVS 0 Interrupts \
         COND_NEW_ENEMY COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE COND_ENEMY_DEAD \
         COND_CAN_RANGE_ATTACK1 COND_CAN_MELEE_ATTACK1 COND_CAN_RANGE_ATTACK2 \
         COND_CAN_MELEE_ATTACK2",
    ),
    (
        "SCHED_ZOMBIE_WANDER_FAIL",
        "Tasks TASK_STOP_MOVING 0 TASK_WAIT 1 \
         TASK_SET_SCHEDULE SCHEDULE:SCHED_ZOMBIE_WANDER_MEDIUM Interrupts COND_NEW_ENEMY \
         COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE COND_ENEMY_DEAD COND_CAN_RANGE_ATTACK1 \
         COND_CAN_MELEE_ATTACK1 COND_CAN_RANGE_ATTACK2 COND_CAN_MELEE_ATTACK2",
    ),
];
pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in ZOMBIE_SCHEDULES {
        let schedule = Schedule::parse(name, text).expect("zombie schedule");
        schedules.0.insert((*name).into(), schedule);
    }
}

/// CNPC_BaseZombie::MaxYawSpeed (not a torso, no move_yaw pose blend).
pub fn max_yaw_speed(activity: &str) -> f32 {
    match activity {
        "ACT_TURN_LEFT" | "ACT_TURN_RIGHT" => 100.,
        "ACT_RUN" => 15.,
        "ACT_WALK" | "ACT_IDLE" => 25.,
        "ACT_RANGE_ATTACK1" | "ACT_RANGE_ATTACK2" | "ACT_MELEE_ATTACK1" | "ACT_MELEE_ATTACK2" => {
            120.
        }
        _ => 90.,
    }
}

/// CNPC_BaseZombie::MeleeAttack1Conditions from the enemy's distance and the facing dot
/// (GatherAttackConditions: origin to origin, 2D facing) and whether the claw hull
/// (the zombie's hull made a cube, along the facing for the reach) meets the enemy.
pub fn melee_condition(distance: f32, dot: f32, hull_reaches_enemy: bool) -> Cond {
    if distance > CLAW_REACH {
        COND_TOO_FAR_TO_ATTACK
    } else if dot < 0.7 {
        COND_NOT_FACING_ATTACK
    } else if hull_reaches_enemy {
        COND_CAN_MELEE_ATTACK1
    } else {
        // "This attack would miss completely. Trick the zombie into moving around."
        COND_TOO_FAR_TO_ATTACK
    }
}

/// One claw swing (AE_ZOMBIE_ATTACK_RIGHT/LEFT/BOTH): view punch (Source degrees) and
/// velocity punch for the player, from the zombie's facing yaw (degrees).
pub fn claw_punch(event: &str, yaw: f32, rolls: (f32, f32)) -> Option<(Vec3, Vec3)> {
    let y = yaw.to_radians();
    let forward = Vec3::new(y.cos(), y.sin(), 0.);
    let right = Vec3::new(y.sin(), -y.cos(), 0.);
    match event.to_ascii_uppercase().as_str() {
        "AE_ZOMBIE_ATTACK_RIGHT" => {
            Some((Vec3::new(-15., -20., -10.), right * 100. + forward * 200.))
        }
        "AE_ZOMBIE_ATTACK_LEFT" => {
            Some((Vec3::new(-15., 20., -10.), right * -100. + forward * 200.))
        }
        // QAngle(45, RandomInt(-5, 5), RandomInt(-5, 5)); the poison zombie's bite
        // (AE_ZOMBIE_POISON_SPIT) swings the same way.
        "AE_ZOMBIE_ATTACK_BOTH" | "AE_ZOMBIE_POISON_SPIT" => Some((
            Vec3::new(
                45.,
                (rolls.0 * 11.).floor() - 5.,
                (rolls.1 * 11.).floor() - 5.,
            ),
            forward * 200.,
        )),
        _ => None,
    }
}

/// The class sound of a zombie animation event (CNPC_BaseZombie::HandleAnimEvent).
pub fn event_sound(event: &str) -> Option<&'static str> {
    Some(match event.to_ascii_uppercase().as_str() {
        "AE_ZOMBIE_STEP_LEFT" => "Zombie.FootstepLeft",
        "AE_ZOMBIE_STEP_RIGHT" => "Zombie.FootstepRight",
        "AE_ZOMBIE_SCUFF_LEFT" => "Zombie.ScuffLeft",
        "AE_ZOMBIE_SCUFF_RIGHT" => "Zombie.ScuffRight",
        "AE_ZOMBIE_STARTSWAT" | "AE_ZOMBIE_ATTACK_SCREAM" => "Zombie.Attack",
        "AE_ZOMBIE_ALERTSOUND" => "Zombie.Alert",
        "AE_ZOMBIE_POUND" => "NPC_BaseZombie.PoundDoor",
        _ => return None,
    })
}

/// An npc_zombie under the AI.
/// The CNPC_BaseZombie classes.
pub fn is_zombie_class(class: &str) -> bool {
    matches!(
        class,
        "npc_zombie"
            | "npc_zombie_torso"
            | "npc_fastzombie"
            | "npc_fastzombie_torso"
            | "npc_poisonzombie"
            | "npc_zombine"
    )
}
#[derive(Clone, Debug, Serialize)]
pub struct Zombie {
    pub kind: Kind,
    #[serde(skip)]
    pub requests: Vec<CopRequest>,
    /// Hosted behaviors the shared code expects on every class (never set here yet).
    pub assault: Option<Assault>,
    pub script_goal: Option<(Vec3, bool)>,
    /// m_flNextMoanSound (Spawn: now + U(1, 4)).
    pub next_moan: Option<f64>,
    pub gagged: bool,
    seed: u64,
}
impl Zombie {
    pub fn new(kind: Kind, gagged: bool, seed: u64) -> Self {
        Self {
            kind,
            requests: Vec::new(),
            assault: None,
            script_goal: None,
            next_moan: None,
            gagged,
            seed: seed | 1,
        }
    }
    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }
    /// CZombie::PrescheduleThink's moan clock: IdleSound (Zombie.Idle; CanPlayMoanSound
    /// is true for small groups unless gagged), then U(2, 5) s until the next.
    /// CNPC_PoisonZombie::IdleSound plays only outside combat.
    pub fn moan(&mut self, now: f64, combat: bool) {
        let first = now + 1. + 3. * f64::from(self.random());
        let next = *self.next_moan.get_or_insert(first);
        if now <= next {
            return;
        }
        if self.gagged || (combat && self.kind == Kind::Poison) {
            self.next_moan = Some(now + 1. + f64::from(self.random()));
            return;
        }
        self.requests
            .push(CopRequest::Sound(self.kind.cue("Zombie.Idle")));
        self.next_moan = Some(now + 2. + 3. * f64::from(self.random()));
    }
}
impl Behavior for Zombie {
    fn select_schedule(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        let c = &npc.conditions;
        match npc.state {
            NpcState::Combat if c.has(COND_LOST_ENEMY) || c.has(COND_ENEMY_UNREACHABLE) => {
                return "SCHED_ZOMBIE_WANDER_MEDIUM".into();
            }
            NpcState::Alert
                if c.has(COND_LOST_ENEMY)
                    || c.has(COND_ENEMY_DEAD)
                    || c.has(COND_ENEMY_UNREACHABLE) =>
            {
                npc.conditions.clear(COND_LOST_ENEMY);
                npc.conditions.clear(COND_ENEMY_UNREACHABLE);
                return "SCHED_ZOMBIE_WANDER_MEDIUM".into();
            }
            _ => {}
        }
        default_select_schedule(npc, ctx)
    }
    fn translate_schedule(&mut self, name: &str, _npc: &AiNpc, _ctx: &Context) -> String {
        match name {
            "SCHED_CHASE_ENEMY" => "SCHED_ZOMBIE_CHASE_ENEMY",
            "SCHED_MELEE_ATTACK1" => "SCHED_ZOMBIE_MELEE_ATTACK1",
            "SCHED_STANDOFF" => "SCHED_ZOMBIE_WANDER_STANDOFF",
            other => other,
        }
        .into()
    }
    fn select_fail_schedule(&mut self, npc: &AiNpc, failed: &str) -> String {
        if failed == "SCHED_ZOMBIE_WANDER_MEDIUM" {
            return "SCHED_ZOMBIE_WANDER_FAIL".into();
        }
        npc.fail_schedule
            .clone()
            .unwrap_or_else(|| "SCHED_FAIL".into())
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
            // Non-episodic: complete at once.
            "TASK_ZOMBIE_WAIT_POST_MELEE" => TaskStatus::Complete,
            // CAI_BaseNPC TASK_SOUND_WAKE -> CZombie::AlertSound.
            "TASK_SOUND_WAKE" => {
                self.requests
                    .push(CopRequest::Sound(self.kind.cue("Zombie.Alert")));
                TaskStatus::Complete
            }
            // TASK_WANDER min * 10000 + max: a walk to a random point that far away.
            "TASK_WANDER" => {
                let packed = arg.number() as u32;
                let (min, max) = ((packed / 10000) as f32, (packed % 10000) as f32);
                let angle = self.random() * std::f32::consts::TAU;
                let distance = min + (max - min) * self.random();
                let goal = ctx.origin + Vec3::new(angle.cos(), angle.sin(), 0.) * distance;
                if motor.path_to(goal, false) {
                    TaskStatus::Complete
                } else {
                    TaskStatus::Failed
                }
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claw_conditions_need_reach_facing_and_a_hit() {
        assert_eq!(melee_condition(60., 1., true), COND_TOO_FAR_TO_ATTACK);
        assert_eq!(melee_condition(40., 0.5, true), COND_NOT_FACING_ATTACK);
        assert_eq!(melee_condition(40., 0.9, false), COND_TOO_FAR_TO_ATTACK);
        assert_eq!(melee_condition(40., 0.9, true), COND_CAN_MELEE_ATTACK1);
    }
    #[test]
    fn claw_punches_follow_the_swinging_hand() {
        // Facing +X: right is -Y.
        let (punch, push) = claw_punch("AE_ZOMBIE_ATTACK_RIGHT", 0., (0., 0.)).unwrap();
        assert_eq!(punch, Vec3::new(-15., -20., -10.));
        assert!(push.distance(Vec3::new(200., -100., 0.)) < 1e-3);
        let (_, push) = claw_punch("AE_ZOMBIE_ATTACK_LEFT", 0., (0., 0.)).unwrap();
        assert!(push.distance(Vec3::new(200., 100., 0.)) < 1e-3);
        let (punch, _) = claw_punch("AE_ZOMBIE_ATTACK_BOTH", 90., (0., 0.999)).unwrap();
        assert_eq!(punch, Vec3::new(45., -5., 5.));
        assert!(claw_punch("AE_ZOMBIE_POUND", 0., (0., 0.)).is_none());
    }
    #[test]
    fn zombie_schedules_parse_and_translate() {
        let mut schedules = Schedules::default();
        add_schedules(&mut schedules);
        for (name, _) in ZOMBIE_SCHEDULES {
            assert!(schedules.0.contains_key(*name));
        }
        let mut z = Zombie::new(Kind::Classic, false, 7);
        let npc = AiNpc::new(1);
        let ctx = Context::default();
        assert_eq!(
            z.translate_schedule("SCHED_CHASE_ENEMY", &npc, &ctx),
            "SCHED_ZOMBIE_CHASE_ENEMY"
        );
        assert_eq!(
            z.translate_schedule("SCHED_MELEE_ATTACK1", &npc, &ctx),
            "SCHED_ZOMBIE_MELEE_ATTACK1"
        );
        assert_eq!(
            z.select_fail_schedule(&npc, "SCHED_ZOMBIE_WANDER_MEDIUM"),
            "SCHED_ZOMBIE_WANDER_FAIL"
        );
    }
    #[test]
    fn idle_moans_come_every_two_to_five_seconds_unless_gagged() {
        let mut z = Zombie::new(Kind::Classic, false, 3);
        let mut moans = Vec::new();
        for t in 0..300 {
            let now = f64::from(t) * 0.1;
            z.moan(now, false);
            if !z.requests.is_empty() {
                moans.push(now);
                z.requests.clear();
            }
        }
        assert!(moans[0] >= 1. && moans[0] <= 4.1, "{moans:?}");
        for pair in moans.windows(2) {
            let gap = pair[1] - pair[0];
            assert!((1.99..=5.11).contains(&gap), "{moans:?}");
        }
        let mut gagged = Zombie::new(Kind::Classic, true, 3);
        for t in 0..300 {
            gagged.moan(f64::from(t) * 0.1, false);
        }
        assert!(gagged.requests.is_empty());
        // The poison zombie idles with its own cue, never in combat.
        let mut poison = Zombie::new(Kind::Poison, false, 3);
        for t in 0..300 {
            poison.moan(f64::from(t) * 0.1, true);
        }
        assert!(poison.requests.is_empty());
        for t in 300..400 {
            poison.moan(f64::from(t) * 0.1, false);
        }
        assert!(poison
            .requests
            .iter()
            .all(|r| matches!(r, CopRequest::Sound("NPC_PoisonZombie.Idle"))));
        assert!(!poison.requests.is_empty());
        assert_eq!(Kind::Poison.claw_reach(), 70.);
    }
}
