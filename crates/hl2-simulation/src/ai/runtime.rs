//! The AI think loop in the NPC stage (CAI_BaseNPC::NPCThink every 0.1 s): per-NPC
//! GatherConditions (sight of the player and hated NPCs, hearing, enemy memory and
//! COND_NEW_ENEMY/SEE_ENEMY/ENEMY_OCCLUDED/LOST_ENEMY), SelectIdealState and
//! MaintainSchedule through the class behavior. Switched by `enabled` (the host's
//! `ai_enable`, on by default like Source's ai_disabled 0). NPCs owned by a
//! scripted_sequence or a choreographed scene are NPC_STATE_SCRIPT and the AI leaves
//! them alone. Simplified from the SDK: every NPC thinks every 0.1 s (no PVS/efficiency
//! throttling, no think staggering). Class layers: npc_metropolice and the humanoids
//! (npc_citizen, npc_barney, npc_alyx, npc_combine_s); other classes have no AI yet.
use super::assault;
use super::conditions::{
    Conditions, COND_CAN_MELEE_ATTACK1, COND_CAN_RANGE_ATTACK1, COND_ENEMY_DEAD,
    COND_ENEMY_OCCLUDED, COND_ENEMY_UNREACHABLE, COND_ENEMY_WENT_NULL, COND_HEAVY_DAMAGE,
    COND_LIGHT_DAMAGE, COND_LOST_ENEMY, COND_LOW_PRIMARY_AMMO, COND_NEW_ENEMY, COND_NONE,
    COND_NOT_FACING_ATTACK, COND_NO_PRIMARY_AMMO, COND_NO_SECONDARY_AMMO, COND_PROVOKED,
    COND_SEE_ENEMY, COND_SEE_FEAR, COND_SEE_HATE, COND_SEE_PLAYER, COND_TOO_CLOSE_TO_ATTACK,
    COND_TOO_FAR_TO_ATTACK, COND_WEAPON_HAS_LOS, COND_WEAPON_SIGHT_OCCLUDED,
};
use super::host::{yaw_of, PhysicsSight, SceneMotor};
use super::humanoid::{self, Humanoid};
use super::metropolice::{CopRequest, Metropolice, PlayerView, PLAYER};
use super::police::{PoliceGoal, Policing};
use super::relationships::{classify, Class, Disposition, Relationships};
use super::schedule::{AiNpc, Behavior, Context, Schedules};
use super::senses::{self, SightWorld, SoundEnt, Viewer};
use super::state::{select_ideal_state, NpcState, StateInputs};
use super::zombie::{self, Zombie};
use crate::{
    entities::Scene,
    npc::{Controller, GoalKey},
    npc_probe::ActorHull,
    npc_weapons::{self, NpcShot, NpcWeapon, NpcWeaponDef, WeaponEvent},
    physics::Physics,
};
use glam::Vec3;
use modkit_core::World;
use serde::Serialize;
use source_assets::{keyvalues, vpk::Vfs};
use std::collections::BTreeMap;
mod equipment;
mod goal_inputs;
mod requests;
mod speech;
mod squads;

/// NPCThink's default interval.
pub const THINK_INTERVAL: f64 = 0.1;
/// SOUND_* interests of npc_metropolice (GetSoundInterests; PLAYER_VEHICLE has no
/// CSoundEnt type here).
const METROPOLICE_SOUNDS: u32 = senses::sound::WORLD
    | senses::sound::COMBAT
    | senses::sound::PLAYER
    | senses::sound::DANGER
    | senses::sound::PHYSICS_DANGER
    | senses::sound::BULLET_IMPACT
    | senses::sound::MOVE_AWAY;
/// m_flFieldOfView of npc_metropolice.
const METROPOLICE_FOV: f32 = -0.2;
/// NPC_Metropolice sentences take their volume/soundlevel from this sound script entry.
pub const METROPOLICE_SENTENCE_PARAMETERS: &str = "NPC_Metropolice.SentenceParameters";
/// CNPC_Combine m_Sentences.Init( this, "NPC_Combine.SentenceParameters" ).
pub const COMBINE_SENTENCE_PARAMETERS: &str = "NPC_Combine.SentenceParameters";

/// The sound script entry holding an NPC sentence's volume/soundlevel/pitch
/// (CAI_Sentence::Init), by sentence or group name ("!NAME" / "#GROUP").
pub fn sentence_parameters(sentence: &str) -> Option<&'static str> {
    let name = sentence.trim_start_matches(['!', '#']).to_ascii_uppercase();
    if name.starts_with("METROPOLICE_") {
        Some(METROPOLICE_SENTENCE_PARAMETERS)
    } else if name.starts_with("COMBINE_") {
        Some(COMBINE_SENTENCE_PARAMETERS)
    } else {
        None
    }
}
/// CAI_BaseNPC sound hooks (PainSound, DeathSound, IdleSound, LostEnemySound,
/// FoundEnemySound) and CNPC_Combine::AnnounceEnemyKill with each class's sentence
/// groups (npc_metropolice.cpp, npc_combine.cpp).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Speech {
    Pain,
    /// The first light hit (cop > 0.8 health: PAIN_LIGHT; soldier > 0.9: TAUNT).
    PainLight,
    /// The first heavy hit (< 0.25 health: cop PAIN_HEAVY, soldier COVER).
    PainHeavy,
    Die,
    LostShort,
    LostLong,
    Found,
    KilledEnemy,
}
fn speech_group(class: &AiClass, speech: Speech) -> Option<&'static str> {
    let combine = matches!(class, AiClass::Humanoid(h) if h.kind == humanoid::Kind::Combine);
    let cop = class.cop().is_some();
    use Speech::*;
    Some(match (cop, combine, speech) {
        (true, _, Pain) => "METROPOLICE_PAIN",
        (true, _, PainLight) => "METROPOLICE_PAIN_LIGHT",
        (true, _, PainHeavy) => "METROPOLICE_PAIN_HEAVY",
        (true, _, Die) => "METROPOLICE_DIE",
        (true, _, LostShort) => "METROPOLICE_LOST_SHORT",
        (true, _, LostLong) => "METROPOLICE_LOST_LONG",
        (true, _, Found) => "METROPOLICE_REFIND_ENEMY",
        (_, true, Pain) => "COMBINE_PAIN",
        (_, true, PainLight) => "COMBINE_TAUNT",
        (_, true, PainHeavy) => "COMBINE_COVER",
        (_, true, Die) => "COMBINE_DIE",
        (_, true, LostShort) => "COMBINE_LOST_SHORT",
        (_, true, LostLong) => "COMBINE_LOST_LONG",
        (_, true, Found) => "COMBINE_REFIND_ENEMY",
        (_, true, KilledEnemy) => "COMBINE_KILL_MONST",
        _ => return None,
    })
}
/// The enemy's FInViewCone(me): a 2D dot from its eye to my WorldSpaceCenter against its
/// m_flFieldOfView (`fov` for NPC enemies, 0.5 without one; players 0.766,
/// CBasePlayer::Spawn).
fn enemy_sees_me(
    id: usize,
    enemy: usize,
    fov: Option<f32>,
    scene: &Scene,
    player: PlayerState,
) -> Option<bool> {
    let center = scene.states[id].origin + Vec3::Z * 36.;
    let viewer = if enemy == PLAYER {
        Viewer {
            entity: PLAYER,
            class: super::relationships::Class::Player,
            eye: player.eye(),
            facing: player.facing,
            field_of_view: 0.766,
            look_distance: f32::MAX,
        }
    } else {
        let state = scene.states.get(enemy)?;
        Viewer {
            entity: enemy,
            class: super::relationships::Class::None,
            eye: state.origin + Vec3::Z * super::host::EYE_HEIGHT,
            facing: state.rotation * Vec3::X,
            field_of_view: fov.unwrap_or(0.5),
            look_distance: f32::MAX,
        }
    };
    Some(senses::in_view_cone(&viewer, center))
}
/// SF_NPC_GAG: no idle sounds (ShouldPlayIdleSound).
const SF_NPC_GAG: u32 = 2;
/// What the host knows about the local player this tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerState {
    pub feet: Vec3,
    pub velocity: Vec3,
    pub crouched: bool,
    pub on_ground: bool,
    pub dead: bool,
    pub suit: bool,
    /// flying/noclip players are not sensed.
    pub noclip: bool,
    /// The view direction (EyeDirection2D uses its yaw).
    pub facing: Vec3,
}
impl PlayerState {
    fn center(&self) -> Vec3 {
        self.feet + Vec3::Z * if self.crouched { 18. } else { 36. }
    }
    fn eye(&self) -> Vec3 {
        self.feet + Vec3::Z * if self.crouched { 28. } else { 64. }
    }
}
/// A hit on the player for the host (damage through the player damage path, the
/// velocity impulse, the view punch and the screen fade).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlayerHit {
    pub attacker: usize,
    pub damage: f32,
    pub punch: Vec3,
    pub impulse: Vec3,
    pub fade: Option<crate::player_damage::ScreenFade>,
}
/// CNPC_Combine's reload animation event (combine_soldier_anims reload / reload_low).
const COMBINE_AE_RELOAD: i32 = 2;
/// CNPC_Combine's kick animation event.
const COMBINE_AE_KICK: i32 = 3;
/// CNPC_Combine's grenade toss animation event.
const COMBINE_AE_GREN_TOSS: i32 = 7;
/// sv_gravity.
const GRAVITY: f32 = 600.;
/// CNPC_CombineS::Precache: the elite flag comes from the model name.
pub fn is_elite_model(model: Option<&str>) -> bool {
    model.is_some_and(|m| {
        m.replace('\\', "/")
            .eq_ignore_ascii_case("models/combine_super_soldier.mdl")
    })
}
/// VecCheckThrow (h_ai.cpp): a throw at `speed` from `start` to `target` whose two
/// halves (start -> apex, target -> apex) are clear; None when blocked.
pub fn vec_check_throw(
    physics: &Physics,
    id: usize,
    start: Vec3,
    target: Vec3,
    speed: f32,
) -> Option<Vec3> {
    let delta = target - start;
    let time = delta.length() / speed;
    if time <= 0. {
        return None;
    }
    let mut velocity = delta / time;
    velocity.z += GRAVITY * time * 0.5;
    let mut apex = start + delta * 0.5;
    apex.z += 0.5 * GRAVITY * (time * 0.5) * (time * 0.5);
    (trace_clear(physics, start, apex, id) && trace_clear(physics, target, apex, id))
        .then_some(velocity)
}
/// VecCheckToss (h_ai.cpp) with no height ratio limit, gravity 1 and the randomized
/// target (`jitter`: four [0, 1) draws): the apex is the midpoint raised up to 300 units
/// (15 under a ceiling); None when either half is blocked or the target is 500 above.
pub fn vec_check_toss(
    physics: &Physics,
    id: usize,
    start: Vec3,
    target: Vec3,
    forward: Vec3,
    jitter: [f32; 4],
) -> Option<Vec3> {
    if target.z - start.z > 500. {
        return None;
    }
    let right = forward.cross(Vec3::Z).normalize_or_zero();
    let r = |u: f32, a: f32| -a + 2. * a * u;
    let target = target
        + right * (r(jitter[0], 8.) + r(jitter[1], 16.))
        + forward * (r(jitter[2], 8.) + r(jitter[3], 16.));
    let mid = start + (target - start) * 0.5;
    let mid = match physics.projectile_ray(mid, mid + Vec3::Z * 300., &[id]) {
        Some(hit) => hit.position - Vec3::Z * 15.,
        None => mid + Vec3::Z * 300.,
    };
    if mid.z < start.z || mid.z < target.z {
        return None;
    }
    let time1 = ((mid.z - start.z) / (0.5 * GRAVITY)).sqrt();
    let time2 = ((mid.z - target.z) / (0.5 * GRAVITY)).sqrt();
    if time1 < 0.1 {
        return None;
    }
    let mut velocity = (target - start) / (time1 + time2);
    velocity.z = GRAVITY * time1;
    let mut apex = start + velocity * time1;
    apex.z = mid.z;
    (trace_clear(physics, start, apex, id) && trace_clear(physics, target, apex, id))
        .then_some(velocity)
}
/// A throw trace's `fraction == 1`: Source ignores the solid a trace starts on or in
/// (startsolid) and only fails on a surface it enters. The flush target is the enemy's
/// last seen feet position, on the floor, where our ray reports a hit at its own start
/// (before this, every packaged flush toss failed).
fn trace_clear(physics: &Physics, from: Vec3, to: Vec3, id: usize) -> bool {
    match physics.projectile_ray(from, to, &[id]) {
        None => true,
        Some(hit) if hit.position.distance(from) < 0.5 => {
            let step = (to - from).normalize_or_zero();
            physics.projectile_ray(from + step, to, &[id]).is_none()
        }
        Some(_) => false,
    }
}
/// A melee swing that struck an NPC (CTraceFilterMelee: DMG_CLUB on a hated NPC); the
/// host applies it through the shared hit path.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct MeleeHit {
    pub attacker: usize,
    pub victim: usize,
    pub damage: f32,
    pub position: Vec3,
    pub direction: Vec3,
}
#[derive(Clone, Debug, Serialize)]
pub enum AiClass {
    Metropolice(Box<Metropolice>),
    Humanoid(Box<Humanoid>),
    Zombie(Box<Zombie>),
}
impl AiClass {
    pub fn cop(&self) -> Option<&Metropolice> {
        match self {
            Self::Metropolice(cop) => Some(cop),
            _ => None,
        }
    }
    pub fn cop_mut(&mut self) -> Option<&mut Metropolice> {
        match self {
            Self::Metropolice(cop) => Some(cop),
            _ => None,
        }
    }
    pub fn humanoid(&self) -> Option<&Humanoid> {
        match self {
            Self::Humanoid(h) => Some(h),
            _ => None,
        }
    }
    /// The hosted CAI_AssaultBehavior.
    pub fn assault(&self) -> Option<&assault::Assault> {
        match self {
            Self::Metropolice(cop) => cop.assault.as_ref(),
            Self::Humanoid(h) => h.assault.as_ref(),
            Self::Zombie(z) => z.assault.as_ref(),
        }
    }
    pub fn assault_mut(&mut self) -> &mut Option<assault::Assault> {
        match self {
            Self::Metropolice(cop) => &mut cop.assault,
            Self::Humanoid(h) => &mut h.assault,
            Self::Zombie(z) => &mut z.assault,
        }
    }
    /// The aiscripted_schedule goal (position, run).
    pub fn script_goal(&self) -> Option<(Vec3, bool)> {
        match self {
            Self::Metropolice(cop) => cop.script_goal,
            Self::Humanoid(h) => h.script_goal,
            Self::Zombie(z) => z.script_goal,
        }
    }
    fn script_goal_mut(&mut self) -> &mut Option<(Vec3, bool)> {
        match self {
            Self::Metropolice(cop) => &mut cop.script_goal,
            Self::Humanoid(h) => &mut h.script_goal,
            Self::Zombie(z) => &mut z.script_goal,
        }
    }
    fn requests_mut(&mut self) -> &mut Vec<CopRequest> {
        match self {
            Self::Metropolice(cop) => &mut cop.requests,
            Self::Humanoid(h) => &mut h.requests,
            Self::Zombie(z) => &mut z.requests,
        }
    }
    fn behavior_mut(&mut self) -> &mut dyn Behavior {
        match self {
            Self::Metropolice(cop) => cop.as_mut(),
            Self::Humanoid(h) => h.as_mut(),
            Self::Zombie(z) => z.as_mut(),
        }
    }
    /// m_flFieldOfView.
    fn field_of_view(&self) -> f32 {
        match self {
            Self::Metropolice(_) => METROPOLICE_FOV,
            Self::Humanoid(h) => h.kind.field_of_view(),
            Self::Zombie(_) => zombie::FIELD_OF_VIEW,
        }
    }
    /// GetSoundInterests.
    fn sound_interests(&self) -> u32 {
        match self {
            Self::Metropolice(_) => METROPOLICE_SOUNDS,
            Self::Humanoid(h) => h.kind.sound_interests(),
            // CAI_BaseNPC::GetSoundInterests (SOUND_PLAYER_VEHICLE has no source here).
            Self::Zombie(_) => {
                use super::senses::sound;
                sound::WORLD | sound::COMBAT | sound::PLAYER | sound::BULLET_IMPACT
            }
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct AiActor {
    pub npc: AiNpc,
    pub class: AiClass,
    pub next_think: f64,
    pub last_think: f64,
    /// The active weapon (Weapon_Equip of `additionalequipment`; DESIGN 14w).
    pub weapon: Option<NpcWeapon>,
    /// The F4 per-spawn disposition toward the player (an ai_relationship aimed at
    /// this NPC); None: the class rules.
    pub player_disposition: Option<Disposition>,
    /// The origin and body yaw (degrees) at the last think (report only).
    pub origin: [f32; 3],
    pub yaw: f32,
    /// An NPC that shot this one since the last think (OnTakeDamage: the attacker).
    pub damaged_by: Option<usize>,
    /// The largest single hit since the last think (IsLightDamage > 0, IsHeavyDamage > 20).
    #[serde(skip)]
    damage_taken: f32,
    /// The player hit this NPC since the last think.
    #[serde(skip)]
    damaged_by_player: bool,
    #[serde(skip)]
    goal: Option<GoalKey>,
    /// Animation events up to this time were handled.
    events_until: f64,
    /// The ACT_IDLE -> ACT_IDLE_ANGRY translation input at the last think.
    idle_angry: bool,
    /// The last think interval: the weapon's m_flAnimTime - m_flPrevAnimTime, which
    /// Operator_FrameUpdate advances once per think (WeaponSoundRealtime's estimate).
    think_dt: f32,
    /// CAI_MoveAndShootOverlay.
    pub move_shoot: super::moveshoot::MoveShoot,
    /// The soldier's GetShotRegulator() for move and shoot (CNPC_Combine has no
    /// bits_CAP_USE_SHOT_REGULATOR, so its standing bursts do not use it).
    #[serde(skip)]
    move_regulator: Option<npc_weapons::ShotRegulator>,
    /// Weapon fire events the overlay queued for the next tick.
    #[serde(skip)]
    move_fire: u32,
    /// bits_MEMORY_PAIN_LIGHT_SOUND (1) / bits_MEMORY_PAIN_HEAVY_SOUND (2).
    #[serde(skip)]
    pain_memory: u8,
    /// m_flNextPainSoundTime, m_flNextLostSoundTime.
    #[serde(skip)]
    next_pain_sound: f64,
    #[serde(skip)]
    next_lost_sound: f64,
    /// The enemy eluded (LostEnemySound said); seeing it again says FoundEnemySound.
    #[serde(skip)]
    enemy_eluded: bool,
    #[serde(skip)]
    death_spoken: bool,
    /// CAI_BaseNPC m_SquadName (the `squadname` keyvalue) until InitSquad joins it.
    #[serde(skip)]
    squad_name: Option<String>,
    /// The squad joined (CAI_SquadManager::FindCreateSquad).
    pub squad: Option<String>,
    /// SetEnemy by SquadNewEnemy since the last think (COND_NEW_ENEMY).
    #[serde(skip)]
    squad_new_enemy: bool,
    /// ChooseEnemy's serial and fail timer.
    #[serde(skip)]
    choose: super::enemy_choice::ChooseState,
    /// CNPC_MetroPolice m_nIdleChatterType (COP_CHATTER_*).
    pub idle_chatter: u8,
    /// bits_MEMORY_PLAYER_HARASSED.
    #[serde(skip)]
    harassed: bool,
    /// CNPC_Combine::m_flStopMoveShootTime.
    #[serde(skip)]
    stop_move_shoot: Option<f64>,
    /// m_flNextDecisionTime for the idle sound roll (UpdateEfficiency's interval).
    #[serde(skip)]
    next_idle_decision: f64,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Diagnostics {
    pub thinks: u64,
    /// CNPC_Combine flush-grenade checks by outcome (out_of_range, moving, no_toss, ready).
    pub grenade_checks: BTreeMap<String, u32>,
    /// Melee swings that struck an NPC.
    pub npc_melee_hits: u64,
    pub scripted_skips: u64,
    pub sentences: u64,
    /// Sentences FOkToMakeSound refused (equal or lower priority while speaking).
    pub culled_sentences: u64,
    pub player_hits: u64,
    pub unknown_goals: BTreeMap<String, u32>,
    /// AI move requests the npc Controller refused, by reason.
    pub blocked_moves: BTreeMap<String, u32>,
    /// NPC weapon shots fired and pellets that struck the player (host-counted).
    pub shots_fired: u64,
    pub bullet_hits_on_player: u64,
    /// Player-ally concepts by outcome ("TLK_WOUND:spoken", "TLK_STARTCOMBAT:semaphore").
    pub ally_speech: BTreeMap<String, u32>,
    /// Sentence groups spoken, and those MatchesCriteria refused (e.g. IN_SQUAD lines of
    /// an NPC without a squad of two).
    pub sentence_groups: BTreeMap<String, u32>,
    pub criteria_refused: BTreeMap<String, u32>,
    /// Squad events: joins, deaths, SquadNewEnemy enemies given, memories shared.
    pub squad_events: BTreeMap<String, u32>,
    /// g_interactionHitByPlayerThrownPhysObj dispatches.
    pub thrown_object_hits: u64,
}
#[derive(Serialize)]
pub struct AiRuntime {
    /// The `ai_enable` switch (the host turns it on by default, like Source).
    pub enabled: bool,
    #[serde(skip)]
    schedules: Schedules,
    #[serde(skip)]
    relationships: Relationships,
    pub sounds: SoundEnt,
    pub actors: BTreeMap<usize, AiActor>,
    /// ai_goal_police entities by id.
    pub goals: BTreeMap<usize, PoliceGoal>,
    serial: usize,
    /// weapon_stunstick script SoundData (melee_hit, melee_miss).
    stunstick_sounds: (Option<String>, Option<String>),
    /// sk_npc_dmg_stunstick from skill.cfg. The runtime is built on the map-load worker
    /// thread, so `tick` hands it to the simulation thread's STUNSTICK_DAMAGE.
    #[serde(skip)]
    stunstick_damage: Option<f32>,
    /// sk_combine_s_kick (CNPC_CombineS::Spawn SetKickDamage); the ConVar default is 0.
    combine_kick_damage: f32,
    /// sk_combine_guard_kick (the elite's SetKickDamage).
    combine_guard_kick_damage: f32,
    /// sk_zombie_dmg_one_slash (every claw event; the ConVar default is 0).
    zombie_slash_damage: f32,
    /// sk_zombie_poison_dmg_spit (the poison zombie's bite).
    zombie_poison_spit_damage: f32,
    /// NPC weapon definitions (installed scripts and skill.cfg).
    #[serde(skip)]
    weapon_defs: BTreeMap<String, NpcWeaponDef>,
    /// FireBullets requests from NPC weapons this tick, for the host's bullet path.
    pub shots: Vec<NpcShot>,
    /// AE_NPC_MUZZLEFLASH events (NPC, options) for the host's effects.
    #[serde(skip)]
    pub muzzle_flashes: Vec<(usize, String)>,
    /// Operators whose weapon called DoMuzzleFlash this tick (CBaseCombatCharacter ->
    /// the active weapon's world model: every NPC gun except the AR2).
    pub weapon_muzzle_flashes: Vec<usize>,
    /// Hits on the player for the host to apply.
    pub player_hits: Vec<PlayerHit>,
    /// Melee hits on NPCs this tick (stunstick swings).
    pub melee_hits: Vec<MeleeHit>,
    /// Combine grenade tosses this tick: (thrower, position, velocity, spin).
    pub grenade_tosses: Vec<(usize, Vec3, Vec3, Vec3)>,
    /// Elite combine balls (CreateCombineBall at 1000 units/s).
    ball_launches: Vec<(usize, Vec3, Vec3)>,
    /// WeaponSound(SINGLE_NPC, soundtime) requests that start later (WeaponSoundRealtime
    /// schedules a machine gun's second sound ahead of the next think).
    #[serde(skip)]
    delayed_sounds: Vec<(f64, crate::sounds::SoundRequest)>,
    pub diagnostics: Diagnostics,
    random: u64,
    /// Debug (F4): the AI does not sense the player (ai_ignoreplayers / notarget).
    pub ignore_player: bool,
    /// Active weapons of NPCs whose class has no AI behavior here (Weapon_Equip from
    /// additionalequipment or F4): held and drawn, never fired.
    pub held_weapons: BTreeMap<usize, NpcWeapon>,
    /// The AI ran last tick (switching it off returns NPCs to idle once).
    #[serde(skip)]
    was_enabled: bool,
    /// CAI_BaseNPC m_flSoundWaitTime / m_nSoundPriority per NPC (FOkToMakeSound).
    #[serde(skip)]
    sound_wait: BTreeMap<usize, (f64, i32)>,
    /// Player-ally speech through the talker response rules.
    #[serde(skip)]
    pub ally: super::ally_speech::AllySpeech,
    /// CNPC_Combine g_fCombineQuestion: 1 after COMBINE_CHECK, 2 after COMBINE_QUEST
    /// (the next idle soldier answers CLEAR / ANSWER).
    #[serde(skip)]
    combine_question: u8,
    /// CRallyPoint::Lock: rally point entity -> the NPC holding it.
    pub rally_locks: BTreeMap<usize, usize>,
    /// CAssaultPoint::m_flTimeLastUsed by entity.
    #[serde(skip)]
    assault_used: BTreeMap<usize, f64>,
    /// StartActive ai_goal_assault entities were activated.
    #[serde(skip)]
    goals_started: bool,
    /// aiscripted_schedule controllers that found their actor (m_bDidFireOnce).
    #[serde(skip)]
    scripted_fired: std::collections::BTreeSet<usize>,
    /// F4 per-spawn allegiance toward the player of any NPC (AI or not): its side in
    /// NPC-vs-NPC fights (DESIGN 14n step 1).
    pub allegiances: BTreeMap<usize, Disposition>,
    /// Entities whose class is npc_* (sight candidates), built on first use.
    #[serde(skip)]
    npc_ids: Option<Vec<usize>>,
    /// ai_relationship overrides (subject, target) -> disposition (target PLAYER for
    /// "!player"), and the pairs each active ai_relationship entity applied.
    #[serde(serialize_with = "serialize_pairs")]
    pub relationship_overrides: BTreeMap<(usize, usize), Disposition>,
    #[serde(skip)]
    applied_relationships: BTreeMap<usize, Vec<(usize, usize)>>,
    /// CAI_SquadManager: squads by name.
    pub squads: BTreeMap<String, super::squad::Squad>,
    /// scripts/actbusy.txt busy animations (activity entries).
    #[serde(skip)]
    busy_anims: Vec<super::actbusy::BusyAnim>,
    /// The map's info_node_hint entities (CAI_HintManager's hints).
    #[serde(skip)]
    busy_hints: Vec<super::actbusy::HintCandidate>,
    /// CAI_Hint::Lock: hint entity -> the NPC using it.
    pub hint_locks: BTreeMap<usize, usize>,
    /// ai_goal_actbusy busysearchrange (SetBusySearchRange changes it).
    actbusy_ranges: BTreeMap<usize, f32>,
    /// Squad work a think asked for (applied right after it).
    #[serde(skip)]
    squad_events: Vec<SquadEvent>,
    /// IdleSound lines a think chose (spoken right after it).
    #[serde(skip)]
    idle_lines: Vec<IdleLine>,
}
/// METROPOLICE_CHATTER_WAIT_FOR_RESPONSE / ASK_QUESTION / RESPONSE (+ the question type).
const COP_CHATTER_WAIT_FOR_RESPONSE: u8 = 0;
const COP_CHATTER_ASK_QUESTION: u8 = 1;
const COP_CHATTER_RESPONSE: u8 = 2;
/// An IdleSound line and what speaking it changes (only when Speak succeeds).
#[derive(Clone, Copy, Debug)]
struct IdleLine {
    id: usize,
    group: &'static str,
    then: IdleThen,
}
#[derive(Clone, Copy, Debug)]
enum IdleThen {
    Nothing,
    /// g_fCombineQuestion.
    CombineQuestion(u8),
    /// Remember (and SquadRemember) bits_MEMORY_PLAYER_HARASSED.
    CopHarassed,
    /// The squadmates answer this question type; the asker waits.
    CopAsked(u8),
    /// The squadmates may ask again.
    CopAnswered,
}
/// CAI_Squad calls a member's think makes on the others.
#[derive(Clone, Debug)]
enum SquadEvent {
    /// CAI_Squad::UpdateEnemyMemory: a first-hand sighting shared with the squad.
    Memory {
        squad: String,
        from: usize,
        enemy: usize,
        position: Vec3,
    },
    /// CAI_Squad::SquadNewEnemy (SelectIdealState, idle/alert with COND_NEW_ENEMY).
    NewEnemy { squad: String, enemy: usize },
}
/// SentenceCriteria_t of a CAI_SentenceBase::Speak call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Criteria {
    Always,
    /// Obeys the gag spawnflag outside combat.
    Normal,
    /// The default: a squad of more than one.
    InSquad,
    SquadLeader,
}
/// The criteria npc_metropolice.cpp, ai_behavior_police.cpp and npc_combine.cpp pass for
/// each sentence group (Speak's default is SENTENCE_CRITERIA_IN_SQUAD).
fn sentence_criteria(group: &str) -> Criteria {
    if group.starts_with("METROPOLICE_PAIN")
        || matches!(
            group,
            "METROPOLICE_DIE"
                | "METROPOLICE_ON_FIRE"
                | "METROPOLICE_HIT_BY_PHYSOBJECT"
                | "COMBINE_PAIN"
                | "COMBINE_TAUNT"
                | "COMBINE_COVER"
                | "COMBINE_DIE"
        )
    {
        Criteria::Always
    } else if [
        "METROPOLICE_MOVE_ALONG",
        "METROPOLICE_BACK_UP",
        "METROPOLICE_DANGER",
        "METROPOLICE_IDLE_HARASS_PLAYER",
    ]
    .iter()
    .any(|g| group.starts_with(g))
        || matches!(
            group,
            "METROPOLICE_MANHACK_KILLED"
                | "METROPOLICE_LAST_OF_SQUAD"
                | "METROPOLICE_FREEZE"
                | "METROPOLICE_OVER_HERE"
                | "METROPOLICE_HES_RUNNING"
                | "METROPOLICE_TAKE_HIM_DOWN"
                | "METROPOLICE_ARREST_IN_POS"
                | "METROPOLICE_COVER_HEAVY_DAMAGE"
                | "METROPOLICE_ACTIVATE_BATON"
                | "METROPOLICE_DEACTIVATE_BATON"
                | "COMBINE_LAST_OF_SQUAD"
                | "COMBINE_GREN"
                | "COMBINE_DANGER"
        )
    {
        Criteria::Normal
    } else if matches!(
        group,
        "METROPOLICE_SO_BEGIN"
            | "METROPOLICE_SO_END"
            | "METROPOLICE_AS_ADV_RALLY"
            | "METROPOLICE_AS_ADV_ASSAULT"
    ) {
        Criteria::SquadLeader
    } else {
        Criteria::InSquad
    }
}
/// A pair-keyed map as a list of (subject, target, value) for the JSON report.
fn serialize_pairs<S: serde::Serializer, V: Serialize>(
    map: &BTreeMap<(usize, usize), V>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(map.iter().map(|((a, b), v)| (a, b, v)))
}
/// The yaw of an entity's "angles" keyvalue (degrees).
fn yaw_keyvalue(e: &modkit_core::Entity) -> f32 {
    modkit_core::parse_vec3(e.get("angles").unwrap_or("0 0 0")).map_or(0., |a| a.y)
}
/// SentencePriority_t of the metrocop sentence groups as npc_metropolice.cpp speaks
/// them (-1 = SENTENCE_PRIORITY_INVALID: plays without the sound timer).
fn sentence_priority(group: &str) -> i32 {
    const HIGH: [&str; 12] = [
        "METROPOLICE_PLAYERHIT",
        "METROPOLICE_HES_RUNNING",
        "METROPOLICE_TAKE_HIM_DOWN",
        "METROPOLICE_REFIND_ENEMY",
        "METROPOLICE_DANGER",
        "METROPOLICE_FT_MOUNT",
        "METROPOLICE_FT_DISMOUNT",
        "METROPOLICE_SO_BEGIN",
        "METROPOLICE_SO_END",
        // TASK_SPEAK_SENTENCE -> SpeakSentence (HIGH).
        "METROPOLICE_ACTIVATE_BATON",
        "METROPOLICE_DEACTIVATE_BATON",
        "METROPOLICE_MOVE_ALONG",
    ];
    const MEDIUM: [&str; 9] = [
        "METROPOLICE_BACK_UP",
        "METROPOLICE_FREEZE",
        "METROPOLICE_OVER_HERE",
        "METROPOLICE_ARREST_IN_POS",
        "METROPOLICE_HEARD_SOMETHING",
        "METROPOLICE_LAST_OF_SQUAD",
        "METROPOLICE_MAN_DOWN",
        "METROPOLICE_FT_APPROACH",
        "METROPOLICE_AS_ADV",
    ];
    // CNPC_Combine: PainSound (PAIN/TAUNT/COVER) and DeathSound speak INVALID +
    // CRITERIA_ALWAYS; the enemy announcements and REFIND_ENEMY are HIGH.
    const COMBINE_HIGH: [&str; 7] = [
        "COMBINE_ANNOUNCE",
        "COMBINE_REFIND_ENEMY",
        "COMBINE_GO_ALERT",
        "COMBINE_MONST",
        "COMBINE_ALERT",
        "COMBINE_KILL_MONST",
        "COMBINE_PLAYER_DEAD",
    ];
    if group.starts_with("METROPOLICE_PAIN")
        || group == "METROPOLICE_DIE"
        || matches!(
            group,
            "COMBINE_PAIN"
                | "COMBINE_TAUNT"
                | "COMBINE_COVER"
                | "COMBINE_DIE"
                | "COMBINE_PLAYERHIT"
                | "COMBINE_LAST_OF_SQUAD"
        )
    {
        -1
    } else if COMBINE_HIGH.iter().any(|g| group.starts_with(g)) {
        2
    } else if group == "COMBINE_THROW_GRENADE" {
        1
    } else if HIGH.iter().any(|g| group.starts_with(g)) {
        2
    } else if MEDIUM.iter().any(|g| group.starts_with(g)) {
        1
    } else {
        0
    }
}
impl AiRuntime {
    /// The AI for the map's NPCs (metrocops) and ai_goal_police entities. `vfs` reads
    /// skill.cfg (sk_npc_dmg_stunstick) and the stunstick's sound script.
    pub fn new(world: &World, vfs: Option<&Vfs>) -> Self {
        let mut schedules = Schedules::default();
        super::police::add_schedules(&mut schedules);
        super::metropolice::add_schedules(&mut schedules);
        humanoid::add_schedules(&mut schedules);
        zombie::add_schedules(&mut schedules);
        super::actbusy::add_schedules(&mut schedules);
        let busy_anims = vfs
            .and_then(|vfs| vfs.read("scripts/actbusy.txt").ok().flatten())
            .map(|text| super::actbusy::parse_busy_anims(&String::from_utf8_lossy(&text)))
            .unwrap_or_default();
        let busy_hints = world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| e.class() == "info_node_hint")
            .map(|(id, e)| {
                let int = |k: &str| e.get(k).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                super::actbusy::HintCandidate {
                    entity: id,
                    origin: e.origin(),
                    yaw: yaw_keyvalue(e),
                    hint_type: int("hinttype"),
                    group: e.get("Group").unwrap_or("").trim().to_owned(),
                    activity: e.get("hintactivity").unwrap_or("").trim().to_owned(),
                    disabled: int("StartHintDisabled") != 0,
                    min_state: int("MinimumState"),
                    max_state: int("MaximumState"),
                }
            })
            .collect();
        let actbusy_ranges = world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| e.class() == "ai_goal_actbusy")
            .map(|(id, e)| {
                (
                    id,
                    e.get("busysearchrange")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(2048.),
                )
            })
            .collect();
        let mut stunstick_sounds = (None, None);
        let mut stunstick_damage = None;
        let mut combine_kick_damage = 0.;
        let mut combine_guard_kick_damage = 0.;
        let mut zombie_slash_damage = 0.;
        let mut zombie_poison_spit_damage = 0.;
        if let Some(vfs) = vfs {
            if let Ok(Some(skill)) = vfs.read("cfg/skill.cfg") {
                if let Ok(tokens) = keyvalues::tokens(&String::from_utf8_lossy(&skill)) {
                    if let Some(value) = tokens
                        .windows(2)
                        .find(|p| p[0].eq_ignore_ascii_case("sk_npc_dmg_stunstick"))
                        .and_then(|p| p[1].parse::<f32>().ok())
                    {
                        super::metropolice::STUNSTICK_DAMAGE.with(|d| d.set(value));
                        stunstick_damage = Some(value);
                    }
                    if let Some(value) = tokens
                        .windows(2)
                        .find(|p| p[0].eq_ignore_ascii_case("sk_combine_s_kick"))
                        .and_then(|p| p[1].parse::<f32>().ok())
                    {
                        combine_kick_damage = value;
                    }
                    if let Some(value) = tokens
                        .windows(2)
                        .find(|p| p[0].eq_ignore_ascii_case("sk_combine_guard_kick"))
                        .and_then(|p| p[1].parse::<f32>().ok())
                    {
                        combine_guard_kick_damage = value;
                    }
                    if let Some(value) = tokens
                        .windows(2)
                        .find(|p| p[0].eq_ignore_ascii_case("sk_zombie_dmg_one_slash"))
                        .and_then(|p| p[1].parse::<f32>().ok())
                    {
                        zombie_slash_damage = value;
                    }
                    if let Some(value) = tokens
                        .windows(2)
                        .find(|p| p[0].eq_ignore_ascii_case("sk_zombie_poison_dmg_spit"))
                        .and_then(|p| p[1].parse::<f32>().ok())
                    {
                        zombie_poison_spit_damage = value;
                    }
                }
            }
            if let Ok(Some(script)) = vfs.read("scripts/weapon_stunstick.txt") {
                if let Ok(entries) = keyvalues::parse(&String::from_utf8_lossy(&script)) {
                    let sounds = entries.first().and_then(|r| r.get("SoundData"));
                    let get = |key: &str| {
                        sounds
                            .and_then(|s| s.get(key))
                            .and_then(|e| e.text())
                            .map(str::to_owned)
                    };
                    stunstick_sounds = (get("melee_hit"), get("melee_miss"));
                }
            }
        }
        let weapon_defs = vfs
            .and_then(|vfs| npc_weapons::definitions(vfs).ok())
            .unwrap_or_default();
        let mut actors = BTreeMap::new();
        let mut goals = BTreeMap::new();
        let mut held_weapons = BTreeMap::new();
        for (id, e) in world.entities.iter().enumerate() {
            if let Some(actor) = Self::actor_for(id, e, &weapon_defs) {
                actors.insert(id, actor);
                continue;
            }
            if let Some(weapon) = Self::equip(
                e.class(),
                e.get("additionalequipment").unwrap_or(""),
                &weapon_defs,
            ) {
                held_weapons.insert(id, weapon);
            }
            if e.class() == "ai_goal_police" {
                {
                    let angles = e
                        .get("angles")
                        .and_then(modkit_core::parse_vec3)
                        .unwrap_or(Vec3::ZERO);
                    goals.insert(
                        id,
                        PoliceGoal {
                            entity: id,
                            origin: e.origin(),
                            yaw: angles.y,
                            radius: e
                                .get("PoliceRadius")
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(0.),
                            target: None,
                            spawnflags: e
                                .get("spawnflags")
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(0),
                            override_knockout: false,
                        },
                    );
                }
            }
        }
        Self {
            enabled: false,
            ignore_player: false,
            held_weapons,
            was_enabled: false,
            sound_wait: BTreeMap::new(),
            ally: vfs
                .map(super::ally_speech::AllySpeech::load)
                .unwrap_or_default(),
            combine_question: 0,
            rally_locks: BTreeMap::new(),
            assault_used: BTreeMap::new(),
            goals_started: false,
            scripted_fired: Default::default(),
            allegiances: BTreeMap::new(),
            npc_ids: None,
            relationship_overrides: BTreeMap::new(),
            applied_relationships: BTreeMap::new(),
            squads: BTreeMap::new(),
            busy_anims,
            busy_hints,
            hint_locks: BTreeMap::new(),
            actbusy_ranges,
            squad_events: Vec::new(),
            idle_lines: Vec::new(),
            schedules,
            relationships: Relationships::default(),
            sounds: SoundEnt::default(),
            actors,
            goals,
            serial: 0,
            stunstick_sounds,
            stunstick_damage,
            combine_kick_damage,
            combine_guard_kick_damage,
            zombie_slash_damage,
            zombie_poison_spit_damage,
            weapon_defs,
            shots: Vec::new(),
            muzzle_flashes: Vec::new(),
            weapon_muzzle_flashes: Vec::new(),
            player_hits: Vec::new(),
            melee_hits: Vec::new(),
            grenade_tosses: Vec::new(),
            ball_launches: Vec::new(),
            delayed_sounds: Vec::new(),
            diagnostics: Diagnostics::default(),
            random: 0x9e37_79b9_7f4a_7c15,
        }
    }
    /// CWeaponStunStick::m_bActive of each NPC whose active weapon is a stunstick turned
    /// on (npc_metropolice SetBatonState).
    pub fn active_stunsticks(&self) -> Vec<usize> {
        self.actors
            .iter()
            .filter(|(_, a)| {
                a.class.cop().is_some_and(|cop| cop.baton_active)
                    && a.weapon
                        .as_ref()
                        .is_some_and(|w| w.class == "weapon_stunstick")
            })
            .map(|(id, _)| *id)
            .collect()
    }
    /// NPCs whose current enemy or enemy memory is the player.
    pub fn npcs_knowing_player(&self) -> std::collections::BTreeSet<usize> {
        self.actors
            .iter()
            .filter(|(_, a)| {
                a.npc.enemy == Some(PLAYER) || a.npc.enemies.find(Some(PLAYER)).is_some()
            })
            .map(|(id, _)| *id)
            .collect()
    }
    pub fn uniform(&mut self) -> f32 {
        self.random() * 2. - 1.
    }
    fn random(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 40) as f32 / (1u32 << 24) as f32
    }
    /// One fixed tick of the NPC stage (before the movement controller tick).
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        world: &World,
        scene: &mut Scene,
        controller: &mut Controller,
        physics: &Physics,
        transients: &[ActorHull],
        player: PlayerState,
    ) {
        // OnTakeDamage_Alive: hits since the last tick, per victim.
        for (victim, by_player, damage) in std::mem::take(&mut scene.npc_damage) {
            if let Some(actor) = self.actors.get_mut(&victim) {
                actor.damage_taken = actor.damage_taken.max(damage);
                actor.damaged_by_player |= by_player;
            }
            self.pain_or_death_sound(victim, world, scene, player);
        }
        if let Some(value) = self.stunstick_damage {
            super::metropolice::STUNSTICK_DAMAGE.with(|d| d.set(value));
        }
        let due = scene.time + 1e-9;
        let (now_due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.delayed_sounds)
            .into_iter()
            .partition(|(at, _)| *at <= due);
        self.delayed_sounds = later;
        scene
            .sounds
            .extend(now_due.into_iter().map(|(_, request)| request));
        self.inputs(world, scene, controller);
        if !self.enabled {
            if self.was_enabled {
                self.was_enabled = false;
                self.stop_all(world, scene, controller);
            }
            return;
        }
        self.was_enabled = true;
        let player = if self.ignore_player {
            PlayerState {
                noclip: true,
                ..player
            }
        } else {
            player
        };
        let now = scene.time;
        self.sounds.purge(now);
        let precriminal = scene.globals.is_on("gordon_precriminal");
        self.update_squads(world, scene);
        let ids: Vec<usize> = self.actors.keys().copied().collect();
        for id in ids {
            let state = &scene.states[id];
            if state.killed || !state.visible {
                continue;
            }
            // NPC_STATE_SCRIPT: scripts own the actor; the AI yields.
            if scene.ai_scripted(id) {
                let actor = self.actors.get_mut(&id).expect("actor");
                if actor.npc.state != NpcState::Script {
                    actor.npc.state = NpcState::Script;
                    actor.npc.schedule = None;
                }
                if let Some(key) = actor.goal.take() {
                    controller.cancel(key);
                    controller.forget(key);
                    scene.ai_forget_move(key);
                }
                if let Some(cop) = actor.class.cop_mut() {
                    cop.scripted = true;
                }
                self.diagnostics.scripted_skips += 1;
                continue;
            }
            let think_at = self.actors[&id].next_think;
            // Animation events of the AI's sequence since the last tick.
            let since = self.actors[&id].events_until.min(now);
            self.actors.get_mut(&id).expect("actor").events_until = now;
            let mut events = scene.ai_events(world, id, since, now);
            // Move and shoot: the restarted ACT_GESTURE_RANGE_ATTACK1's fire event.
            let queued = std::mem::take(&mut self.actors.get_mut(&id).expect("actor").move_fire);
            if queued > 0 {
                if let Some(event) = self.actors[&id]
                    .weapon
                    .as_ref()
                    .and_then(|w| self.weapon_def(&w.class))
                    .and_then(|d| d.fire)
                    .map(|f| f.event)
                {
                    events.extend((0..queued).map(|_| (String::new(), event)));
                }
            }
            // Operator_FrameUpdate: the weapon world model's events go to its operator.
            if let Some(weapon) = self.actors.get_mut(&id).and_then(|a| a.weapon.as_mut()) {
                if let Some(rig) = self
                    .weapon_defs
                    .get(&weapon.class)
                    .and_then(|d| world.rigs.get(&npc_weapons::rig_key(&d.world_model)))
                {
                    for (name, event, options) in weapon.due_events(rig, now) {
                        // AE_NPC_MUZZLEFLASH is a client event (DispatchMuzzleEffect).
                        if name.eq_ignore_ascii_case("AE_NPC_MUZZLEFLASH") {
                            self.muzzle_flashes.push((id, options));
                        } else {
                            events.push((name, event));
                        }
                    }
                }
            }
            if !events.is_empty() {
                let knock_out = self.knock_out_player(id, world, scene, physics, player);
                let ctx = self.context(id, world, scene, player, now, 0.);
                let aim = self.weapon_aim(id, scene, player, now);
                let def = self.actors[&id]
                    .weapon
                    .as_ref()
                    .and_then(|w| self.weapon_def(&w.class));
                let mut state = self.random;
                let mut uniform = move || {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state >> 40) as f32 / (1u32 << 23) as f32 - 1.
                };
                let npc_class = world.entities.get(id).map_or("", |e| e.class());
                let actor = self.actors.get_mut(&id).expect("actor");
                let AiActor {
                    npc,
                    class,
                    weapon,
                    think_dt,
                    ..
                } = actor;
                let mut fired = false;
                let mut shot_sound_times = Vec::new();
                let mut reloaded = false;
                for (name, event) in events {
                    if !(3000..4000).contains(&event) {
                        // Class animation events.
                        if let Some(cop) = class.cop_mut() {
                            cop.anim_event(&name, event, &ctx, knock_out);
                        } else if let AiClass::Zombie(z) = class {
                            // CNPC_BaseZombie::HandleAnimEvent: claws and class sounds.
                            let rolls = ((uniform() + 1.) * 0.5, (uniform() + 1.) * 0.5);
                            if let Some((punch, impulse)) =
                                zombie::claw_punch(&name, ctx.yaw, rolls)
                            {
                                let yaw = ctx.yaw.to_radians();
                                z.requests.push(CopRequest::Claw {
                                    start: ctx.origin + Vec3::Z * 36.,
                                    forward: Vec3::new(yaw.cos(), yaw.sin(), 0.),
                                    punch,
                                    impulse,
                                    reach: z.kind.claw_reach(),
                                    bite: name.eq_ignore_ascii_case("AE_ZOMBIE_POISON_SPIT"),
                                });
                            } else if let Some(sound) = zombie::event_sound(&name) {
                                let cue = z.kind.cue(sound);
                                z.requests.push(CopRequest::Sound(cue));
                            }
                        } else if let AiClass::Humanoid(h) = class {
                            // CNPC_Combine::HandleAnimEvent COMBINE_AE_KICK (3); the other
                            // humanoid AE_* events are not modeled yet.
                            if h.kind == humanoid::Kind::Combine && event == COMBINE_AE_RELOAD {
                                // "We never actually run out of ammo, just need to refill
                                // the clip": RELOAD_NPC, both clips full, the ammo
                                // conditions cleared. Without it a soldier's clip stayed
                                // empty and he reloaded forever.
                                if let (Some(w), Some(d)) = (weapon.as_mut(), def.as_ref()) {
                                    w.clip1 = d.max_clip1;
                                    reloaded = true;
                                }
                                for c in [
                                    COND_LOW_PRIMARY_AMMO,
                                    COND_NO_PRIMARY_AMMO,
                                    COND_NO_SECONDARY_AMMO,
                                ] {
                                    npc.conditions.clear(c);
                                }
                            } else if h.kind == humanoid::Kind::Combine && event == COMBINE_AE_KICK
                            {
                                let yaw = ctx.yaw.to_radians();
                                h.requests.push(CopRequest::Kick {
                                    start: ctx.origin + Vec3::Z * 36.,
                                    forward: Vec3::new(yaw.cos(), yaw.sin(), 0.),
                                });
                                h.requests.push(CopRequest::Sentence("COMBINE_KICK"));
                            } else if h.kind == humanoid::Kind::Combine
                                && event == COMBINE_AE_GREN_TOSS
                            {
                                h.requests.push(CopRequest::GrenadeToss);
                            } else if name.eq_ignore_ascii_case("COMBINE_AE_BEGIN_ALTFIRE") {
                                h.requests
                                    .push(CopRequest::Sound("Weapon_CombineGuard.Special1"));
                            } else if name.eq_ignore_ascii_case("COMBINE_AE_ALTFIRE") && h.elite {
                                // EVENT_WEAPON_AR2_ALTFIRE on the active weapon, then
                                // DelaySquadAltFireAttack(10); m_iNumGrenades is not spent.
                                if weapon.as_ref().is_some_and(|w| w.class == "weapon_ar2") {
                                    h.requests.push(CopRequest::AltFire(h.alt_fire_target));
                                }
                                h.now = now;
                                h.delay_alt_fire(10.);
                            }
                        }
                        continue;
                    }
                    // Weapon events reach only the active weapon (Operator_HandleAnimEvent);
                    // without one they are dropped (an unarmed cop swings at the air).
                    let outcome = match (weapon.as_mut(), def.as_ref()) {
                        (Some(w), Some(d)) => w.handle_event(
                            event,
                            d,
                            id,
                            npc_class,
                            aim,
                            &mut uniform,
                            &mut self.shots,
                        ),
                        _ => WeaponEvent::None,
                    };
                    match outcome {
                        WeaponEvent::MeleeHit => {
                            if let Some(cop) = class.cop_mut() {
                                cop.anim_event(&name, event, &ctx, knock_out);
                            }
                        }
                        WeaponEvent::Reloaded | WeaponEvent::ReloadSound => reloaded = true,
                        WeaponEvent::Fired => {
                            fired = true;
                            if let (Some(w), Some(spec)) =
                                (weapon.as_mut(), def.as_ref().and_then(|d| d.fire))
                            {
                                if npc_weapons::machine_gun(&w.class) {
                                    shot_sound_times.extend(w.realtime_sound_times(
                                        now,
                                        *think_dt,
                                        spec.fire_rate,
                                    ));
                                } else {
                                    shot_sound_times.push(now);
                                }
                            }
                            self.diagnostics.shots_fired += 1;
                            // CWeaponAR2::FireNPCPrimaryAttack leaves DoMuzzleFlash out.
                            if weapon.as_ref().is_some_and(|w| w.class != "weapon_ar2") {
                                self.weapon_muzzle_flashes.push(id);
                            }
                            // Operator_HandleAnimEvent: InsertSound(SOUND_COMBAT |
                            // SOUND_CONTEXT_GUNFIRE, operator origin, SOUNDENT_VOLUME_*, 0.2).
                            if let Some(volume) = def
                                .as_ref()
                                .and_then(|d| d.fire)
                                .map(|f| f.sound_volume)
                                .filter(|&v| v > 0.)
                            {
                                self.sounds.insert(
                                    senses::sound::COMBAT,
                                    scene.states[id].origin,
                                    volume,
                                    0.2,
                                    Some(id),
                                    now,
                                );
                            }
                        }
                        _ => {}
                    }
                }
                self.random = self.random.wrapping_add(0x9e37_79b9_7f4a_7c15) | 1;
                // WeaponSound(SINGLE_NPC / RELOAD_NPC) at the operator; a machine gun's
                // later realtime sounds wait in `delayed_sounds`.
                if fired && shot_sound_times.is_empty() {
                    shot_sound_times.push(now);
                }
                let origin = scene.states[id].origin + Vec3::Z * 55.;
                for (times, key) in [
                    (shot_sound_times, "single_shot_npc"),
                    (if reloaded { vec![now] } else { Vec::new() }, "reload_npc"),
                ] {
                    let Some(cue) = def.as_ref().and_then(|d| d.sounds.get(key)) else {
                        continue;
                    };
                    for at in times {
                        let mut requests = vec![crate::sounds::SoundRequest {
                            origin: Some(origin),
                            ..cue.clone().into()
                        }];
                        // SINGLE_NPC plays on CHAN_WEAPON of the operator: a new shot
                        // sound replaces the previous one (native recording: run levels
                        // match a single attack, not stacked tails).
                        if key == "single_shot_npc" {
                            let emitter = crate::sounds::entity_emitter(id);
                            requests[0].emitter = Some(emitter);
                            requests.insert(0, crate::sounds::SoundRequest::stop(cue, emitter));
                        }
                        for request in requests {
                            if at <= now + 1e-9 {
                                scene.sounds.push(request);
                            } else {
                                self.delayed_sounds.push((at, request));
                            }
                        }
                    }
                }
                self.apply_requests(id, world, scene, physics, player);
            }
            if now + 1e-9 < think_at {
                continue;
            }
            self.think(
                id,
                world,
                scene,
                controller,
                physics,
                transients,
                player,
                precriminal,
            );
            self.apply_squad_events(scene);
            self.speak_idle_lines(world, scene);
        }
    }
    fn context(
        &mut self,
        id: usize,
        _world: &World,
        scene: &Scene,
        player: PlayerState,
        now: f64,
        _dt: f32,
    ) -> Context {
        let actor = &self.actors[&id];
        let enemy = actor.npc.enemy.map(|e| {
            let position = if e == PLAYER {
                player.feet
            } else {
                scene.states[e].origin
            };
            (e, position)
        });
        let enemy_eye = actor.npc.enemy.map(|e| {
            if e == PLAYER {
                player.eye()
            } else {
                scene.states[e].origin + Vec3::Z * super::host::EYE_HEIGHT
            }
        });
        let enemy_lkp = actor
            .npc
            .enemy
            .and_then(|e| actor.npc.enemies.find(Some(e)))
            .map(|m| m.last_known);
        let fire = actor
            .weapon
            .as_ref()
            .and_then(|w| self.weapon_def(&w.class))
            .and_then(|d| d.fire);
        let weapon_range = fire.map(|f| (f.min_range, f.max_range));
        let weapon_burst = fire.map(|f| (f.min_burst, f.max_burst, f.fire_rate));
        let is_cop = actor.class.cop().is_some();
        let cop_target = actor.class.cop().and_then(|c| c.target_entity);
        let random = self.random();
        Context {
            now,
            origin: scene.states[id].origin,
            enemy,
            // SetTarget(player): the cop's target is the player while precriminal; an
            // NPC enemy after SetTarget(GetEnemy()) in combat.
            target: match cop_target {
                Some(t) if t != PLAYER => scene.states.get(t).map(|s| s.origin),
                _ => (is_cop && !player.noclip).then_some(player.feet),
            },
            best_sound: None,
            player: (!player.noclip).then_some(player.feet),
            yaw: yaw_of(scene, id),
            random,
            enemy_eye,
            enemy_lkp,
            weapon_range,
            weapon_burst,
            best_sound_volume: 0.,
            // m_flDistTooFar (SF_NPC_LONG_RANGE is not read yet).
            dist_too_far: 1024.,
            enemy_sees_me: self.enemy_sees(id, scene, player),
        }
    }
    /// `enemy_sees_me` for the current enemy (see [`enemy_sees_me`]).
    fn enemy_sees(&self, id: usize, scene: &Scene, player: PlayerState) -> Option<bool> {
        let enemy = self.actors.get(&id)?.npc.enemy?;
        let fov = self.actors.get(&enemy).map(|a| a.class.field_of_view());
        enemy_sees_me(id, enemy, fov, scene, player)
    }
    #[allow(clippy::too_many_arguments)]
    fn think(
        &mut self,
        id: usize,
        world: &World,
        scene: &mut Scene,
        controller: &mut Controller,
        physics: &Physics,
        transients: &[ActorHull],
        player: PlayerState,
        precriminal: bool,
    ) {
        let now = scene.time;
        let mut ctx = self.context(id, world, scene, player, now, 0.);
        let origin = ctx.origin;
        // Speech dice (ShouldPlayIdleSound RandomInt(0, 99), IdleSound's choices,
        // the lost-sound delay U(5, 15)).
        let speech_rolls = [self.random(), self.random(), self.random(), self.random()];
        // FindHint's bits_HINT_NODE_RANDOM pick.
        let hint_roll = self.random();
        // The active gun's firing data and clip (GatherAttackConditions, CheckAmmo).
        let gun = self.actors[&id].weapon.as_ref().and_then(|w| {
            self.weapon_def(&w.class)
                .and_then(|d| d.fire)
                .map(|f| (f, w.clip1, w.class == "weapon_rpg", w.missile_out))
        });
        let eye = origin + Vec3::Z * 64.;
        let facing = scene.states[id].rotation * Vec3::X;
        let sight = PhysicsSight { physics, world };
        let class_name = world.entities.get(id).map_or("", |e| e.class());
        let my_class = classify(class_name, precriminal);
        let field_of_view = self.actors[&id].class.field_of_view();
        let sound_interests = self.actors[&id].class.sound_interests();
        let player_visible = !player.noclip
            && !player.dead
            && senses::in_view_cone(
                &Viewer {
                    entity: id,
                    class: my_class,
                    eye,
                    facing,
                    field_of_view,
                    look_distance: senses::LOOK_DISTANCE,
                },
                player.eye(),
            )
            && player.eye().distance(eye) <= senses::LOOK_DISTANCE
            && sight.visible(eye, player.eye(), &[id]);
        let fvisible = !player.noclip && sight.visible(eye, player.center(), &[id]);
        // NPC-vs-NPC sight (DESIGN 14n step 1): hated, live NPCs in the view cone and look
        // distance with LOS to their center.
        let npc_ids = self
            .npc_ids
            .get_or_insert_with(|| {
                world
                    .entities
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.class().starts_with("npc_"))
                    .map(|(i, _)| i)
                    .collect()
            })
            .clone();
        let viewer = Viewer {
            entity: id,
            class: my_class,
            eye,
            facing,
            field_of_view,
            look_distance: senses::LOOK_DISTANCE,
        };
        let seen_npcs: Vec<(usize, Vec3)> = npc_ids
            .into_iter()
            .filter(|&other| {
                let Some(state) = scene.states.get(other) else {
                    return false;
                };
                let center = state.origin + Vec3::Z * 36.;
                other != id
                    && !state.killed
                    && state.visible
                    && center.distance(eye) <= senses::LOOK_DISTANCE
                    && self.npc_disposition(world, id, other, precriminal) == Disposition::HT
                    && senses::in_view_cone(&viewer, center)
                    && sight.visible(eye, center, &[id, other])
            })
            .map(|other| (other, scene.states[other].origin))
            .collect();
        // CAI_PolicingBehavior::GatherConditions: the goal entity is gone (killed, e.g.
        // the can cop's OnKnockOut -> cupcop_policegoal Kill): the behavior disables.
        if let Some(cop) = self.actors.get_mut(&id).and_then(|a| a.class.cop_mut()) {
            if cop
                .policing
                .as_ref()
                .is_some_and(|p| scene.states.get(p.goal.entity).is_none_or(|s| s.killed))
            {
                cop.policing = None;
                cop.chase_until = 0.;
            }
        }
        // Policing goal state (the goal entity can change through inputs).
        let goal_entity = self.actors[&id]
            .class
            .cop()
            .and_then(|cop| cop.policing.as_ref().map(|p| p.goal.entity));
        let goal = goal_entity.and_then(|g| {
            let mut goal = self.goals.get(&g)?.clone();
            let target = Self::goal_target(world, scene, g);
            goal.target = target.map(|t| t.0);
            Some((goal, target))
        });
        let player_override = self.relationship_overrides.get(&(id, PLAYER)).copied();
        let attacker_disposition = self.actors[&id]
            .damaged_by
            .map(|a| self.npc_disposition(world, id, a, precriminal));
        let relationships = &self.relationships;
        let precriminal_hate = relationships
            .get(my_class, classify("player", precriminal))
            .0
            == Disposition::HT;
        let low_clip = self.actors[&id].weapon.as_ref().is_some_and(|w| {
            self.weapon_def(&w.class)
                .is_some_and(|d| d.fire.is_some() && w.clip1 > 0 && w.clip1 < d.max_clip1 / 4 + 1)
        });
        // Enemies' fields of view for TASK_ANNOUNCE_ATTACK (combine soldiers only), read
        // before this actor is borrowed: sensing below may pick a new enemy.
        let enemy_fovs: Vec<(usize, f32)> = if matches!(
            &self.actors[&id].class,
            AiClass::Humanoid(h) if h.kind == humanoid::Kind::Combine
        ) {
            self.actors
                .iter()
                .map(|(&e, a)| (e, a.class.field_of_view()))
                .collect()
        } else {
            Vec::new()
        };
        // IRelationType/IRelationPriority (and IsAlive) of every NPC this think may weigh
        // in BestEnemy: remembered ones, sighted ones and the attacker.
        let npc_relations: BTreeMap<usize, (Disposition, i32)> = {
            let actor = &self.actors[&id];
            actor
                .npc
                .enemies
                .known()
                .into_iter()
                .map(|(e, _)| e)
                .chain(seen_npcs.iter().map(|&(e, _)| e))
                .chain(actor.damaged_by)
                .filter(|&e| e != PLAYER)
                .filter(|&e| scene.states.get(e).is_some_and(|s| !s.killed && s.visible))
                .map(|e| {
                    let class =
                        classify(world.entities.get(e).map_or("", |x| x.class()), precriminal);
                    (
                        e,
                        (
                            self.npc_disposition(world, id, e, precriminal),
                            self.relationships.get(my_class, class).1,
                        ),
                    )
                })
                .collect()
        };
        let player_priority = self
            .relationships
            .get(my_class, classify("player", precriminal))
            .1;
        let actor = self.actors.get_mut(&id).expect("actor");
        actor.npc.conditions = Conditions::default();
        // CheckAmmo: an empty clip (NPCs have unlimited reserve ammunition), and
        // CAI_BaseHumanoid::CheckAmmo's low clip (below a quarter of the clip plus one).
        if gun.is_some_and(|(_, clip, _, _)| clip == 0) {
            actor.npc.conditions.set(COND_NO_PRIMARY_AMMO);
        } else if low_clip {
            actor.npc.conditions.set(COND_LOW_PRIMARY_AMMO);
        }
        if let Some(assault) = actor.class.assault_mut().as_mut() {
            if std::mem::take(&mut assault.provoke) {
                actor.npc.conditions.set(COND_PROVOKED);
            }
        }
        // The class's view of the player and its relationship toward him.
        let class_hates_player = match &mut actor.class {
            AiClass::Metropolice(cop) => {
                cop.scripted = false;
                cop.precriminal = precriminal;
                cop.now = now;
                cop.player = (!player.noclip && !player.dead).then(|| PlayerView {
                    feet: player.feet,
                    center: player.center(),
                    velocity: player.velocity,
                    visible: fvisible,
                    on_me: (player.feet.z - (origin.z + 72.)).abs() < 2.
                        && (player.feet - origin).truncate().length() < 32.,
                    on_ground: player.on_ground,
                    suit: player.suit,
                });
                if let (Some(policing), Some((goal, target))) = (&mut cop.policing, goal) {
                    policing.goal = goal;
                    policing.target_center =
                        target.map(|(t, at)| if t == PLAYER { player.center() } else { at });
                    policing.target_visible = if target.is_some_and(|t| t.0 == PLAYER) {
                        fvisible
                    } else {
                        true
                    };
                    // CAI_PolicingBehavior: SetTarget(goal->GetTarget()), the player even
                    // when notarget (his senses ignore him; the policing tasks still face
                    // him).
                    if target.is_some_and(|t| t.0 == PLAYER) {
                        ctx.target = Some(player.feet);
                    }
                }
                // Look: the player when hated, never while precriminal unless the cop is
                // chasing him.
                cop.hates_player(precriminal_hate || !precriminal, now)
            }
            AiClass::Humanoid(h) => {
                h.now = now;
                h.armed = actor.weapon.is_some();
                precriminal_hate
            }
            AiClass::Zombie(_) => precriminal_hate,
        };
        // A cop keeps the player as its enemy for the rest of a chase.
        let chasing = actor.class.cop().is_some_and(|cop| cop.chase_until > now);
        if actor.npc.state == NpcState::Script || actor.npc.state == NpcState::None {
            actor.npc.state = NpcState::Idle;
        }
        if player_visible {
            actor.npc.conditions.set(COND_SEE_PLAYER);
        }
        // F4 allegiance, then an ai_relationship toward !player, then the class rules.
        let hates = match actor.player_disposition.or(player_override) {
            Some(disposition) => disposition == Disposition::HT,
            None => class_hates_player,
        };
        let previous_enemy = actor.npc.enemy;
        // SetEnemy from the squad (SquadNewEnemy) since the last think.
        if std::mem::take(&mut actor.squad_new_enemy) {
            actor.npc.conditions.set(COND_NEW_ENEMY);
        }
        if hates && player_visible {
            actor.npc.conditions.set(COND_SEE_HATE);
            actor
                .npc
                .enemies
                .update(Some(PLAYER), player.feet, 0., true, now);
        }
        // CAI_BaseNPC::OnTakeDamage_Alive: any damage is light, more than 20 heavy; a
        // hated player attacker is remembered (UpdateEnemyMemory) and becomes the enemy
        // when there is none.
        let damage_taken = std::mem::take(&mut actor.damage_taken);
        if damage_taken > 0. {
            actor.npc.conditions.set(COND_LIGHT_DAMAGE);
        }
        if damage_taken > 20. {
            actor.npc.conditions.set(COND_HEAVY_DAMAGE);
        }
        if std::mem::take(&mut actor.damaged_by_player) && hates && !player.noclip {
            actor
                .npc
                .enemies
                .update(Some(PLAYER), player.feet, 0., true, now);
        }
        // Damage from an NPC: COND_LIGHT_DAMAGE, the attacker remembered where it stands
        // (unless liked); ChooseEnemy weighs it.
        if let Some(attacker) = actor.damaged_by.take() {
            actor.npc.conditions.set(COND_LIGHT_DAMAGE);
            if let Some(state) = scene.states.get(attacker).filter(|s| !s.killed) {
                if attacker_disposition != Some(Disposition::LI) {
                    actor
                        .npc
                        .enemies
                        .update(Some(attacker), state.origin, 0., true, now);
                }
            }
        }
        // Hated NPCs in sight: memory (BestEnemy below weighs them).
        for &(other, feet) in &seen_npcs {
            actor.npc.conditions.set(COND_SEE_HATE);
            actor.npc.enemies.update(Some(other), feet, 0., true, now);
        }
        actor.npc.enemies.refresh(
            now,
            |e| {
                if e == PLAYER {
                    Some(player.feet)
                } else {
                    scene.states.get(e).map(|s| s.origin)
                }
            },
            |e| {
                if e == PLAYER {
                    player.dead
                } else {
                    scene.states.get(e).is_none_or(|s| s.killed)
                }
            },
        );
        // ChooseEnemy (GatherConditions after RefreshMemories): only when the running
        // schedule can be interrupted by COND_NEW_ENEMY (or COND_LOST_ENEMY with an eluded
        // enemy), or there is none; ShouldChooseNewEnemy, then BestEnemy over the memory.
        // A killed NPC enemy is gone at once (its entity no longer thinks), like an
        // EHANDLE that went NULL.
        {
            let relation = |e: usize| -> super::enemy_choice::Relation {
                if e == PLAYER {
                    // FL_NOTARGET (noclip here) and IsAlive; the policing chase keeps him.
                    (!player.noclip && !player.dead).then_some((
                        if hates || chasing {
                            Disposition::HT
                        } else {
                            Disposition::NU
                        },
                        player_priority,
                    ))
                } else {
                    npc_relations.get(&e).copied()
                }
            };
            let initial = actor.npc.enemy;
            let went_null = initial.is_some_and(|e| {
                e != PLAYER && scene.states.get(e).is_none_or(|s| s.killed || !s.visible)
            });
            let eluded = went_null
                || initial
                    .is_some_and(|e| actor.npc.enemies.find(Some(e)).is_some_and(|m| m.eluded));
            let schedule = actor
                .npc
                .schedule
                .as_ref()
                .and_then(|n| self.schedules.0.get(n));
            let (new_enemy_interrupts, lost_enemy_interrupts) = match schedule {
                Some(s) if actor.npc.task < s.tasks.len() => {
                    let custom = actor.class.behavior_mut().custom_interrupts(&actor.npc);
                    let all = s.interrupts.union(custom);
                    (all.has(COND_NEW_ENEMY), all.has(COND_LOST_ENEMY))
                }
                _ => (true, true),
            };
            let skip = !went_null && !new_enemy_interrupts && !(lost_enemy_interrupts && eluded);
            if !skip {
                let c = actor.npc.conditions;
                let sees_hostile = c.has(COND_SEE_HATE) || c.has(COND_SEE_FEAR);
                let mut chosen = if went_null { None } else { initial };
                // ForceChooseNewEnemy (RememberUnreachable of the enemy).
                if std::mem::take(&mut actor.npc.force_choose_enemy) {
                    actor.choose.serial = -1;
                }
                if super::enemy_choice::should_choose_new_enemy(
                    chosen,
                    &actor.npc.enemies,
                    &mut actor.choose,
                    chosen.and_then(relation),
                    sees_hostile,
                    false,
                    now,
                ) {
                    let position = |e: usize| {
                        if e == PLAYER {
                            Some(player.feet)
                        } else {
                            scene.states.get(e).map(|s| s.origin)
                        }
                    };
                    let npc = &actor.npc;
                    chosen = super::enemy_choice::best_enemy(
                        &npc.enemies,
                        now,
                        relation,
                        |e| position(e).is_some_and(|p| npc.unreachable_now(e, p, now)),
                        |e| position(e).map_or(f32::MAX, |p| p.distance_squared(origin)),
                        |e| {
                            if e == PLAYER {
                                player_visible
                            } else {
                                seen_npcs.iter().any(|&(o, _)| o == e)
                            }
                        },
                        |e| {
                            if e == PLAYER {
                                player.center().distance(origin) < senses::LOOK_DISTANCE && fvisible
                            } else {
                                position(e).is_some_and(|p| {
                                    let center = p + Vec3::Z * 36.;
                                    center.distance(origin) < senses::LOOK_DISTANCE
                                        && sight.visible(eye, center, &[id, e])
                                })
                            }
                        },
                    );
                }
                if chosen != initial || went_null {
                    if went_null {
                        actor.npc.conditions.set(COND_ENEMY_DEAD);
                    }
                    actor.npc.enemy = chosen;
                    // Forget(bits_MEMORY_HAD_LOS): the LOS outputs fire again.
                    actor.npc.had_enemy_los = false;
                    if chosen.is_none() && !went_null {
                        actor.npc.conditions.set(COND_ENEMY_WENT_NULL);
                        if eluded {
                            actor.npc.conditions.set(COND_LOST_ENEMY);
                        }
                    }
                    if chosen.is_none() {
                        let requests = actor.class.requests_mut();
                        if initial == Some(PLAYER) {
                            requests.push(CopRequest::Output("OnLostPlayer"));
                        }
                        if initial.is_some() {
                            requests.push(CopRequest::Output("OnLostEnemy"));
                        }
                    }
                }
            }
        }
        // UpdateEnemyMemory first-hand -> CAI_Squad::UpdateEnemyMemory to the others.
        if let Some(squad) = &actor.squad {
            for (enemy, position) in actor.npc.enemies.seen_at(now) {
                self.squad_events.push(SquadEvent::Memory {
                    squad: squad.clone(),
                    from: id,
                    enemy,
                    position,
                });
            }
        }
        let has_baton = actor.class.cop().is_some_and(|cop| cop.has_baton);
        // bits_CAP_INNATE_MELEE_ATTACK1: the combine soldier's kick.
        let innate_kick = actor
            .class
            .humanoid()
            .is_some_and(|h| h.kind == humanoid::Kind::Combine);
        // bits_CAP_INNATE_MELEE_ATTACK1: zombie claws.
        let claws = match &actor.class {
            AiClass::Zombie(z) => Some(z.kind.claw_reach()),
            _ => None,
        };
        if let Some(enemy) = actor.npc.enemy {
            if previous_enemy != Some(enemy) {
                actor.npc.conditions.set(COND_NEW_ENEMY);
            }
            let seen = if enemy == PLAYER {
                player_visible
            } else {
                seen_npcs.iter().any(|&(o, _)| o == enemy)
            };
            // The enemy's feet, center, eyes and velocity (the player's view or an NPC's
            // standing hull).
            let (t_feet, t_center, t_eye, t_velocity) = if enemy == PLAYER {
                (player.feet, player.center(), player.eye(), player.velocity)
            } else {
                let o = scene.states[enemy].origin;
                (o, o + Vec3::Z * 36., o + Vec3::Z * 64., Vec3::ZERO)
            };
            // GatherEnemyConditions: OnFoundPlayer/OnFoundEnemy when LOS to the enemy
            // is gained (bits_MEMORY_HAD_LOS), OnLostPlayerLOS/OnLostEnemyLOS when lost.
            let los = enemy == PLAYER && fvisible;
            if los != actor.npc.had_enemy_los {
                actor.npc.had_enemy_los = los;
                let (a, b) = if los {
                    ("OnFoundPlayer", "OnFoundEnemy")
                } else {
                    ("OnLostPlayerLOS", "OnLostEnemyLOS")
                };
                let requests = actor.class.requests_mut();
                requests.push(CopRequest::Output(a));
                requests.push(CopRequest::Output(b));
            }
            if seen {
                actor.npc.conditions.set(COND_SEE_ENEMY);
                // CWeaponStunStick::WeaponMeleeAttack1Condition (player in 48 units in
                // 0.35 s, within 70 of height and facing within 0.7).
                let delta = t_center - (origin + Vec3::Z * 36.);
                let forward = facing.truncate().normalize_or_zero();
                let projected = (t_feet + t_velocity * 0.35 - origin).truncate();
                if has_baton {
                    if delta.z.abs() > 70. {
                        actor.npc.conditions.set(COND_TOO_FAR_TO_ATTACK);
                    } else if delta.truncate().normalize_or_zero().dot(forward) < 0.7 {
                        actor.npc.conditions.set(COND_NOT_FACING_ATTACK);
                    } else if projected.length() <= 48. || delta.truncate().length() <= 48. {
                        actor.npc.conditions.set(COND_CAN_MELEE_ATTACK1);
                    } else {
                        actor.npc.conditions.set(COND_TOO_FAR_TO_ATTACK);
                    }
                } else if let Some((spec, clip, rpg, missile_out)) = gun {
                    // GatherAttackConditions: the weapon's LOS from Weapon_ShootPosition to
                    // the enemy's eyes, else its BodyTarget, and
                    // CBaseCombatWeapon::WeaponRangeAttack1Condition(flDot, flDist).
                    // WeaponLOSCondition sets COND_WEAPON_SIGHT_OCCLUDED when the world
                    // blocks the shot (friends do not block our sight traces yet).
                    let shoot = origin + Vec3::Z * 55.;
                    let los = sight.visible(shoot, t_eye, &[id, enemy])
                        || sight.visible(shoot, t_center, &[id, enemy]);
                    if los {
                        actor.npc.conditions.set(COND_WEAPON_HAS_LOS);
                    } else {
                        actor.npc.conditions.set(COND_WEAPON_SIGHT_OCCLUDED);
                    }
                    let to = t_feet - origin;
                    let dot = to.truncate().normalize_or_zero().dot(forward);
                    // CWeaponRPG::WeaponRangeAttack1Condition ignores vertical distance.
                    let distance = if rpg {
                        to.truncate().length()
                    } else {
                        to.length()
                    };
                    // CWeaponSMG1::Equip: allies reach 3000 units.
                    let max_range = npc_weapons::equip_max_range(
                        actor.weapon.as_ref().map_or("", |w| w.class.as_str()),
                        my_class == Class::PlayerAlly,
                        &spec,
                    );
                    // The RPG: nothing while its missile flies; a 48-unit hull must clear
                    // 120 units along the shot (COND_WEAPON_SIGHT_OCCLUDED otherwise).
                    let rpg_blocked = rpg && distance >= spec.min_range && {
                        let start = origin + Vec3::Z * 55.;
                        let direction = (t_center - start).normalize_or_zero();
                        physics
                            .projectile_sweep(
                                start,
                                start + direction * 120.,
                                crate::physics::ProjectileHull::Box(Vec3::splat(24.)),
                                &[id],
                            )
                            .is_some()
                    };
                    let condition = if rpg && missile_out {
                        COND_NONE
                    } else if rpg_blocked {
                        COND_WEAPON_SIGHT_OCCLUDED
                    } else if clip == 0 {
                        COND_NO_PRIMARY_AMMO
                    } else if distance < spec.min_range {
                        COND_TOO_CLOSE_TO_ATTACK
                    } else if distance > max_range {
                        COND_TOO_FAR_TO_ATTACK
                    } else if dot < 0.5 {
                        COND_NOT_FACING_ATTACK
                    } else {
                        COND_CAN_RANGE_ATTACK1
                    };
                    if condition != COND_NONE && (condition != COND_CAN_RANGE_ATTACK1 || los) {
                        actor.npc.conditions.set(condition);
                    }
                }
                // CNPC_Combine::MeleeAttack1Conditions: within 64 units, facing within
                // 0.7, no more than 64 units of height apart, never a headcrab, and a
                // center-to-center line that reaches the enemy.
                if innate_kick {
                    let delta = t_center - (origin + Vec3::Z * 36.);
                    let dot = delta
                        .truncate()
                        .normalize_or_zero()
                        .dot(facing.truncate().normalize_or_zero());
                    let headcrab = enemy != PLAYER
                        && world
                            .entities
                            .get(enemy)
                            .is_some_and(|e| e.class().starts_with("npc_headcrab"));
                    if delta.length() <= 64.
                        && dot >= 0.7
                        && (t_feet.z - origin.z).abs() <= 64.
                        && !headcrab
                        && sight.visible(origin + Vec3::Z * 36., t_center, &[id, enemy])
                    {
                        actor.npc.conditions.set(COND_CAN_MELEE_ATTACK1);
                    }
                }
                // CNPC_BaseZombie::MeleeAttack1Conditions: within the claw reach (origin
                // to origin), facing within 0.7, and the cube hull swept along the facing
                // meets the enemy (an NPC or breakable in the way is not modeled).
                if let Some(reach) = claws {
                    let to = t_feet - origin;
                    let forward = facing.truncate().normalize_or_zero().extend(0.);
                    let dot = to.truncate().normalize_or_zero().dot(forward.truncate());
                    let start = origin + Vec3::Z * 36.;
                    let end = start + forward * reach;
                    let half = if enemy == PLAYER { 16. } else { 13. };
                    let cube = Vec3::splat(13.);
                    let reaches = super::metropolice::segment_hits_box(
                        start,
                        end,
                        t_feet + Vec3::new(-half, -half, 0.) - cube,
                        t_feet + Vec3::new(half, half, 72.) + cube,
                    );
                    let distance = to.length() * zombie::CLAW_REACH / reach;
                    actor
                        .npc
                        .conditions
                        .set(zombie::melee_condition(distance, dot, reaches));
                }
            } else {
                actor.npc.conditions.set(COND_ENEMY_OCCLUDED);
            }
            // GatherEnemyConditions: not seen for 8 s and standing at its last known
            // position (2D within 48 units) without seeing it: the enemy eluded me
            // (MarkEnemyAsEluded; the next ChooseEnemy drops it with COND_LOST_ENEMY).
            // An unreachable enemy (on the node network) raises COND_ENEMY_UNREACHABLE.
            let enemy_position = if enemy == PLAYER {
                player.feet
            } else {
                scene.states[enemy].origin
            };
            let on_network = controller.graph().is_some_and(|g| !g.nodes.is_empty());
            let unreachable = on_network && actor.npc.is_unreachable(enemy, enemy_position, now);
            if unreachable {
                actor.npc.conditions.set(COND_ENEMY_UNREACHABLE);
            }
            if now - actor.npc.enemies.last_time_seen(Some(enemy)) > 8. && !seen {
                if let Some(lkp) = actor.npc.enemies.find(Some(enemy)).map(|m| m.last_known) {
                    if (lkp - origin).truncate().length() < 48. {
                        actor.npc.enemies.mark_eluded(Some(enemy));
                    }
                    // Unreachable, unseen and its last known position out of sight: eluded.
                    if unreachable && !sight.visible(eye, lkp, &[id]) {
                        actor.npc.enemies.mark_eluded(Some(enemy));
                    }
                }
            }
        }
        // CNPC_Combine grenades without a squad: CanGrenadeEnemy(false) (the flush, at the
        // last seen position) and CanGrenadeEnemy() (the last known position, e.g. the 20%
        // grenade before reloading) -> CanThrowGrenade -> CheckCanThrowGrenade (VecCheckThrow
        // at 650 in view and visible, else a high VecCheckToss with 64 units of headroom).
        // Probed here without side effects; the class applies them when it asks. Squad
        // slots and the 4-unit hull of the throw traces are not modeled.
        let moving = scene.states[id].is_moving();
        let enemy_now = actor.npc.enemy.map(|e| {
            if e == PLAYER {
                player.feet
            } else {
                scene.states[e].origin
            }
        });
        let seen_enemy = actor.npc.conditions.has(COND_SEE_ENEMY);
        let memory = actor
            .npc
            .enemy
            .and_then(|e| actor.npc.enemies.find(Some(e)))
            .map(|m| (m.time_last_seen, m.last_seen_position, m.last_known));
        if let AiClass::Humanoid(h) = &mut actor.class {
            // CWeaponRPG state the citizen's RPG schedules read (m_hMissile).
            let rpg = actor.weapon.as_ref().filter(|w| w.class == "weapon_rpg");
            h.rpg = rpg.is_some();
            h.missile_out = rpg.is_some_and(|w| w.missile_out);
            h.shoot_position = origin + Vec3::Z * 55.;
            // The previous think's CanGrenadeEnemy outcomes.
            for outcome in h.grenade_outcomes.drain(..) {
                *self
                    .diagnostics
                    .grenade_checks
                    .entry(outcome.into())
                    .or_default() += 1;
            }
            h.throw_probes = Default::default();
            h.flush_window = false;
            // CanAltFireEnemy(true)'s trace: the enemy (the player or a player ally) at its
            // last known position plus 0.75 of its view offset, a 24-unit hull from the
            // shoot position along MASK_SHOT; valid when it travels >= 65% and > 128 units.
            h.alt_fire_probe = None;
            if h.elite {
                let ally = |e: usize| {
                    e == PLAYER
                        || world.entities.get(e).is_some_and(|ent| {
                            matches!(
                                classify(ent.class(), precriminal),
                                Class::PlayerAlly | Class::PlayerAllyVital
                            )
                        })
                };
                if let (Some(enemy), Some((_, _, last_known))) = (actor.npc.enemy, memory) {
                    if ally(enemy) {
                        let target = last_known + Vec3::Z * 64. * 0.75;
                        let start = origin + Vec3::Z * 55.;
                        let length = start.distance(target);
                        let travelled = physics
                            .projectile_sweep(
                                start,
                                target,
                                crate::physics::ProjectileHull::Box(Vec3::splat(12.)),
                                &[id],
                            )
                            .map_or(length, |hit| hit.position.distance(start));
                        if length > 0. && travelled / length >= 0.65 && travelled > 128. {
                            h.alt_fire_probe = Some(target);
                        }
                    }
                }
            }
            if let (true, Some((last_time, last_seen, last_known))) =
                (h.kind == humanoid::Kind::Combine && h.grenades > 0, memory)
            {
                h.flush_window = !seen_enemy
                    && now - last_time <= 3.
                    && enemy_now.is_some_and(|p| p.distance(last_seen) <= 256.);
                let probe = |target: Vec3| {
                    if !(128. ..=1024.).contains(&target.distance(origin)) {
                        return humanoid::ThrowProbe::OutOfRange;
                    }
                    if moving {
                        return humanoid::ThrowProbe::Moving;
                    }
                    let in_view =
                        senses::in_view_cone(&viewer, target) && sight.visible(eye, target, &[id]);
                    let bits = ((now * 1000.) as u64) ^ (id as u64).wrapping_mul(0x9e37_79b9);
                    let jitter = [0u64, 1, 2, 3].map(|i| {
                        (bits
                            .wrapping_mul(6_364_136_223_846_793_005)
                            .wrapping_add(i.wrapping_mul(1_442_695_040_888_963_407))
                            >> 40) as f32
                            / (1u64 << 24) as f32
                    });
                    let toss = if in_view {
                        vec_check_throw(physics, id, eye, target, 650.)
                    } else if physics
                        .projectile_ray(eye, eye + Vec3::Z * 64., &[id])
                        .is_none()
                    {
                        vec_check_toss(physics, id, eye, target, facing, jitter)
                    } else {
                        None
                    };
                    match toss {
                        Some(velocity) => humanoid::ThrowProbe::Ready { velocity, in_view },
                        None => humanoid::ThrowProbe::Blocked { in_view },
                    }
                };
                if now >= h.next_grenade_check {
                    if h.flush_window {
                        h.throw_probes.last_seen = Some(probe(last_seen));
                    }
                    h.throw_probes.last_known = Some(probe(last_known));
                }
            }
        }
        // Speech (CAI_BaseNPC sound hooks; queued like the class's own sentences).
        {
            let mut lines: Vec<&'static str> = Vec::new();
            let c = actor.npc.conditions;
            let player_criminal = !precriminal;
            let cop = actor.class.cop().is_some();
            // LostEnemySound when the enemy eludes (COND_LOST_ENEMY); FoundEnemySound when
            // it is seen again (UpdateEnemyMemory of an eluded enemy). Cops speak these
            // only when the player is a criminal.
            if c.has(COND_LOST_ENEMY) && !actor.enemy_eluded {
                actor.enemy_eluded = true;
                actor.npc.enemies.mark_eluded(actor.npc.enemy);
                if now > actor.next_lost_sound && (!cop || player_criminal) {
                    let long = actor.npc.enemy.is_none()
                        || now - actor.npc.enemies.last_time_seen(actor.npc.enemy) > 10.;
                    if let Some(g) = speech_group(
                        &actor.class,
                        if long {
                            Speech::LostLong
                        } else {
                            Speech::LostShort
                        },
                    ) {
                        lines.push(g);
                        actor.next_lost_sound = now + 5. + 10. * f64::from(speech_rolls[2]);
                    }
                }
            } else if actor.enemy_eluded && (c.has(COND_SEE_ENEMY) || c.has(COND_SEE_HATE)) {
                actor.enemy_eluded = false;
                if let Some(g) = speech_group(&actor.class, Speech::Found) {
                    lines.push(g);
                }
            }
            // CNPC_Combine::SelectIdealState: AnnounceEnemyKill on COND_ENEMY_DEAD.
            if c.has(COND_ENEMY_DEAD) {
                if let Some(g) = speech_group(&actor.class, Speech::KilledEnemy) {
                    lines.push(g);
                }
            }
            // CAI_PlayerAlly: TLK_STARTCOMBAT (GatherEnemyConditions: an enemy seen after
            // 30 s without one) and TLK_ENEMY_DEAD (OnKilledNPC; here when the enemy died).
            let mut concepts: Vec<&'static str> = Vec::new();
            if matches!(&actor.class, AiClass::Humanoid(h) if matches!(h.kind, humanoid::Kind::Companion { .. }))
            {
                if self
                    .ally
                    .start_combat(id, actor.npc.enemy.is_some(), c.has(COND_SEE_ENEMY), now)
                {
                    concepts.push("TLK_STARTCOMBAT");
                }
                if c.has(COND_ENEMY_DEAD) {
                    concepts.push("TLK_ENEMY_DEAD");
                }
            }
            actor
                .class
                .requests_mut()
                .extend(concepts.into_iter().map(CopRequest::Concept));
            // ShouldPlayIdleSound: idle or alert, 1 in 100 per think, not gagged.
            let gagged = world.entities.get(id).is_some_and(|e| {
                e.get("spawnflags")
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .is_some_and(|f| f & SF_NPC_GAG != 0)
            });
            let idle_or_alert = matches!(actor.npc.state, NpcState::Idle | NpcState::Alert);
            // GatherConditions samples (and rolls IdleSound) only with a player in the
            // NPC's PVS, or in combat, once per decision (UpdateEfficiency's interval):
            // cops out of sight never chatter and far ones roll less often.
            let in_pvs = scene.look_targets.in_pvs(id);
            let to_npc = origin - player.eye();
            let player_facing = in_pvs && player.facing.dot(to_npc.normalize_or_zero()) > 0.;
            let decision_due = now >= actor.next_idle_decision;
            if decision_due {
                let efficiency = super::efficiency::efficiency(
                    actor.npc.state,
                    in_pvs,
                    player_facing,
                    to_npc.length(),
                );
                actor.next_idle_decision = now + efficiency.decision_interval() - 1e-6;
            }
            let samples = decision_due && (in_pvs || actor.npc.state == NpcState::Combat);
            // CNPC_MetroPolice::ShouldPlayIdleSound: a cop a squadmate asked answers as soon
            // as FOkToMakeSound allows (the squad's gate, else his own; never gagged).
            let cop_answers = cop && idle_or_alert && actor.idle_chatter >= COP_CHATTER_RESPONSE;
            let should_play = if !samples {
                false
            } else if cop_answers {
                !gagged
                    && match actor.squad.as_ref().and_then(|n| self.squads.get(n)) {
                        Some(squad) => squad.ok_to_make_sound(0, now),
                        None => self
                            .sound_wait
                            .get(&id)
                            .is_none_or(|&(until, last)| now > until || 0 > last),
                    }
            } else {
                idle_or_alert && (speech_rolls[0] * 100.) as u32 == 0 && !gagged
            };
            if should_play {
                let pick = speech_rolls[1];
                let choice = ((speech_rolls[3] * 3.) as u8).min(2);
                let mut idle = |group: &'static str, then: IdleThen| {
                    self.idle_lines.push(IdleLine { id, group, then });
                };
                match &actor.class {
                    // CNPC_Combine::IdleSound: a check-in or question the next idle soldier
                    // answers, or a statement (g_fCombineQuestion, set when spoken).
                    AiClass::Humanoid(h) if h.kind == humanoid::Kind::Combine => {
                        match self.combine_question {
                            0 if pick < 0.5 => {}
                            0 => idle(
                                ["COMBINE_CHECK", "COMBINE_QUEST", "COMBINE_IDLE"]
                                    [usize::from(choice)],
                                IdleThen::CombineQuestion([1, 2, 0][usize::from(choice)]),
                            ),
                            1 => idle("COMBINE_CLEAR", IdleThen::CombineQuestion(0)),
                            _ => idle("COMBINE_ANSWER", IdleThen::CombineQuestion(0)),
                        }
                    }
                    // CNPC_MetroPolice::IdleSound: a precriminal player within 128 units is
                    // harassed once (bits_MEMORY_PLAYER_HARASSED, shared by the squad); else
                    // half the time a squad cop asks a check-in or question his squadmates
                    // answer (g_interactionMetrocopIdleChatter), or anyone says an idle line.
                    AiClass::Metropolice(_) => match actor.idle_chatter {
                        COP_CHATTER_WAIT_FOR_RESPONSE => {}
                        COP_CHATTER_ASK_QUESTION => {
                            let near = !player_criminal
                                && (player.center() - (origin + Vec3::Z * 36.)).length_squared()
                                    < 128. * 128.;
                            if near && !actor.harassed {
                                idle("METROPOLICE_IDLE_HARASS_PLAYER", IdleThen::CopHarassed);
                            } else if pick >= 0.5 {
                                let criminal = usize::from(player_criminal);
                                if actor.squad.is_none() || choice == 2 {
                                    idle(
                                        ["METROPOLICE_IDLE", "METROPOLICE_IDLE_CR"][criminal],
                                        IdleThen::Nothing,
                                    );
                                } else {
                                    idle(
                                        [
                                            ["METROPOLICE_IDLE_CHECK", "METROPOLICE_IDLE_QUEST"],
                                            [
                                                "METROPOLICE_IDLE_CHECK_CR",
                                                "METROPOLICE_IDLE_QUEST_CR",
                                            ],
                                        ][criminal][usize::from(choice)],
                                        IdleThen::CopAsked(choice),
                                    );
                                }
                            }
                        }
                        response => idle(
                            [
                                ["METROPOLICE_IDLE_CLEAR", "METROPOLICE_IDLE_ANSWER"],
                                ["METROPOLICE_IDLE_CLEAR_CR", "METROPOLICE_IDLE_ANSWER_CR"],
                            ][usize::from(player_criminal)]
                                [usize::from((response - COP_CHATTER_RESPONSE).min(1))],
                            IdleThen::CopAnswered,
                        ),
                    },
                    _ => {}
                }
            }
            actor
                .class
                .requests_mut()
                .extend(lines.into_iter().map(CopRequest::Sentence));
        }
        // Listen.
        let (heard, best) = senses::listen(eye, id, sound_interests, &self.sounds);
        actor.npc.conditions = actor.npc.conditions.union(heard);
        ctx.best_sound = best.map(|s| s.origin);
        ctx.best_sound_volume = best.map_or(0., |s| s.volume);
        // SelectIdealState: squad members idle or alert share a new enemy first.
        if let (Some(squad), Some(enemy)) = (&actor.squad, actor.npc.enemy) {
            if matches!(actor.npc.state, NpcState::Idle | NpcState::Alert)
                && actor.npc.conditions.has(COND_NEW_ENEMY)
            {
                self.squad_events.push(SquadEvent::NewEnemy {
                    squad: squad.clone(),
                    enemy,
                });
            }
        }
        let (ideal, _) = select_ideal_state(
            actor.npc.state,
            &StateInputs {
                conditions: actor.npc.conditions,
                has_enemy: actor.npc.enemy.is_some(),
                since_unknown_enemy: 999.,
                threat: None,
                best_sound: best.map(|s| {
                    (
                        s.origin,
                        s.kind
                            & (senses::sound::COMBAT
                                | senses::sound::DANGER
                                | senses::sound::BULLET_IMPACT)
                            != 0,
                    )
                }),
                // CAI_BaseNPC::ShouldGoToIdleState is false for cops: alert returns to
                // idle only through TASK_SUGGEST_STATE (SCHED_ALERT_STAND).
                go_idle: false,
                ideal: match actor.npc.ideal_state {
                    NpcState::None | NpcState::Script => NpcState::Idle,
                    other => other,
                },
            },
        );
        // SetIdealState(SelectIdealState()): later thinks fall back to this ideal.
        actor.npc.ideal_state = ideal;
        if ideal != actor.npc.state {
            actor.npc.state = ideal;
            actor.npc.schedule = None;
        }
        ctx.enemy = actor.npc.enemy.map(|e| {
            (
                e,
                if e == PLAYER {
                    player.feet
                } else {
                    scene.states[e].origin
                },
            )
        });
        ctx.enemy_sees_me = actor.npc.enemy.and_then(|e| {
            let fov = enemy_fovs.iter().find(|(id, _)| *id == e).map(|f| f.1);
            enemy_sees_me(id, e, fov, scene, player)
        });
        if let AiClass::Metropolice(cop) = &mut actor.class {
            cop.gather_conditions(&mut actor.npc, &ctx);
        }
        let dt = (now - actor.last_think).clamp(0., 0.2) as f32;
        actor.last_think = now;
        if dt > 0. {
            actor.think_dt = dt;
        }
        actor.origin = origin.to_array();
        actor.yaw = yaw_of(scene, id);
        actor.next_think = now + THINK_INTERVAL;
        // CZombie::PrescheduleThink's idle moans.
        // CAI_ActBusyBehavior::FindActBusyHintNode for a behavior about to search.
        if let AiClass::Metropolice(cop) = &mut actor.class {
            cop.actbusy.candidate = None;
            if cop.actbusy.wants_hint(now) {
                let group = world
                    .entities
                    .get(id)
                    .and_then(|e| e.get("hintgroup"))
                    .unwrap_or("")
                    .trim();
                let state = match actor.npc.state {
                    NpcState::Combat => 3,
                    NpcState::Alert => 2,
                    _ => 1,
                };
                let locks = &self.hint_locks;
                cop.actbusy.candidate = super::actbusy::find_hint(
                    &self.busy_hints,
                    &self.busy_anims,
                    group,
                    origin,
                    cop.actbusy.search_range,
                    state,
                    |hint| locks.get(&hint).is_some_and(|&holder| holder != id),
                    cop.actbusy.use_nearest,
                    hint_roll,
                );
            }
        }
        if let AiClass::Zombie(z) = &mut actor.class {
            z.moan(now, actor.npc.state == NpcState::Combat);
        }
        // MaxYawSpeed and NPC_TranslateActivity's ACT_IDLE_ANGRY by class.
        let weapon_drawn = actor.weapon.is_some();
        let (yaw_speed, idle_angry) = match &actor.class {
            AiClass::Metropolice(cop) => (
                if actor.npc.activity.starts_with("ACT_WALK") {
                    25.
                } else if actor.npc.activity.starts_with("ACT_RUN") {
                    15.
                } else {
                    45.
                },
                weapon_drawn && (actor.npc.state == NpcState::Combat || cop.baton_active),
            ),
            AiClass::Humanoid(h) => (
                h.kind.max_yaw_speed(&actor.npc.activity),
                weapon_drawn && h.idle_angry(actor.npc.state, now),
            ),
            AiClass::Zombie(_) => (zombie::max_yaw_speed(&actor.npc.activity), false),
        };
        let on_fire = matches!(&actor.class, AiClass::Zombie(_)) && scene.fires.burning(id);
        let mut motor = SceneMotor {
            entity: id,
            world,
            scene,
            controller,
            physics,
            transients,
            goal: &mut actor.goal,
            serial: &mut self.serial,
            dt: if dt > 0. { dt } else { THINK_INTERVAL as f32 },
            yaw_speed,
            idle_angry,
            weapon: actor.weapon.as_ref().map(|w| w.class.clone()),
            combine: matches!(&actor.class, AiClass::Humanoid(h) if h.kind == humanoid::Kind::Combine),
            walk_only: matches!(&actor.class, AiClass::Zombie(_)),
            on_fire,
            weapon_activity: None,
            blocked_moves: &mut self.diagnostics.blocked_moves,
        };
        actor.npc.maintain(
            &self.schedules,
            actor.class.behavior_mut(),
            &ctx,
            &mut motor,
        );
        // CAI_PlayerAlly::OnStartSchedule: TLK_HIDEANDRELOAD.
        if matches!(&actor.class, AiClass::Humanoid(h) if matches!(h.kind, humanoid::Kind::Companion { .. }))
            && self
                .ally
                .schedule_started(id, actor.npc.schedule.as_deref())
            && actor.npc.schedule.as_deref() == Some("SCHED_HIDE_AND_RELOAD")
        {
            actor
                .class
                .requests_mut()
                .push(CopRequest::Concept("TLK_HIDEANDRELOAD"));
        }
        // StartTaskOverlay / RunTaskOverlay: move and shoot (bits_CAP_MOVE_SHOOT: the
        // soldier, the companions and non-efficient cops).
        {
            let schedule = actor.npc.schedule.clone();
            let task_name = schedule.as_deref().and_then(|s| {
                self.schedules
                    .0
                    .get(s)
                    .and_then(|sc| sc.tasks.get(actor.npc.task))
                    .map(|(t, _)| t.clone())
            });
            let combine =
                matches!(&actor.class, AiClass::Humanoid(h) if h.kind == humanoid::Kind::Combine);
            // CNPC_Combine::ShouldMoveAndShoot: not while fleeing a danger sound.
            let allowed = !(combine
                && matches!(
                    schedule.as_deref(),
                    Some(
                        "SCHED_TAKE_COVER_FROM_BEST_SOUND"
                            | "SCHED_COMBINE_TAKE_COVER_FROM_BEST_SOUND"
                            | "SCHED_COMBINE_RUN_AWAY_FROM_BEST_SOUND"
                    )
                ));
            let ranged = actor
                .weapon
                .as_ref()
                .and_then(|w| {
                    self.weapon_defs
                        .get(&w.class)
                        .cloned()
                        .or_else(|| npc_weapons::builtin(&w.class))
                })
                .and_then(|d| d.fire)
                .filter(|_| {
                    actor
                        .weapon
                        .as_ref()
                        .is_some_and(|w| w.class != "weapon_rpg")
                });
            if combine && actor.move_regulator.is_none() {
                if let Some(spec) = ranged.as_ref() {
                    actor.move_regulator = Some(npc_weapons::ShotRegulator::for_weapon(
                        spec,
                        npc_weapons::RegulatorRule::Weapon,
                        now,
                        0x9e37_79b9_7f4a_7c15 ^ id as u64,
                    ));
                }
            }
            // m_flStopMoveShootTime: hiding to reload (or empty) stops aiming at the enemy
            // after U(0.4, 0.6) s for 5 s, so the soldier faces the way it runs.
            if combine {
                let reloading = schedule.as_deref() == Some("SCHED_COMBINE_HIDE_AND_RELOAD")
                    || actor.npc.conditions.has(COND_NO_PRIMARY_AMMO);
                if !reloading {
                    actor.stop_move_shoot = None;
                } else if actor.stop_move_shoot.is_none() {
                    actor.stop_move_shoot = Some(now + 0.4 + 0.2 * f64::from(ctx.random));
                }
                if actor.stop_move_shoot.is_some_and(|t| now >= t) {
                    actor
                        .move_shoot
                        .suspend(5., actor.move_regulator.as_mut(), now);
                    actor.stop_move_shoot = Some(f64::INFINITY);
                }
            }
            let regulator = if combine {
                actor.move_regulator.as_mut()
            } else {
                actor.npc.shot_regulator.as_mut()
            };
            let moving = motor.goal.is_some() && motor.scene.states[id].is_moving();
            let has_aim = motor.controller.has_move_aim(id);
            let out = actor.move_shoot.think(
                schedule
                    .as_deref()
                    .zip(task_name.as_deref())
                    .map(|(s, t)| (s, t, actor.npc.task)),
                allowed,
                (
                    actor.npc.state == NpcState::Script,
                    ranged.is_some(),
                    has_aim,
                ),
                moving,
                actor.npc.conditions,
                actor.npc.enemy.is_some(),
                regulator,
                now,
            );
            if let Some(aim) = out.aim {
                motor.controller.set_move_aim(id, aim);
            }
            if out.shoot {
                // OnRangeAttack1: OnFiredWeapon (and the class's attack time).
                let distance = ctx.enemy.map(|(_, p)| p.distance(origin));
                let regulator = if combine {
                    actor.move_regulator.as_mut()
                } else {
                    actor.npc.shot_regulator.as_mut()
                };
                if let Some(r) = regulator {
                    r.on_range_attack1(now, distance);
                }
                if let AiClass::Humanoid(h) = &mut actor.class {
                    h.last_attack = now;
                }
                actor.move_fire += 1;
            }
            let lkp = ctx.enemy_lkp.or(ctx.enemy.map(|(_, p)| p));
            motor.scene.states[id].ai_move_facing = match (out.face_enemy, lkp) {
                (true, Some(point)) => Some((point, now + 0.8)),
                _ => motor.scene.states[id]
                    .ai_move_facing
                    .filter(|(_, until)| now < *until),
            };
        }
        // The idle activity is translated again when the angry-idle inputs change
        // (MaintainActivity re-translates the ideal ACT_IDLE).
        if idle_angry != actor.idle_angry {
            actor.idle_angry = idle_angry;
            if actor.npc.activity == "ACT_IDLE" {
                super::schedule::Motor::set_activity(&mut motor, "ACT_IDLE");
            }
        }
        // Weapon_SetActivity on the weapon's world model for this think's activities.
        if let (Some((activity, duration)), Some(weapon)) =
            (motor.weapon_activity.take(), actor.weapon.as_mut())
        {
            let rig = self
                .weapon_defs
                .get(&weapon.class)
                .and_then(|d| world.rigs.get(&npc_weapons::rig_key(&d.world_model)));
            if let Some(rig) = rig {
                weapon.set_activity(rig, &activity, duration, now);
            }
        }
        self.diagnostics.thinks += 1;
        // Assault point outputs (OnArrival, OnAssaultClear; activator = this NPC).
        let mut unlock = false;
        if let Some(assault) = actor.class.assault_mut().as_mut() {
            for (entity, output) in std::mem::take(&mut assault.outputs) {
                scene.fire(entity, output, id);
            }
            unlock = std::mem::take(&mut assault.unlock);
        }
        if unlock {
            self.rally_locks.retain(|_, holder| *holder != id);
        }
        // Policing outputs on the goal entity.
        if let Some(policing) = actor.class.cop_mut().and_then(|c| c.policing.as_mut()) {
            let goal = policing.goal.entity;
            for output in std::mem::take(&mut policing.outputs) {
                scene.fire(goal, output, id);
            }
        }
        // Actbusy hint locks (CAI_Hint::Lock/Unlock) and goal outputs.
        if let Some(cop) = actor.class.cop_mut() {
            for hint in cop.actbusy.released.drain(..) {
                if self.hint_locks.get(&hint) == Some(&id) {
                    self.hint_locks.remove(&hint);
                }
            }
            if let Some(hint) = &cop.actbusy.hint {
                self.hint_locks.insert(hint.entity, id);
            }
            if let Some(goal) = cop.actbusy.goal {
                for output in std::mem::take(&mut cop.actbusy.outputs) {
                    scene.fire(goal, output, id);
                }
            }
        }
        self.apply_requests(id, world, scene, physics, player);
    }
}

#[cfg(test)]
mod tests;
