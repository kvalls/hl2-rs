//! The weapon-using humanoid classes beyond the metrocop (DESIGN 14n steps 2-3), as
//! CAI_BaseNPC/CAI_BaseHumanoid hosts with their SDK 2013 class overrides:
//! CNPC_PlayerCompanion + CNPC_Citizen (npc_citizen17.cpp, npc_playercompanion.cpp;
//! npc_barney and npc_alyx inherit the companion layer) and CNPC_Combine
//! (npc_combine.cpp; npc_combine_s). Both host the assault behavior and aiscripted
//! schedules like the metrocop. Not modeled yet: squads and squad slots (every slot is
//! free, as for an NPC without a squad), grenades and the AR2 alt-fire, the combine
//! kick, crouching, readiness, the follow/lead/actbusy/standoff/functank behaviors,
//! speech (response rules, COMBINE_* sentences), medic heals, mortars and the RPG
//! schedules. SDK behavior; retail not compared.
use super::assault::Assault;
use super::conditions::*;
use super::metropolice::CopRequest;
use super::schedule::{
    base_select_schedule, AiNpc, Behavior, Context, Motor, Schedule, Schedules, TaskArg, TaskStatus,
};
use super::senses::sound;
use super::state::NpcState;
use glam::Vec3;
use serde::Serialize;

/// The class layer above CAI_BaseHumanoid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Kind {
    /// CNPC_PlayerCompanion; `citizen`: CNPC_Citizen's overrides on top.
    Companion { citizen: bool },
    /// CNPC_Combine (npc_combine_s).
    Combine,
}
impl Kind {
    pub fn for_class(class: &str) -> Option<Self> {
        Some(match class {
            "npc_citizen" => Self::Companion { citizen: true },
            "npc_barney" | "npc_alyx" => Self::Companion { citizen: false },
            "npc_combine_s" => Self::Combine,
            _ => return None,
        })
    }
    /// m_flFieldOfView (CNPC_PlayerCompanion::Spawn 0.02, CNPC_Combine::Spawn -0.2).
    pub fn field_of_view(self) -> f32 {
        match self {
            Self::Companion { .. } => 0.02,
            Self::Combine => -0.2,
        }
    }
    /// GetSoundInterests (the readiness sounds have no CSoundEnt type here).
    pub fn sound_interests(self) -> u32 {
        let common =
            sound::WORLD | sound::COMBAT | sound::PLAYER | sound::DANGER | sound::BULLET_IMPACT;
        match self {
            Self::Companion { .. } => common | sound::MOVE_AWAY,
            Self::Combine => common | sound::PHYSICS_DANGER | sound::MOVE_AWAY,
        }
    }
    /// MaxYawSpeed for an activity (CNPC_Combine's table; the companions inherit
    /// CAI_BaseNPC's 45).
    pub fn max_yaw_speed(self, activity: &str) -> f32 {
        match self {
            Self::Companion { .. } => 45.,
            Self::Combine => match activity {
                "ACT_TURN_LEFT" | "ACT_TURN_RIGHT" => 45.,
                "ACT_RUN" | "ACT_RUN_HURT" => 15.,
                "ACT_WALK" | "ACT_WALK_CROUCH" => 25.,
                _ => 35.,
            },
        }
    }
}

/// npc_citizen17.cpp RPG_SAFE_DISTANCE (CMissile::EXPLOSION_RADIUS + 64).
pub const RPG_SAFE_DISTANCE: f32 = 200. + 64.;

/// CNPC_Combine's PrescheduleThink condition while IsOnFire.
pub const COND_COMBINE_ON_FIRE: Cond = Cond(LAST_SHARED_CONDITION + 1);
/// CNPC_Combine's schedules used here (SDK text; COND_COMBINE_NO_FIRE and
/// COND_COMBINE_ATTACK_SLOT_AVAILABLE need squads and are left out of the interrupts).
const COMBINE_SCHEDULES: &[(&str, &str)] = &[
    (
        "SCHED_COMBINE_BURNING_STAND",
        "Tasks TASK_SET_ACTIVITY ACTIVITY:ACT_COMBINE_BUGBAIT TASK_RANDOMIZE_FRAMERATE 20 \
         TASK_WAIT 2 TASK_WAIT_RANDOM 3 TASK_COMBINE_DIE_INSTANTLY DMG_BURN TASK_WAIT 1.0 \
         Interrupts",
    ),
    (
        "SCHED_COMBINE_TAKE_COVER_FROM_BEST_SOUND",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_COMBINE_RUN_AWAY_FROM_BEST_SOUND \
         TASK_STOP_MOVING 0 TASK_COMBINE_SIGNAL_BEST_SOUND 0 TASK_FIND_COVER_FROM_BEST_SOUND 0 \
         TASK_RUN_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 TASK_REMEMBER MEMORY:INCOVER \
         TASK_FACE_REASONABLE 0 Interrupts",
    ),
    (
        "SCHED_COMBINE_AR2_ALTFIRE",
        "Tasks TASK_STOP_MOVING 0 TASK_ANNOUNCE_ATTACK 1 \
         TASK_COMBINE_PLAY_SEQUENCE_FACE_ALTFIRE_TARGET ACTIVITY:ACT_COMBINE_AR2_ALTFIRE \
         Interrupts",
    ),
    (
        "SCHED_COMBINE_RUN_AWAY_FROM_BEST_SOUND",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_COWER TASK_GET_PATH_AWAY_FROM_BEST_SOUND 600 \
         TASK_RUN_PATH_TIMED 2 TASK_STOP_MOVING 0 Interrupts",
    ),
    (
        "SCHED_COMBINE_COMBAT_FAIL",
        "Tasks TASK_STOP_MOVING 0 TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE TASK_WAIT_FACE_ENEMY 2 \
         TASK_WAIT_PVS 0 Interrupts COND_CAN_RANGE_ATTACK1 COND_CAN_RANGE_ATTACK2 \
         COND_CAN_MELEE_ATTACK1 COND_CAN_MELEE_ATTACK2",
    ),
    (
        "SCHED_COMBINE_ASSAULT",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_COMBINE_ESTABLISH_LINE_OF_FIRE \
         TASK_SET_TOLERANCE_DISTANCE 48 TASK_GET_PATH_TO_ENEMY_LKP 0 \
         TASK_COMBINE_IGNORE_ATTACKS 0.2 TASK_SPEAK_SENTENCE 0 TASK_RUN_PATH 0 \
         TASK_WAIT_FOR_MOVEMENT 0 TASK_COMBINE_IGNORE_ATTACKS 0.0 Interrupts COND_NEW_ENEMY \
         COND_ENEMY_DEAD COND_ENEMY_UNREACHABLE COND_CAN_RANGE_ATTACK1 COND_CAN_MELEE_ATTACK1 \
         COND_CAN_RANGE_ATTACK2 COND_CAN_MELEE_ATTACK2 COND_TOO_FAR_TO_ATTACK COND_HEAR_DANGER \
         COND_HEAR_MOVE_AWAY",
    ),
    (
        "SCHED_COMBINE_ESTABLISH_LINE_OF_FIRE",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_FAIL_ESTABLISH_LINE_OF_FIRE \
         TASK_SET_TOLERANCE_DISTANCE 48 TASK_GET_PATH_TO_ENEMY_LKP_LOS 0 \
         TASK_COMBINE_SET_STANDING 1 TASK_SPEAK_SENTENCE 1 TASK_RUN_PATH 0 \
         TASK_WAIT_FOR_MOVEMENT 0 TASK_COMBINE_IGNORE_ATTACKS 0.0 \
         TASK_SET_SCHEDULE SCHEDULE:SCHED_COMBAT_FACE Interrupts COND_NEW_ENEMY COND_ENEMY_DEAD \
         COND_CAN_MELEE_ATTACK1 COND_CAN_MELEE_ATTACK2 COND_HEAR_DANGER COND_HEAR_MOVE_AWAY \
         COND_HEAVY_DAMAGE",
    ),
    (
        "SCHED_COMBINE_PRESS_ATTACK",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_COMBINE_ESTABLISH_LINE_OF_FIRE \
         TASK_SET_TOLERANCE_DISTANCE 72 TASK_GET_PATH_TO_ENEMY_LKP 0 \
         TASK_COMBINE_SET_STANDING 1 TASK_RUN_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 \
         Interrupts COND_NEW_ENEMY COND_ENEMY_DEAD COND_ENEMY_UNREACHABLE COND_NO_PRIMARY_AMMO \
         COND_LOW_PRIMARY_AMMO COND_TOO_CLOSE_TO_ATTACK COND_CAN_MELEE_ATTACK1 \
         COND_CAN_MELEE_ATTACK2 COND_HEAR_DANGER COND_HEAR_MOVE_AWAY",
    ),
    (
        "SCHED_COMBINE_COMBAT_FACE",
        "Tasks TASK_STOP_MOVING 0 TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE TASK_FACE_ENEMY 0 \
         TASK_WAIT 1.5 Interrupts COND_NEW_ENEMY COND_ENEMY_DEAD COND_CAN_RANGE_ATTACK1 \
         COND_CAN_RANGE_ATTACK2",
    ),
    (
        "SCHED_COMBINE_HIDE_AND_RELOAD",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_RELOAD TASK_FIND_COVER_FROM_ENEMY 0 \
         TASK_RUN_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 TASK_REMEMBER MEMORY:INCOVER \
         TASK_FACE_ENEMY 0 TASK_RELOAD 0 Interrupts COND_CAN_MELEE_ATTACK1 \
         COND_CAN_MELEE_ATTACK2 COND_HEAVY_DAMAGE COND_HEAR_DANGER COND_HEAR_MOVE_AWAY",
    ),
    (
        "SCHED_COMBINE_WAIT_IN_COVER",
        "Tasks TASK_STOP_MOVING 0 TASK_COMBINE_SET_STANDING 0 TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE \
         TASK_WAIT_FACE_ENEMY 1 Interrupts COND_NEW_ENEMY COND_CAN_RANGE_ATTACK1 \
         COND_CAN_RANGE_ATTACK2 COND_CAN_MELEE_ATTACK1 COND_CAN_MELEE_ATTACK2 COND_HEAR_DANGER \
         COND_HEAR_MOVE_AWAY",
    ),
    (
        "SCHED_COMBINE_TAKE_COVER1",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_COMBINE_TAKECOVER_FAILED TASK_STOP_MOVING 0 \
         TASK_WAIT 0.2 TASK_FIND_COVER_FROM_ENEMY 0 TASK_RUN_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 \
         TASK_REMEMBER MEMORY:INCOVER TASK_SET_SCHEDULE SCHEDULE:SCHED_COMBINE_WAIT_IN_COVER \
         Interrupts",
    ),
    (
        "SCHED_COMBINE_RANGE_ATTACK2",
        "Tasks TASK_STOP_MOVING 0 TASK_COMBINE_FACE_TOSS_DIR 0 TASK_ANNOUNCE_ATTACK 2 \
         TASK_PLAY_SEQUENCE ACTIVITY:ACT_RANGE_ATTACK2 TASK_COMBINE_DEFER_SQUAD_GRENADES 0 \
         TASK_SET_SCHEDULE SCHEDULE:SCHED_COMBINE_WAIT_IN_COVER Interrupts",
    ),
    (
        "SCHED_COMBINE_GRENADE_AND_RELOAD",
        "Tasks TASK_STOP_MOVING 0 TASK_COMBINE_FACE_TOSS_DIR 0 TASK_ANNOUNCE_ATTACK 2 \
         TASK_PLAY_SEQUENCE ACTIVITY:ACT_RANGE_ATTACK2 TASK_COMBINE_DEFER_SQUAD_GRENADES 0 \
         TASK_SET_SCHEDULE SCHEDULE:SCHED_HIDE_AND_RELOAD Interrupts",
    ),
    (
        "SCHED_COMBINE_TAKECOVER_FAILED",
        "Tasks TASK_STOP_MOVING 0 Interrupts",
    ),
    (
        "SCHED_COMBINE_RANGE_ATTACK1",
        "Tasks TASK_STOP_MOVING 0 TASK_FACE_ENEMY 0 TASK_ANNOUNCE_ATTACK 1 TASK_WAIT_RANDOM 0.3 \
         TASK_RANGE_ATTACK1 0 TASK_COMBINE_IGNORE_ATTACKS 0.5 Interrupts COND_NEW_ENEMY \
         COND_ENEMY_DEAD COND_HEAVY_DAMAGE COND_LIGHT_DAMAGE COND_LOW_PRIMARY_AMMO \
         COND_NO_PRIMARY_AMMO COND_WEAPON_BLOCKED_BY_FRIEND COND_TOO_CLOSE_TO_ATTACK \
         COND_GIVE_WAY COND_HEAR_DANGER COND_HEAR_MOVE_AWAY",
    ),
    (
        "SCHED_COMBINE_PATROL",
        "Tasks TASK_STOP_MOVING 0 TASK_WANDER 900540 TASK_WALK_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 \
         TASK_STOP_MOVING 0 TASK_FACE_REASONABLE 0 TASK_WAIT 3 TASK_WAIT_RANDOM 3 \
         TASK_SET_SCHEDULE SCHEDULE:SCHED_COMBINE_PATROL Interrupts COND_ENEMY_DEAD \
         COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE COND_HEAR_DANGER COND_HEAR_MOVE_AWAY COND_NEW_ENEMY \
         COND_SEE_ENEMY COND_CAN_RANGE_ATTACK1 COND_CAN_RANGE_ATTACK2",
    ),
    (
        "SCHED_COMBINE_PATROL_ENEMY",
        "Tasks TASK_STOP_MOVING 0 TASK_WAIT_FACE_ENEMY 1 TASK_WAIT_FACE_ENEMY_RANDOM 3 \
         Interrupts COND_ENEMY_DEAD COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE COND_HEAR_DANGER \
         COND_HEAR_MOVE_AWAY COND_NEW_ENEMY COND_SEE_ENEMY COND_CAN_RANGE_ATTACK1 \
         COND_CAN_RANGE_ATTACK2",
    ),
];
/// CNPC_PlayerCompanion's schedules used here (COND_PC_SAFE_FROM_MORTAR needs mortars
/// and is left out of the interrupts).
const COMPANION_SCHEDULES: &[(&str, &str)] = &[
    (
        "SCHED_PC_MOVE_TOWARDS_COVER_FROM_BEST_SOUND",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_FLEE_FROM_BEST_SOUND TASK_STOP_MOVING 0 \
         TASK_SET_TOLERANCE_DISTANCE 24 TASK_STORE_BESTSOUND_REACTORIGIN_IN_SAVEPOSITION 0 \
         TASK_FIND_COVER_FROM_BEST_SOUND 0 TASK_RUN_PATH_TIMED 1.0 TASK_STOP_MOVING 0 \
         TASK_FACE_SAVEPOSITION 0 TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE Interrupts",
    ),
    (
        "SCHED_PC_TAKE_COVER_FROM_BEST_SOUND",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_FLEE_FROM_BEST_SOUND TASK_STOP_MOVING 0 \
         TASK_SET_TOLERANCE_DISTANCE 24 TASK_STORE_BESTSOUND_REACTORIGIN_IN_SAVEPOSITION 0 \
         TASK_FIND_COVER_FROM_BEST_SOUND 0 TASK_RUN_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 \
         TASK_STOP_MOVING 0 TASK_FACE_SAVEPOSITION 0 TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE \
         Interrupts COND_NEW_ENEMY",
    ),
    (
        "SCHED_CITIZEN_RANGE_ATTACK1_RPG",
        "Tasks TASK_STOP_MOVING 0 TASK_FACE_ENEMY 0 TASK_ANNOUNCE_ATTACK 1 \
         TASK_RANGE_ATTACK1 0 TASK_CIT_RPG_AUGER 1 Interrupts",
    ),
    (
        "SCHED_PC_FLEE_FROM_BEST_SOUND",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_COWER TASK_GET_PATH_AWAY_FROM_BEST_SOUND 600 \
         TASK_RUN_PATH_TIMED 1.5 TASK_STOP_MOVING 0 TASK_TURN_LEFT 179 Interrupts COND_NEW_ENEMY",
    ),
];
pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in COMBINE_SCHEDULES.iter().chain(COMPANION_SCHEDULES) {
        let schedule = Schedule::parse(name, text).expect("humanoid schedule");
        schedules.0.insert((*name).into(), schedule);
    }
}

/// A combine soldier, citizen, Barney or Alyx under the AI.
#[derive(Clone, Debug, Serialize)]
pub struct Humanoid {
    pub kind: Kind,
    /// Sentences, outputs and sounds for the runtime (the metrocop's request type).
    #[serde(skip)]
    pub requests: Vec<CopRequest>,
    /// CAI_AssaultBehavior (ai_goal_assault).
    pub assault: Option<Assault>,
    /// The aiscripted_schedule goal (position, run).
    #[serde(skip)]
    pub script_goal: Option<(Vec3, bool)>,
    /// The assault behavior selected the current schedule.
    pub running_assault: bool,
    /// The time of this think (custom interrupts read it).
    #[serde(skip)]
    pub now: f64,
    /// GetActiveWeapon().
    pub armed: bool,
    /// m_flLastAttackTime (OnRangeAttack1).
    pub last_attack: f64,
    /// TASK_ANNOUNCE_ATTACK's SetWait(2.0).
    announce_until: f64,
    /// CNPC_Combine m_iNumGrenades (the NumGrenades keyvalue; 0 by default).
    pub grenades: i32,
    /// m_flNextGrenadeCheck.
    pub next_grenade_check: f64,
    /// m_vecTossVelocity from the last successful CheckCanThrowGrenade.
    pub toss_velocity: Vec3,
    /// This think's side-effect-free CheckCanThrowGrenade results (computed by the
    /// runtime, which owns the traces): toward the enemy's last seen and last known
    /// positions. `can_grenade_enemy` applies CanThrowGrenade's gate and next-check time
    /// only when a schedule branch asks, like the SDK's lazy calls.
    #[serde(skip)]
    pub throw_probes: ThrowProbes,
    /// The flush window: the enemy unseen for at most COMBINE_GRENADE_FLUSH_TIME (3 s) and
    /// within COMBINE_GRENADE_FLUSH_DIST (256) of where it was last seen.
    #[serde(skip)]
    pub flush_window: bool,
    /// CanGrenadeEnemy outcomes this think (runtime diagnostics).
    #[serde(skip)]
    pub grenade_outcomes: Vec<&'static str>,
    /// CNPC_Combine TASK_RANGE_ATTACK1: m_nShots left in the burst, m_flShotDelay
    /// (GetFireRate) and m_flNextAttack.
    pub shots: i32,
    shot_delay: f32,
    next_attack: f64,
    /// The active weapon is an RPG, and its missile still flies (CWeaponRPG::m_hMissile).
    #[serde(skip)]
    pub rpg: bool,
    #[serde(skip)]
    pub missile_out: bool,
    /// Weapon_ShootPosition this think (the laser's shoot position).
    #[serde(skip)]
    pub shoot_position: Vec3,
    /// The laser dot handed to UpdateNPCLaserPosition while guiding (TASK_CIT_RPG_AUGER).
    pub rpg_laser: Option<Vec3>,
    /// m_vecNPCLaserDot before the per-think imprecision.
    rpg_laser_base: Vec3,
    /// m_bRPGAvoidPlayer.
    rpg_avoid_player: bool,
    /// TASK_CIT_RPG_AUGER's wait end (the task's argument in seconds).
    rpg_wait_until: f64,
    /// The missile was seen in flight since the auger started.
    rpg_launched: bool,
    /// CNPC_CombineS::m_fIsElite (models/combine_super_soldier.mdl).
    pub elite: bool,
    /// m_flNextAltFireTime (the spawn time; DelayAltFireAttack pushes it).
    pub next_alt_fire: f64,
    /// m_vecAltFireTarget.
    pub alt_fire_target: Vec3,
    /// This think's CanAltFireEnemy(true) trace result (computed by the runtime): the
    /// chest point at the enemy's last known position when a 24-unit hull from the
    /// shoot position travels at least 65% of the way and more than 128 units.
    #[serde(skip)]
    pub alt_fire_probe: Option<Vec3>,
    /// The class's random stream (random->RandomInt in the class code).
    seed: u64,
}
/// A CheckCanThrowGrenade result without its side effects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThrowProbe {
    OutOfRange,
    Moving,
    Blocked { in_view: bool },
    Ready { velocity: Vec3, in_view: bool },
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ThrowProbes {
    pub last_seen: Option<ThrowProbe>,
    pub last_known: Option<ThrowProbe>,
}
impl Humanoid {
    pub fn new(kind: Kind, armed: bool, seed: u64) -> Self {
        Self {
            seed: seed | 1,
            kind,
            requests: Vec::new(),
            assault: None,
            script_goal: None,
            running_assault: false,
            now: 0.,
            armed,
            last_attack: f64::NEG_INFINITY,
            announce_until: 0.,
            grenades: 0,
            next_grenade_check: 0.,
            toss_velocity: Vec3::ZERO,
            throw_probes: ThrowProbes::default(),
            flush_window: false,
            grenade_outcomes: Vec::new(),
            shots: 0,
            shot_delay: 0.,
            next_attack: 0.,
            rpg: false,
            missile_out: false,
            shoot_position: Vec3::ZERO,
            rpg_laser: None,
            rpg_laser_base: Vec3::ZERO,
            rpg_avoid_player: false,
            rpg_wait_until: 0.,
            rpg_launched: false,
            elite: false,
            next_alt_fire: 0.,
            alt_fire_target: Vec3::ZERO,
            alt_fire_probe: None,
        }
    }
    /// CNPC_Combine::CanAltFireEnemy(true): elites only, not before m_flNextAltFireTime or
    /// m_flNextGrenadeCheck, at least one grenade (the shot does not spend it), an enemy
    /// that is the player or a player ally (the runtime's probe), and a clear ball path.
    pub fn can_alt_fire_enemy(&mut self) -> bool {
        if !self.elite
            || self.now < self.next_alt_fire
            || self.now < self.next_grenade_check
            || self.grenades < 1
        {
            return false;
        }
        match self.alt_fire_probe {
            Some(target) => {
                self.alt_fire_target = target;
                true
            }
            None => false,
        }
    }
    /// CNPC_Combine::DelayAltFireAttack (squadless DelaySquadAltFireAttack).
    pub fn delay_alt_fire(&mut self, delay: f64) {
        self.next_alt_fire = self.next_alt_fire.max(self.now + delay);
    }
    /// CNPC_Combine::CanGrenadeEnemy(bUseFreeKnowledge) -> CanThrowGrenade: no grenades or
    /// before m_flNextGrenadeCheck: no; out of 128..1024: no, check again in 1 s; moving:
    /// no; else CheckCanThrowGrenade: the toss velocity on success, next check in 1 s
    /// either way.
    pub fn can_grenade_enemy(&mut self, free_knowledge: bool) -> bool {
        if self.grenades < 1 || self.now < self.next_grenade_check {
            return false;
        }
        let probe = if free_knowledge {
            self.throw_probes.last_known
        } else {
            self.throw_probes.last_seen
        };
        let (outcome, ready) = match probe {
            None => return false,
            Some(ThrowProbe::OutOfRange) => {
                self.next_grenade_check = self.now + 1.;
                ("out_of_range", false)
            }
            Some(ThrowProbe::Moving) => ("moving", false),
            Some(ThrowProbe::Blocked { in_view }) => {
                self.next_grenade_check = self.now + 1.;
                (if in_view { "no_throw" } else { "no_toss" }, false)
            }
            Some(ThrowProbe::Ready { velocity, in_view }) => {
                self.next_grenade_check = self.now + 1.;
                self.toss_velocity = velocity;
                (if in_view { "ready_throw" } else { "ready_toss" }, true)
            }
        };
        self.grenade_outcomes.push(outcome);
        ready
    }
    /// random->RandomInt(min, max).
    fn random_int(&mut self, min: u32, max: u32) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        let span = u64::from(max.saturating_sub(min)) + 1;
        min + ((self.seed >> 11) % span) as u32
    }
    /// random->RandomInt(0, 1) == 1.
    fn coin(&mut self) -> bool {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        self.seed >> 63 == 1
    }
    /// NPC_TranslateActivity's ACT_IDLE -> ACT_IDLE_ANGRY (CNPC_Combine: standing in
    /// combat or alert, or while the assault behavior runs; CNPC_PlayerCompanion: combat or
    /// alert within 3 s of the last attack). Readiness translation is not modeled.
    pub fn idle_angry(&self, state: NpcState, now: f64) -> bool {
        let alert = matches!(state, NpcState::Combat | NpcState::Alert);
        match self.kind {
            Kind::Combine => alert || self.running_assault,
            Kind::Companion { .. } => alert && now - self.last_attack < 3.,
        }
    }
    fn select(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        let was_assault = std::mem::take(&mut self.running_assault);
        if was_assault {
            if let Some(assault) = &mut self.assault {
                assault.on_schedule_change(npc);
            }
        }
        let schedule = match self.kind {
            Kind::Companion { .. } => self.select_companion(npc, ctx),
            Kind::Combine => self.select_combine(npc, ctx),
        };
        if was_assault && !self.running_assault {
            if let Some(assault) = &mut self.assault {
                assault.end_selection();
            }
        }
        schedule
    }
    /// BehaviorSelectSchedule: the assault behavior when it can select (its base
    /// selection is CAI_BaseNPC's).
    fn behavior_select(&mut self, npc: &mut AiNpc, ctx: &Context) -> Option<String> {
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
    /// CNPC_PlayerCompanion::SelectSchedule (CNPC_Citizen adds nothing reachable here:
    /// no trains, heals, item retrieval, inspections or patrols).
    fn select_companion(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        // SelectScheduleDanger.
        if npc.conditions.has(COND_HEAR_DANGER) && ctx.best_sound.is_some() {
            return "SCHED_TAKE_COVER_FROM_BEST_SOUND".into();
        }
        if npc.conditions.has(COND_HEAR_MOVE_AWAY) {
            return "SCHED_MOVE_AWAY".into();
        }
        // SelectSchedulePriorityAction: player pushing (COND_PLAYER_PUSHING) is not
        // gathered yet.
        if let Some(schedule) = self.behavior_select(npc, ctx) {
            return schedule;
        }
        // SelectScheduleCombat (citizens: the manhack melee case needs manhacks).
        if npc.state == NpcState::Combat
            && (npc.conditions.has(COND_NO_PRIMARY_AMMO)
                || npc.conditions.has(COND_LOW_PRIMARY_AMMO))
        {
            return "SCHED_HIDE_AND_RELOAD".into();
        }
        base_select_schedule(npc, ctx, self.armed)
    }
    /// CNPC_Combine::SelectSchedule without a squad.
    fn select_combine(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        // CNPC_Combine::SelectSchedule: burning comes first (after rappelling).
        if npc.conditions.has(COND_COMBINE_ON_FIRE) {
            return "SCHED_COMBINE_BURNING_STAND".into();
        }
        if npc.state != NpcState::Script {
            if npc.conditions.has(COND_HEAR_MOVE_AWAY) {
                return "SCHED_MOVE_AWAY".into();
            }
            if !matches!(npc.state, NpcState::Dead | NpcState::Prone) {
                if npc.conditions.has(COND_HEAR_PHYSICS_DANGER) {
                    return "SCHED_FLINCH_PHYSICS".into();
                }
                // A danger sound: "COMBINE_DANGER" and cover (the approaching-danger
                // face-ideal-yaw case needs danger-approach sound contexts).
                if npc.conditions.has(COND_HEAR_DANGER) && ctx.best_sound.is_some() {
                    self.requests.push(CopRequest::Sentence("COMBINE_DANGER"));
                    return "SCHED_TAKE_COVER_FROM_BEST_SOUND".into();
                }
            }
            if let Some(schedule) = self.behavior_select(npc, ctx) {
                return schedule;
            }
        }
        match npc.state {
            NpcState::Idle | NpcState::Alert => {
                // Hurt by an unseen danger: SCHED_TAKE_COVER_FROM_ORIGIN (the danger
                // memory's facing test is not modeled). Combat sounds investigate only as
                // a squad's nearest member; patrols need m_bShouldPatrol.
                if (npc.conditions.has(COND_LIGHT_DAMAGE) || npc.conditions.has(COND_HEAVY_DAMAGE))
                    && npc.enemy.is_none()
                {
                    return "SCHED_TAKE_COVER_FROM_ORIGIN".into();
                }
            }
            NpcState::Combat => {
                if let Some(schedule) = self.combine_combat(npc, ctx) {
                    return schedule;
                }
            }
            _ => {}
        }
        base_select_schedule(npc, ctx, self.armed)
    }
    /// CNPC_Combine::SelectCombatSchedule without a squad (the new-enemy block is
    /// squad-only; no grenades, kick, crouch or physcannon charge).
    fn combine_combat(&mut self, npc: &mut AiNpc, _ctx: &Context) -> Option<String> {
        let c = npc.conditions;
        if c.has(COND_ENEMY_DEAD) {
            return None;
        }
        if (c.has(COND_NO_PRIMARY_AMMO) || c.has(COND_LOW_PRIMARY_AMMO))
            && !c.has(COND_CAN_MELEE_ATTACK1)
        {
            return Some("SCHED_HIDE_AND_RELOAD".into());
        }
        // A standing soldier hit runs for cover (the 50% crouch needs crouching).
        if c.has(COND_LIGHT_DAMAGE) && npc.enemy.is_some() {
            return Some("SCHED_TAKE_COVER_FROM_ENEMY".into());
        }
        // SelectScheduleAttack.
        if c.has(COND_CAN_MELEE_ATTACK1) {
            return Some("SCHED_MELEE_ATTACK1".into());
        }
        if c.has(COND_CAN_RANGE_ATTACK1) {
            return Some("SCHED_RANGE_ATTACK1".into());
        }
        // The flush: an enemy out of sight for at most 3 s and within 256 units of where it
        // was last seen gets a grenade (CanGrenadeEnemy(false): its last seen position).
        if !c.has(COND_SEE_ENEMY) && self.flush_window && self.can_grenade_enemy(false) {
            return Some("SCHED_RANGE_ATTACK2".into());
        }
        if c.has(COND_ENEMY_OCCLUDED) {
            // Slots are free without a squad: charge in and break the enemy's cover.
            return Some("SCHED_ESTABLISH_LINE_OF_FIRE".into());
        }
        if c.has(COND_SEE_ENEMY) && !c.has(COND_CAN_RANGE_ATTACK1) {
            if c.has(COND_TOO_FAR_TO_ATTACK) {
                return Some("SCHED_COMBINE_PRESS_ATTACK".into());
            }
            return Some("SCHED_COMBINE_ASSAULT".into());
        }
        None
    }
    /// CNPC_Combine::TranslateSchedule without a squad.
    fn translate_combine(&mut self, name: &str, npc: &AiNpc) -> String {
        let c = npc.conditions;
        match name {
            // The 50% grenade cover needs COND_CAN_RANGE_ATTACK2 (no grenades yet).
            "SCHED_TAKE_COVER_FROM_ENEMY" => "SCHED_COMBINE_TAKE_COVER1".into(),
            "SCHED_TAKE_COVER_FROM_BEST_SOUND" => "SCHED_COMBINE_TAKE_COVER_FROM_BEST_SOUND".into(),
            "SCHED_COMBINE_TAKECOVER_FAILED" => {
                if c.has(COND_CAN_RANGE_ATTACK1) {
                    self.translate_combine("SCHED_RANGE_ATTACK1", npc)
                } else {
                    self.translate_combine("SCHED_FAIL", npc)
                }
            }
            "SCHED_FAIL_ESTABLISH_LINE_OF_FIRE" => {
                if c.has(COND_SEE_ENEMY) {
                    self.translate_combine("SCHED_TAKE_COVER_FROM_ENEMY", npc)
                } else if !self.assault.as_ref().is_some_and(|a| a.active()) {
                    self.translate_combine("SCHED_COMBINE_PATROL", npc)
                } else {
                    name.into()
                }
            }
            "SCHED_COMBINE_ASSAULT" => "SCHED_COMBINE_ASSAULT".into(),
            // An elite that could put a combine ball at the enemy's last known position.
            "SCHED_ESTABLISH_LINE_OF_FIRE" => {
                if self.can_alt_fire_enemy() {
                    "SCHED_COMBINE_AR2_ALTFIRE".into()
                } else {
                    "SCHED_COMBINE_ESTABLISH_LINE_OF_FIRE".into()
                }
            }
            // "If I COULD throw a grenade and I need to reload, 20% chance I'll throw a
            // grenade before I hide to reload" (CanGrenadeEnemy() at the last known position).
            "SCHED_HIDE_AND_RELOAD" => {
                if self.can_grenade_enemy(true) && self.random_int(0, 100) < 20 {
                    "SCHED_COMBINE_GRENADE_AND_RELOAD".into()
                } else {
                    "SCHED_COMBINE_HIDE_AND_RELOAD".into()
                }
            }
            // The NPC weapons have no bits_CAP_WEAPON_RANGE_ATTACK2: the innate toss.
            "SCHED_RANGE_ATTACK2" => "SCHED_COMBINE_RANGE_ATTACK2".into(),
            "SCHED_RANGE_ATTACK1" => {
                if c.has(COND_NO_PRIMARY_AMMO) || c.has(COND_LOW_PRIMARY_AMMO) {
                    self.translate_combine("SCHED_HIDE_AND_RELOAD", npc)
                } else if self.can_alt_fire_enemy() {
                    "SCHED_COMBINE_AR2_ALTFIRE".into()
                } else {
                    "SCHED_COMBINE_RANGE_ATTACK1".into()
                }
            }
            "SCHED_FAIL" => {
                if npc.enemy.is_some() {
                    "SCHED_COMBINE_COMBAT_FAIL".into()
                } else {
                    "SCHED_FAIL".into()
                }
            }
            "SCHED_COMBINE_PATROL" => {
                if npc.enemy.is_some() {
                    "SCHED_COMBINE_PATROL_ENEMY".into()
                } else {
                    "SCHED_COMBINE_PATROL".into()
                }
            }
            other => other.into(),
        }
    }
    /// CNPC_PlayerCompanion::TranslateSchedule (+ CNPC_Citizen's, which only matter with
    /// a command goal, a dead player or an RPG).
    fn translate_companion(&self, name: &str, npc: &AiNpc, ctx: &Context) -> String {
        // CNPC_Citizen: an RPG fires its own schedule unless the enemy is within
        // RPG_SAFE_DISTANCE (EXPLOSION_RADIUS 200 + 64) of the player (so never at the
        // player himself); mortars and striders (squad slots) are not modeled.
        if name == "SCHED_RANGE_ATTACK1"
            && self.rpg
            && matches!(self.kind, Kind::Companion { citizen: true })
        {
            let near_player = match (ctx.enemy, ctx.player) {
                (Some((_, enemy)), Some(player)) => enemy.distance(player) < RPG_SAFE_DISTANCE,
                _ => false,
            };
            return if near_player {
                "SCHED_STANDOFF".into()
            } else {
                "SCHED_CITIZEN_RANGE_ATTACK1_RPG".into()
            };
        }
        match name {
            "SCHED_COWER" => "SCHED_PC_COWER".into(),
            "SCHED_TAKE_COVER_FROM_BEST_SOUND" => {
                "SCHED_PC_MOVE_TOWARDS_COVER_FROM_BEST_SOUND".into()
            }
            "SCHED_FLEE_FROM_BEST_SOUND" => "SCHED_PC_FLEE_FROM_BEST_SOUND".into(),
            // A rest in the shot regulator: stand off (no squad slots to fail).
            "SCHED_RANGE_ATTACK1"
                if npc
                    .shot_regulator
                    .as_ref()
                    .is_some_and(|r| r.is_in_rest_interval(ctx.now)) =>
            {
                "SCHED_STANDOFF".into()
            }
            other => other.into(),
        }
    }
    /// CNPC_Citizen::RunTask TASK_CIT_RPG_AUGER: guide the missile until it is gone,
    /// pulling the laser up to 90 units per think toward the enemy (or 90 units up once
    /// the enemy came within RPG_SAFE_DISTANCE of the player), with 18 sin(t) imprecision.
    /// The SDK starts guiding on the first think after the launch event.
    fn rpg_auger(&mut self, ctx: &Context) -> TaskStatus {
        let done = |h: &mut Self, status| {
            h.rpg_laser = None;
            status
        };
        if !self.rpg {
            return done(self, TaskStatus::Failed);
        }
        if !self.missile_out {
            // Before the launch event: wait; after it, the missile's end ends the task.
            if self.rpg_launched || ctx.now >= self.rpg_wait_until + 2. {
                return done(self, TaskStatus::Complete);
            }
            return TaskStatus::Running;
        }
        let Some((_, enemy)) = ctx.enemy else {
            return done(self, TaskStatus::Failed);
        };
        if !self.rpg_launched {
            self.rpg_launched = true;
            // GetActualShootPosition: SetNPCLaserPosition(vecTarget).
            self.rpg_laser_base = ctx.enemy_eye.unwrap_or(enemy);
        }
        let mut laser = self.rpg_laser_base;
        if !self.rpg_avoid_player {
            if ctx
                .player
                .is_some_and(|p| enemy.distance(p) < RPG_SAFE_DISTANCE)
            {
                self.rpg_avoid_player = true;
            } else {
                let to = enemy - laser;
                let length = to.length();
                laser += to / length.max(1e-6) * length.min(90.);
            }
        }
        if self.rpg_avoid_player {
            laser.z += 90.;
        }
        self.rpg_laser_base = laser;
        let imprecision = 18. * (ctx.now as f32).sin();
        self.rpg_laser = Some(laser + Vec3::splat(imprecision));
        TaskStatus::Running
    }
}
impl Behavior for Humanoid {
    fn select_schedule(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        self.now = ctx.now;
        self.select(npc, ctx)
    }
    fn select_fail_schedule(&mut self, npc: &AiNpc, failed: &str) -> String {
        if self.running_assault && npc.fail_schedule.is_none() {
            return "SCHED_FAIL".into();
        }
        // CNPC_Citizen::SelectFailSchedule: a failed fallback or weapon-range move
        // shoots half the time when it can, else stands off.
        if matches!(self.kind, Kind::Companion { citizen: true })
            && matches!(
                failed,
                "SCHED_ESTABLISH_LINE_OF_FIRE_FALLBACK" | "SCHED_MOVE_TO_WEAPON_RANGE"
            )
        {
            return if self.armed
                && npc.conditions.has(COND_SEE_ENEMY)
                && !npc.conditions.has(COND_NO_PRIMARY_AMMO)
                && self.coin()
            {
                "SCHED_RANGE_ATTACK1".into()
            } else {
                "SCHED_STANDOFF".into()
            };
        }
        npc.fail_schedule
            .clone()
            .unwrap_or_else(|| "SCHED_FAIL".into())
    }
    fn translate_schedule(&mut self, name: &str, npc: &AiNpc, ctx: &Context) -> String {
        let name = match self.kind {
            Kind::Combine => self.translate_combine(name, npc),
            Kind::Companion { .. } => self.translate_companion(name, npc, ctx),
        };
        if self.running_assault {
            if let Some(t) = self.assault.as_ref().and_then(|a| a.translate(&name)) {
                return t.into();
            }
        }
        name
    }
    fn custom_interrupts(&self, npc: &AiNpc) -> Conditions {
        let mut c = Conditions::default();
        // CNPC_Combine::BuildScheduleTestBits: fire interrupts all but the burning stand.
        if self.kind == Kind::Combine
            && npc.schedule.as_deref() != Some("SCHED_COMBINE_BURNING_STAND")
        {
            c.set(COND_COMBINE_ON_FIRE);
        }
        if self.running_assault {
            if let Some(assault) = &self.assault {
                c = c.union(assault.custom_interrupts(npc, self.now));
            }
        }
        c
    }
    fn start_task(
        &mut self,
        npc: &mut AiNpc,
        task: &str,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        if self.running_assault {
            if let Some(assault) = &mut self.assault {
                if let Some(status) = assault.start_task(npc, task, arg, ctx, motor) {
                    return Some(status);
                }
            }
        }
        Some(match task {
            // Squad-only (m_flNextAttack with more than two squad members) / crouch state /
            // the squad hand signal.
            "TASK_COMBINE_IGNORE_ATTACKS"
            | "TASK_COMBINE_SET_STANDING"
            | "TASK_COMBINE_SIGNAL_BEST_SOUND"
            | "TASK_COMBINE_DEFER_SQUAD_GRENADES" => TaskStatus::Complete,
            // m_flPlaybackRate = U(0.8, 1.2): the sequence rate is not modeled.
            "TASK_RANDOMIZE_FRAMERATE" => TaskStatus::Complete,
            "TASK_COMBINE_DIE_INSTANTLY" => {
                self.requests
                    .push(CopRequest::DieInstantly(crate::player_damage::DMG_BURN));
                TaskStatus::Complete
            }
            "TASK_COMBINE_PLAY_SEQUENCE_FACE_ALTFIRE_TARGET" => {
                let activity = match arg {
                    TaskArg::Activity(a) => a.clone(),
                    _ => "ACT_COMBINE_AR2_ALTFIRE".into(),
                };
                npc.activity = activity.clone();
                motor.set_activity(&activity);
                motor.face(super::schedule::Goal::Position(self.alt_fire_target));
                TaskStatus::Running
            }
            // Turn toward a point 64 toss-velocities along the throw.
            "TASK_COMBINE_FACE_TOSS_DIR" => {
                if motor.face(super::schedule::Goal::Position(
                    ctx.origin + self.toss_velocity * 64.,
                )) {
                    TaskStatus::Complete
                } else {
                    TaskStatus::Running
                }
            }
            "TASK_AISCRIPTED_GET_PATH_TO_GOAL" => match self.script_goal.take() {
                Some((goal, run)) if motor.path_to(goal, run) => {
                    let activity = if run { "ACT_RUN" } else { "ACT_WALK" };
                    npc.activity = activity.into();
                    motor.set_activity(activity);
                    TaskStatus::Complete
                }
                _ => TaskStatus::Failed,
            },
            // CNPC_Combine::StartTask: a burst of GetRandomBurst() shots, the first now;
            // no shot regulator.
            "TASK_RANGE_ATTACK1" if self.kind == Kind::Combine => {
                let (min, max, rate) = ctx.weapon_burst.unwrap_or((1, 1, 0.1));
                self.shots = self.random_int(min, max) as i32;
                self.shot_delay = rate;
                self.next_attack = ctx.now + f64::from(rate) - 0.1;
                self.last_attack = ctx.now;
                npc.activity = "ACT_RANGE_ATTACK1".into();
                motor.set_activity("ACT_RANGE_ATTACK1");
                TaskStatus::Running
            }
            "TASK_RANGE_ATTACK1" => {
                self.last_attack = ctx.now;
                return None;
            }
            // CNPC_Citizen::StartTask: SetWait(arg), m_bRPGAvoidPlayer = false.
            // CNPC_Combine::StartTask: announce a primary attack (COMBINE_ANNOUNCE, HIGH)
            // when the player enemy does not face me and I have not attacked for 3 s,
            // then wait 2 s in ACT_IDLE; a grenade (2) always says COMBINE_THROW_GRENADE
            // (MEDIUM) and waits 2 s. Crouching (ACT_COWER) is not modeled.
            "TASK_ANNOUNCE_ATTACK" if self.kind == Kind::Combine => {
                let grenade = matches!(arg, TaskArg::Number(n) if *n as i32 != 1);
                let announce = grenade
                    || (ctx
                        .enemy
                        .is_some_and(|(e, _)| e == super::metropolice::PLAYER)
                        && ctx.enemy_sees_me == Some(false)
                        && ctx.now - self.last_attack > 3.);
                if !announce {
                    return Some(TaskStatus::Complete);
                }
                if grenade {
                    self.requests
                        .push(CopRequest::Sentence("COMBINE_THROW_GRENADE"));
                } else {
                    self.last_attack = ctx.now;
                    self.requests.push(CopRequest::Sentence("COMBINE_ANNOUNCE"));
                }
                self.announce_until = ctx.now + 2.;
                npc.activity = "ACT_IDLE".into();
                motor.set_activity("ACT_IDLE");
                TaskStatus::Running
            }
            "TASK_CIT_RPG_AUGER" => {
                let seconds = match arg {
                    TaskArg::Number(n) => *n,
                    _ => 0.,
                };
                self.rpg_wait_until = ctx.now + f64::from(seconds);
                self.rpg_avoid_player = false;
                self.rpg_launched = false;
                TaskStatus::Running
            }
            _ => return None,
        })
    }
    fn run_task(
        &mut self,
        npc: &mut AiNpc,
        task: &str,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        if self.running_assault {
            if let Some(assault) = &mut self.assault {
                if let Some(status) = assault.run_task(npc, task, arg, ctx, motor) {
                    return Some(status);
                }
            }
        }
        if task == "TASK_CIT_RPG_AUGER" {
            return Some(self.rpg_auger(ctx));
        }
        // Stop waiting once the enemy faces me or is gone, or after the 2 s.
        if task == "TASK_ANNOUNCE_ATTACK" && self.kind == Kind::Combine {
            return Some(
                if ctx.enemy.is_none()
                    || ctx.enemy_sees_me != Some(false)
                    || ctx.now >= self.announce_until
                {
                    TaskStatus::Complete
                } else {
                    TaskStatus::Running
                },
            );
        }
        if task == "TASK_COMBINE_PLAY_SEQUENCE_FACE_ALTFIRE_TARGET" {
            motor.face(super::schedule::Goal::Position(self.alt_fire_target));
            return Some(if motor.activity_finished() {
                TaskStatus::Complete
            } else {
                TaskStatus::Running
            });
        }
        // CNPC_Combine::RunTask: keep facing the LKP; once the shot's activity ends and
        // m_flNextAttack passed, shoot again until the burst is spent.
        if task == "TASK_RANGE_ATTACK1" && self.kind == Kind::Combine {
            if let Some(lkp) = ctx.enemy_lkp.or(ctx.enemy.map(|e| e.1)) {
                motor.face(super::schedule::Goal::Position(lkp));
            }
            if ctx.now >= self.next_attack && motor.activity_finished() {
                self.shots -= 1;
                if self.shots > 0 {
                    npc.activity = "ACT_RANGE_ATTACK1".into();
                    motor.set_activity("ACT_RANGE_ATTACK1");
                    self.last_attack = ctx.now;
                    self.next_attack = ctx.now + f64::from(self.shot_delay) - 0.1;
                } else {
                    return Some(TaskStatus::Complete);
                }
            }
            return Some(TaskStatus::Running);
        }
        if task == "TASK_RANGE_ATTACK1" {
            self.last_attack = ctx.now;
        }
        if task == "TASK_COMBINE_FACE_TOSS_DIR" {
            return Some(
                if motor.face(super::schedule::Goal::Position(
                    ctx.origin + self.toss_velocity * 64.,
                )) {
                    TaskStatus::Complete
                } else {
                    TaskStatus::Running
                },
            );
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::schedule::tests::FakeMotor;
    use super::*;
    use crate::npc_weapons::{builtin, RegulatorRule, ShotRegulator};

    fn setup(kind: Kind) -> (Humanoid, AiNpc, Schedules) {
        let mut schedules = Schedules::default();
        add_schedules(&mut schedules);
        let mut npc = AiNpc::new(1);
        npc.state = NpcState::Combat;
        npc.ideal_state = NpcState::Combat;
        npc.enemy = Some(2);
        npc.enemies.update(Some(2), Vec3::X * 300., 0., true, 0.);
        let spec = builtin("weapon_smg1").and_then(|d| d.fire).expect("smg1");
        npc.shot_regulator = Some(ShotRegulator::for_weapon(
            &spec,
            RegulatorRule::Weapon,
            0.,
            7,
        ));
        (Humanoid::new(kind, true, 11), npc, schedules)
    }
    fn ctx(now: f64) -> Context {
        Context {
            now,
            enemy: Some((2, Vec3::X * 300.)),
            enemy_eye: Some(Vec3::new(300., 0., 64.)),
            weapon_range: Some((0., 1400.)),
            random: 0.5,
            ..Default::default()
        }
    }
    fn think(h: &mut Humanoid, npc: &mut AiNpc, s: &Schedules, m: &mut FakeMotor, c: &[Cond]) {
        for &cond in c {
            npc.conditions.set(cond);
        }
        npc.maintain(s, h, &ctx(5.), m);
    }

    #[test]
    fn a_burning_soldier_flails_then_dies_of_burns() {
        let (mut h, mut npc, s) = setup(Kind::Combine);
        let mut m = FakeMotor::default();
        think(&mut h, &mut npc, &s, &mut m, &[COND_COMBINE_ON_FIRE]);
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_COMBINE_BURNING_STAND"));
        assert!(m.activities.iter().any(|a| a == "ACT_COMBINE_BUGBAIT"));
        for i in 1..=60 {
            npc.maintain(&s, &mut h, &ctx(5. + f64::from(i) * 0.1), &mut m);
        }
        assert!(h
            .requests
            .contains(&CopRequest::DieInstantly(crate::player_damage::DMG_BURN)));
        // Fire interrupts other schedules.
        npc.schedule = Some("SCHED_RANGE_ATTACK1".into());
        assert!(h.custom_interrupts(&npc).has(COND_COMBINE_ON_FIRE));
    }

    #[test]
    fn combine_announce_a_primary_attack_on_a_player_who_does_not_face_them() {
        use super::super::metropolice::PLAYER;
        let (mut h, mut npc, _) = setup(Kind::Combine);
        let mut m = FakeMotor::default();
        let mut c = ctx(10.);
        c.enemy = Some((PLAYER, Vec3::X * 300.));
        let primary = TaskArg::Number(1.);
        // The player faces the soldier: no announcement.
        c.enemy_sees_me = Some(true);
        assert_eq!(
            h.start_task(&mut npc, "TASK_ANNOUNCE_ATTACK", &primary, &c, &mut m),
            Some(TaskStatus::Complete)
        );
        // Facing away and no attack for 3 s: COMBINE_ANNOUNCE, then wait up to 2 s.
        c.enemy_sees_me = Some(false);
        assert_eq!(
            h.start_task(&mut npc, "TASK_ANNOUNCE_ATTACK", &primary, &c, &mut m),
            Some(TaskStatus::Running)
        );
        assert!(h
            .requests
            .contains(&CopRequest::Sentence("COMBINE_ANNOUNCE")));
        assert_eq!(npc.activity, "ACT_IDLE");
        c.now = 11.;
        assert_eq!(
            h.run_task(&mut npc, "TASK_ANNOUNCE_ATTACK", &primary, &c, &mut m),
            Some(TaskStatus::Running)
        );
        // Turning toward him ends the wait early.
        c.enemy_sees_me = Some(true);
        assert_eq!(
            h.run_task(&mut npc, "TASK_ANNOUNCE_ATTACK", &primary, &c, &mut m),
            Some(TaskStatus::Complete)
        );
        // Within 3 s of the last attack (the announcement counts) it is skipped.
        h.requests.clear();
        c.enemy_sees_me = Some(false);
        c.now = 12.;
        assert_eq!(
            h.start_task(&mut npc, "TASK_ANNOUNCE_ATTACK", &primary, &c, &mut m),
            Some(TaskStatus::Complete)
        );
        // NPC enemies are never announced; a grenade always is.
        c.now = 20.;
        c.enemy = Some((2, Vec3::X * 300.));
        assert_eq!(
            h.start_task(&mut npc, "TASK_ANNOUNCE_ATTACK", &primary, &c, &mut m),
            Some(TaskStatus::Complete)
        );
        assert_eq!(
            h.start_task(
                &mut npc,
                "TASK_ANNOUNCE_ATTACK",
                &TaskArg::Number(2.),
                &c,
                &mut m
            ),
            Some(TaskStatus::Running)
        );
        assert_eq!(
            h.requests,
            vec![CopRequest::Sentence("COMBINE_THROW_GRENADE")]
        );
        c.now = 22.5;
        assert_eq!(
            h.run_task(
                &mut npc,
                "TASK_ANNOUNCE_ATTACK",
                &TaskArg::Number(2.),
                &c,
                &mut m
            ),
            Some(TaskStatus::Complete)
        );
    }
    #[test]
    fn rpg_citizens_stand_off_near_the_player_and_guide_their_missile() {
        let (mut h, npc, _) = setup(Kind::Companion { citizen: true });
        h.rpg = true;
        let mut c = ctx(5.);
        // The enemy is the player himself (within RPG_SAFE_DISTANCE): no rocket.
        c.player = Some(Vec3::X * 300.);
        assert_eq!(
            h.translate_schedule("SCHED_RANGE_ATTACK1", &npc, &c),
            "SCHED_STANDOFF"
        );
        c.player = Some(Vec3::new(-400., 0., 0.));
        assert_eq!(
            h.translate_schedule("SCHED_RANGE_ATTACK1", &npc, &c),
            "SCHED_CITIZEN_RANGE_ATTACK1_RPG"
        );
        // TASK_CIT_RPG_AUGER: waits for the launch, guides while the missile flies (the
        // dot moves at most 90 units per think toward the enemy), ends with the missile.
        let mut m = FakeMotor::default();
        let mut npc = npc;
        h.start_task(
            &mut npc,
            "TASK_CIT_RPG_AUGER",
            &TaskArg::Number(1.),
            &c,
            &mut m,
        );
        assert_eq!(h.rpg_auger(&c), TaskStatus::Running);
        assert_eq!(h.rpg_laser, None);
        h.missile_out = true;
        c.enemy = Some((2, Vec3::new(300., 500., 0.)));
        assert_eq!(h.rpg_auger(&c), TaskStatus::Running);
        // The dot starts at the shoot target (the enemy eye) and is pulled 90 units.
        let first = h.rpg_laser_base;
        assert!(
            (first.distance(Vec3::new(300., 0., 64.)) - 90.).abs() < 1e-3,
            "{first}"
        );
        assert!(h.rpg_laser.is_some());
        h.rpg_auger(&c);
        assert!((h.rpg_laser_base - first).length() <= 90. + 1e-3);
        h.missile_out = false;
        assert_eq!(h.rpg_auger(&c), TaskStatus::Complete);
        assert_eq!(h.rpg_laser, None);
    }

    #[test]
    fn elites_alt_fire_only_with_a_grenade_a_clear_path_and_after_the_delay() {
        let (mut h, npc, _) = setup(Kind::Combine);
        let c = ctx(5.);
        h.now = 5.;
        h.alt_fire_probe = Some(Vec3::new(300., 0., 48.));
        // Not an elite.
        assert_eq!(
            h.translate_schedule("SCHED_RANGE_ATTACK1", &npc, &c),
            "SCHED_COMBINE_RANGE_ATTACK1"
        );
        h.elite = true;
        // No grenades: CanAltFireEnemy refuses (designers disable balls with 0).
        assert_eq!(
            h.translate_schedule("SCHED_RANGE_ATTACK1", &npc, &c),
            "SCHED_COMBINE_RANGE_ATTACK1"
        );
        h.grenades = 1;
        assert_eq!(
            h.translate_schedule("SCHED_RANGE_ATTACK1", &npc, &c),
            "SCHED_COMBINE_AR2_ALTFIRE"
        );
        assert_eq!(h.alt_fire_target, Vec3::new(300., 0., 48.));
        assert_eq!(
            h.translate_schedule("SCHED_ESTABLISH_LINE_OF_FIRE", &npc, &c),
            "SCHED_COMBINE_AR2_ALTFIRE"
        );
        // After a shot: 10 s of plain attacks; a blocked path never alt-fires.
        h.delay_alt_fire(10.);
        assert_eq!(
            h.translate_schedule("SCHED_RANGE_ATTACK1", &npc, &c),
            "SCHED_COMBINE_RANGE_ATTACK1"
        );
        h.now = 15.1;
        h.alt_fire_probe = None;
        assert_eq!(
            h.translate_schedule("SCHED_RANGE_ATTACK1", &npc, &c),
            "SCHED_COMBINE_RANGE_ATTACK1"
        );
    }

    #[test]
    fn combine_soldier_shoots_a_visible_enemy_and_hunts_an_occluded_one() {
        let (mut h, mut npc, s) = setup(Kind::Combine);
        let mut m = FakeMotor::default();
        think(
            &mut h,
            &mut npc,
            &s,
            &mut m,
            &[COND_SEE_ENEMY, COND_CAN_RANGE_ATTACK1],
        );
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_COMBINE_RANGE_ATTACK1"));
        // Occluded: ESTABLISH_LINE_OF_FIRE -> the combine variant; the fake motor finds no
        // line-of-fire node, so the task fails to SCHED_FAIL_ESTABLISH_LINE_OF_FIRE, which
        // (enemy unseen, no assault) translates to a patrol with an enemy.
        let (mut h, mut npc, s) = setup(Kind::Combine);
        think(&mut h, &mut npc, &s, &mut m, &[COND_ENEMY_OCCLUDED]);
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_COMBINE_PATROL_ENEMY"));
        assert!(npc
            .last_failed_task
            .as_deref()
            .is_some_and(|t| t.ends_with("TASK_GET_PATH_TO_ENEMY_LKP_LOS")));
        // Seen but out of range: press the attack.
        let (mut h, mut npc, s) = setup(Kind::Combine);
        think(
            &mut h,
            &mut npc,
            &s,
            &mut m,
            &[COND_SEE_ENEMY, COND_TOO_FAR_TO_ATTACK],
        );
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_COMBINE_PRESS_ATTACK"));
        assert_eq!(m.path, Some(Vec3::X * 300.));
    }
    #[test]
    fn combine_soldier_takes_cover_when_hit_and_hides_to_reload() {
        let (mut h, mut npc, s) = setup(Kind::Combine);
        let mut m = FakeMotor::default();
        think(
            &mut h,
            &mut npc,
            &s,
            &mut m,
            &[COND_SEE_ENEMY, COND_CAN_RANGE_ATTACK1, COND_LIGHT_DAMAGE],
        );
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_COMBINE_TAKE_COVER1"));
        // After its 0.2 s wait: no cover node (fake motor) -> TAKECOVER_FAILED, which
        // shoots back while it can.
        npc.conditions.set(COND_SEE_ENEMY);
        npc.conditions.set(COND_CAN_RANGE_ATTACK1);
        npc.maintain(&s, &mut h, &ctx(5.3), &mut m);
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_COMBINE_RANGE_ATTACK1"));
        assert!(npc
            .last_failed_task
            .as_deref()
            .is_some_and(|t| t.starts_with("SCHED_COMBINE_TAKE_COVER1/")));
        let (mut h, mut npc, s) = setup(Kind::Combine);
        think(
            &mut h,
            &mut npc,
            &s,
            &mut m,
            &[COND_SEE_ENEMY, COND_LOW_PRIMARY_AMMO],
        );
        // HIDE_AND_RELOAD -> no cover -> its fail schedule SCHED_RELOAD.
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_RELOAD"));
    }
    #[test]
    fn companion_attacks_reloads_and_faces_the_enemy_during_rests() {
        let (mut h, mut npc, s) = setup(Kind::Companion { citizen: true });
        let mut m = FakeMotor::default();
        think(
            &mut h,
            &mut npc,
            &s,
            &mut m,
            &[COND_SEE_ENEMY, COND_CAN_RANGE_ATTACK1],
        );
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_RANGE_ATTACK1"));
        // The base CAI_BaseNPC::SelectCombatSchedule faces the enemy during a rest.
        let (mut h, mut npc, s) = setup(Kind::Companion { citizen: true });
        npc.shot_regulator.as_mut().unwrap().reset(false, 5.);
        think(
            &mut h,
            &mut npc,
            &s,
            &mut m,
            &[COND_SEE_ENEMY, COND_CAN_RANGE_ATTACK1],
        );
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_COMBAT_FACE"));
        // Low clip: hide and reload (no cover found: reload in place).
        let (mut h, mut npc, s) = setup(Kind::Companion { citizen: true });
        think(
            &mut h,
            &mut npc,
            &s,
            &mut m,
            &[COND_SEE_ENEMY, COND_LOW_PRIMARY_AMMO],
        );
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_RELOAD"));
        // A danger sound: the companion's short cover run.
        let (mut h, mut npc, _) = setup(Kind::Companion { citizen: false });
        let c = Context {
            best_sound: Some(Vec3::Y * 100.),
            ..ctx(5.)
        };
        npc.conditions.set(COND_HEAR_DANGER);
        let schedule = h.select_schedule(&mut npc, &c);
        assert_eq!(
            h.translate_schedule(&schedule, &npc, &c),
            "SCHED_PC_MOVE_TOWARDS_COVER_FROM_BEST_SOUND"
        );
    }
    #[test]
    fn companion_regulator_doubles_bursts_up_close() {
        let spec = builtin("weapon_smg1").and_then(|d| d.fire).expect("smg1");
        let mut r = ShotRegulator::for_weapon(&spec, RegulatorRule::PlayerCompanion, 0., 3);
        let (min_rest, burst) = (r.min_rest, r.burst_remaining);
        r.on_update(Some(100.));
        assert_eq!(r.burst_remaining, burst * 2);
        assert!((r.min_rest - min_rest * 0.6).abs() < 1e-6);
        let mut far = ShotRegulator::for_weapon(&spec, RegulatorRule::PlayerCompanion, 0., 3);
        far.on_update(Some(400.));
        assert_eq!(far.burst_remaining, burst);
    }
    #[test]
    fn angry_idle_follows_the_class_rule() {
        let mut h = Humanoid::new(Kind::Combine, true, 1);
        assert!(h.idle_angry(NpcState::Alert, 0.));
        assert!(!h.idle_angry(NpcState::Idle, 0.));
        h.kind = Kind::Companion { citizen: true };
        assert!(!h.idle_angry(NpcState::Combat, 10.));
        h.last_attack = 8.;
        assert!(h.idle_angry(NpcState::Combat, 10.));
    }
    #[test]
    fn combine_range_attack_fires_its_own_burst_without_a_regulator() {
        // CNPC_Combine::StartTask/RunTask TASK_RANGE_ATTACK1 with an SMG1 (burst 2-5, fire
        // rate 0.075): a shot now, another each time the activity ends, then complete.
        for seed in 1..40 {
            let mut h = Humanoid::new(Kind::Combine, true, seed);
            let mut npc = AiNpc::new(1);
            let mut motor = FakeMotor::default();
            let ctx = |now: f64| Context {
                now,
                enemy: Some((2, Vec3::X * 300.)),
                weapon_burst: Some((2, 5, 0.075)),
                ..Default::default()
            };
            let arg = TaskArg::Number(0.);
            assert_eq!(
                h.start_task(&mut npc, "TASK_RANGE_ATTACK1", &arg, &ctx(0.), &mut motor),
                Some(TaskStatus::Running)
            );
            let burst = h.shots;
            assert!((2..=5).contains(&burst), "{burst}");
            motor.finished = true;
            let mut now = 0.;
            let status = loop {
                now += 0.1;
                let status =
                    h.run_task(&mut npc, "TASK_RANGE_ATTACK1", &arg, &ctx(now), &mut motor);
                if status != Some(TaskStatus::Running) {
                    break status;
                }
            };
            assert_eq!(status, Some(TaskStatus::Complete));
            let shots = motor
                .activities
                .iter()
                .filter(|a| *a == "ACT_RANGE_ATTACK1")
                .count();
            assert_eq!(shots as i32, burst);
            // One think per shot after the first (the 0.133 s shoot sequence finishes at
            // its first frame advance), then the completing think.
            assert!((now - 0.1 * f64::from(burst)).abs() < 1e-6, "{now} {burst}");
        }
    }
    #[test]
    fn soldiers_throw_before_reloading_one_time_in_five_when_they_can() {
        let npc = AiNpc::new(1);
        let ready = ThrowProbe::Ready {
            velocity: Vec3::new(300., 0., 200.),
            in_view: true,
        };
        let mut throws = 0;
        for seed in 1..=1000 {
            let mut h = Humanoid::new(Kind::Combine, true, seed);
            h.grenades = 5;
            h.now = 10.;
            h.throw_probes.last_known = Some(ready);
            let name = h.translate_combine("SCHED_HIDE_AND_RELOAD", &npc);
            // CanGrenadeEnemy ran (and succeeded) before the dice: next check in 1 s.
            assert_eq!(h.next_grenade_check, 11.);
            assert_eq!(h.toss_velocity, Vec3::new(300., 0., 200.));
            if name == "SCHED_COMBINE_GRENADE_AND_RELOAD" {
                throws += 1;
            } else {
                assert_eq!(name, "SCHED_COMBINE_HIDE_AND_RELOAD");
            }
        }
        assert!((150..=250).contains(&throws), "{throws}");
        // A blocked throw, or a check before m_flNextGrenadeCheck, never throws.
        let mut h = Humanoid::new(Kind::Combine, true, 3);
        h.grenades = 5;
        h.now = 10.;
        h.throw_probes.last_known = Some(ThrowProbe::Blocked { in_view: false });
        assert!(!h.can_grenade_enemy(true));
        assert_eq!(h.next_grenade_check, 11.);
        h.throw_probes.last_known = Some(ready);
        assert!(!h.can_grenade_enemy(true));
        h.now = 11.;
        assert!(h.can_grenade_enemy(true));
        // No grenades left: no.
        h.grenades = 0;
        h.now = 20.;
        assert!(!h.can_grenade_enemy(true));
    }
}
