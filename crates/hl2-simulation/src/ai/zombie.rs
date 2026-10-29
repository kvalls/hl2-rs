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
/// CZombie's class conditions after CNPC_BaseZombie's (CAN_SWAT_ATTACK, RELEASECRAB,
/// LOCAL_MELEE_OBSTRUCTION).
pub const COND_BLOCKED_BY_DOOR: Cond = Cond(LAST_SHARED_CONDITION + 3);
pub const COND_DOOR_OPENED: Cond = Cond(LAST_SHARED_CONDITION + 4);
/// m_NextTimeToStartDoorBash's interval (CSimTimer 3.0).
pub const DOOR_BASH_INTERVAL: f64 = 3.;
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
/// CNPC_BaseZombie::SelectFailSchedule swats an obstructor lighter than this (kg).
pub const ATTACKITEM_MAX_MASS: f32 = 100.;
/// ClawAttack's push on the swatted obstructor (5 x 24, the damage force).
pub const ATTACKITEM_FORCE: f32 = 120.;
/// IsPathTaskFailure: the failed task built or followed a route (FAIL_NO_ROUTE..ILLEGAL).
pub fn path_task(task: &str) -> bool {
    matches!(
        task,
        "TASK_GET_CHASE_PATH_TO_ENEMY"
            | "TASK_GET_PATH_TO_ENEMY"
            | "TASK_GET_PATH_TO_ENEMY_LKP"
            | "TASK_GET_PATH_TO_ENEMY_LOS"
            | "TASK_GET_PATH_TO_TARGET"
            | "TASK_GET_PATH_TO_SAVEPOSITION"
            | "TASK_GET_PATH_TO_PLAYER"
            | "TASK_ZOMBIE_GET_PATH_TO_PHYSOBJ"
            | "TASK_ZOMBIE_CHARGE_ENEMY"
            | "TASK_RUN_PATH"
            | "TASK_WALK_PATH"
            | "TASK_WAIT_FOR_MOVEMENT"
    )
}
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
            // npc_zombie_torso is CZombie placed with m_fIsTorso.
            "npc_zombie" | "npc_zombie_torso" => Some(Self::Classic),
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
        "SCHED_ZOMBIE_BASH_DOOR",
        "Tasks TASK_SET_ACTIVITY ACTIVITY:ACT_ZOMBIE_TANTRUM \
         TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_TAKE_COVER_FROM_ENEMY TASK_ZOMBIE_YAW_TO_DOOR 0 \
         TASK_FACE_IDEAL 0 TASK_ZOMBIE_ATTACK_DOOR 0 Interrupts COND_ENEMY_DEAD COND_NEW_ENEMY",
    ),
    (
        "SCHED_ZOMBIE_ATTACKITEM",
        "Tasks TASK_FACE_ENEMY 0 TASK_MELEE_ATTACK1 0 Interrupts COND_ENEMY_DEAD COND_NEW_ENEMY",
    ),
    // CZombie's (npc_zombie.cpp).
    (
        "SCHED_ZOMBIE_WANDER_ANGRILY",
        "Tasks TASK_WANDER 480240 TASK_WALK_PATH 0 TASK_WAIT_FOR_MOVEMENT 4 Interrupts \
         COND_ENEMY_DEAD COND_NEW_ENEMY",
    ),
    (
        "SCHED_ZOMBIE_CHARGE_ENEMY",
        "Tasks TASK_ZOMBIE_CHARGE_ENEMY 0 TASK_WALK_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 \
         TASK_PLAY_SEQUENCE ACTIVITY:ACT_ZOMBIE_TANTRUM Interrupts COND_ENEMY_DEAD \
         COND_NEW_ENEMY",
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

/// CNPC_BaseZombie::MaxYawSpeed (torsos turn at 60; no move_yaw pose blend).
pub fn max_yaw_speed(activity: &str, torso: bool) -> f32 {
    if torso {
        return 60.;
    }
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

/// ZOMBIE_TORSO_HEALTH_FACTOR: BecomeTorso halves max health and refills health to it.
pub const TORSO_HEALTH_FACTOR: f32 = 0.5;
/// BecomeTorso lifts the torso this far (where it was on the whole zombie) and spawns the
/// legs this far below it.
pub const TORSO_LIFT: f32 = 40.;
pub const DMG_REMOVENORAGDOLL: u32 = 1 << 22;
/// CNPC_BaseZombie::OnTakeDamage_Alive's split decision for one damage after the base
/// damage was applied (retail FUN_1033b6b0 follows the SDK).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Split {
    /// No split.
    None,
    /// BecomeTorso (it had its head): a live torso, or one that dies at once when the
    /// damage reached its max health (threshold 1).
    Torso { dies: bool },
    /// Headless after a lethal blast (RELEASE_RAGDOLL): DieChopped, one time in three.
    Chopped,
}
/// The damage outcome for a CZombie (`torso`: already split; `slumped`: CZombie never
/// splits a slumped zombie; `roll3`: RandomInt(1, 3) == 1). The poison zombie overrides
/// ShouldBecomeTorso to false. Returns the split and whether a lethal blast released the
/// headcrab as a ragdoll (headless).
#[allow(clippy::too_many_arguments)]
pub fn split(
    kind: Kind,
    torso: bool,
    slumped: bool,
    damage_type: u32,
    damage: f32,
    max_health: f32,
    health_after: f32,
    roll3: bool,
) -> (Split, bool) {
    use crate::player_damage::DMG_BLAST;
    let threshold = (damage / max_health.max(1e-6)).min(1.);
    // ShouldReleaseHeadcrab (health <= 0): a blast kill releases a ragdoll crab.
    let headless = health_after <= 0.
        && damage_type & DMG_REMOVENORAGDOLL == 0
        && damage_type & DMG_BLAST != 0;
    let should = kind == Kind::Classic
        && damage_type & DMG_REMOVENORAGDOLL == 0
        && !torso
        && !slumped
        && damage_type & DMG_BLAST != 0
        && threshold >= 0.5;
    let split = match (should, headless) {
        (false, _) => Split::None,
        // CanBecomeLiveTorso: !m_fIsHeadless.
        (true, false) => Split::Torso {
            dies: threshold >= 1.,
        },
        (true, true) if roll3 => Split::Chopped,
        (true, true) => Split::None,
    };
    (split, headless)
}
/// One BecomeTorso for the host: the zombie entity, the legs raggib's force (the damage
/// force x 0.5), the torso's new max health, whether it dies at once (threshold 1, with
/// the damage force) and the blast's ignition time (50% of blasts: 5-10 s).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct TorsoSplit {
    pub entity: usize,
    pub legs_force: Vec3,
    pub max_health: f32,
    pub dies: Option<Vec3>,
    pub ignite: Option<f32>,
}
/// A lethal blast's ReleaseHeadcrab(RELEASE_RAGDOLL) for the host: the crab becomes a
/// ragdoll at the "headcrab" attachment pushed with the damage force x 0.25, the body
/// dies headless; `chopped` (DieChopped, one in three) splits the body into legs and a
/// headless torso instead: (legs force, torso force).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct CrabRelease {
    pub entity: usize,
    pub crab_force: Vec3,
    pub chopped: Option<(Vec3, Vec3)>,
}
/// DieChopped's gib forces from the damage force and four rolls in [0, 1): the legs get
/// (U(-400, 400), U(-400, 400), U(0, 250)); the torso CalcDamageForceVector x U(0.04,
/// 0.06) with its z replaced by 6000 x U(0.8, 1.2).
pub fn chopped_forces(force: Vec3, rolls: [f32; 5]) -> (Vec3, Vec3) {
    let legs = Vec3::new(
        -400. + 800. * rolls[0],
        -400. + 800. * rolls[1],
        250. * rolls[2],
    );
    let mut torso = force * (0.04 + 0.02 * rolls[3]);
    torso.z = 100. * 12. * 5. * (0.8 + 0.4 * rolls[4]);
    (legs, torso)
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
    /// m_fIsTorso (npc_zombie_torso, or a zombie split by a blast).
    pub torso: bool,
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
    /// m_hObstructor (OnInsufficientStopDist: the entity that blocked a move while a swat
    /// was allowed), with its WorldSpaceCenter and mass.
    pub obstructor: Option<(usize, Vec3, f32)>,
    /// m_vPositionCharged (TASK_ZOMBIE_CHARGE_ENEMY).
    pub charge_position: Option<Vec3>,
    /// m_hBlockingDoor and m_flDoorBashYaw (OnObstructingDoor: a closed brush door that
    /// blocked a move).
    pub blocking_door: Option<(usize, f32)>,
    /// m_DurationDoorBash's end and m_NextTimeToStartDoorBash.
    pub door_bash_until: f64,
    pub next_door_bash: f64,
    /// The last think's time (SelectFailSchedule reads the door bash timer).
    pub now: f64,
    seed: u64,
}
impl Zombie {
    pub fn new(kind: Kind, gagged: bool, seed: u64) -> Self {
        Self {
            kind,
            torso: false,
            requests: Vec::new(),
            assault: None,
            script_goal: None,
            next_moan: None,
            gagged,
            physics_ent: None,
            next_swat: 0.,
            next_swat_scan: 0.,
            obstructor: None,
            charge_position: None,
            blocking_door: None,
            door_bash_until: 0.,
            next_door_bash: 0.,
            now: 0.,
            seed: seed | 1,
        }
    }
    /// CZombie::SelectDoorBash: RandomInt(1, 3) == 1 swings, else pounds.
    fn door_bash_activity(&mut self) -> &'static str {
        if self.random() < 1. / 3. {
            "ACT_MELEE_ATTACK1"
        } else {
            "ACT_ZOMBIE_WALLPOUND"
        }
    }
    /// GatherConditions scans for swats only in combat and not as a torso.
    pub fn swats(&self) -> bool {
        self.kind.swats() && !self.torso
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
        self.now = now;
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
        match npc.schedule.as_deref() {
            Some("SCHED_ZOMBIE_CHASE_ENEMY") => interrupts.set(COND_ZOMBIE_CAN_SWAT_ATTACK),
            Some(
                "SCHED_ZOMBIE_BASH_DOOR"
                | "SCHED_ZOMBIE_WANDER_ANGRILY"
                | "SCHED_ZOMBIE_CHARGE_ENEMY",
            ) => interrupts.set(COND_DOOR_OPENED),
            _ => {}
        }
        interrupts
    }
    /// CZombie::SelectFailSchedule (door bashing not modeled), then
    /// CNPC_BaseZombie::SelectFailSchedule, then the base m_failSchedule.
    fn select_fail_schedule(&mut self, npc: &AiNpc, failed: &str) -> String {
        let path_failure = npc
            .last_failed_task
            .as_deref()
            .and_then(|t| t.rsplit('/').next())
            .is_some_and(path_task);
        if self.kind == Kind::Classic {
            // A closed brush door in the way: bash it (not again right after a bash).
            if self.blocking_door.is_some() {
                if self.now >= self.next_door_bash && failed != "SCHED_ZOMBIE_BASH_DOOR" {
                    return "SCHED_ZOMBIE_BASH_DOOR".into();
                }
                self.blocking_door = None;
            }
            // RandomInt(1, 100) < 50.
            if failed != "SCHED_ZOMBIE_CHARGE_ENEMY"
                && path_failure
                && ((self.random() * 100.) as u32) < 49
            {
                return "SCHED_ZOMBIE_CHARGE_ENEMY".into();
            }
            if failed != "SCHED_ZOMBIE_WANDER_ANGRILY"
                && matches!(
                    failed,
                    "SCHED_TAKE_COVER_FROM_ENEMY" | "SCHED_CHASE_ENEMY_FAILED"
                )
            {
                return "SCHED_ZOMBIE_WANDER_ANGRILY".into();
            }
        }
        if failed == "SCHED_ZOMBIE_WANDER_MEDIUM" {
            return "SCHED_ZOMBIE_WANDER_FAIL".into();
        }
        // Swat the obstructor (CanSwatPhysicsObjects, not as a torso).
        let obstructor = self.obstructor.take();
        if let Some((entity, center, _)) = obstructor
            .filter(|_| self.swats() && path_failure)
            .filter(|(_, _, mass)| *mass < ATTACKITEM_MAX_MASS)
        {
            self.physics_ent = Some((entity, center));
            return "SCHED_ZOMBIE_ATTACKITEM".into();
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
            // SetIdealYaw(m_flDoorBashYaw), turned to by TASK_FACE_IDEAL.
            "TASK_ZOMBIE_YAW_TO_DOOR" => match self.blocking_door {
                Some(_) => TaskStatus::Complete,
                None => TaskStatus::Failed,
            },
            "TASK_FACE_IDEAL" => match self.blocking_door {
                Some((_, yaw)) if motor.face(super::schedule::Goal::Yaw(yaw)) => {
                    TaskStatus::Complete
                }
                Some(_) => TaskStatus::Running,
                None => TaskStatus::Complete,
            },
            // m_DurationDoorBash (2-6 s) of SelectDoorBash: one in three a claw swing,
            // else ACT_ZOMBIE_WALLPOUND.
            "TASK_ZOMBIE_ATTACK_DOOR" => {
                self.door_bash_until = ctx.now + 2. + 4. * f64::from(self.random());
                let activity = self.door_bash_activity();
                npc.activity = activity.into();
                motor.set_activity(activity);
                TaskStatus::Running
            }
            // SetVectorGoalFromTarget(enemy origin): a route to where the enemy stands.
            "TASK_ZOMBIE_CHARGE_ENEMY" => match ctx.enemy {
                Some((_, position)) if motor.path_to(position, false) => {
                    self.charge_position = Some(position);
                    TaskStatus::Complete
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
        npc: &mut AiNpc,
        task: &str,
        _arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        match task {
            "TASK_ZOMBIE_SWAT_ITEM" => Some(if motor.activity_finished() {
                TaskStatus::Complete
            } else {
                TaskStatus::Running
            }),
            "TASK_FACE_IDEAL" => Some(match self.blocking_door {
                Some((_, yaw)) if !motor.face(super::schedule::Goal::Yaw(yaw)) => {
                    TaskStatus::Running
                }
                _ => TaskStatus::Complete,
            }),
            "TASK_ZOMBIE_ATTACK_DOOR" => Some(if !motor.activity_finished() {
                TaskStatus::Running
            } else if ctx.now >= self.door_bash_until {
                self.next_door_bash = ctx.now + DOOR_BASH_INTERVAL;
                TaskStatus::Complete
            } else {
                let activity = self.door_bash_activity();
                npc.activity = activity.into();
                motor.set_activity(activity);
                TaskStatus::Running
            }),
            _ => None,
        }
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
    fn failed_chases_charge_wander_or_swat_the_obstructor() {
        let mut npc = AiNpc::new(1);
        npc.fail_schedule = Some("SCHED_CHASE_ENEMY_FAILED".into());
        npc.last_failed_task = Some("SCHED_ZOMBIE_CHASE_ENEMY/TASK_WAIT_FOR_MOVEMENT".into());
        // CZombie: about half of path failures charge, the rest fall through to the base
        // zombie, which swats a light obstructor.
        let mut charges = 0;
        let mut swats = 0;
        for seed in 0..200u64 {
            // Runtime seeds are large (0x3c6e_f372_fe94_f82b ^ id).
            let mut z = Zombie::new(
                Kind::Classic,
                false,
                0x9e37_79b9_7f4a_7c15u64.wrapping_mul(seed + 1),
            );
            z.obstructor = Some((7, Vec3::new(40., 0., 20.), 15.));
            match z
                .select_fail_schedule(&npc, "SCHED_ZOMBIE_CHASE_ENEMY")
                .as_str()
            {
                "SCHED_ZOMBIE_CHARGE_ENEMY" => charges += 1,
                "SCHED_ZOMBIE_ATTACKITEM" => {
                    assert_eq!(z.physics_ent, Some((7, Vec3::new(40., 0., 20.))));
                    swats += 1;
                }
                other => panic!("{other}"),
            }
        }
        assert!((70..130).contains(&charges), "{charges}");
        assert_eq!(charges + swats, 200);
        // A failed charge never charges again; a heavy obstructor is not swatted.
        let mut z = Zombie::new(Kind::Classic, false, 3);
        z.obstructor = Some((7, Vec3::ZERO, 150.));
        assert_eq!(
            z.select_fail_schedule(&npc, "SCHED_ZOMBIE_CHARGE_ENEMY"),
            "SCHED_CHASE_ENEMY_FAILED"
        );
        // A cover run that finds no cover (not a path failure) wanders angrily; a torso
        // never swats.
        let mut no_cover = npc.clone();
        no_cover.last_failed_task =
            Some("SCHED_CHASE_ENEMY_FAILED/TASK_FIND_COVER_FROM_ENEMY".into());
        assert_eq!(
            z.select_fail_schedule(&no_cover, "SCHED_CHASE_ENEMY_FAILED"),
            "SCHED_ZOMBIE_WANDER_ANGRILY"
        );
        let mut torso = Zombie::new(Kind::Classic, false, 3);
        torso.torso = true;
        torso.obstructor = Some((7, Vec3::ZERO, 15.));
        assert_ne!(
            torso.select_fail_schedule(&npc, "SCHED_ZOMBIE_CHARGE_ENEMY"),
            "SCHED_ZOMBIE_ATTACKITEM"
        );
        // Without a path failure (an animation task failed) there is no swat.
        npc.last_failed_task = Some("SCHED_ZOMBIE_MELEE_ATTACK1/TASK_MELEE_ATTACK1".into());
        let mut z = Zombie::new(Kind::Poison, false, 3);
        z.obstructor = Some((7, Vec3::ZERO, 15.));
        assert_eq!(
            z.select_fail_schedule(&npc, "SCHED_ZOMBIE_MELEE_ATTACK1"),
            "SCHED_CHASE_ENEMY_FAILED"
        );
    }
    #[test]
    fn die_chopped_throws_the_legs_randomly_and_the_torso_up() {
        let (legs, torso) = chopped_forces(Vec3::new(10000., 0., 2000.), [0., 1., 0.5, 0.5, 0.5]);
        assert_eq!(legs, Vec3::new(-400., 400., 125.));
        // x 0.05 of the damage force, z replaced by 6000.
        assert!(torso.distance(Vec3::new(500., 0., 6000.)) < 1e-2, "{torso}");
        let (_, low) = chopped_forces(Vec3::X, [0.; 5]);
        assert!((low.z - 4800.).abs() < 1e-3);
    }
    #[test]
    fn closed_brush_doors_get_bashed_then_rested() {
        let mut npc = AiNpc::new(1);
        npc.fail_schedule = Some("SCHED_CHASE_ENEMY_FAILED".into());
        npc.last_failed_task = Some("SCHED_ZOMBIE_CHASE_ENEMY/TASK_WAIT_FOR_MOVEMENT".into());
        let mut z = Zombie::new(Kind::Classic, false, 0x1234_5678_9abc_def1);
        z.blocking_door = Some((9, 90.));
        z.now = 10.;
        // CZombie bashes before it would charge or swat.
        assert_eq!(
            z.select_fail_schedule(&npc, "SCHED_ZOMBIE_CHASE_ENEMY"),
            "SCHED_ZOMBIE_BASH_DOOR"
        );
        // Not right after a bash (m_NextTimeToStartDoorBash), nor after a failed bash: the
        // door is forgotten.
        z.next_door_bash = 12.;
        assert_ne!(
            z.select_fail_schedule(&npc, "SCHED_ZOMBIE_CHASE_ENEMY"),
            "SCHED_ZOMBIE_BASH_DOOR"
        );
        assert_eq!(z.blocking_door, None);
        z.blocking_door = Some((9, 90.));
        z.next_door_bash = 0.;
        assert_ne!(
            z.select_fail_schedule(&npc, "SCHED_ZOMBIE_BASH_DOOR"),
            "SCHED_ZOMBIE_BASH_DOOR"
        );
        // SelectDoorBash: about one swing in three, else wall pounding.
        let swings = (0..300)
            .filter(|_| z.door_bash_activity() == "ACT_MELEE_ATTACK1")
            .count();
        assert!((60..140).contains(&swings), "{swings}");
        // The open door interrupts the bash.
        npc.schedule = Some("SCHED_ZOMBIE_BASH_DOOR".into());
        assert!(z.custom_interrupts(&npc).has(COND_DOOR_OPENED));
    }
    #[test]
    fn blasts_split_zombies_like_on_take_damage_alive() {
        use crate::player_damage::{DMG_BLAST, DMG_BULLET};
        let c = Kind::Classic;
        // A full 50 HP zombie: 25-49 blast damage leaves a live torso.
        assert_eq!(
            split(c, false, false, DMG_BLAST, 25., 50., 25., false),
            (Split::Torso { dies: false }, false)
        );
        assert_eq!(
            split(c, false, false, DMG_BLAST, 49., 50., 1., true),
            (Split::Torso { dies: false }, false)
        );
        // Under half of max health: no split.
        assert_eq!(
            split(c, false, false, DMG_BLAST, 24., 50., 26., true).0,
            Split::None
        );
        // A lethal blast releases the crab (headless): chopped one time in three.
        assert_eq!(
            split(c, false, false, DMG_BLAST, 50., 50., 0., true),
            (Split::Chopped, true)
        );
        assert_eq!(
            split(c, false, false, DMG_BLAST, 30., 50., -5., false),
            (Split::None, true)
        );
        // Threshold 1 with health left (health above max): a torso that dies at once.
        assert_eq!(
            split(c, false, false, DMG_BLAST, 60., 50., 10., false).0,
            Split::Torso { dies: true }
        );
        // Bullets, torsos, slumped zombies, REMOVENORAGDOLL and poison zombies never split.
        assert_eq!(
            split(c, false, false, DMG_BULLET, 30., 50., 20., true).0,
            Split::None
        );
        assert_eq!(
            split(c, true, false, DMG_BLAST, 30., 50., 20., true).0,
            Split::None
        );
        assert_eq!(
            split(c, false, true, DMG_BLAST, 30., 50., 20., true).0,
            Split::None
        );
        assert_eq!(
            split(
                c,
                false,
                false,
                DMG_BLAST | DMG_REMOVENORAGDOLL,
                30.,
                50.,
                20.,
                true
            ),
            (Split::None, false)
        );
        assert_eq!(
            split(Kind::Poison, false, false, DMG_BLAST, 100., 175., 75., true).0,
            Split::None
        );
        assert_eq!(max_yaw_speed("ACT_WALK", true), 60.);
        assert_eq!(max_yaw_speed("ACT_WALK", false), 25.);
    }
    #[test]
    #[ignore = "requires owned installed HL2"]
    fn owned_zombie_halves_list_their_sequences() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let wanted = ["crawl".to_owned()].into();
        let motion = source_assets::models::read_root_motion(
            &vfs,
            "models/zombie/classic_torso.mdl",
            &wanted,
        )
        .unwrap();
        for (name, s) in &motion {
            println!(
                "{name} flags {} act {} blend {} of {}",
                s.flags,
                s.activity_name,
                s.central_blend,
                s.blends.len()
            );
            if let Some(track) = s.blends.get(s.central_blend) {
                println!("frames {} records {:?}", track.frame_count, track.records);
            }
        }
        let locomotion = crate::npc::Locomotion::from_named(
            &motion,
            [
                (crate::npc::MoveStyle::Walk, "crawl", "ACT_WALK"),
                (crate::npc::MoveStyle::Run, "crawl", "ACT_WALK"),
            ],
        );
        assert!(locomotion.complete(), "the torso crawls with its walk");
        for model in [
            "models/zombie/classic.mdl",
            "models/zombie/classic_torso.mdl",
            "models/zombie/classic_legs.mdl",
        ] {
            println!(
                "{model} eyes {:?}",
                source_assets::models::read_eye_position(&vfs, model)
            );
            let names = source_assets::animation::sequence_names(&vfs, model).unwrap();
            let wanted = names.iter().map(|n| n.to_lowercase()).collect();
            let rig = source_assets::animation::load(&vfs, model, &wanted).unwrap();
            for s in &rig.sequences {
                let clip = rig.clips.get(&s.name.to_lowercase());
                println!(
                    "{model} {} {} w{} {:.2}s {:?}",
                    s.name,
                    s.activity,
                    s.weight,
                    clip.map_or(0., |c| c.duration()),
                    clip.map(|c| c.events.iter().map(|e| e.name.clone()).collect::<Vec<_>>())
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
