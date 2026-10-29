//! npc_zombie (CNPC_BaseZombie + CZombie, SDK npc_basezombie.cpp / npc_zombie.cpp):
//! the classic zombie's schedules (chase, claw melee, wander after losing the enemy),
//! its claw animation events, its sounds and physics-object swats (FindNearestPhysicsObject,
//! SCHED_ZOMBIE_(MOVE_)SWATITEM, AE_ZOMBIE_SWATITEM). Not modeled yet: headcrab release, torso
//! split, the blocked-path obstructor swat (SCHED_ZOMBIE_ATTACKITEM), door bashing, burning, the moan envelope (idle moans
//! play the plain Zombie.Idle script), slumped zombies, the fast zombie and the poison
//! zombie's headcrab throws, breathing loops and idle cadence (its idles use the classic
//! moan clock outside combat).
use super::assault::Assault;
use super::conditions::*;
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
/// COND_ZOMBIE_CAN_SWAT_ATTACK (the first class condition).
pub const COND_ZOMBIE_CAN_SWAT_ATTACK: Cond = Cond(LAST_SHARED_CONDITION);
/// ZOMBIE_MAX_PHYSOBJ_MASS: the heaviest object a zombie swats (kg).
pub const MAX_PHYSOBJ_MASS: f32 = 60.;
/// ZOMBIE_PHYSOBJ_SWATDIST: swat from this close (UTIL_DistApprox2D to the object center).
pub const PHYSOBJ_SWATDIST: f32 = 80.;
/// ZOMBIE_PLAYER_MAX_SWAT_DIST: no swats while the enemy is farther than this.
pub const PLAYER_MAX_SWAT_DIST: f32 = 1000.;
/// ZOMBIE_FARTHEST_PHYSICS_OBJECT (40 feet); the search box is half of it at most.
pub const FARTHEST_PHYSICS_OBJECT: f32 = 480.;
/// ZOMBIE_SWAT_DELAY: no new object scan this long after a swat.
pub const SWAT_DELAY: f64 = 5.;
/// GatherConditions rescans this often while it has no object.
pub const SWAT_SCAN_INTERVAL: f64 = 2.;
pub const AE_SWATITEM: &str = "AE_ZOMBIE_SWATITEM";
/// A VPhysics object near a zombie (CZombieSwatEntitiesEnum already required a body <= the
/// mass limit, asleep and moveable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwatCandidate {
    pub entity: usize,
    /// WorldSpaceCenter.
    pub center: Vec3,
    /// BoundingRadius: half the diagonal of its collision bounds.
    pub radius: f32,
    /// The top of its first solid's world AABB (CollideGetAABB).
    pub top: f32,
    /// FVisible from the zombie's eyes.
    pub visible: bool,
    /// physics_prop_ragdoll / prop_ragdoll (never swatted).
    pub ragdoll: bool,
}
/// UTIL_DistApprox2D: |dx| + |dy|.
pub fn dist_approx_2d(a: Vec3, b: Vec3) -> f32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}
/// The search box half extents around the zombie (UTIL_EntitiesInBox in FindNearestPhysicsObject).
pub fn swat_search_box(origin: Vec3, enemy: Vec3, hull_height: f32) -> Option<Vec3> {
    let dist = (enemy - origin).length();
    (dist <= PLAYER_MAX_SWAT_DIST).then(|| {
        let near = dist.min(FARTHEST_PHYSICS_OBJECT * 0.5);
        Vec3::new(near, near, hull_height * 2.)
    })
}
/// CNPC_BaseZombie::FindNearestPhysicsObject's choice among the objects in the box: nearest
/// (UTIL_DistApprox2D) below min(enemy distance, 240), toward the enemy (2D dot >= 0.8), nearer
/// the zombie than the enemy, reaching above the knees, center below the eyes, visible, not a
/// ragdoll (HL2 is not hl2_episodic: the enemy need not see it).
pub fn find_nearest_physics_object(
    origin: Vec3,
    enemy: Vec3,
    knees_z: f32,
    eye_z: f32,
    candidates: &[SwatCandidate],
) -> Option<usize> {
    let to_enemy = enemy - origin;
    let dist = to_enemy.length();
    if dist > PLAYER_MAX_SWAT_DIST {
        return None;
    }
    let dir_enemy = (to_enemy / dist.max(1e-6)).truncate().extend(0.);
    let mut nearest = dist.min(FARTHEST_PHYSICS_OBJECT * 0.5);
    let mut best = None;
    for c in candidates {
        let d = dist_approx_2d(origin, c.center);
        if d >= nearest {
            continue;
        }
        let dir_object = (c.center - origin)
            .normalize_or_zero()
            .truncate()
            .extend(0.);
        if dir_enemy.dot(dir_object) < 0.8 {
            continue;
        }
        if d >= dist_approx_2d(c.center, enemy) {
            continue;
        }
        if c.center.z + c.radius < knees_z || c.center.z > eye_z || c.top < knees_z {
            continue;
        }
        if !c.visible || c.ragdoll {
            continue;
        }
        best = Some(c.entity);
        nearest = d;
    }
    best
}
/// CNPC_BaseZombie::GetSwatActivity: the arm on the object's side of the facing (yaw in
/// degrees), MID when its center is above the zombie's center, else LOW.
pub fn swat_activity(
    yaw: f32,
    origin: Vec3,
    object_origin: Vec3,
    center_z: f32,
    object_center_z: f32,
) -> &'static str {
    let y = yaw.to_radians();
    let right = Vec3::new(y.sin(), -y.cos(), 0.);
    let to = (object_origin - origin)
        .normalize_or_zero()
        .truncate()
        .extend(0.);
    let mid = center_z - object_center_z < 0.;
    match (right.dot(to) >= 0., mid) {
        (true, true) => "ACT_ZOM_SWATRIGHTMID",
        (true, false) => "ACT_ZOM_SWATRIGHTLOW",
        (false, true) => "ACT_ZOM_SWATLEFTMID",
        (false, false) => "ACT_ZOM_SWATLEFTLOW",
    }
}
/// AE_ZOMBIE_SWATITEM's AddVelocity: 800 in/s from the object center toward the enemy center
/// plus 200 up, and the local angular velocity (deg/s) from two rolls in [0, 1).
pub fn swat_velocity(object_center: Vec3, enemy_center: Vec3, rolls: (f32, f32)) -> (Vec3, Vec3) {
    let v = (enemy_center - object_center).normalize_or_zero() * 800. + Vec3::Z * 200.;
    (
        v,
        Vec3::new(-180. + 360. * rolls.0, 20., -360. + 720. * rolls.1),
    )
}
/// The zombie classes under this AI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Kind {
    /// npc_zombie (CZombie).
    Classic,
    /// npc_poisonzombie (CNPC_PoisonZombie): melee only here.
    Poison,
}
impl Kind {
    /// CanSwatPhysicsObjects for the classes here. CNPC_PoisonZombie inherits true, but
    /// models/zombie/poison.mdl has no ACT_ZOM_SWAT* sequences and native behavior with one
    /// is not observed, so only the classic zombie swats for now.
    pub fn swats(self) -> bool {
        self == Self::Classic
    }
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
        "SCHED_ZOMBIE_MOVE_SWATITEM",
        "Tasks TASK_ZOMBIE_DELAY_SWAT 3 TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_CHASE_ENEMY          TASK_ZOMBIE_GET_PATH_TO_PHYSOBJ 0 TASK_WALK_PATH 0 TASK_WAIT_FOR_MOVEMENT 0          TASK_FACE_ENEMY 0 TASK_ZOMBIE_SWAT_ITEM 0 Interrupts COND_ENEMY_DEAD COND_NEW_ENEMY",
    ),
    (
        "SCHED_ZOMBIE_SWATITEM",
        "Tasks TASK_ZOMBIE_DELAY_SWAT 3 TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_CHASE_ENEMY          TASK_FACE_ENEMY 0 TASK_ZOMBIE_SWAT_ITEM 0 Interrupts COND_ENEMY_DEAD COND_NEW_ENEMY",
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
    /// m_hPhysicsEnt and its WorldSpaceCenter (the runtime refreshes the center each think
    /// and drops the object when its body is gone).
    pub physics_ent: Option<(usize, Vec3)>,
    /// m_flNextSwat (TASK_ZOMBIE_DELAY_SWAT) and m_flNextSwatScan (Spawn: now).
    pub next_swat: f64,
    pub next_swat_scan: f64,
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
            physics_ent: None,
            next_swat: 0.,
            next_swat_scan: 0.,
            seed: seed | 1,
        }
    }
    /// DistToPhysicsEnt (ZOMBIE_PHYSOBJ_SWATDIST + 1 without an object).
    pub fn dist_to_physics_ent(&self, origin: Vec3) -> f32 {
        self.physics_ent
            .map_or(PHYSOBJ_SWATDIST + 1., |(_, center)| {
                dist_approx_2d(origin, center)
            })
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
            NpcState::Combat if c.has(COND_ZOMBIE_CAN_SWAT_ATTACK) => {
                return "SCHED_ZOMBIE_SWATITEM".into();
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
    fn translate_schedule(&mut self, name: &str, _npc: &AiNpc, ctx: &Context) -> String {
        match name {
            // Walk to an object that is out of reach, else swat it from here.
            "SCHED_ZOMBIE_SWATITEM" if self.dist_to_physics_ent(ctx.origin) > PHYSOBJ_SWATDIST => {
                "SCHED_ZOMBIE_MOVE_SWATITEM"
            }
            "SCHED_CHASE_ENEMY" => "SCHED_ZOMBIE_CHASE_ENEMY",
            "SCHED_MELEE_ATTACK1" => "SCHED_ZOMBIE_MELEE_ATTACK1",
            "SCHED_STANDOFF" => "SCHED_ZOMBIE_WANDER_STANDOFF",
            other => other,
        }
        .into()
    }
    /// SCHED_ZOMBIE_CHASE_ENEMY's class interrupt (the text above can only name shared
    /// conditions): a swattable object breaks off the chase.
    fn custom_interrupts(&self, npc: &AiNpc) -> Conditions {
        let mut interrupts = Conditions::default();
        if npc.schedule.as_deref() == Some("SCHED_ZOMBIE_CHASE_ENEMY") {
            interrupts.set(COND_ZOMBIE_CAN_SWAT_ATTACK);
        }
        interrupts
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
        npc: &mut AiNpc,
        task: &str,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        Some(match task {
            // Non-episodic: complete at once.
            "TASK_ZOMBIE_WAIT_POST_MELEE" => TaskStatus::Complete,
            "TASK_ZOMBIE_DELAY_SWAT" => {
                self.next_swat = ctx.now + f64::from(arg.number());
                TaskStatus::Complete
            }
            // A walk to the object's center.
            "TASK_ZOMBIE_GET_PATH_TO_PHYSOBJ" => match self.physics_ent {
                Some((_, center)) if motor.path_to(center, false) => TaskStatus::Complete,
                _ => TaskStatus::Failed,
            },
            // Gone (broken, swatted by another zombie) or moved out of reach: fail; else the
            // swat activity for its side and height (GetSwatActivity reads the object's
            // origin for the side; its center stands in here).
            "TASK_ZOMBIE_SWAT_ITEM" => match self.physics_ent {
                Some((_, center)) if self.dist_to_physics_ent(ctx.origin) <= PHYSOBJ_SWATDIST => {
                    let activity =
                        swat_activity(ctx.yaw, ctx.origin, center, ctx.origin.z + 36., center.z);
                    npc.activity = activity.into();
                    motor.set_activity(activity);
                    TaskStatus::Running
                }
                _ => TaskStatus::Failed,
            },
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
    fn run_task(
        &mut self,
        _npc: &mut AiNpc,
        task: &str,
        _arg: &TaskArg,
        _ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        (task == "TASK_ZOMBIE_SWAT_ITEM").then(|| {
            if motor.activity_finished() {
                TaskStatus::Complete
            } else {
                TaskStatus::Running
            }
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
    fn swat_objects_follow_find_nearest_physics_object() {
        let object = |entity, x: f32, y: f32, z: f32| SwatCandidate {
            entity,
            center: Vec3::new(x, y, z),
            radius: 20.,
            top: z + 15.,
            visible: true,
            ragdoll: false,
        };
        let (origin, enemy) = (Vec3::ZERO, Vec3::new(600., 0., 0.));
        let (knees, eyes) = (18., 64.);
        let pick = |c: &[SwatCandidate]| find_nearest_physics_object(origin, enemy, knees, eyes, c);
        // The nearest of two objects toward the enemy.
        assert_eq!(
            pick(&[object(1, 200., 0., 20.), object(2, 100., 10., 20.)]),
            Some(2)
        );
        // Behind the zombie or off the line to the enemy (2D dot below 0.8).
        assert_eq!(pick(&[object(1, -100., 0., 20.)]), None);
        assert_eq!(pick(&[object(1, 100., 100., 20.)]), None);
        // Beyond min(enemy distance, 240).
        assert_eq!(pick(&[object(1, 250., 0., 20.)]), None);
        // Under the knees (center + radius and the AABB top), over the eyes.
        assert_eq!(
            pick(&[SwatCandidate {
                radius: 3.,
                top: 6.,
                ..object(1, 100., 0., 2.)
            }]),
            None
        );
        assert_eq!(
            pick(&[SwatCandidate {
                top: 10.,
                ..object(1, 100., 0., 10.)
            }]),
            None
        );
        assert_eq!(pick(&[object(1, 100., 0., 70.)]), None);
        // Hidden or a ragdoll.
        assert_eq!(
            pick(&[SwatCandidate {
                visible: false,
                ..object(1, 100., 0., 20.)
            }]),
            None
        );
        assert_eq!(
            pick(&[SwatCandidate {
                ragdoll: true,
                ..object(1, 100., 0., 20.)
            }]),
            None
        );
        // Nearer the enemy than the zombie.
        let close = Vec3::new(150., 0., 0.);
        assert_eq!(
            find_nearest_physics_object(origin, close, knees, eyes, &[object(1, 100., 0., 20.)]),
            None
        );
        // An enemy beyond ZOMBIE_PLAYER_MAX_SWAT_DIST: no swats and no search box.
        let far = Vec3::new(1001., 0., 0.);
        assert_eq!(
            find_nearest_physics_object(origin, far, knees, eyes, &[object(1, 100., 0., 20.)]),
            None
        );
        assert_eq!(swat_search_box(origin, far, 72.), None);
        assert_eq!(
            swat_search_box(origin, enemy, 72.),
            Some(Vec3::new(240., 240., 144.))
        );
    }

    #[test]
    #[ignore = "requires owned installed HL2"]
    fn owned_zombie_swat_sequences_carry_the_swat_events() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let wanted = source_assets::models::ZOMBIE_AI_ACTIVITIES
            .iter()
            .map(|a| a.to_lowercase())
            .collect();
        // The poison zombie inherits CanSwatPhysicsObjects but its model has no swat sequences.
        let poison =
            source_assets::animation::load(&vfs, "models/zombie/poison.mdl", &wanted).unwrap();
        assert!(source_assets::models::ZOMBIE_AI_ACTIVITIES
            .iter()
            .all(|a| poison.lookup_sequence(a, None, |_| 0).is_none()));
        for model in ["models/zombie/classic.mdl"] {
            let rig = source_assets::animation::load(&vfs, model, &wanted).unwrap();
            for activity in source_assets::models::ZOMBIE_AI_ACTIVITIES {
                let sequence = rig
                    .lookup_sequence(activity, None, |_| 0)
                    .unwrap_or_else(|| panic!("{model} {activity}"));
                let clip = &rig.clips[sequence];
                let events: Vec<_> = clip.events.iter().map(|e| e.name.to_uppercase()).collect();
                println!(
                    "{model} {activity}: {sequence} {:.2} s {events:?}",
                    clip.duration()
                );
                assert!(
                    events.iter().any(|e| e == AE_SWATITEM),
                    "{model} {activity} {events:?}"
                );
            }
        }
    }

    #[test]
    fn swat_arm_height_and_kick_follow_the_sdk() {
        // Facing +x: right is -y. An object above the zombie's center is MID.
        let o = Vec3::ZERO;
        assert_eq!(
            swat_activity(0., o, Vec3::new(50., -20., 0.), 36., 40.),
            "ACT_ZOM_SWATRIGHTMID"
        );
        assert_eq!(
            swat_activity(0., o, Vec3::new(50., -20., 0.), 36., 20.),
            "ACT_ZOM_SWATRIGHTLOW"
        );
        assert_eq!(
            swat_activity(0., o, Vec3::new(50., 20., 0.), 36., 40.),
            "ACT_ZOM_SWATLEFTMID"
        );
        assert_eq!(
            swat_activity(90., o, Vec3::new(-20., 50., 0.), 36., 20.),
            "ACT_ZOM_SWATLEFTLOW"
        );
        let (v, w) = swat_velocity(Vec3::ZERO, Vec3::new(100., 0., 0.), (0.5, 0.5));
        assert_eq!(v, Vec3::new(800., 0., 200.));
        assert_eq!(w, Vec3::new(0., 20., 0.));
        assert_eq!(
            swat_velocity(Vec3::ZERO, Vec3::X, (0., 1.)).1,
            Vec3::new(-180., 20., 360.)
        );
    }

    #[test]
    fn swat_schedules_move_to_far_objects_and_wait_between_swats() {
        let mut z = Zombie::new(Kind::Classic, false, 7);
        let mut npc = AiNpc::new(1);
        npc.state = NpcState::Combat;
        npc.conditions.set(COND_ZOMBIE_CAN_SWAT_ATTACK);
        let ctx = Context::default();
        assert_eq!(z.select_schedule(&mut npc, &ctx), "SCHED_ZOMBIE_SWATITEM");
        // Without an object the translation keeps the schedule (its task then fails).
        z.physics_ent = Some((5, Vec3::new(100., 0., 20.)));
        assert_eq!(
            z.translate_schedule("SCHED_ZOMBIE_SWATITEM", &npc, &ctx),
            "SCHED_ZOMBIE_MOVE_SWATITEM"
        );
        z.physics_ent = Some((5, Vec3::new(50., 20., 20.)));
        assert_eq!(z.dist_to_physics_ent(Vec3::ZERO), 70.);
        assert_eq!(
            z.translate_schedule("SCHED_ZOMBIE_SWATITEM", &npc, &ctx),
            "SCHED_ZOMBIE_SWATITEM"
        );
        z.physics_ent = None;
        assert_eq!(z.dist_to_physics_ent(Vec3::ZERO), PHYSOBJ_SWATDIST + 1.);
        // The chase breaks off for a swat; other schedules keep their own interrupts.
        npc.schedule = Some("SCHED_ZOMBIE_CHASE_ENEMY".into());
        assert!(z.custom_interrupts(&npc).has(COND_ZOMBIE_CAN_SWAT_ATTACK));
        npc.schedule = Some("SCHED_ZOMBIE_MELEE_ATTACK1".into());
        assert!(!z.custom_interrupts(&npc).has(COND_ZOMBIE_CAN_SWAT_ATTACK));
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
