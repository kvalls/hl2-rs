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
use super::conditions::*;
use super::headcrab::{self, Headcrab};
use super::host::{yaw_of, PhysicsSight, SceneMotor};
use super::humanoid::{self, Humanoid};
use super::metropolice::{CopRequest, Metropolice, PlayerView, PLAYER};
use super::police::{PoliceGoal, Policing};
use super::relationships::{classify, Class, Disposition, Relationships};
use super::schedule::{self, AiNpc, Behavior, Context, Schedules};
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
    Headcrab(Box<Headcrab>),
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
            Self::Headcrab(h) => h.assault.as_ref(),
        }
    }
    pub fn assault_mut(&mut self) -> &mut Option<assault::Assault> {
        match self {
            Self::Metropolice(cop) => &mut cop.assault,
            Self::Humanoid(h) => &mut h.assault,
            Self::Zombie(z) => &mut z.assault,
            Self::Headcrab(h) => &mut h.assault,
        }
    }
    /// The aiscripted_schedule goal (position, run).
    pub fn script_goal(&self) -> Option<(Vec3, bool)> {
        match self {
            Self::Metropolice(cop) => cop.script_goal,
            Self::Humanoid(h) => h.script_goal,
            Self::Zombie(z) => z.script_goal,
            Self::Headcrab(h) => h.script_goal,
        }
    }
    fn script_goal_mut(&mut self) -> &mut Option<(Vec3, bool)> {
        match self {
            Self::Metropolice(cop) => &mut cop.script_goal,
            Self::Humanoid(h) => &mut h.script_goal,
            Self::Zombie(z) => &mut z.script_goal,
            Self::Headcrab(h) => &mut h.script_goal,
        }
    }
    fn requests_mut(&mut self) -> &mut Vec<CopRequest> {
        match self {
            Self::Metropolice(cop) => &mut cop.requests,
            Self::Humanoid(h) => &mut h.requests,
            Self::Zombie(z) => &mut z.requests,
            Self::Headcrab(h) => &mut h.requests,
        }
    }
    fn behavior_mut(&mut self) -> &mut dyn Behavior {
        match self {
            Self::Metropolice(cop) => cop.as_mut(),
            Self::Humanoid(h) => h.as_mut(),
            Self::Zombie(z) => z.as_mut(),
            Self::Headcrab(h) => h.as_mut(),
        }
    }
    /// m_flFieldOfView.
    fn field_of_view(&self) -> f32 {
        match self {
            Self::Metropolice(_) => METROPOLICE_FOV,
            Self::Humanoid(h) => h.kind.field_of_view(),
            Self::Zombie(_) => zombie::FIELD_OF_VIEW,
            Self::Headcrab(_) => headcrab::FIELD_OF_VIEW,
        }
    }
    /// GetSoundInterests.
    fn sound_interests(&self) -> u32 {
        match self {
            Self::Metropolice(_) => METROPOLICE_SOUNDS,
            Self::Humanoid(h) => h.kind.sound_interests(),
            // CAI_BaseNPC::GetSoundInterests (SOUND_PLAYER_VEHICLE has no source here).
            Self::Zombie(_) | Self::Headcrab(_) => {
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
    /// This think's largest hit (CNPC_PoisonZombie::OnTakeDamage_Alive's 20% rule).
    #[serde(skip)]
    largest_hit: f32,
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
    /// Off the ground (FL_ONGROUND clear): velocity and the last fall step.
    pub fall: Option<(Vec3, f64)>,
    /// SetOwnerEntity: the thrower whose box the falling NPC does not hit (a poison
    /// zombie's thrown crab) until it lands.
    pub fall_owner: Option<usize>,
    /// The controller plays the black crab's panic locomotion (Scurry).
    pub panic_locomotion: bool,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Diagnostics {
    pub thinks: u64,
    /// NPC falls that ended (entity, z, what the trace hit, start solid, fraction); the
    /// last 16.
    pub fall_landings: Vec<(usize, f32, Option<usize>, bool, f32)>,
    /// Falling NPCs pushed out of the player's box before their step (tick-order overlap).
    pub player_box_pushouts: u32,
    /// SCHED_ZOMBIE_ATTACKITEM claws that struck their obstructor.
    pub obstructor_swats: u32,
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
    /// sk_headcrab_melee_dmg (the leap's bite).
    headcrab_melee_damage: f32,
    /// Black crab bites on the player (crab, bite damage) for the host: CBlackHeadcrab::
    /// TouchDamage needs the player's health (`headcrab::poison_bite`).
    pub poison_bites: Vec<(usize, f32)>,
    /// AE_ZOMBIE_POISON_THROW_CRAB this tick for the host: (zombie, EyePosition, yaw, the
    /// enemy's eyes); the host creates the black crab and calls `thrown_headcrab`.
    pub crab_throws: Vec<(usize, Vec3, f32, Option<Vec3>)>,
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
    /// Zombie swats this tick for the host's physics: (object entity, velocity in/s, local
    /// angular deg/s), IPhysicsObject::AddVelocity.
    pub prop_swats: Vec<(usize, Vec3, Vec3)>,
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
        headcrab::add_schedules(&mut schedules);
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
        let mut headcrab_melee_damage = 0.;
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
                    if let Some(value) = tokens
                        .windows(2)
                        .find(|p| p[0].eq_ignore_ascii_case("sk_headcrab_melee_dmg"))
                        .and_then(|p| p[1].parse::<f32>().ok())
                    {
                        headcrab_melee_damage = value;
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
            headcrab_melee_damage,
            poison_bites: Vec::new(),
            crab_throws: Vec::new(),
            weapon_defs,
            shots: Vec::new(),
            muzzle_flashes: Vec::new(),
            weapon_muzzle_flashes: Vec::new(),
            player_hits: Vec::new(),
            melee_hits: Vec::new(),
            grenade_tosses: Vec::new(),
            ball_launches: Vec::new(),
            prop_swats: Vec::new(),
            delayed_sounds: Vec::new(),
            diagnostics: Diagnostics::default(),
            random: 0x9e37_79b9_7f4a_7c15,
        }
    }
    /// A uniform value in [-1, 1) from the AI's stream (shot spread, blood noise).
    /// The world model of an NPC's active weapon (CBaseCombatWeapon::Equip follows the
    /// owner bone-merged), lowercase; None when unarmed.
    /// The NPC's active weapon class (an AI actor's or a held weapon).
    pub fn weapon_class(&self, id: usize) -> Option<String> {
        self.actors
            .get(&id)
            .and_then(|a| a.weapon.as_ref())
            .or_else(|| self.held_weapons.get(&id))
            .map(|w| w.class.clone())
    }
    pub fn weapon_world_model(&self, id: usize) -> Option<String> {
        let class = &self
            .actors
            .get(&id)
            .and_then(|a| a.weapon.as_ref())
            .or_else(|| self.held_weapons.get(&id))?
            .class;
        self.weapon_model(class)
    }
    /// A weapon class's world model, lowercase.
    pub fn weapon_model(&self, class: &str) -> Option<String> {
        self.weapon_defs
            .get(class)
            .map(|d| d.world_model.to_lowercase())
            .filter(|m| !m.is_empty())
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
    /// SetPoliceGoal / ActivateBaton on cops and EnableKnockOut / DisableKnockOut on
    /// ai_goal_police (processed even while the AI is off, so map-spawn inputs stick).
    fn inputs(&mut self, world: &World, scene: &mut Scene, controller: &mut Controller) {
        if !self.goals_started && !self.actors.is_empty() {
            // CAI_GoalEntity::Activate at spawn for StartActive goals.
            self.goals_started = true;
            for (id, e) in world.entities.iter().enumerate() {
                if e.class() == "ai_relationship"
                    && e.get("StartActive").is_some_and(|v| v.trim() == "1")
                {
                    self.apply_relationship(world, scene, id, true);
                }
                if e.class() == "ai_goal_assault"
                    && e.get("StartActive").is_some_and(|v| v.trim() == "1")
                {
                    scene
                        .ai_inputs
                        .push((id, "activate".into(), String::new(), usize::MAX));
                }
            }
        }
        for (target, input, parameter, _) in std::mem::take(&mut scene.ai_inputs) {
            let class = world.entities.get(target).map_or("", |e| e.class());
            if class == "ai_goal_assault" {
                self.assault_goal_input(world, scene, target, &input);
                continue;
            }
            if class == "aiscripted_schedule" {
                self.scripted_schedule_input(world, scene, target, &input);
                continue;
            }
            if class == "ai_goal_actbusy" {
                self.actbusy_goal_input(world, scene, target, &input, &parameter);
                continue;
            }
            if class == "ai_relationship" {
                self.apply_relationship(world, scene, target, input == "applyrelationship");
                continue;
            }
            if matches!(class, "info_node_link" | "info_node_link_controller") {
                // CAI_DynamicLink(Controller)::InputTurnOn/Off: the graph links' state.
                if !controller.set_dynamic_link(target, input == "turnon") {
                    *self
                        .diagnostics
                        .unknown_goals
                        .entry(format!("dynamic link #{target}"))
                        .or_default() += 1;
                }
                continue;
            }
            match input.as_str() {
                "setpolicegoal" => {
                    let goal = world.entities.iter().enumerate().find_map(|(id, e)| {
                        (e.class() == "ai_goal_police"
                            && e.get("targetname")
                                .is_some_and(|n| n.eq_ignore_ascii_case(&parameter)))
                        .then_some(id)
                    });
                    match (
                        goal.and_then(|g| self.goals.get(&g)),
                        self.actors.get_mut(&target),
                    ) {
                        (Some(goal), Some(actor)) if actor.class.cop().is_some() => {
                            let cop = actor.class.cop_mut().expect("cop");
                            // CAI_PolicingBehavior::Enable clears the schedule.
                            cop.policing = Some(Policing::new(goal.clone()));
                            actor.npc.schedule = None;
                        }
                        _ => {
                            *self
                                .diagnostics
                                .unknown_goals
                                .entry(parameter.clone())
                                .or_default() += 1;
                        }
                    }
                }
                "activatebaton" => {
                    if let Some(actor) = self.actors.get_mut(&target) {
                        if let AiClass::Metropolice(cop) = &mut actor.class {
                            let on = !matches!(parameter.trim(), "" | "0");
                            cop.set_baton_state(&mut actor.npc, on);
                        }
                    }
                }
                "enableknockout" | "disableknockout" => {
                    if let Some(goal) = self.goals.get_mut(&target) {
                        goal.override_knockout = input == "enableknockout";
                    }
                }
                _ => {}
            }
        }
    }
    /// ai_goal_actbusy inputs: Activate (EnableGoal on its actors), Deactivate,
    /// ForceNPCToActBusy ("<npc> [hint] [teleport] [nearest] [$activity] [max time]";
    /// a named hint, teleports and forced activities are not reconstructed) and
    /// SetBusySearchRange. Only metrocops host the behavior so far.
    fn actbusy_goal_input(
        &mut self,
        world: &World,
        scene: &Scene,
        goal: usize,
        input: &str,
        parameter: &str,
    ) {
        let now = scene.time;
        let range = self.actbusy_ranges.get(&goal).copied().unwrap_or(2048.);
        match input {
            "activate" | "deactivate" => {
                for id in self.goal_actors(world, goal) {
                    let Some(actor) = self.actors.get_mut(&id) else {
                        continue;
                    };
                    let AiClass::Metropolice(cop) = &mut actor.class else {
                        continue;
                    };
                    if input == "activate" {
                        if !cop.actbusy.enabled {
                            cop.actbusy.enable(Some(goal), range, now);
                        }
                    } else {
                        cop.actbusy.disable(&mut actor.npc, now);
                    }
                }
            }
            "forcenpctoactbusy" => {
                let mut tokens = parameter.split_whitespace();
                let name = tokens.next().unwrap_or("").to_lowercase();
                let mut nearest = false;
                let mut max_time = None;
                for token in tokens {
                    match token {
                        "nearest" => nearest = true,
                        t if t.starts_with("teleport")
                            || t.starts_with('$')
                            || t.starts_with("see:") =>
                        {
                            *self
                                .diagnostics
                                .unknown_goals
                                .entry(format!("actbusy {t}"))
                                .or_default() += 1;
                        }
                        t => match t.parse::<f32>() {
                            Ok(v) => max_time = Some(v),
                            // A hint node name.
                            Err(_) => {
                                *self
                                    .diagnostics
                                    .unknown_goals
                                    .entry(format!("actbusy hint {t}"))
                                    .or_default() += 1;
                            }
                        },
                    }
                }
                let target = self.actors.keys().copied().find(|&id| {
                    world.entities.get(id).is_some_and(|e| {
                        crate::entities::glob(
                            &name,
                            &e.get("targetname").unwrap_or("").to_lowercase(),
                        )
                    })
                });
                let Some(actor) = target.and_then(|id| self.actors.get_mut(&id)) else {
                    *self
                        .diagnostics
                        .unknown_goals
                        .entry(format!("actbusy npc {name}"))
                        .or_default() += 1;
                    return;
                };
                if let AiClass::Metropolice(cop) = &mut actor.class {
                    cop.actbusy
                        .force(Some(goal), range, max_time, nearest, &mut actor.npc, now);
                }
            }
            "setbusysearchrange" => {
                if let Ok(v) = parameter.trim().parse() {
                    self.actbusy_ranges.insert(goal, v);
                }
            }
            _ => {}
        }
    }
    /// CAI_GoalEntity actors: the NPCs whose targetname (SearchType 0) or classname
    /// (SearchType 1) matches the goal's "actor" keyvalue.
    fn goal_actors(&self, world: &World, goal: usize) -> Vec<usize> {
        let Some(e) = world.entities.get(goal) else {
            return Vec::new();
        };
        let name = e.get("actor").unwrap_or("").to_lowercase();
        let by_class = e.get("SearchType").is_some_and(|v| v.trim() == "1");
        if name.is_empty() {
            return Vec::new();
        }
        self.actors
            .keys()
            .copied()
            .filter(|&id| {
                world.entities.get(id).is_some_and(|a| {
                    let key = if by_class {
                        a.class().to_owned()
                    } else {
                        a.get("targetname").unwrap_or("").to_owned()
                    };
                    crate::entities::glob(&name, &key.to_lowercase())
                })
            })
            .collect()
    }
    /// Entities of a class matching a name (FindEntityByName: a trailing '*' matches
    /// any suffix).
    fn named(world: &World, class: &str, name: &str) -> Vec<usize> {
        let name = name.to_lowercase();
        world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.class() == class
                    && e.get("targetname")
                        .is_some_and(|n| crate::entities::glob(&name, &n.to_lowercase()))
            })
            .map(|(id, _)| id)
            .collect()
    }
    fn assault_point(world: &World, id: usize) -> assault::AssaultPoint {
        let e = &world.entities[id];
        let num = |k: &str, d: f32| e.get(k).and_then(|v| v.trim().parse().ok()).unwrap_or(d);
        let flag = |k: &str| {
            e.get(k)
                .is_some_and(|v| v.trim() != "0" && !v.trim().is_empty())
        };
        assault::AssaultPoint {
            entity: id,
            name: e.get("targetname").unwrap_or("").to_owned(),
            origin: e.origin(),
            yaw: yaw_keyvalue(e),
            next: e.get("nextassaultpoint").unwrap_or("").to_owned(),
            timeout: num("assaulttimeout", 3.),
            clear_on_contact: flag("clearoncontact"),
            allow_diversion: flag("allowdiversion"),
            diversion_radius: num("allowdiversionradius", 0.),
            never_timeout: flag("nevertimeout"),
            strict: flag("strict"),
            tolerance: num("assaulttolerance", assault::CUE_POINT_TOLERANCE),
            clear_on_arrival: num("spawnflags", 0.) as u32 & 1 != 0,
        }
    }
    /// ai_goal_assault inputs: Activate (EnableGoal: SetParameters for each actor),
    /// Deactivate (DisableGoal), BeginAssault (ReceiveAssaultCue(CUE_ENTITY_INPUT)).
    fn assault_goal_input(&mut self, world: &World, scene: &mut Scene, goal: usize, input: &str) {
        // Held templates (not spawned yet) and dead NPCs are not actors.
        let actors: Vec<usize> = self
            .goal_actors(world, goal)
            .into_iter()
            .filter(|&id| scene.states[id].visible && !scene.states[id].killed)
            .collect();
        match input {
            "activate" | "updateactors" => {
                let e = &world.entities[goal];
                let rally_name = e.get("rallypoint").unwrap_or("").to_owned();
                let cue = assault::Cue::from_keyvalue(
                    e.get("AssaultCue")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(1),
                );
                let random_select = e.get("RallySelectMethod").is_some_and(|v| v.trim() == "1");
                for id in actors {
                    if input == "updateactors"
                        && self.actors[&id]
                            .class
                            .assault()
                            .is_some_and(|a| a.goal == goal && a.active())
                    {
                        continue;
                    }
                    self.release_rally(id);
                    let origin = scene.states[id].origin;
                    // Unlocked rally points: highest priority, then nearest (or random).
                    let mut rallies: Vec<usize> =
                        Self::named(world, "assault_rallypoint", &rally_name)
                            .into_iter()
                            .filter(|r| !self.rally_locks.contains_key(r))
                            .collect();
                    let priority = |r: usize| {
                        world.entities[r]
                            .get("priority")
                            .and_then(|v| v.trim().parse::<i32>().ok())
                            .unwrap_or(0)
                    };
                    let best = if random_select {
                        (!rallies.is_empty()).then(|| {
                            let i = (self.random() * rallies.len() as f32) as usize;
                            rallies[i.min(rallies.len() - 1)]
                        })
                    } else {
                        rallies.sort_by(|&a, &b| {
                            priority(b).cmp(&priority(a)).then(
                                world.entities[a]
                                    .origin()
                                    .distance_squared(origin)
                                    .total_cmp(
                                        &world.entities[b].origin().distance_squared(origin),
                                    ),
                            )
                        });
                        rallies.first().copied()
                    };
                    let Some(rally) = best else {
                        *self
                            .diagnostics
                            .unknown_goals
                            .entry(format!("rally {rally_name}"))
                            .or_default() += 1;
                        continue;
                    };
                    let r = &world.entities[rally];
                    let rally_point = assault::RallyPoint {
                        entity: rally,
                        origin: r.origin(),
                        yaw: yaw_keyvalue(r),
                        assault_point: r.get("assaultpoint").unwrap_or("").to_owned(),
                        delay: r
                            .get("assaultdelay")
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0.),
                        priority: priority(rally),
                    };
                    // The assault chain by name, following nextassaultpoint.
                    let mut chain: Vec<(String, Vec<assault::AssaultPoint>)> = Vec::new();
                    let mut name = rally_point.assault_point.clone();
                    while !name.is_empty()
                        && chain.len() < 64
                        && !chain.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name))
                    {
                        let points: Vec<_> = Self::named(world, "assault_assaultpoint", &name)
                            .into_iter()
                            .map(|p| Self::assault_point(world, p))
                            .collect();
                        let next = points.first().map(|p| p.next.clone()).unwrap_or_default();
                        chain.push((name.clone(), points));
                        name = next;
                    }
                    // FindAssaultPoint: one candidate, else random excluding the most
                    // recently used (the hull fit test is not modeled).
                    let candidates = chain.first().map(|c| c.1.clone()).unwrap_or_default();
                    let pick = match candidates.len() {
                        0 => None,
                        1 => Some(0),
                        n => {
                            let recent = (0..n).max_by(|&a, &b| {
                                let t = |i: usize| {
                                    self.assault_used
                                        .get(&candidates[i].entity)
                                        .copied()
                                        .unwrap_or(-1.)
                                };
                                t(a).total_cmp(&t(b))
                            });
                            let others: Vec<usize> =
                                (0..n).filter(|&i| Some(i) != recent).collect();
                            let i = (self.random() * others.len() as f32) as usize;
                            Some(others[i.min(others.len() - 1)])
                        }
                    };
                    let Some(pick) = pick else {
                        *self
                            .diagnostics
                            .unknown_goals
                            .entry(format!("assault point {}", rally_point.assault_point))
                            .or_default() += 1;
                        continue;
                    };
                    let point = candidates[pick].clone();
                    self.assault_used.insert(point.entity, scene.time);
                    self.rally_locks.insert(rally, id);
                    let seed = 0x9e37_79b9_7f4a_7c15 ^ (id as u64) << 8;
                    let actor = self.actors.get_mut(&id).expect("actor");
                    *actor.class.assault_mut() = Some(assault::Assault::new(
                        goal,
                        cue,
                        rally_point,
                        point,
                        chain,
                        seed,
                    ));
                    // InitializeBehavior: ClearSchedule.
                    actor.npc.schedule = None;
                }
            }
            "deactivate" => {
                for id in actors {
                    let actor = self.actors.get_mut(&id).expect("actor");
                    if let Some(a) = actor
                        .class
                        .assault_mut()
                        .as_mut()
                        .filter(|a| a.goal == goal)
                    {
                        a.disable();
                        actor.npc.schedule = None;
                    }
                    self.release_rally(id);
                }
            }
            "beginassault" => {
                for id in actors {
                    let actor = self.actors.get_mut(&id).expect("actor");
                    if let Some(a) = actor.class.assault_mut().as_mut() {
                        a.receive_cue(assault::Cue::EntityInput);
                        a.provoke = true;
                    }
                }
            }
            _ => {}
        }
    }
    /// CAI_Relationship ApplyRelationship / RevertRelationship: subjects and targets by
    /// targetname or classname ("!player" for the player), within `radius` of the entity
    /// when set; disposition 1 hate, 2 fear, 3 like, 4 neutral; Reciprocal applies it both
    /// ways. Revert removes the pairs this entity applied. Rank (priority) is not modeled.
    fn apply_relationship(&mut self, world: &World, scene: &Scene, rel: usize, apply: bool) {
        if let Some(pairs) = self.applied_relationships.remove(&rel) {
            for pair in pairs {
                self.relationship_overrides.remove(&pair);
            }
        }
        if !apply {
            return;
        }
        let e = &world.entities[rel];
        let disposition = match e.get("disposition").map(str::trim) {
            Some("1") => Disposition::HT,
            Some("2") => Disposition::FR,
            Some("3") => Disposition::LI,
            Some("4") => Disposition::NU,
            _ => return,
        };
        let radius = e
            .get("radius")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(0.);
        let at = e.origin();
        let matching = |pattern: &str| -> Vec<usize> {
            let pattern = pattern.trim().to_lowercase();
            if pattern.is_empty() {
                return Vec::new();
            }
            if pattern == "!player" {
                return vec![PLAYER];
            }
            world
                .entities
                .iter()
                .enumerate()
                .filter(|(i, x)| {
                    (crate::entities::glob(
                        &pattern,
                        &x.get("targetname").unwrap_or("").to_lowercase(),
                    ) || crate::entities::glob(&pattern, x.class()))
                        && (radius <= 0.
                            || scene
                                .states
                                .get(*i)
                                .is_some_and(|s| s.origin.distance(at) <= radius))
                })
                .map(|(i, _)| i)
                .collect()
        };
        let subjects = matching(e.get("subject").unwrap_or(""));
        let targets = matching(e.get("target").unwrap_or(""));
        let reciprocal = e.get("Reciprocal").is_some_and(|v| v.trim() == "1");
        let mut pairs = Vec::new();
        for &a in &subjects {
            for &b in &targets {
                if a == b {
                    continue;
                }
                pairs.push((a, b));
                if reciprocal {
                    pairs.push((b, a));
                }
            }
        }
        for &pair in &pairs {
            self.relationship_overrides.insert(pair, disposition);
        }
        self.applied_relationships.insert(rel, pairs);
    }
    /// UnlockRallyPoint for an NPC.
    fn release_rally(&mut self, id: usize) {
        self.rally_locks.retain(|_, holder| *holder != id);
    }
    /// aiscripted_schedule StartSchedule/StopSchedule (CAI_ScriptedSchedule): walk or run
    /// the m_iszEntity NPC(s) to the goal entity (ScheduledMoveToGoalEntity with
    /// SCHED_IDLE_WALK interrupts); forcestate sets the NPC state first.
    fn scripted_schedule_input(
        &mut self,
        world: &World,
        scene: &mut Scene,
        script: usize,
        input: &str,
    ) {
        let e = &world.entities[script];
        let name = e.get("m_iszEntity").unwrap_or("").to_lowercase();
        let graball = e.get("graball").is_some_and(|v| v.trim() == "1");
        let flags = e
            .get("spawnflags")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0);
        // InputStartSchedule: once, unless SF_SCRIPT_REPEATABLE (4); StopSchedule does
        // nothing before the first start.
        let fired = self.scripted_fired.contains(&script);
        if (input == "startschedule" && fired && flags & 4 == 0)
            || (input == "stopschedule" && !fired)
        {
            return;
        }
        // Acquisition: live NPCs matching the selector within m_flRadius of the
        // controller (0: anywhere).
        let radius = e
            .get("m_flRadius")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(0.);
        let at = e.origin();
        let mut targets: Vec<usize> = self
            .actors
            .keys()
            .copied()
            .filter(|&id| {
                !scene.states[id].killed
                    && scene.states[id].visible
                    && (radius <= 0. || scene.states[id].origin.distance(at) <= radius)
                    && world.entities.get(id).is_some_and(|a| {
                        crate::entities::glob(
                            &name,
                            &a.get("targetname").unwrap_or("").to_lowercase(),
                        ) || crate::entities::glob(&name, a.class())
                    })
            })
            .collect();
        if !graball {
            targets.truncate(1);
        }
        if input == "startschedule" {
            if targets.is_empty() {
                // ScriptThink: no actor yet, try again in a second.
                scene.send_delayed(script, "StartSchedule", "", 1.);
                return;
            }
            self.scripted_fired.insert(script);
        }
        let schedule = e
            .get("schedule")
            .and_then(|v| v.trim().parse::<i32>().ok())
            .unwrap_or(0);
        let goal_name = e.get("goalent").unwrap_or("");
        let goal = world.entities.iter().enumerate().find_map(|(id, g)| {
            g.get("targetname")
                .is_some_and(|n| n.eq_ignore_ascii_case(goal_name))
                .then(|| {
                    if scene.states.get(id).is_some_and(|s| s.origin != Vec3::ZERO) {
                        scene.states[id].origin
                    } else {
                        g.origin()
                    }
                })
        });
        let state = match e
            .get("forcestate")
            .and_then(|v| v.trim().parse::<i32>().ok())
        {
            Some(1) => Some(NpcState::Idle),
            Some(2) => Some(NpcState::Alert),
            Some(3) => Some(NpcState::Combat),
            _ => None,
        };
        for id in targets {
            let actor = self.actors.get_mut(&id).expect("actor");
            if input == "stopschedule" {
                if actor.class.script_goal_mut().take().is_some() {
                    actor.npc.schedule = None;
                }
                continue;
            }
            let Some(goal) = goal else {
                *self
                    .diagnostics
                    .unknown_goals
                    .entry(format!("aiscripted goal {goal_name}"))
                    .or_default() += 1;
                continue;
            };
            // SCHED_SCRIPT_WALK_TO_GOAL 1, RUN_TO_GOAL 2, ENEMY_IS_GOAL_AND_RUN_TO_GOAL 6;
            // path goals (4, 5) and enemy-is-goal (3) are not modeled.
            let run = match schedule {
                1 => false,
                2 | 6 => true,
                other => {
                    *self
                        .diagnostics
                        .unknown_goals
                        .entry(format!("aiscripted schedule {other}"))
                        .or_default() += 1;
                    continue;
                }
            };
            if let Some(state) = state {
                actor.npc.state = state;
                actor.npc.ideal_state = state;
            }
            *actor.class.script_goal_mut() = Some((goal, run));
            actor.npc.force_schedule("SCHED_AISCRIPTED_MOVE_TO_GOAL");
        }
    }
    /// The PoliceTarget: "!player" or a named entity (resolved each think).
    fn goal_target(world: &World, scene: &Scene, goal: usize) -> Option<(usize, Vec3)> {
        let name = world.entities.get(goal)?.get("PoliceTarget")?;
        if name.eq_ignore_ascii_case("!player") {
            return Some((PLAYER, Vec3::ZERO));
        }
        world.entities.iter().enumerate().find_map(|(id, e)| {
            (e.get("targetname")
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
                && !scene.states[id].killed)
                .then(|| (id, scene.states[id].origin + Vec3::Z * 36.))
        })
    }
    /// The AI actor of an entity whose class has a behavior (npc_metropolice and the
    /// humanoids).
    fn actor_for(
        id: usize,
        e: &modkit_core::Entity,
        defs: &BTreeMap<String, NpcWeaponDef>,
    ) -> Option<AiActor> {
        let kind = humanoid::Kind::for_class(e.class());
        let zombie = zombie::Kind::for_class(e.class());
        let crab = headcrab::is_headcrab_class(e.class());
        if e.class() != "npc_metropolice" && kind.is_none() && zombie.is_none() && !crab {
            return None;
        }
        let flags: u32 = e
            .get("spawnflags")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let weapon = Self::equip(e.class(), e.get("additionalequipment").unwrap_or(""), defs);
        let baton = weapon
            .as_ref()
            .is_some_and(|w| w.class == "weapon_stunstick");
        let mut npc = AiNpc::new(id);
        npc.shot_regulator = Self::regulator(e.class(), weapon.as_ref(), defs, id);
        let class = match kind {
            _ if crab => AiClass::Headcrab(Box::new(Headcrab::for_class(e.class()))),
            _ if zombie.is_some() => {
                let mut z = Zombie::new(
                    zombie.unwrap_or(zombie::Kind::Classic),
                    flags & SF_NPC_GAG != 0,
                    0x3c6e_f372_fe94_f82b ^ id as u64,
                );
                z.torso = e.class().ends_with("_torso");
                AiClass::Zombie(Box::new(z))
            }
            Some(kind) => {
                let mut h =
                    Humanoid::new(kind, weapon.is_some(), 0x6a09_e667_f3bc_c909 ^ id as u64);
                h.grenades = e
                    .get("NumGrenades")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                h.elite = kind == humanoid::Kind::Combine && is_elite_model(e.get("model"));
                AiClass::Humanoid(Box::new(h))
            }
            None => AiClass::Metropolice(Box::new(Metropolice::new(flags, baton))),
        };
        Some(AiActor {
            npc,
            class,
            next_think: 0.,
            last_think: 0.,
            weapon,
            player_disposition: None,
            origin: [0.; 3],
            yaw: 0.,
            damaged_by: None,
            damage_taken: 0.,
            largest_hit: 0.,
            damaged_by_player: false,
            goal: None,
            events_until: 0.,
            idle_angry: false,
            think_dt: THINK_INTERVAL as f32,
            move_shoot: Default::default(),
            move_regulator: None,
            move_fire: 0,
            stop_move_shoot: None,
            pain_memory: 0,
            next_pain_sound: 0.,
            next_lost_sound: 0.,
            enemy_eluded: false,
            death_spoken: false,
            squad_name: e
                .get("squadname")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            squad: None,
            squad_new_enemy: false,
            choose: Default::default(),
            idle_chatter: COP_CHATTER_ASK_QUESTION,
            harassed: false,
            next_idle_decision: 0.,
            fall: None,
            fall_owner: None,
            panic_locomotion: false,
        })
    }
    /// MOVETYPE_STEP falling (sv_gravity 600) for NPCs off the ground (a zombie torso
    /// after BecomeTorso): a hull trace down each tick until it lands (FL_ONGROUND).
    fn step_falls(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        player: PlayerState,
    ) {
        use crate::npc_probe::NpcCollisionWorld;
        let now = scene.time;
        // Airborne crab steps that did not bite the player: (crab, swept box start, end,
        // mins, maxs, hit a solid after the world-ignore window, landed).
        let mut npc_touches = Vec::new();
        let mut panics = Vec::new();
        // PhysicsTryMove traces with the NPC's solid mask, which the player's box blocks
        // (NPCs have colliders; the player has none, so it is a transient box here). A
        // dead player is not solid.
        let player_box = (!player.noclip && !player.dead).then(|| crate::npc_probe::ActorHull {
            entity: None,
            feet: player.feet,
            hull: crate::npc_probe::Hull {
                mins: Vec3::new(-16., -16., 0.),
                maxs: Vec3::new(16., 16., if player.crouched { 36. } else { 72. }),
            },
        });
        let transients: Vec<_> = player_box.into_iter().collect();
        for (&id, actor) in self.actors.iter_mut() {
            let Some((velocity, last)) = actor.fall else {
                continue;
            };
            if scene.states[id].killed {
                actor.fall = None;
                continue;
            }
            let dt = (now - last).clamp(0., 0.1) as f32;
            let mut velocity = velocity - Vec3::Z * 600. * dt;
            let hull = world
                .entities
                .get(id)
                .and_then(|e| crate::ai_hull::class_hull(e.class()))
                .unwrap_or(crate::ai_hull::HullType::Human)
                .bounds();
            let mut origin = scene.states[id].origin;
            // Our player moves after the NPCs against their colliders from before this
            // tick's NPC moves, so a leaping crab can start a tick a few units inside the
            // player's box (Source moves both on current positions and never overlaps):
            // push it out sideways first; if that fails the player box is left out.
            let mut transients = transients.as_slice();
            if let Some(player_box) = player_box {
                let (lo, hi) = (
                    player_box.feet + player_box.hull.mins,
                    player_box.feet + player_box.hull.maxs,
                );
                if let Some(out) = push_out_of_box(origin, hull, lo, hi, |p| {
                    !physics
                        .npc_trace_hull(
                            p,
                            p,
                            hull,
                            crate::npc_probe::Query {
                                excluded_entities: &[id],
                                ..Default::default()
                            },
                        )
                        .trace
                        .start_solid
                }) {
                    origin = out;
                    self.diagnostics.player_box_pushouts += 1;
                } else if overlaps(origin + hull.mins, origin + hull.maxs, lo, hi) {
                    transients = &[];
                }
            }
            let end = origin + velocity * dt;
            let trace = |excluded: &[usize]| {
                physics.npc_trace_hull(
                    origin,
                    end,
                    hull,
                    crate::npc_probe::Query {
                        excluded_entities: excluded,
                        transients,
                        ..Default::default()
                    },
                )
            };
            // A dead entity's colliders stay until the next collider pose stage (the zombie
            // a torso replaces this tick, whose mesh collider the torso starts inside): they
            // do not hold the faller up.
            let mut excluded = vec![id];
            excluded.extend(actor.fall_owner);
            let mut hit = trace(&excluded);
            // CHalfLife2::ShouldCollide: HL2COLLISION_GROUP_HEADCRAB never collides with
            // itself (a poison zombie's nest crabs eject from overlapping spots).
            let crab = matches!(actor.class, AiClass::Headcrab(_));
            let passable = |e: usize| {
                scene.states.get(e).is_some_and(|s| s.killed)
                    || crab
                        && world
                            .entities
                            .get(e)
                            .is_some_and(|x| headcrab::is_headcrab_class(x.class()))
            };
            while let Some(dead) = hit.entity.filter(|e| excluded.len() < 8 && passable(*e)) {
                excluded.push(dead);
                hit = trace(&excluded);
            }
            let fraction = if hit.trace.start_solid {
                0.
            } else {
                hit.trace.fraction.clamp(0., 1.)
            };
            let position = origin + (end - origin) * fraction;
            scene.states[id].origin = position;
            // The player's box was the first solid in this step's path.
            let player_hit = hit.transient && hit.entity.is_none() && fraction < 1.;
            // A floor (or a stuck start) lands; a wall or ceiling stops that part of the
            // motion and the fall goes on. The player's box never lands it (below).
            let landed = fraction < 1.
                && !player_hit
                && (hit.trace.start_solid
                    || hit.trace.normal.z > 0.7
                    || velocity.z <= 0. && velocity.truncate().length() < 1.);
            // PhysicsTryMove's ClipVelocity (overbounce 1): a crab that strikes the player
            // keeps only the motion along his box and slides down beside him.
            if fraction < 1. && !landed {
                let n = hit.trace.normal;
                velocity -= n * velocity.dot(n).min(0.);
            }
            // Onto his head (JumpAttack aims at the eyes inside his box, so a crab can land
            // on top, as the SDK notes): Source stands it on him and its AI walks it off;
            // ours cannot stand on the player, so it drops off his nearest side at once.
            if let (true, Some(player_box)) = (player_hit && hit.trace.normal.z > 0.7, player_box) {
                let lo = player_box.feet + player_box.hull.mins;
                let mut hi = player_box.feet + player_box.hull.maxs;
                hi.z = hi.z.max(position.z + hull.maxs.z + 1.);
                let clear = |p: Vec3| {
                    !physics
                        .npc_trace_hull(
                            p,
                            p,
                            hull,
                            crate::npc_probe::Query {
                                excluded_entities: &[id],
                                ..Default::default()
                            },
                        )
                        .trace
                        .start_solid
                };
                if let Some(off) = push_out_of_box(position, hull, lo, hi, clear) {
                    scene.states[id].origin = off;
                    velocity.x = 0.;
                    velocity.y = 0.;
                    self.diagnostics.player_box_pushouts += 1;
                }
            }
            // CBaseHeadcrab::LeapTouch while airborne: the hated player's hull bites (once);
            // another solid after the world-ignore window fails the attack; landing ends it.
            if let AiClass::Headcrab(crab) = &mut actor.class {
                let touching = player_hit;
                // LeapTouch starts with m_bMidJump = false: any touch or landing ends the
                // jump (a thrown crab never runs the leap task that also clears it).
                if touching || landed {
                    crab.mid_jump = false;
                }
                if crab.leap_touch {
                    if touching && actor.npc.enemy == Some(PLAYER) && !player.dead {
                        self.diagnostics.player_hits += 1;
                        crab.leap_touch = false;
                        crab.attack_failed = false;
                        crab.bites += 1;
                        if crab.black {
                            self.poison_bites.push((id, self.headcrab_melee_damage));
                        } else {
                            self.player_hits.push(PlayerHit {
                                attacker: id,
                                damage: self.headcrab_melee_damage,
                                punch: Vec3::ZERO,
                                impulse: Vec3::ZERO,
                                fade: None,
                            });
                        }
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(position + Vec3::Z * 12.),
                            ..crab.sound("Bite").into()
                        });
                    } else {
                        let failed = fraction < 1. && now >= crab.ignore_world_until && !landed;
                        npc_touches.push((id, origin, end, hull.mins, hull.maxs, failed, landed));
                    }
                }
            }
            actor.fall = (!landed).then_some((velocity, now));
            // CBlackHeadcrab::EjectTouch: on the ground, Panic(U(2, 8)).
            if let AiClass::Headcrab(crab) = &mut actor.class {
                if landed && std::mem::take(&mut crab.eject_touch) {
                    panics.push(id);
                }
            }
            if actor.fall.is_none() {
                actor.fall_owner = None;
                let landings = &mut self.diagnostics.fall_landings;
                if landings.len() >= 16 {
                    landings.remove(0);
                }
                landings.push((
                    id,
                    scene.states[id].origin.z,
                    hit.entity,
                    hit.trace.start_solid,
                    fraction,
                ));
            }
        }
        // LeapTouch on an NPC: a hated one (that takes damage) in the step bites
        // (TouchDamage: sk_headcrab_melee_dmg, through the shared melee hit path); else a
        // solid after the world-ignore window fails the attack and a landing ends it.
        for id in panics {
            let duration = 2. + 6. * f64::from(self.random());
            if let Some(AiClass::Headcrab(crab)) = self.actors.get_mut(&id).map(|a| &mut a.class) {
                crab.start_panic(now, duration);
            }
        }
        for (id, start, end, mins, maxs, failed, landed) in npc_touches {
            let victim = self.melee_victim(id, world, scene, start, end, mins, maxs);
            let damage = self.headcrab_melee_damage;
            let Some(actor) = self.actors.get_mut(&id) else {
                continue;
            };
            let AiClass::Headcrab(crab) = &mut actor.class else {
                continue;
            };
            // CBlackHeadcrab::TouchDamage leaves the victim one health.
            let health = victim.map_or(0., |v| scene.states[v].value);
            let damage = if crab.black {
                headcrab::poison_bite(damage, health).map(|(bite, poison)| bite + poison)
            } else {
                Some(damage)
            };
            if let (Some(victim), Some(damage)) = (victim, damage) {
                // The victim's box stops the leap like the player's (NPC traces do not
                // include NPC boxes).
                if let Some((velocity, _)) = &mut actor.fall {
                    velocity.x = 0.;
                    velocity.y = 0.;
                }
                crab.leap_touch = false;
                crab.attack_failed = false;
                crab.bites += 1;
                self.diagnostics.npc_melee_hits += 1;
                self.melee_hits.push(MeleeHit {
                    attacker: id,
                    victim,
                    damage,
                    position: scene.states[victim].origin + Vec3::Z * center_z(world, victim),
                    direction: (end - start).normalize_or_zero(),
                });
                scene.sounds.push(crate::sounds::SoundRequest {
                    origin: Some(end + Vec3::Z * 12.),
                    ..crab.sound("Bite").into()
                });
            } else if failed {
                crab.attack_failed = true;
                crab.leap_touch = false;
            } else if landed {
                crab.leap_touch = false;
            }
        }
    }
    /// CNPC_BaseZombie::BecomeTorso's AI part on the torso entity that replaces the
    /// zombie (DESIGN 14o session 37; Source keeps one entity): the zombie's state, enemy,
    /// memories and squad carry over; ClearSchedule, SetSchedule(SCHED_FALL_TO_GROUND)
    /// from 40 units up with no torso impulse; SetZombieModel's SetActivity(ACT_IDLE).
    pub fn become_torso(
        &mut self,
        world: &World,
        scene: &mut Scene,
        zombie: usize,
        torso: usize,
    ) -> bool {
        let now = scene.time;
        let Some(old) = self.actors.get(&zombie) else {
            return false;
        };
        let npc = old.npc.clone();
        let (squad, squad_name, disposition) = (
            old.squad.clone(),
            old.squad_name.clone(),
            old.player_disposition,
        );
        let Some(actor) = self.actors.get_mut(&torso) else {
            return false;
        };
        actor.npc = npc;
        actor.npc.entity = torso;
        actor.npc.force_schedule("SCHED_FALL_TO_GROUND");
        actor.npc.activity = "ACT_IDLE".into();
        scene.ai_set_activity(world, torso, "ACT_IDLE");
        actor.squad = squad;
        actor.squad_name = squad_name;
        actor.player_disposition = disposition;
        actor.fall = Some((Vec3::ZERO, now));
        actor.next_think = now;
        self.npc_ids = None;
        true
    }
    /// ReleaseHeadcrab's live crab (RELEASE_IMMEDIATE): it takes the zombie's enemy
    /// (SetEnemy) and memory, idles (SetActivity(ACT_IDLE)), leaps no sooner than 1 s
    /// (m_flNextAttack) and falls from the head (SF_NPC_FALL_TO_GROUND).
    pub fn released_headcrab(&mut self, zombie: usize, crab: usize, now: f64) -> bool {
        let Some(old) = self.actors.get(&zombie) else {
            return false;
        };
        let (enemy, enemies) = (old.npc.enemy, old.npc.enemies.clone());
        let Some(actor) = self.actors.get_mut(&crab) else {
            return false;
        };
        actor.npc.enemy = enemy;
        actor.npc.enemies = enemies;
        if enemy.is_some() {
            actor.npc.state = NpcState::Combat;
        }
        actor.npc.activity = "ACT_IDLE".into();
        actor.fall = Some((Vec3::ZERO, now));
        actor.next_think = now;
        if let AiClass::Headcrab(h) = &mut actor.class {
            h.next_attack = now + 1.;
        }
        self.npc_ids = None;
        true
    }
    /// AE_ZOMBIE_POISON_THROW_CRAB's crab (CreateNoSpawn at the zombie's EyePosition, owner
    /// the zombie, SF_NPC_FALL_TO_GROUND): ACT_RANGE_ATTACK1, the zombie's enemy, and
    /// ThrowAt(the enemy's eyes) = JumpAttack thrown (LeapTouch armed); no target: it falls.
    pub fn thrown_headcrab(
        &mut self,
        zombie: usize,
        crab: usize,
        target: Option<Vec3>,
        scene: &mut Scene,
    ) -> bool {
        let now = scene.time;
        let Some(old) = self.actors.get(&zombie) else {
            return false;
        };
        let (enemy, enemies) = (old.npc.enemy, old.npc.enemies.clone());
        let origin = scene.states[crab].origin;
        let Some(actor) = self.actors.get_mut(&crab) else {
            return false;
        };
        actor.npc.enemy = enemy;
        actor.npc.enemies = enemies;
        if enemy.is_some() {
            actor.npc.state = NpcState::Combat;
        }
        actor.npc.activity = "ACT_RANGE_ATTACK1".into();
        actor.next_think = now;
        actor.fall_owner = Some(zombie);
        let velocity = target.map_or(Vec3::ZERO, |t| {
            headcrab::jump_velocity(origin, t, 600., true)
        });
        if let AiClass::Headcrab(h) = &mut actor.class {
            if target.is_some() {
                // Leap: MoveOrigin(0, 0, 1), LeapTouch, the world-ignore window.
                scene.states[crab].origin.z += 1.;
                h.mid_jump = true;
                h.leap_touch = true;
                h.ignore_world_until = now + headcrab::IGNORE_WORLD_COLLISION_TIME;
            }
        }
        actor.fall = Some((velocity, now));
        self.npc_ids = None;
        true
    }
    /// CNPC_PoisonZombie::Event_Killed (not a torso): the nest crabs to evacuate (and
    /// whether one was in its hand, which only turns its bodygroup off), with the zombie's
    /// enemy for them; the nest empties. None for other NPCs.
    pub fn take_poison_nest(&mut self, zombie: usize) -> Option<([bool; 3], bool, Option<usize>)> {
        let actor = self.actors.get_mut(&zombie)?;
        let enemy = actor.npc.enemy;
        match &mut actor.class {
            AiClass::Zombie(z) if z.kind == zombie::Kind::Poison && !z.torso => {
                let nest = z.nest.take().unwrap_or_default();
                Some((nest, std::mem::take(&mut z.holding_crab), enemy))
            }
            _ => None,
        }
    }
    /// CBlackHeadcrab::Eject (EvacuateNest): off the ground with `velocity`, the zombie's
    /// attacker as enemy (here its enemy), alert, ACT_RANGE_ATTACK1, EjectTouch = LeapTouch
    /// then Panic(U(2, 8)) on landing, the zombie left out of its fall trace.
    pub fn ejected_headcrab(
        &mut self,
        zombie: usize,
        crab: usize,
        velocity: Vec3,
        enemy: Option<usize>,
        scene: &mut Scene,
    ) -> bool {
        let now = scene.time;
        let enemies = self.actors.get(&zombie).map(|a| a.npc.enemies.clone());
        let Some(actor) = self.actors.get_mut(&crab) else {
            return false;
        };
        actor.npc.enemy = enemy;
        if let Some(enemies) = enemies {
            actor.npc.enemies = enemies;
        }
        actor.npc.state = NpcState::Alert;
        actor.npc.activity = "ACT_RANGE_ATTACK1".into();
        actor.next_think = now;
        actor.fall_owner = Some(zombie);
        actor.fall = Some((velocity, now));
        if let AiClass::Headcrab(h) = &mut actor.class {
            h.leap_touch = true;
            h.ignore_world_until = now;
            h.eject_touch = h.black;
        }
        self.npc_ids = None;
        true
    }
    /// The NumGrenades keyvalue of a spawned CNPC_Combine (F4; DESIGN 14r would set it
    /// on the new entity before Spawn). Other classes ignore it.
    pub fn set_grenades(&mut self, id: usize, grenades: i32) {
        if let Some(AiClass::Humanoid(h)) = self.actors.get_mut(&id).map(|a| &mut a.class) {
            if h.kind == humanoid::Kind::Combine {
                h.grenades = grenades.max(0);
            }
        }
    }
    /// CAI_BaseNPC::Spawn: `additionalequipment` (not empty or "0") becomes the active
    /// weapon for classes with bits_CAP_USE_WEAPONS (Weapon_Equip; Spawn does not run
    /// Weapon_CanUse).
    fn equip(
        class: &str,
        equipment: &str,
        defs: &BTreeMap<String, NpcWeaponDef>,
    ) -> Option<NpcWeapon> {
        let equipment = equipment.trim().to_ascii_lowercase();
        if equipment.is_empty() || equipment == "0" || !npc_weapons::uses_weapons(class) {
            return None;
        }
        let def = defs
            .get(&equipment)
            .cloned()
            .or_else(|| npc_weapons::builtin(&equipment))?;
        Some(NpcWeapon::equip(&def))
    }
    /// OnUpdateShotRegulator of `npc_class` for a gun (Reset(true) at equip).
    fn regulator(
        npc_class: &str,
        weapon: Option<&NpcWeapon>,
        defs: &BTreeMap<String, NpcWeaponDef>,
        id: usize,
    ) -> Option<npc_weapons::ShotRegulator> {
        let class = &weapon?.class;
        let spec = defs
            .get(class)
            .cloned()
            .or_else(|| npc_weapons::builtin(class))?
            .fire?;
        // CNPC_Combine never adds bits_CAP_USE_SHOT_REGULATOR: its TASK_RANGE_ATTACK1
        // fires GetRandomBurst() shots itself (Humanoid::start_task).
        if npc_class.eq_ignore_ascii_case("npc_combine_s") {
            return None;
        }
        let rule = match (npc_class, class.as_str()) {
            ("npc_metropolice", "weapon_pistol") => npc_weapons::RegulatorRule::MetropolicePistol,
            ("npc_citizen" | "npc_barney" | "npc_alyx", _) => {
                npc_weapons::RegulatorRule::PlayerCompanion
            }
            _ => npc_weapons::RegulatorRule::Weapon,
        };
        Some(npc_weapons::ShotRegulator::for_weapon(
            &spec,
            rule,
            0.,
            0x2545_f491_4f6c_dd1d ^ id as u64,
        ))
    }
    fn weapon_def(&self, class: &str) -> Option<NpcWeaponDef> {
        self.weapon_defs
            .get(class)
            .cloned()
            .or_else(|| npc_weapons::builtin(class))
    }
    /// Weapon_ShootPosition (m_HackedGunPos (0, 0, 55) of npc_metropolice) and the
    /// GetActualShootPosition target: the enemy's last known position plus its noisy
    /// BodyTarget offset (player: view offset x U(0.7, 1); NPC: the bell curve from the
    /// low body point to the eyes), and the seconds since the enemy became valid.
    fn weapon_aim(
        &mut self,
        id: usize,
        scene: &Scene,
        player: PlayerState,
        now: f64,
    ) -> Option<(Vec3, Vec3, f32)> {
        let actor = &self.actors[&id];
        let enemy = actor.npc.enemy?;
        let memory = actor.npc.enemies.find(Some(enemy)).cloned();
        let shoot = scene.states[id].origin + Vec3::Z * 55.;
        let since_valid = memory.as_ref().map_or(0., |m| (now - m.time_valid) as f32);
        let target = if enemy == PLAYER {
            let feet = memory.map_or(player.feet, |m| m.last_known);
            let view = if player.crouched { 28. } else { 64. };
            feet + Vec3::Z * view * (0.7 + 0.3 * self.random())
        } else {
            let feet = memory.map_or(scene.states.get(enemy)?.origin, |m| m.last_known);
            feet + Vec3::Z * (27. + 37. * (0.5 * self.random() + 0.5 * self.random()))
        };
        Some((shoot, target, since_valid))
    }
    /// A runtime-spawned entity (DESIGN 14r step 3): give it an AI actor when its
    /// class has a behavior. True when one was added.
    /// Every runtime spawn calls it (also non-AI classes): the sight candidate list is
    /// rebuilt to include the new entity.
    pub fn add_actor(&mut self, id: usize, e: &modkit_core::Entity) -> bool {
        self.npc_ids = None;
        match Self::actor_for(id, e, &self.weapon_defs) {
            Some(actor) => {
                self.actors.insert(id, actor);
                true
            }
            None => false,
        }
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
        // PhysicsStep's gravity runs whether or not the NPC thinks (ai_disable).
        self.step_falls(world, scene, physics, player);
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
                // The enemy's WorldSpaceCenter, where AE_ZOMBIE_SWATITEM sends the object.
                let swat_target = self.actors[&id].npc.enemy.map(|e| {
                    if e == PLAYER {
                        player.center()
                    } else {
                        scene.states[e].origin + Vec3::Z * center_z(world, e)
                    }
                });
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
                            // CNPC_PoisonZombie::HandleAnimEvent: the crab throw.
                            if name.eq_ignore_ascii_case(zombie::AE_POISON_PICKUP_CRAB) {
                                z.pick_up_crab();
                            } else if name.eq_ignore_ascii_case(zombie::AE_POISON_THROW_WARN_SOUND)
                            {
                                z.requests
                                    .push(CopRequest::Sound("NPC_PoisonZombie.ThrowWarn"));
                                z.breathe_off_short();
                            } else if name.eq_ignore_ascii_case(zombie::AE_POISON_THROW_SOUND) {
                                z.requests.push(CopRequest::Sound("NPC_PoisonZombie.Throw"));
                                z.breathe_off_short();
                            } else if name.eq_ignore_ascii_case(zombie::AE_POISON_THROW_CRAB) {
                                if z.holding_crab {
                                    z.throw_held_crab(ctx.now, ctx.enemy_eye);
                                }
                            } else if name.eq_ignore_ascii_case(zombie::AE_SWATITEM) {
                                // Send the object toward the enemy's center, forget it and
                                // scan again no sooner than ZOMBIE_SWAT_DELAY.
                                if let (Some((entity, center)), Some(target)) =
                                    (z.physics_ent.take(), swat_target)
                                {
                                    let (velocity, angular) =
                                        zombie::swat_velocity(center, target, rolls);
                                    z.requests.push(CopRequest::Swat {
                                        entity,
                                        velocity,
                                        angular,
                                    });
                                    z.next_swat_scan = ctx.now + zombie::SWAT_DELAY;
                                }
                            } else if let Some((punch, impulse)) =
                                zombie::claw_punch(&name, ctx.yaw, rolls)
                            {
                                let yaw = ctx.yaw.to_radians();
                                z.requests.push(CopRequest::Claw {
                                    start: ctx.origin + Vec3::Z * center_z(world, id),
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
                        } else if let AiClass::Headcrab(h) = class {
                            // CBaseHeadcrab::HandleAnimEvent: the telegraph commits to the
                            // enemy's eyes; the jump leaps at them (or hops without an enemy).
                            if name.eq_ignore_ascii_case(headcrab::AE_JUMP_TELEGRAPH) {
                                h.requests.push(CopRequest::Sound(h.sound("Telegraph")));
                                h.committed_jump = ctx.enemy_eye;
                            } else if name.eq_ignore_ascii_case(headcrab::AE_FLINCH_HOP) && h.black
                            {
                                // CBlackHeadcrab: a spastic hop (forward while already
                                // panicking, else random), then Panic(U(2, 5)).
                                let rolls = [
                                    (uniform() + 1.) * 0.5,
                                    (uniform() + 1.) * 0.5,
                                    (uniform() + 1.) * 0.5,
                                ];
                                let yaw = ctx.yaw.to_radians();
                                let forward = h.panic.then(|| Vec3::new(yaw.cos(), yaw.sin(), 0.));
                                h.requests
                                    .push(CopRequest::Leap(headcrab::jump_flinch(forward, rolls)));
                                let duration = 2. + 3. * f64::from((uniform() + 1.) * 0.5);
                                h.start_panic(ctx.now, duration);
                            } else if name.eq_ignore_ascii_case(headcrab::AE_POISON_FOOTSTEP)
                                && h.black
                            {
                                let cue = if npc.activity == "ACT_WALK" {
                                    "FootstepWalk"
                                } else {
                                    "Footstep"
                                };
                                h.requests.push(CopRequest::Sound(h.sound(cue)));
                            } else if name.eq_ignore_ascii_case(headcrab::AE_POISON_THREAT_SOUND)
                                && h.black
                            {
                                h.requests.push(CopRequest::Sound(h.sound("Threat")));
                                h.requests.push(CopRequest::Sound(h.sound("Alert")));
                            } else if name.eq_ignore_ascii_case(headcrab::AE_JUMPATTACK)
                                && !h.mid_jump
                            {
                                let velocity = match h.committed_jump.take().or(ctx.enemy_eye) {
                                    Some(target) => {
                                        headcrab::jump_velocity(ctx.origin, target, 600., false)
                                    }
                                    None => headcrab::random_hop(ctx.yaw),
                                };
                                h.requests.push(CopRequest::Sound(h.sound("Attack")));
                                h.requests.push(CopRequest::Leap(velocity));
                                h.mid_jump = true;
                                h.leap_touch = true;
                                h.ignore_world_until =
                                    ctx.now + headcrab::IGNORE_WORLD_COLLISION_TIME;
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
    /// IdleSound's lines, and their effects when Speak succeeds.
    fn speak_idle_lines(&mut self, world: &World, scene: &mut Scene) {
        for line in std::mem::take(&mut self.idle_lines) {
            if !self.say(line.id, line.group, world, scene) {
                continue;
            }
            let squadmates: Vec<usize> = self.actors[&line.id]
                .squad
                .as_ref()
                .and_then(|n| self.squads.get(n))
                .map_or(Vec::new(), |s| {
                    s.members
                        .iter()
                        .copied()
                        .filter(|m| *m != line.id)
                        .collect()
                });
            let mut set = |id: usize, f: &mut dyn FnMut(&mut AiActor)| {
                if let Some(actor) = self.actors.get_mut(&id) {
                    f(actor);
                }
            };
            match line.then {
                IdleThen::Nothing => {}
                IdleThen::CombineQuestion(q) => self.combine_question = q,
                IdleThen::CopHarassed => {
                    for m in std::iter::once(line.id).chain(squadmates) {
                        set(m, &mut |a| a.harassed = true);
                    }
                }
                IdleThen::CopAsked(question) => {
                    set(line.id, &mut |a| {
                        a.idle_chatter = COP_CHATTER_WAIT_FOR_RESPONSE
                    });
                    for m in squadmates {
                        set(m, &mut |a| a.idle_chatter = COP_CHATTER_RESPONSE + question);
                    }
                }
                IdleThen::CopAnswered => {
                    for m in std::iter::once(line.id).chain(squadmates) {
                        set(m, &mut |a| a.idle_chatter = COP_CHATTER_ASK_QUESTION);
                    }
                }
            }
        }
    }
    /// CAI_BaseNPC::InitSquad for NPCs in the world (map NPCs on the first tick, maker
    /// and runtime copies when they appear; hidden templates never join), and
    /// Event_Killed's RemoveFromSquad(death) -> NotifyDeadFriend for the killed.
    fn update_squads(&mut self, world: &World, scene: &mut Scene) {
        let ids: Vec<usize> = self.actors.keys().copied().collect();
        for id in ids {
            let state = &scene.states[id];
            let actor = &self.actors[&id];
            if state.killed {
                if let Some(name) = actor.squad.clone() {
                    self.remove_from_squad(id, &name, true, world, scene);
                }
                continue;
            }
            if actor.squad.is_none() && state.visible {
                if let Some(name) = actor.squad_name.clone() {
                    self.add_to_squad(id, &name, scene.time);
                }
            }
        }
    }
    /// CAI_Squad::AddToSquad: join in order; a later member learns the first member's
    /// enemies (UpdateEnemyMemory informed by it, not first-hand).
    fn add_to_squad(&mut self, id: usize, name: &str, now: f64) {
        let squad = self
            .squads
            .entry(name.to_owned())
            .or_insert_with(|| super::squad::Squad::new(name));
        squad.add(id);
        let first = squad.members[0];
        *self
            .diagnostics
            .squad_events
            .entry("join".into())
            .or_default() += 1;
        let known = if first == id {
            Vec::new()
        } else {
            self.actors[&first].npc.enemies.known()
        };
        let actor = self.actors.get_mut(&id).expect("actor");
        actor.squad = Some(name.to_owned());
        for (enemy, position) in known {
            actor
                .npc
                .enemies
                .update(Some(enemy), position, 0., false, now);
        }
    }
    /// CAI_Squad::RemoveFromSquad; on death the remaining members get NotifyDeadFriend.
    fn remove_from_squad(
        &mut self,
        id: usize,
        name: &str,
        death: bool,
        world: &World,
        scene: &mut Scene,
    ) {
        let Some(squad) = self.squads.get_mut(name) else {
            return;
        };
        squad.remove(id);
        let members = squad.members.clone();
        let remaining = squad.num_members();
        if let Some(actor) = self.actors.get_mut(&id) {
            actor.squad = None;
            actor.squad_name = None;
        }
        if !death {
            return;
        }
        *self
            .diagnostics
            .squad_events
            .entry("death".into())
            .or_default() += 1;
        let friend_class = world.entities.get(id).map_or("", |e| e.class());
        for member in members {
            if scene.states.get(member).is_none_or(|s| s.killed) {
                continue;
            }
            let Some(actor) = self.actors.get_mut(&member) else {
                continue;
            };
            // CNPC_MetroPolice::NotifyDeadFriend resets idle chatter waiting for an answer.
            if actor.class.cop().is_some()
                && friend_class != "npc_manhack"
                && actor.idle_chatter == COP_CHATTER_WAIT_FOR_RESPONSE
            {
                actor.idle_chatter = COP_CHATTER_ASK_QUESTION;
            }
            // NotifyDeadFriend: CNPC_MetroPolice (no line for a dead manhack) and
            // CNPC_Combine; LAST_OF_SQUAD when fewer than two remain.
            let group = match &actor.class {
                AiClass::Metropolice(_) if friend_class != "npc_manhack" => {
                    Some(if remaining < 2 {
                        "METROPOLICE_LAST_OF_SQUAD"
                    } else {
                        "METROPOLICE_MAN_DOWN"
                    })
                }
                AiClass::Humanoid(h) if h.kind == humanoid::Kind::Combine => {
                    Some(if remaining < 2 {
                        "COMBINE_LAST_OF_SQUAD"
                    } else {
                        "COMBINE_MAN_DOWN"
                    })
                }
                _ => None,
            };
            if let Some(group) = group {
                self.say(member, group, world, scene);
            }
        }
    }
    /// Squad calls made by the last think: shared first-hand sightings
    /// (CAI_Squad::UpdateEnemyMemory) and SquadNewEnemy (a member without an enemy, or
    /// whose other enemy is unseen for over 3 s, takes the new one).
    fn apply_squad_events(&mut self, scene: &Scene) {
        let now = scene.time;
        for event in std::mem::take(&mut self.squad_events) {
            let (name, from) = match &event {
                SquadEvent::Memory { squad, from, .. } => (squad, Some(*from)),
                SquadEvent::NewEnemy { squad, .. } => (squad, None),
            };
            let Some(squad) = self.squads.get(name) else {
                continue;
            };
            let members: Vec<usize> = squad.members.clone();
            for member in members {
                if Some(member) == from || scene.states.get(member).is_none_or(|s| s.killed) {
                    continue;
                }
                let Some(actor) = self.actors.get_mut(&member) else {
                    continue;
                };
                match &event {
                    SquadEvent::Memory {
                        enemy, position, ..
                    } => {
                        actor
                            .npc
                            .enemies
                            .update(Some(*enemy), *position, 0., false, now);
                        *self
                            .diagnostics
                            .squad_events
                            .entry("memory".into())
                            .or_default() += 1;
                    }
                    SquadEvent::NewEnemy { enemy, .. } => {
                        let current = actor.npc.enemy;
                        if super::squad::should_take_squad_enemy(
                            current,
                            *enemy,
                            actor.npc.conditions.has(COND_SEE_ENEMY),
                            now - actor.npc.enemies.last_time_seen(current),
                        ) {
                            actor.npc.enemy = Some(*enemy);
                            actor.squad_new_enemy = true;
                            *self
                                .diagnostics
                                .squad_events
                                .entry("new_enemy".into())
                                .or_default() += 1;
                        }
                    }
                }
            }
        }
    }
    /// Would a stunstick hit knock the player out (the cop's policing goal)?
    fn knock_out_player(
        &self,
        id: usize,
        world: &World,
        scene: &Scene,
        physics: &Physics,
        player: PlayerState,
    ) -> bool {
        let Some(policing) = self.actors[&id]
            .class
            .cop()
            .and_then(|c| c.policing.as_ref())
        else {
            return false;
        };
        let visible = PhysicsSight { physics, world }.visible(
            scene.states[id].origin + Vec3::Z * eye_z(world, id),
            player.center(),
            &[id],
        );
        policing.goal.should_knock_out(player.center(), visible)
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
        let airborne = actor.fall.is_some();
        let innate_range = matches!(actor.class, AiClass::Headcrab(_))
            .then_some((headcrab::MIN_JUMP_DIST, headcrab::MAX_JUMP_DIST));
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
            airborne,
            innate_range,
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
        let eye = origin + Vec3::Z * eye_z(world, id);
        let center = origin + Vec3::Z * center_z(world, id);
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
            AiClass::Zombie(_) | AiClass::Headcrab(_) => precriminal_hate,
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
        actor.largest_hit = damage_taken;
        let flinched = actor.npc.memory.contains(schedule::MEMORY_FLINCHED);
        // CBlackHeadcrab::IsHeavyDamage: any hit over 1 is heavy while it has not flinched.
        let heavy = match &actor.class {
            AiClass::Headcrab(h) if h.black => !flinched && damage_taken > 1.,
            _ => damage_taken > 20.,
        };
        if damage_taken > 0. {
            actor.npc.conditions.set(COND_LIGHT_DAMAGE);
            // OnTakeDamage_Alive: Forget(bits_MEMORY_INCOVER), and bits_MEMORY_FLINCHED
            // once m_flNextFlinchTime has passed.
            actor.npc.memory.remove(schedule::MEMORY_INCOVER);
            if flinched && now > actor.npc.next_flinch {
                actor.npc.memory.remove(schedule::MEMORY_FLINCHED);
            }
        }
        if heavy {
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
                (
                    o,
                    o + Vec3::Z * center_z(world, enemy),
                    o + Vec3::Z * eye_z(world, enemy),
                    Vec3::ZERO,
                )
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
                let delta = t_center - center;
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
                    let delta = t_center - center;
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
                        && sight.visible(center, t_center, &[id, enemy])
                    {
                        actor.npc.conditions.set(COND_CAN_MELEE_ATTACK1);
                    }
                }
                // CBaseHeadcrab::RangeAttack1Conditions (EnemyDistance between the centers,
                // less the height the two boxes can overlap; the high-enemy hull check is
                // not modeled).
                if let AiClass::Headcrab(crab) = &mut actor.class {
                    let enemy_height = 72.;
                    let mut delta = t_center - center;
                    let overlap = (enemy_height + 24.) * 0.5;
                    delta.z = if delta.z > overlap {
                        delta.z - overlap
                    } else if delta.z < -overlap {
                        delta.z + overlap
                    } else {
                        0.
                    };
                    let forward = facing.truncate().normalize_or_zero();
                    let dot = (t_feet - origin)
                        .truncate()
                        .normalize_or_zero()
                        .dot(forward);
                    let check = headcrab::LeapCheck {
                        ready: now >= crab.next_attack,
                        on_ground: actor.fall.is_none(),
                        dot,
                        distance: delta.length(),
                        clear: sight.visible(eye, t_eye, &[id, enemy]),
                        high_blocked: false,
                    };
                    let condition = headcrab::range_condition(check);
                    crab.last_check =
                        Some((check.distance, check.dot, condition.map(|c| c.0 as i32)));
                    if let Some(c) = condition {
                        actor.npc.conditions.set(c);
                    }
                }
                // CNPC_BaseZombie::MeleeAttack1Conditions: within the claw reach (origin
                // to origin), facing within 0.7, and the cube hull swept along the facing
                // meets the enemy (an NPC or breakable in the way is not modeled).
                if let Some(reach) = claws {
                    let to = t_feet - origin;
                    let forward = facing.truncate().normalize_or_zero().extend(0.);
                    let dot = to.truncate().normalize_or_zero().dot(forward.truncate());
                    let start = center;
                    let end = start + forward * reach;
                    let half = if enemy == PLAYER { 16. } else { 13. };
                    let cube = Vec3::splat(
                        world
                            .entities
                            .get(id)
                            .and_then(|e| crate::ai_hull::class_hull(e.class()))
                            .map_or(13., |h| h.bounds().maxs.x),
                    );
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
                // CNPC_PoisonZombie::RangeAttack1/2Conditions (EnemyDistance between the
                // centers less the boxes' height overlap; a CAN condition needs the line
                // from the eyes, GatherAttackConditions' weapon LOS).
                let poison = match &mut actor.class {
                    AiClass::Zombie(z) if z.kind == zombie::Kind::Poison && !z.torso => Some(z),
                    _ => None,
                };
                if let Some(z) = poison {
                    if z.nest.is_none() {
                        z.nest = Some(std::array::from_fn(|i| {
                            scene.bodygroup(world, id, zombie::BODYGROUP_NEST_BASE + i) == Some(1)
                        }));
                    }
                    let mut delta = t_center - center;
                    let overlap = 72.;
                    delta.z = if delta.z > overlap {
                        delta.z - overlap
                    } else if delta.z < -overlap {
                        delta.z + overlap
                    } else {
                        0.
                    };
                    let dot = (t_feet - origin)
                        .truncate()
                        .normalize_or_zero()
                        .dot(facing.truncate().normalize_or_zero());
                    let los = sight.visible(eye, t_eye, &[id, enemy]);
                    for condition in z.poison_range_conditions(now, delta.length(), dot) {
                        let Some(c) = condition else { continue };
                        if !matches!(c, COND_CAN_RANGE_ATTACK1 | COND_CAN_RANGE_ATTACK2) || los {
                            actor.npc.conditions.set(c);
                        }
                    }
                }
                // GatherAttackConditions: when any attack is possible the range complaints
                // go (a poison zombie ready to throw at a player inside the throw's 250
                // units backed away forever instead of leaping a crab or clawing).
                clear_range_complaints(&mut actor.npc.conditions);
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
        // CZombie::GatherConditions: the blocking door opened (or is opening): COND_DOOR_OPENED
        // and forget it; still closed: COND_BLOCKED_BY_DOOR.
        if let AiClass::Zombie(z) = &mut actor.class {
            actor.npc.conditions.clear(zombie::COND_BLOCKED_BY_DOOR);
            actor.npc.conditions.clear(zombie::COND_DOOR_OPENED);
            if let Some((door, _)) = z.blocking_door {
                if scene.states.get(door).is_some_and(|s| s.door_closed()) {
                    actor.npc.conditions.set(zombie::COND_BLOCKED_BY_DOOR);
                } else {
                    actor.npc.conditions.set(zombie::COND_DOOR_OPENED);
                    z.blocking_door = None;
                }
            }
        }
        // CNPC_BaseZombie::GatherConditions: in combat, every 2 s while it has no object,
        // FindNearestPhysicsObject; COND_ZOMBIE_CAN_SWAT_ATTACK while it has one, m_flNextSwat
        // has passed and it sees the enemy. The object's center follows its body.
        if let AiClass::Zombie(z) = &mut actor.class {
            z.physics_ent = z.physics_ent.and_then(|(object, _)| {
                physics
                    .entity_body(object)
                    .and_then(|body| physics.bounds_center(body))
                    .map(|center| (object, center))
            });
            let enemy_feet = actor.npc.enemy.map(|e| {
                if e == PLAYER {
                    player.feet
                } else {
                    scene.states[e].origin
                }
            });
            if let (NpcState::Combat, Some(enemy_feet), true) =
                (actor.npc.state, enemy_feet, z.swats())
            {
                if now >= z.next_swat_scan && z.physics_ent.is_none() {
                    // The human hull: 72 units tall, knees at a quarter of it.
                    if let Some(half) = zombie::swat_search_box(origin, enemy_feet, 72.) {
                        let candidates =
                            swat_candidates(physics, world, &sight, id, origin, eye, half);
                        z.physics_ent = zombie::find_nearest_physics_object(
                            origin,
                            enemy_feet,
                            origin.z + 18.,
                            eye.z,
                            &candidates,
                        )
                        .and_then(|object| {
                            candidates
                                .iter()
                                .find(|c| c.entity == object)
                                .map(|c| (object, c.center))
                        });
                    }
                    z.next_swat_scan = now + zombie::SWAT_SCAN_INTERVAL;
                }
            }
            if z.physics_ent.is_some()
                && now >= z.next_swat
                && actor.npc.conditions.has(COND_SEE_ENEMY)
            {
                actor
                    .npc
                    .conditions
                    .set(zombie::COND_ZOMBIE_CAN_SWAT_ATTACK);
            } else {
                actor
                    .npc
                    .conditions
                    .clear(zombie::COND_ZOMBIE_CAN_SWAT_ATTACK);
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
        if let AiClass::Zombie(z) = &mut actor.class {
            if z.kind == zombie::Kind::Poison {
                let max_health = scene
                    .npc_max_health
                    .get(&id)
                    .copied()
                    .unwrap_or_else(|| {
                        crate::gameplay::npc_spawn_health(scene, &world.entities[id])
                    })
                    .max(1.);
                z.poison_think(
                    now,
                    actor.npc.conditions.has(COND_NEW_ENEMY),
                    std::mem::take(&mut actor.largest_hit) / max_health,
                );
                let origin = scene.states[id].origin;
                let enemy_distance = actor.npc.enemy.and_then(|e| {
                    let position = if e == PLAYER {
                        Some(player.feet)
                    } else {
                        scene.states.get(e).map(|s| s.origin)
                    };
                    position.map(|p| (p - origin).length())
                });
                z.breathe(enemy_distance);
            }
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
            AiClass::Zombie(z) => (zombie::max_yaw_speed(&actor.npc.activity, z.torso), false),
            AiClass::Headcrab(h) => (headcrab::max_yaw_speed(&actor.npc.activity, h.black), false),
        };
        if let AiClass::Headcrab(crab) = &mut actor.class {
            crab.sees_enemy = actor.npc.conditions.has(COND_SEE_ENEMY);
        }
        // CNPC_BaseZombie::OnInsufficientStopDist: a move blocked by an entity while a swat
        // is allowed remembers it as m_hObstructor (only VPhysics objects can be swatted).
        if let AiClass::Zombie(z) = &mut actor.class {
            let blocker = actor
                .goal
                .and_then(|key| controller.status(key))
                .and_then(|status| match status {
                    crate::npc::GoalState::Blocked(crate::npc::GoalBlockReason::Collision {
                        blocker: Some(b),
                        ..
                    }) => Some(*b),
                    _ => None,
                });
            // CZombie::OnObstructingDoor: a closed brush door (CBaseDoor) blocked the move;
            // face it (the hit normal reversed: here the direction to its center).
            if let Some(door) = blocker.filter(|b| {
                world
                    .entities
                    .get(*b)
                    .is_some_and(|e| matches!(e.class(), "func_door" | "func_door_rotating"))
                    && scene.states.get(*b).is_some_and(|s| s.door_closed())
            }) {
                let origin = scene.states[id].origin;
                let center = physics
                    .entity_world_bounds(door)
                    .map_or(origin, |(lo, hi)| (lo + hi) * 0.5);
                let to = center - origin;
                z.blocking_door = Some((door, to.y.atan2(to.x).to_degrees()));
            }
            if let Some(b) = blocker.filter(|_| now >= z.next_swat) {
                if let (Some((mass, _, _)), Some(center)) = (
                    physics.entity_rest_state(b),
                    physics
                        .dynamic
                        .get(&b)
                        .and_then(|h| physics.bounds_center(*h)),
                ) {
                    z.obstructor = Some((b, center, mass));
                }
            }
        }
        let on_fire = matches!(&actor.class, AiClass::Zombie(_)) && scene.fires.burning(id);
        // CBlackHeadcrab::NPC_TranslateActivity: ACT_RUN/ACT_WALK scurry while panicking or
        // burning (the controller's locomotion variant).
        if let AiClass::Headcrab(h) = &actor.class {
            if h.black {
                let panic = h.panic || scene.fires.burning(id);
                if panic != actor.panic_locomotion {
                    actor.panic_locomotion = panic;
                    controller.set_locomotion(id, if panic { headcrab::RUN_PANIC } else { "" });
                }
            }
        }
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
    /// CAI_SentenceBase::Speak: FOkToMakeSound (no squads yet) culls a sentence of equal
    /// or lower priority while the last one's timer runs; JustMadeSound(priority, 2 s)
    /// sets it to 2 + U(1.5, 2.0) s. False when culled.
    /// The sound cues ally speech can play (audio preload).
    pub fn speech_cues(&self) -> Vec<String> {
        self.ally.cues()
    }
    /// CAI_PlayerAlly::SpeakIfAllowed(concept): the talker rules pick a response for this
    /// NPC's criteria; its sound plays at the speaker (lip sync through the actor).
    fn ally_say(
        &mut self,
        id: usize,
        concept: &'static str,
        world: &World,
        scene: &mut Scene,
        player: PlayerState,
    ) -> bool {
        let Some(e) = world.entities.get(id) else {
            return false;
        };
        let Some(actor) = self.actors.get(&id) else {
            return false;
        };
        let state = &scene.states[id];
        let model = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))
            .map_or("", |i| i.model.as_str());
        let enemy = actor.npc.enemy.map(|en| {
            if en == PLAYER {
                "player"
            } else {
                world.entities.get(en).map_or("", |x| x.class())
            }
        });
        let npc_state = format!("{:?}", actor.npc.state);
        let weapon = actor.weapon.as_ref().map(|w| w.class.clone());
        let gagged = e
            .get("spawnflags")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .is_some_and(|f| f & SF_NPC_GAG != 0);
        let speaker = super::ally_speech::Speaker {
            id,
            classname: e.class(),
            name: e.get("targetname").unwrap_or(""),
            // An NPC nobody hurt yet keeps 0 here and has its spawn health.
            health: if state.killed {
                0.
            } else if state.value > 0. {
                state.value
            } else {
                crate::gameplay::npc_spawn_health(scene, e)
            },
            max_health: crate::gameplay::npc_spawn_health(scene, e),
            npc_state: &npc_state,
            enemy,
            weapon: weapon.as_deref(),
            // sv_npc_talker_maxdist measures to the player whether or not he flies.
            distance_to_player: (state.origin + Vec3::Z * 36.).distance(player.center()),
            map: &world.name,
            gender: if model.to_ascii_lowercase().contains("female") {
                "female"
            } else {
                "male"
            },
            gagged,
            now: scene.time,
        };
        let mut rolls = [0f32; 8];
        for r in &mut rolls {
            *r = self.random();
        }
        let mut next = rolls.into_iter().cycle();
        let mut random = move || next.next().unwrap_or(0.5);
        let sound = match self.ally.speak(concept, &speaker, &mut random) {
            Ok(sound) => {
                *self
                    .diagnostics
                    .ally_speech
                    .entry(format!("{concept}:spoken"))
                    .or_default() += 1;
                sound
            }
            Err(reason) => {
                *self
                    .diagnostics
                    .ally_speech
                    .entry(format!("{concept}:{reason}"))
                    .or_default() += 1;
                return false;
            }
        };
        let origin = state.origin;
        scene.sounds.push(crate::sounds::SoundRequest {
            origin: Some(origin + Vec3::Z * 64.),
            // GenderExpandString needs the speaker's model: without it a `$gender`
            // wave picks from both genders (owner report: female citizens said male
            // pain lines and the reverse).
            actor: Some(crate::sounds::SoundActor {
                name: e.get("targetname").unwrap_or("").into(),
                model: model.to_owned(),
                entity: Some(id),
            }),
            ..sound.into()
        });
        true
    }
    /// CAI_SentenceBase::Speak: MatchesCriteria (the group's SentenceCriteria_t), then a
    /// prioritized line asks FOkToMakeSound (the squad's gate in a squad, else the NPC's)
    /// and JustMadeSound(priority, 2 s) sets the NPC's and the squad's waits to
    /// 2 + U(1.5, 2.0) s. False when refused or culled.
    fn say(&mut self, id: usize, group: &'static str, world: &World, scene: &mut Scene) -> bool {
        let origin = scene.states[id].origin;
        let Some(actor) = self.actors.get(&id) else {
            return false;
        };
        let squad = actor.squad.as_ref().and_then(|name| self.squads.get(name));
        let gagged = actor.npc.state != NpcState::Combat
            && world.entities.get(id).is_some_and(|e| {
                e.get("spawnflags")
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .is_some_and(|f| f & SF_NPC_GAG != 0)
            });
        let matches = match sentence_criteria(group) {
            Criteria::Always => true,
            Criteria::Normal => !gagged,
            Criteria::InSquad => !gagged && squad.is_some_and(|s| s.num_members() > 1),
            Criteria::SquadLeader => {
                !gagged && squad.is_some_and(|s| s.num_members() > 1 && s.leader() == Some(id))
            }
        };
        if !matches {
            *self
                .diagnostics
                .criteria_refused
                .entry(group.into())
                .or_default() += 1;
            return false;
        }
        let squad_name = actor.squad.clone();
        let priority = sentence_priority(group);
        if priority >= 0 {
            let now = scene.time;
            let ok = match squad_name.as_ref().and_then(|n| self.squads.get(n)) {
                Some(squad) => squad.ok_to_make_sound(priority, now),
                None => self
                    .sound_wait
                    .get(&id)
                    .is_none_or(|&(until, last)| now > until || priority > last),
            };
            if !ok {
                self.diagnostics.culled_sentences += 1;
                return false;
            }
            let wait = now + 2. + 1.5 + 0.5 * f64::from(self.random());
            self.sound_wait.insert(id, (wait, priority));
            let squad_wait = now + 2. + 1.5 + 0.5 * f64::from(self.random());
            if let Some(squad) = squad_name.as_ref().and_then(|n| self.squads.get_mut(n)) {
                squad.just_made_sound(priority, squad_wait);
            }
        }
        self.diagnostics.sentences += 1;
        *self
            .diagnostics
            .sentence_groups
            .entry(group.into())
            .or_default() += 1;
        scene.sounds.push(crate::sounds::SoundRequest {
            origin: Some(origin + Vec3::Z * 64.),
            actor: Some(crate::sounds::SoundActor {
                name: scene
                    .entity(world, id)
                    .and_then(|e| e.get("targetname"))
                    .unwrap_or("")
                    .into(),
                model: String::new(),
                entity: Some(id),
            }),
            ..format!("#{group}").into()
        });
        true
    }
    /// CAI_BaseNPC::OnTakeDamage_Alive -> PainSound, or Event_Killed -> DeathSound
    /// (CNPC_MetroPolice/CNPC_Combine): pain at most once a second, the first light and
    /// heavy hits with their own groups; the death line once.
    fn pain_or_death_sound(
        &mut self,
        id: usize,
        world: &World,
        scene: &mut Scene,
        player: PlayerState,
    ) {
        let now = scene.time;
        let Some(e) = world.entities.get(id) else {
            return;
        };
        let max = crate::gameplay::npc_spawn_health(scene, e).max(1.);
        let Some(actor) = self.actors.get_mut(&id) else {
            return;
        };
        let state = &scene.states[id];
        // CHeadcrab::PainSound / DeathSound.
        if let AiClass::Headcrab(crab) = &actor.class {
            let cue = if state.killed {
                if std::mem::replace(&mut actor.death_spoken, true) {
                    return;
                }
                crab.sound("Die")
            } else {
                if now < actor.next_pain_sound || state.value <= 0. {
                    return;
                }
                actor.next_pain_sound = now + 1.;
                crab.sound("Pain")
            };
            let origin = state.origin;
            scene.sounds.push(crate::sounds::SoundRequest {
                origin: Some(origin + Vec3::Z * 12.),
                ..cue.into()
            });
            return;
        }
        // CZombie / CNPC_PoisonZombie PainSound and DeathSound.
        if let AiClass::Zombie(z) = &mut actor.class {
            let kind = z.kind;
            let cue = if state.killed {
                if std::mem::replace(&mut actor.death_spoken, true) {
                    return;
                }
                // StopLoopingSounds: the breathing patches go with the body.
                z.stop_breathing();
                kind.cue("Zombie.Die")
            } else {
                if now < actor.next_pain_sound || state.value <= 0. {
                    return;
                }
                actor.next_pain_sound = now + 1.;
                // CNPC_PoisonZombie::PainSound: BreatheOffShort first.
                if kind == zombie::Kind::Poison {
                    z.breathe_off_short();
                }
                kind.cue("Zombie.Pain")
            };
            let origin = state.origin;
            for request in z.requests.drain(..) {
                if let CopRequest::Patch { cue, op } = request {
                    scene.sounds.push(crate::sounds::SoundRequest {
                        origin: Some(origin + Vec3::Z * 64.),
                        ..crate::sounds::SoundRequest::patch(
                            cue,
                            crate::sounds::entity_emitter(id),
                            op,
                        )
                    });
                }
            }
            scene.sounds.push(crate::sounds::SoundRequest {
                origin: Some(origin + Vec3::Z * 64.),
                ..cue.into()
            });
            return;
        }
        // CAI_PlayerAlly::PainSound speaks TLK_WOUND; the companions' DeathSound emits
        // their die sound (CNPC_Citizen/CNPC_Barney/CNPC_Alyx).
        if matches!(&actor.class, AiClass::Humanoid(h) if matches!(h.kind, humanoid::Kind::Companion { .. }))
        {
            if state.killed {
                if std::mem::replace(&mut actor.death_spoken, true) {
                    return;
                }
                let die = match e.class() {
                    "npc_barney" => "npc_barney.die",
                    "npc_alyx" => "npc_alyx.die",
                    _ => "NPC_Citizen.Die",
                };
                let origin = state.origin;
                // EmitSound from the NPC: GenderExpandString reads its model (the
                // citizens' die waves are vo/npc/$gender01/pain*.wav; owner report: dying
                // citizens moaned in the other gender's voice).
                let model = world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(id))
                    .map_or("", |i| i.model.as_str());
                scene.sounds.push(crate::sounds::SoundRequest {
                    origin: Some(origin + Vec3::Z * 64.),
                    actor: Some(crate::sounds::SoundActor {
                        name: e.get("targetname").unwrap_or("").into(),
                        model: model.to_owned(),
                        entity: Some(id),
                    }),
                    ..die.into()
                });
                return;
            }
            if now < actor.next_pain_sound || state.value <= 0. {
                return;
            }
            actor.next_pain_sound = now + 1.;
            self.ally_say(id, "TLK_WOUND", world, scene, player);
            return;
        }
        let speech = if state.killed {
            if std::mem::replace(&mut actor.death_spoken, true) {
                return;
            }
            Speech::Die
        } else {
            let ratio = state.value / max;
            if now < actor.next_pain_sound || ratio <= 0. {
                return;
            }
            actor.next_pain_sound = now + 1.;
            let combine =
                matches!(&actor.class, AiClass::Humanoid(h) if h.kind == humanoid::Kind::Combine);
            if combine {
                if actor.pain_memory & 1 == 0 && ratio > 0.9 {
                    actor.pain_memory |= 1;
                    Speech::PainLight
                } else if actor.pain_memory & 2 == 0 && ratio < 0.25 {
                    actor.pain_memory |= 2;
                    Speech::PainHeavy
                } else {
                    Speech::Pain
                }
            } else if actor.pain_memory & 2 == 0 && ratio < 0.25 {
                actor.pain_memory |= 3;
                Speech::PainHeavy
            } else if actor.pain_memory & 1 == 0 && ratio > 0.8 {
                actor.pain_memory |= 1;
                Speech::PainLight
            } else {
                Speech::Pain
            }
        };
        if let Some(group) = speech_group(&actor.class, speech) {
            self.say(id, group, world, scene);
        }
    }
    /// Carry out the cop's requests (sentences, look targets, outputs, sounds, hits).
    fn apply_requests(
        &mut self,
        id: usize,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        player: PlayerState,
    ) {
        let origin = scene.states[id].origin;
        let requests = {
            let actor = self.actors.get_mut(&id).expect("actor");
            std::mem::take(actor.class.requests_mut())
        };
        for request in requests {
            match request {
                CopRequest::Sentence(group) => {
                    self.say(id, group, world, scene);
                }
                CopRequest::Concept(concept) => {
                    self.ally_say(id, concept, world, scene, player);
                }
                CopRequest::LookAtPlayer {
                    importance,
                    duration,
                } => scene.look_targets.add(
                    id,
                    crate::attention::Target::Player,
                    importance,
                    scene.time,
                    f64::from(duration),
                ),
                CopRequest::Output(name) => scene.fire(id, name, usize::MAX),
                CopRequest::Swat {
                    entity,
                    velocity,
                    angular,
                } => {
                    self.prop_swats.push((entity, velocity, angular));
                    scene.sounds.push(crate::sounds::SoundRequest {
                        origin: Some(origin + Vec3::Z * 36.),
                        ..String::from("NPC_BaseZombie.Swat").into()
                    });
                }
                CopRequest::Patch { cue, op } => {
                    scene.sounds.push(crate::sounds::SoundRequest {
                        origin: Some(origin + Vec3::Z * 64.),
                        ..crate::sounds::SoundRequest::patch(
                            cue,
                            crate::sounds::entity_emitter(id),
                            op,
                        )
                    });
                }
                CopRequest::Sound(name) => {
                    let cue = match name {
                        "melee_hit" => self.stunstick_sounds.0.clone(),
                        "melee_miss" => self.stunstick_sounds.1.clone(),
                        other => Some(other.to_owned()),
                    };
                    if let Some(cue) = cue {
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(origin + Vec3::Z * 36.),
                            ..cue.into()
                        });
                    }
                }
                CopRequest::PlayerHit {
                    damage,
                    punch,
                    impulse,
                    fade,
                    knock_out,
                } => {
                    self.diagnostics.player_hits += 1;
                    if knock_out {
                        let cop = self.actors[&id].class.cop();
                        if let Some(goal) =
                            cop.and_then(|c| c.policing.as_ref()).map(|p| p.goal.entity)
                        {
                            scene.fire(goal, "OnKnockOut", usize::MAX);
                        }
                    }
                    self.player_hits.push(PlayerHit {
                        attacker: id,
                        damage,
                        punch,
                        impulse,
                        fade: fade.map(|(color, duration, hold, flags)| {
                            crate::player_damage::ScreenFade::new(color, duration, hold, flags)
                        }),
                    });
                }
                CopRequest::CallForJustice => self.call_for_justice(id, world, scene, physics),
                CopRequest::AltFire(target) => {
                    // FireNPCSecondaryAttack: WeaponSound(WPN_DOUBLE) and a ball from
                    // Weapon_ShootPosition toward the elite's m_vecAltFireTarget.
                    let start = origin + Vec3::Z * 55.;
                    let aim = (target - start).normalize_or_zero();
                    self.ball_launches.push((id, start, aim * 1000.));
                    let cue = self.actors[&id]
                        .weapon
                        .as_ref()
                        .and_then(|w| self.weapon_def(&w.class))
                        .and_then(|d| d.sounds.get("double_shot").cloned());
                    if let Some(cue) = cue {
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(start),
                            ..cue.into()
                        });
                    }
                }
                CopRequest::GrenadeToss => {
                    let position = scene
                        .attachment_frames(world, id, &["lefthand"])
                        .first()
                        .copied()
                        .flatten()
                        .map_or(origin + Vec3::Z * 48., |m| m.w_axis.truncate());
                    let spin = Vec3::new(
                        self.random() * 2000. - 1000.,
                        self.random() * 2000. - 1000.,
                        self.random() * 2000. - 1000.,
                    );
                    let now = scene.time;
                    if let Some(AiClass::Humanoid(h)) =
                        self.actors.get_mut(&id).map(|a| &mut a.class)
                    {
                        self.grenade_tosses
                            .push((id, position, h.toss_velocity, spin));
                        h.grenades -= 1;
                        // Wait six seconds before even looking again.
                        h.next_grenade_check = now + 6.;
                    }
                }
                CopRequest::Claw {
                    start,
                    forward,
                    punch,
                    impulse,
                    reach,
                    bite,
                } => {
                    let enemy = self.actors[&id].npc.enemy;
                    let enemy_center = enemy.map(|e| {
                        if e == PLAYER {
                            player.center()
                        } else {
                            scene.states[e].origin + Vec3::Z * center_z(world, e)
                        }
                    });
                    // ClawAttack: no swing through a wall between the zombie and its
                    // enemy (an 8-unit hull to the enemy's center, MASK_SOLID_BRUSHONLY:
                    // world brushes and static props, not physics props).
                    if enemy_center.is_some_and(|c| {
                        physics
                            .filtered_ray(start, c, &|entity| entity == usize::MAX)
                            .is_some()
                    }) {
                        continue;
                    }
                    let end = start + forward * reach;
                    // The hull made a cube (the x extents).
                    let cube = world
                        .entities
                        .get(id)
                        .and_then(|e| crate::ai_hull::class_hull(e.class()))
                        .map_or(13., |h| h.bounds().maxs.x);
                    let (mins, maxs) = (-Vec3::splat(cube), Vec3::splat(cube));
                    let damage = if bite {
                        self.zombie_poison_spit_damage
                    } else {
                        self.zombie_slash_damage
                    };
                    let player_hit = enemy == Some(PLAYER)
                        && super::metropolice::segment_hits_box(
                            start,
                            end,
                            player.feet + Vec3::new(-16., -16., 0.) - maxs,
                            player.feet + Vec3::new(16., 16., 72.) - mins,
                        );
                    let hit = if player_hit {
                        self.player_hits.push(PlayerHit {
                            attacker: id,
                            damage,
                            punch,
                            impulse,
                            fade: None,
                        });
                        self.diagnostics.player_hits += 1;
                        true
                    } else if let Some(victim) =
                        self.melee_victim(id, world, scene, start, end, mins, maxs)
                    {
                        self.diagnostics.npc_melee_hits += 1;
                        let center =
                            scene.states[victim].origin + Vec3::Z * center_z(world, victim);
                        self.melee_hits.push(MeleeHit {
                            attacker: id,
                            victim,
                            damage,
                            position: center,
                            direction: forward,
                        });
                        true
                    } else {
                        // SCHED_ZOMBIE_ATTACKITEM: a miss strikes m_hPhysicsEnt (TakeDamage
                        // with a 120 push toward it, then m_flNextSwat = now + U(2, 4)).
                        let attack_item = self.actors[&id].npc.schedule.as_deref()
                            == Some("SCHED_ZOMBIE_ATTACKITEM");
                        let roll = self.random();
                        match &mut self.actors.get_mut(&id).expect("actor").class {
                            AiClass::Zombie(z) if attack_item => match z.physics_ent.take() {
                                Some((object, center)) => {
                                    let me = origin + Vec3::Z * center_z(world, id);
                                    let direction = (center - me).normalize_or_zero();
                                    scene.entity_damage.push(crate::projectiles::Damage {
                                        target: crate::projectiles::DamageTarget::Entity(object),
                                        amount: damage,
                                        kind: crate::player_damage::DMG_SLASH,
                                        dissolve: false,
                                        direction,
                                        origin: center,
                                        attacker: crate::projectiles::Attacker::Entity(id),
                                    });
                                    if let Some((mass, _, _)) = physics.entity_rest_state(object) {
                                        self.prop_swats.push((
                                            object,
                                            direction * zombie::ATTACKITEM_FORCE / mass.max(1.),
                                            Vec3::ZERO,
                                        ));
                                    }
                                    z.next_swat = scene.time + 2. + 2. * f64::from(roll);
                                    self.diagnostics.obstructor_swats += 1;
                                    true
                                }
                                None => false,
                            },
                            _ => false,
                        }
                    };
                    scene.sounds.push(crate::sounds::SoundRequest {
                        origin: Some(origin + Vec3::Z * 36.),
                        ..String::from(if hit {
                            "Zombie.AttackHit"
                        } else {
                            "Zombie.AttackMiss"
                        })
                        .into()
                    });
                }
                CopRequest::SetBodygroup { group, value } => {
                    scene.set_bodygroup(world, id, group, value);
                }
                CopRequest::ThrowCrab { target } => {
                    let eye = origin + Vec3::Z * eye_z(world, id);
                    self.crab_throws.push((id, eye, yaw_of(scene, id), target));
                }
                CopRequest::Leap(velocity) => {
                    // CBaseHeadcrab::Leap: off the ground (MoveOrigin(0, 0, 1) with headroom)
                    // with the velocity; step_falls carries it and its LeapTouch.
                    scene.states[id].origin.z += 1.;
                    if let Some(actor) = self.actors.get_mut(&id) {
                        actor.fall = Some((velocity, scene.time));
                    }
                }
                CopRequest::Kick { start, forward } => {
                    let end = start + forward * 70.;
                    let (mins, maxs) = (-Vec3::new(16., 16., 18.), Vec3::new(16., 16., 18.));
                    let elite = self.actors[&id].class.humanoid().is_some_and(|h| h.elite);
                    let damage = if elite {
                        self.combine_guard_kick_damage
                    } else {
                        self.combine_kick_damage
                    };
                    let player_hit = self.actors[&id].npc.enemy == Some(PLAYER)
                        && super::metropolice::segment_hits_box(
                            start,
                            end,
                            player.feet + Vec3::new(-16., -16., 0.) - maxs,
                            player.feet + Vec3::new(16., 16., 72.) - mins,
                        );
                    let hit = if player_hit {
                        // ViewPunch(-12, -7, 0) and forward * 100 + up * 50.
                        self.player_hits.push(PlayerHit {
                            attacker: id,
                            damage,
                            punch: Vec3::new(-12., -7., 0.),
                            impulse: forward * 100. + Vec3::Z * 50.,
                            fade: None,
                        });
                        self.diagnostics.player_hits += 1;
                        true
                    } else if let Some(victim) =
                        self.melee_victim(id, world, scene, start, end, mins, maxs)
                    {
                        self.diagnostics.npc_melee_hits += 1;
                        let center =
                            scene.states[victim].origin + Vec3::Z * center_z(world, victim);
                        self.melee_hits.push(MeleeHit {
                            attacker: id,
                            victim,
                            damage,
                            position: center,
                            direction: forward,
                        });
                        true
                    } else {
                        false
                    };
                    if hit {
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(origin + Vec3::Z * 36.),
                            ..String::from("NPC_Combine.WeaponBash").into()
                        });
                    }
                    self.diagnostics.sentences += 1;
                    scene.sounds.push(crate::sounds::SoundRequest {
                        origin: Some(origin + Vec3::Z * 64.),
                        actor: Some(crate::sounds::SoundActor {
                            name: scene
                                .entity(world, id)
                                .and_then(|e| e.get("targetname"))
                                .unwrap_or("")
                                .into(),
                            model: String::new(),
                            entity: Some(id),
                        }),
                        ..String::from("#COMBINE_KICK").into()
                    });
                }
                CopRequest::Swing {
                    start,
                    end,
                    mins,
                    maxs,
                    damage,
                } => {
                    let victim = self.melee_victim(id, world, scene, start, end, mins, maxs);
                    let cue = if let Some(victim) = victim {
                        self.diagnostics.npc_melee_hits += 1;
                        let center =
                            scene.states[victim].origin + Vec3::Z * center_z(world, victim);
                        self.melee_hits.push(MeleeHit {
                            attacker: id,
                            victim,
                            damage,
                            position: center,
                            direction: (center - (origin + Vec3::Z * 36.)).normalize_or_zero(),
                        });
                        // CTraceFilterMelee: InsertSound(SOUND_COMBAT, damage position, 200, 0.2).
                        self.sounds.insert(
                            senses::sound::COMBAT,
                            center,
                            200.,
                            0.2,
                            Some(id),
                            scene.time,
                        );
                        self.stunstick_sounds.0.clone()
                    } else {
                        self.stunstick_sounds.1.clone()
                    };
                    if let Some(cue) = cue {
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(origin + Vec3::Z * 36.),
                            ..cue.into()
                        });
                    }
                }
            }
        }
    }
    /// CheckTraceHullAttack's NPC part: the nearest NPC along the swept box (`mins`..`maxs`
    /// around the segment) whose hull it touches and that the attacker hates
    /// (CTraceFilterMelee only damages hated combat characters). World blocking is not
    /// traced (the player swing does not trace it either).
    #[allow(clippy::too_many_arguments)]
    fn melee_victim(
        &self,
        id: usize,
        world: &World,
        scene: &Scene,
        start: Vec3,
        end: Vec3,
        mins: Vec3,
        maxs: Vec3,
    ) -> Option<usize> {
        let precriminal = scene.globals.is_on("gordon_precriminal");
        world
            .entities
            .iter()
            .enumerate()
            .filter(|(other, e)| *other != id && e.class().starts_with("npc_"))
            .filter_map(|(other, e)| {
                let state = scene.states.get(other)?;
                if state.killed || !state.visible {
                    return None;
                }
                let (hull_min, hull_max) = crate::physics::npc_hull(e.class())?;
                let box_min = state.origin + hull_min - maxs;
                let box_max = state.origin + hull_max - mins;
                (super::metropolice::segment_hits_box(start, end, box_min, box_max)
                    && self.npc_disposition(world, id, other, precriminal) == Disposition::HT)
                    .then(|| {
                        (
                            other,
                            (state.origin + Vec3::Z * 36.).distance_squared(start),
                        )
                    })
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(other, _)| other)
    }
    /// AdministerJustice's search: the first other allowed-to-respond cop within 512
    /// units that sees this cop and the player.
    fn call_for_justice(&mut self, id: usize, world: &World, scene: &Scene, physics: &Physics) {
        let origin = scene.states[id].origin + Vec3::Z * 36.;
        let sight = PhysicsSight { physics, world };
        let helper = self.actors.iter().find_map(|(other, actor)| {
            let cop = actor.class.cop()?;
            let center = scene.states[*other].origin + Vec3::Z * 36.;
            (*other != id
                && !scene.states[*other].killed
                && cop.spawnflags & super::metropolice::SF_METROPOLICE_ALLOWED_TO_RESPOND != 0
                && center.distance(origin) < 512.
                && sight.visible(origin, center, &[id, *other])
                && cop.player.is_some_and(|p| p.visible))
            .then_some(*other)
        });
        if let Some(helper) = helper {
            let now = scene.time;
            let ctx = Context {
                now,
                origin: scene.states[helper].origin,
                yaw: yaw_of(scene, helper),
                random: self.random(),
                ..Default::default()
            };
            let actor = self.actors.get_mut(&helper).expect("actor");
            if let AiClass::Metropolice(cop) = &mut actor.class {
                cop.administer_justice(&mut actor.npc, &ctx);
            }
        }
    }
    /// g_interactionHitByPlayerThrownPhysObj for an NPC struck by a prop the player
    /// launched or threw (the host's physics-collision hook).
    /// g_interactionHitByPlayerThrownPhysObj from `prop`. CNPC_MetroPolice also fires
    /// OnCupCopped when the prop is named cupcop_can.
    pub fn hit_by_player_thrown_object(
        &mut self,
        id: usize,
        prop: usize,
        world: &World,
        scene: &mut Scene,
    ) {
        let cop = self
            .actors
            .get(&id)
            .is_some_and(|a| a.class.cop().is_some());
        let can = world
            .entities
            .get(prop)
            .and_then(|e| e.get("targetname"))
            .is_some_and(|n| n.eq_ignore_ascii_case("cupcop_can"));
        if cop && can {
            scene.fire(id, "OnCupCopped", usize::MAX);
        }
        let now = scene.time;
        let random = self.random();
        let Some(actor) = self.actors.get_mut(&id) else {
            return;
        };
        let ctx = Context {
            now,
            origin: scene.states[id].origin,
            yaw: yaw_of(scene, id),
            random,
            ..Default::default()
        };
        // npc_metropolice's interaction; the other classes do not handle it.
        if let AiClass::Metropolice(cop) = &mut actor.class {
            cop.hit_by_player_thrown_object(&mut actor.npc, &ctx);
        }
        self.diagnostics.thrown_object_hits += 1;
    }
    /// +USE on an AI NPC (npc_metropolice's SetUse(PrecriminalUse)). False when the AI
    /// is off or `id` is not an AI cop, so the host falls back to the ordinary use (the
    /// companions' CommanderUse/speech is not reconstructed).
    pub fn player_used(
        &mut self,
        id: usize,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        player: PlayerState,
    ) -> bool {
        if !self.enabled || self.actors.get(&id).is_none_or(|a| a.class.cop().is_none()) {
            return false;
        }
        if !scene.ai_scripted(id) {
            let ctx = self.context(id, world, scene, player, scene.time, 0.);
            let actor = self.actors.get_mut(&id).expect("actor");
            if let AiClass::Metropolice(cop) = &mut actor.class {
                cop.precriminal_use(&mut actor.npc, &ctx);
            }
            self.apply_requests(id, world, scene, physics, player);
        }
        true
    }
    /// A spawned NPC's additionalequipment (F4 / npc_create_equipment) through
    /// Weapon_Equip (classes without bits_CAP_USE_WEAPONS get nothing). The cop's baton
    /// exists only with weapon_stunstick; classes without AI here just hold it.
    pub fn set_equipment(&mut self, id: usize, class: &str, equipment: &str) {
        let weapon = Self::equip(class, equipment, &self.weapon_defs);
        let regulator = Self::regulator(class, weapon.as_ref(), &self.weapon_defs, id);
        if let Some(actor) = self.actors.get_mut(&id) {
            match &mut actor.class {
                AiClass::Metropolice(cop) => {
                    cop.has_baton = weapon
                        .as_ref()
                        .is_some_and(|w| w.class == "weapon_stunstick");
                }
                AiClass::Humanoid(h) => h.armed = weapon.is_some(),
                AiClass::Zombie(_) | AiClass::Headcrab(_) => {}
            }
            actor.weapon = weapon;
            actor.npc.shot_regulator = regulator;
        } else if let Some(weapon) = weapon {
            self.held_weapons.insert(id, weapon);
        } else {
            self.held_weapons.remove(&id);
        }
    }
    /// An NPC's bullet struck AI actor `target` (the victim responds at its next think).
    /// CMissile::Explode / removal -> the owner's CWeaponRPG::m_hMissile clears.
    pub fn npc_missile_died(&mut self, owner: usize) {
        if let Some(weapon) = self.actors.get_mut(&owner).and_then(|a| a.weapon.as_mut()) {
            weapon.missile_out = false;
        }
    }
    /// An NPC's guiding laser dot (TASK_CIT_RPG_AUGER): (position, shoot position).
    pub fn npc_laser(&self, owner: usize) -> Option<(Vec3, Vec3)> {
        let h = self.actors.get(&owner)?.class.humanoid()?;
        Some((h.rpg_laser?, h.shoot_position))
    }
    /// CWeaponAR2::FireNPCSecondaryAttack launches this tick: (shooter, origin, velocity).
    pub fn take_ball_launches(&mut self) -> Vec<(usize, Vec3, Vec3)> {
        std::mem::take(&mut self.ball_launches)
    }
    pub fn npc_attacked(&mut self, target: usize, attacker: usize) {
        if target == attacker {
            return;
        }
        if let Some(actor) = self.actors.get_mut(&target) {
            actor.damaged_by = Some(attacker);
        }
    }
    /// CSoundEnt::InsertSound for host-side sounds (the player's gunfire).
    pub fn insert_combat_sound(&mut self, origin: Vec3, volume: f32, now: f64) {
        self.sounds
            .insert(senses::sound::COMBAT, origin, volume, 0.2, None, now);
    }
    /// A spawn's disposition toward the player (F4 allegiance).
    pub fn set_player_disposition(&mut self, id: usize, disposition: Option<Disposition>) {
        if let Some(actor) = self.actors.get_mut(&id) {
            actor.player_disposition = disposition;
        }
        match disposition {
            Some(d) => self.allegiances.insert(id, d),
            None => self.allegiances.remove(&id),
        };
    }
    /// IRelationType of NPC `subject` toward NPC `target`. F4 allegiances make sides: two
    /// NPCs with allegiances hate each other when they differ and like each other when
    /// they match; one with an allegiance hates an NPC whose class (SDK table) hates the
    /// player when its allegiance is friendly, and one whose class likes the player when
    /// it is hostile (our extension: the SDK has only per-entity ai_relationship). Without
    /// allegiances the SDK class table decides.
    pub fn npc_disposition(
        &self,
        world: &World,
        subject: usize,
        target: usize,
        precriminal: bool,
    ) -> Disposition {
        let class = |id: usize| {
            classify(
                world.entities.get(id).map_or("", |e| e.class()),
                precriminal,
            )
        };
        let side = |id: usize| -> Option<Disposition> {
            self.allegiances.get(&id).copied().or_else(|| {
                match self.relationships.get(class(id), Class::Player).0 {
                    Disposition::HT => Some(Disposition::HT),
                    Disposition::LI => Some(Disposition::LI),
                    _ => None,
                }
            })
        };
        if let Some(&d) = self.relationship_overrides.get(&(subject, target)) {
            return d;
        }
        match (
            self.allegiances.get(&subject),
            self.allegiances.get(&target),
        ) {
            (Some(a), Some(b)) => {
                if a == b {
                    Disposition::LI
                } else {
                    Disposition::HT
                }
            }
            (Some(a), None) => match side(target) {
                Some(b) if b != *a => Disposition::HT,
                _ => Disposition::NU,
            },
            (None, Some(b)) => match side(subject) {
                Some(a) if a != *b => Disposition::HT,
                _ => Disposition::NU,
            },
            (None, None) => self.relationships.get(class(subject), class(target)).0,
        }
    }
    /// The weapons `class` can be given on a model with `activities` (upper case):
    /// bits_CAP_USE_WEAPONS and Weapon_CanUse over the installed weapon definitions.
    pub fn usable_weapons(
        &self,
        class: &str,
        activities: &std::collections::BTreeSet<String>,
    ) -> Vec<String> {
        npc_weapons::usable_weapons(class, activities, &self.weapon_defs)
    }
    /// Switching the AI off (debug): every AI NPC drops its schedule, enemy and
    /// movement goal and returns to its idle activity.
    fn stop_all(&mut self, world: &World, scene: &mut Scene, controller: &mut Controller) {
        for (&id, actor) in self.actors.iter_mut() {
            if let Some(key) = actor.goal.take() {
                controller.cancel(key);
                controller.forget(key);
                scene.ai_forget_move(key);
            }
            actor.npc.schedule = None;
            actor.npc.enemy = None;
            if !scene.ai_scripted(id) {
                scene.ai_set_activity(world, id, "ACT_IDLE");
            }
        }
    }
    /// CSoundEnt::InsertSound from gameplay (gunfire, impacts, explosions, footsteps).
    pub fn insert_sound(&mut self, kind: u32, origin: Vec3, volume: f32, duration: f64, now: f64) {
        self.sounds
            .insert(kind, origin, volume, duration, None, now);
    }
}

/// CZombieSwatEntitiesEnum over UTIL_EntitiesInBox(origin -+ half): entities whose VPhysics
/// body is at most ZOMBIE_MAX_PHYSOBJ_MASS, asleep and moveable, with what
/// FindNearestPhysicsObject then reads (center, BoundingRadius from the world bounds, AABB
/// top, FVisible from the eyes, ragdoll classes).
fn swat_candidates(
    physics: &Physics,
    world: &World,
    sight: &PhysicsSight,
    zombie: usize,
    origin: Vec3,
    eye: Vec3,
    half: Vec3,
) -> Vec<zombie::SwatCandidate> {
    let (lo, hi) = (origin - half, origin + half);
    physics
        .dynamic
        .iter()
        .filter_map(|(&entity, &body)| {
            let (mass, asleep, moveable) = physics.entity_rest_state(entity)?;
            if mass > zombie::MAX_PHYSOBJ_MASS || !asleep || !moveable {
                return None;
            }
            let (mins, maxs) = physics.entity_world_bounds(entity)?;
            if maxs.cmplt(lo).any() || mins.cmpgt(hi).any() {
                return None;
            }
            let center = physics.bounds_center(body)?;
            let class = world.entities.get(entity).map_or("", |e| e.class());
            Some(zombie::SwatCandidate {
                entity,
                center,
                radius: (maxs - mins).length() * 0.5,
                top: maxs.z,
                visible: sight.visible(eye, center, &[zombie, entity]),
                ragdoll: matches!(class, "physics_prop_ragdoll" | "prop_ragdoll"),
            })
        })
        .collect()
}

/// Strict box overlap (touching faces do not overlap).
fn overlaps(lo: Vec3, hi: Vec3, box_lo: Vec3, box_hi: Vec3) -> bool {
    lo.cmplt(box_hi).all() && hi.cmpgt(box_lo).all()
}
/// The nearest horizontal spot (DIST_EPSILON outside) where `hull` at `origin` no longer
/// overlaps the box, shallowest side first, among those `clear` accepts; None when it does
/// not overlap or no side is clear.
fn push_out_of_box(
    origin: Vec3,
    hull: crate::npc_probe::Hull,
    box_lo: Vec3,
    box_hi: Vec3,
    clear: impl Fn(Vec3) -> bool,
) -> Option<Vec3> {
    let (lo, hi) = (origin + hull.mins, origin + hull.maxs);
    if !overlaps(lo, hi, box_lo, box_hi) {
        return None;
    }
    let mut moves = [
        Vec3::X * (box_hi.x - lo.x),
        -Vec3::X * (hi.x - box_lo.x),
        Vec3::Y * (box_hi.y - lo.y),
        -Vec3::Y * (hi.y - box_lo.y),
    ];
    moves.sort_by(|a, b| a.length().total_cmp(&b.length()));
    moves
        .into_iter()
        .map(|m| origin + m + m.normalize_or_zero() * 0.03125)
        .find(|p| clear(*p))
}
/// WorldSpaceCenter / EyePosition heights of an NPC by class (`ai_hull`).
fn center_z(world: &World, id: usize) -> f32 {
    world
        .entities
        .get(id)
        .map_or(36., |e| crate::ai_hull::center_height(e.class()))
}
fn eye_z(world: &World, id: usize) -> f32 {
    world
        .entities
        .get(id)
        .map_or(64., |e| crate::ai_hull::eye_height(e.class()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::Entity;

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        }
    }
    /// A cop at the origin facing +X, a police goal with the player as its target and a
    /// relay that counts the goal's first warning (synthetic map data).
    fn world() -> World {
        World {
            entities: vec![
                entity(&[
                    ("classname", "npc_metropolice"),
                    ("targetname", "cop"),
                    ("origin", "0 0 0"),
                    ("additionalequipment", "weapon_stunstick"),
                ]),
                entity(&[
                    ("classname", "ai_goal_police"),
                    ("targetname", "post"),
                    ("origin", "0 0 9"),
                    ("angles", "0 0 0"),
                    ("PoliceRadius", "100"),
                    ("PoliceTarget", "!player"),
                    ("OnFirstWarning", "counter,Add,1,0,-1"),
                ]),
                entity(&[
                    ("classname", "env_global"),
                    ("globalstate", "gordon_precriminal"),
                    ("initialstate", "1"),
                    ("spawnflags", "1"),
                ]),
            ],
            ..Default::default()
        }
    }
    struct Rig {
        world: World,
        scene: Scene,
        physics: Physics,
        controller: Controller,
        ai: AiRuntime,
    }
    fn rig() -> Rig {
        let world = world();
        let scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let ai = AiRuntime::new(&world, None);
        Rig {
            world,
            scene,
            physics,
            controller: Controller::new(None, Default::default()),
            ai,
        }
    }
    fn run(r: &mut Rig, ticks: usize, player: PlayerState) {
        for _ in 0..ticks {
            r.scene.time += 0.015;
            r.ai.tick(
                &r.world,
                &mut r.scene,
                &mut r.controller,
                &r.physics,
                &[],
                player,
            );
        }
    }
    fn player_at(x: f32) -> PlayerState {
        PlayerState {
            feet: Vec3::new(x, 0., 0.),
            on_ground: true,
            ..Default::default()
        }
    }

    #[test]
    fn ai_is_off_by_default_and_shoves_a_close_player_when_enabled() {
        let mut r = rig();
        run(&mut r, 20, player_at(30.));
        assert!(r.ai.actors[&0].npc.schedule.is_none());
        assert!(r.scene.sounds.is_empty());
        r.ai.enabled = true;
        run(&mut r, 20, player_at(30.));
        let cop = r.ai.actors[&0].class.cop().expect("cop");
        assert!(cop.precriminal);
        assert!(cop.warnings >= 1);
        assert!(r
            .scene
            .sounds
            .iter()
            .any(|s| s.name.starts_with("#METROPOLICE_BACK_UP") && s.origin.is_some()));
        assert!(r.ai.diagnostics.thinks >= 2);
    }

    #[test]
    fn using_a_calm_cop_counts_as_bothering_him() {
        let mut r = rig();
        let far = player_at(500.);
        assert!(!r.ai.player_used(0, &r.world, &mut r.scene, &r.physics, far));
        r.ai.enabled = true;
        run(&mut r, 5, far);
        assert!(!r.ai.player_used(2, &r.world, &mut r.scene, &r.physics, far));
        for _ in 0..3 {
            assert!(r.ai.player_used(0, &r.world, &mut r.scene, &r.physics, far));
        }
        let cop = r.ai.actors[&0].class.cop().expect("cop");
        assert_eq!(
            cop.warnings,
            super::super::metropolice::METROPOLICE_MAX_WARNINGS
        );
        assert!(r
            .scene
            .sounds
            .iter()
            .any(|s| s.name.starts_with("#METROPOLICE_BACK_UP")));
    }

    #[test]
    fn a_pooled_cop_is_hidden_until_placed_then_thinks() {
        let mut world = World {
            entities: vec![entity(&[
                ("classname", "env_global"),
                ("globalstate", "gordon_precriminal"),
                ("initialstate", "1"),
                ("spawnflags", "1"),
            ])],
            ..Default::default()
        };
        crate::ai::spawn::append_pool(&mut world);
        let id =
            crate::ai::spawn::free_slot(&world, "npc_metropolice", &Default::default()).unwrap();
        let mut r = Rig {
            scene: Scene::new(&world),
            physics: Physics::new(&world),
            ai: AiRuntime::new(&world, None),
            controller: Controller::new(None, Default::default()),
            world,
        };
        r.ai.enabled = true;
        assert!(!r.scene.states[id].visible);
        run(&mut r, 20, player_at(30.));
        assert_eq!(r.ai.diagnostics.thinks, 0, "hidden slots do not think");
        r.scene.states[id].origin = Vec3::ZERO;
        r.scene.states[id].visible = true;
        r.ai.set_equipment(id, "npc_metropolice", "weapon_stunstick");
        run(&mut r, 20, player_at(30.));
        let cop = r.ai.actors[&id].class.cop().expect("cop");
        assert!(cop.has_baton && cop.warnings >= 1);
    }

    #[test]
    fn debug_switches_ignore_the_player_and_stop_the_ai() {
        let mut r = rig();
        r.ai.enabled = true;
        r.ai.ignore_player = true;
        run(&mut r, 20, player_at(30.));
        let cop = r.ai.actors[&0].class.cop().expect("cop");
        assert_eq!(cop.warnings, 0, "an ignored player is not shoved");
        r.ai.ignore_player = false;
        // F4 allegiance: a foe spawn hates the player, a friend never makes him its enemy.
        r.ai.set_player_disposition(0, Some(Disposition::HT));
        run(&mut r, 20, player_at(30.));
        assert_eq!(r.ai.actors[&0].npc.enemy, Some(PLAYER), "forced hate");
        r.ai.enabled = false;
        run(&mut r, 1, player_at(30.));
        assert!(r.ai.actors[&0].npc.schedule.is_none());
        assert!(r.ai.actors[&0].npc.enemy.is_none());
        r.ai.enabled = true;
        r.ai.set_player_disposition(0, Some(Disposition::LI));
        run(&mut r, 20, player_at(30.));
        assert_ne!(r.ai.actors[&0].npc.enemy, Some(PLAYER), "a friend");
    }

    #[test]
    fn weapon_equip_goes_to_any_weapon_user_and_never_to_natural_attackers() {
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "npc_monk"),
                    ("additionalequipment", "weapon_smg1"),
                ]),
                entity(&[
                    ("classname", "npc_zombie"),
                    ("additionalequipment", "weapon_smg1"),
                ]),
                entity(&[
                    ("classname", "npc_citizen"),
                    ("additionalequipment", "weapon_smg1"),
                ]),
            ],
            ..Default::default()
        };
        let mut ai = AiRuntime::new(&world, None);
        // Map additionalequipment: the monk holds it (his class has no AI layer here yet),
        // the zombie has no bits_CAP_USE_WEAPONS, the citizen's AI actor equips it.
        assert_eq!(ai.held_weapons[&0].class, "weapon_smg1");
        assert!(!ai.held_weapons.contains_key(&1));
        assert_eq!(ai.actors[&2].weapon.as_ref().unwrap().class, "weapon_smg1");
        assert!(ai.actors[&2].npc.shot_regulator.is_some());
        ai.set_equipment(1, "npc_zombie", "weapon_pistol");
        assert!(!ai.held_weapons.contains_key(&1));
        ai.set_equipment(0, "npc_monk", "");
        assert!(!ai.held_weapons.contains_key(&0), "unequipped");
        ai.set_equipment(0, "npc_monk", "weapon_ar2");
        assert_eq!(ai.held_weapons[&0].class, "weapon_ar2");
        ai.set_equipment(2, "npc_citizen", "weapon_ar2");
        assert_eq!(ai.actors[&2].weapon.as_ref().unwrap().class, "weapon_ar2");
        assert_eq!(
            ai.actors[&2].npc.shot_regulator.as_ref().unwrap().rule,
            npc_weapons::RegulatorRule::PlayerCompanion
        );
    }

    #[test]
    fn a_runtime_spawned_cop_gets_an_actor_and_thinks() {
        let mut r = rig();
        r.ai.enabled = true;
        let cop = modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_metropolice".into()),
                ("origin".into(), "0 60 0".into()),
                ("additionalequipment".into(), "weapon_stunstick".into()),
            ],
        };
        let id = r.scene.spawn_entity(&mut r.world, cop.clone());
        assert!(r.ai.add_actor(id, &cop));
        // Classes without an AI layer here (Kleiner: CAI_BaseActor, no combat).
        let kleiner = modkit_core::Entity {
            properties: vec![("classname".into(), "npc_kleiner".into())],
        };
        assert!(!r.ai.add_actor(id + 1, &kleiner));
        run(&mut r, 20, player_at(30.));
        let spawned = r.ai.actors[&id].class.cop().expect("cop");
        assert!(spawned.has_baton);
        assert!(r.ai.actors[&id].last_think > 0.);
    }

    #[test]
    fn scripted_cops_are_left_to_their_script() {
        let mut r = rig();
        r.ai.enabled = true;
        r.scene.states[0].scripted_by = Some(2);
        run(&mut r, 20, player_at(30.));
        assert_eq!(r.ai.actors[&0].npc.state, NpcState::Script);
        assert!(r.ai.actors[&0].npc.schedule.is_none());
        assert!(r.scene.sounds.is_empty());
        assert!(r.ai.diagnostics.scripted_skips > 0);
        r.scene.states[0].scripted_by = None;
        run(&mut r, 10, player_at(30.));
        assert_ne!(r.ai.actors[&0].npc.state, NpcState::Script);
    }

    #[test]
    fn policing_warns_a_noclip_player_like_native_notarget() {
        // The oracle runs noclip + notarget, and native still harasses at 80 units: the
        // policing conditions read the goal target, not the cop's senses.
        let mut r = rig();
        r.scene
            .ai_inputs
            .push((0, "setpolicegoal".into(), "post".into(), usize::MAX));
        r.ai.enabled = true;
        let player = PlayerState {
            noclip: true,
            ..player_at(150.)
        };
        run(&mut r, 40, player);
        let cop = r.ai.actors[&0].class.cop().expect("cop");
        let policing = cop.policing.as_ref().unwrap();
        assert!(policing.warnings >= 1, "{:?}", r.ai.actors[&0].npc.schedule);
        // One warning per 4-6 s: the warn schedule does not fail and re-warn at once.
        run(&mut r, 150, player);
        let cop = r.ai.actors[&0].class.cop().expect("cop");
        let policing = cop.policing.as_ref().unwrap();
        assert_eq!(policing.warnings, 1, "{:?}", r.ai.actors[&0].npc.schedule);
        assert!(!policing.hostile);
        // The idle stand is interrupted once the next harass time passes (4-6 s), not
        // only when its own wait ends.
        run(&mut r, 300, player);
        let cop = r.ai.actors[&0].class.cop().expect("cop");
        assert_eq!(cop.policing.as_ref().unwrap().warnings, 2);
    }

    #[test]
    fn assault_goal_and_scripted_schedule_inputs_reach_the_cop() {
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "npc_metropolice"),
                    ("targetname", "cop1"),
                    ("origin", "450 0 0"),
                    ("additionalequipment", "weapon_pistol"),
                ]),
                entity(&[
                    ("classname", "ai_goal_assault"),
                    ("targetname", "cop1_assault"),
                    ("actor", "cop1"),
                    ("rallypoint", "cop1_rp"),
                    ("AssaultCue", "1"),
                ]),
                entity(&[
                    ("classname", "assault_rallypoint"),
                    ("targetname", "cop1_rp"),
                    ("origin", "632 0 32"),
                    ("angles", "0 180 0"),
                    ("assaultpoint", "cop1_ap"),
                    ("priority", "1"),
                ]),
                entity(&[
                    ("classname", "assault_assaultpoint"),
                    ("targetname", "cop1_ap"),
                    ("origin", "632 0 8"),
                    ("assaulttimeout", "3.0"),
                ]),
                entity(&[
                    ("classname", "aiscripted_schedule"),
                    ("targetname", "cop1_sched"),
                    ("m_iszEntity", "cop1"),
                    ("goalent", "cop1_goal"),
                    ("schedule", "2"),
                ]),
                entity(&[
                    ("classname", "info_target"),
                    ("targetname", "cop1_goal"),
                    ("origin", "376 -60 0"),
                ]),
            ],
            ..Default::default()
        };
        let scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let ai = AiRuntime::new(&world, None);
        let mut r = Rig {
            world,
            scene,
            physics,
            controller: Controller::new(None, Default::default()),
            ai,
        };
        let far = player_at(-2000.);
        // StartSchedule: a run to the goal entity, once (no SF_SCRIPT_REPEATABLE).
        r.scene
            .ai_inputs
            .push((4, "startschedule".into(), String::new(), usize::MAX));
        run(&mut r, 1, far);
        let actor = &r.ai.actors[&0];
        assert_eq!(
            actor.npc.schedule.as_deref(),
            Some("SCHED_AISCRIPTED_MOVE_TO_GOAL")
        );
        let cop = actor.class.cop().expect("cop");
        assert_eq!(cop.script_goal, Some((Vec3::new(376., -60., 0.), true)));
        r.ai.actors.get_mut(&0).unwrap().npc.schedule = None;
        r.scene
            .ai_inputs
            .push((4, "startschedule".into(), String::new(), usize::MAX));
        run(&mut r, 1, far);
        assert!(r.ai.actors[&0].npc.schedule.is_none());
        // Activate: the rally point is locked by the cop, the assault point resolved.
        r.scene
            .ai_inputs
            .push((1, "activate".into(), String::new(), usize::MAX));
        run(&mut r, 1, far);
        let cop = r.ai.actors[&0].class.cop().expect("cop");
        let assault = cop.assault.as_ref().expect("assault");
        assert!(assault.active());
        assert_eq!(assault.rally.entity, 2);
        assert_eq!(assault.rally.yaw, 180.);
        assert_eq!(assault.point.entity, 3);
        assert_eq!(r.ai.rally_locks.get(&2), Some(&0));
        // BeginAssault cues it; Deactivate disables it and frees the rally point.
        r.scene
            .ai_inputs
            .push((1, "beginassault".into(), String::new(), usize::MAX));
        r.scene
            .ai_inputs
            .push((1, "deactivate".into(), String::new(), usize::MAX));
        run(&mut r, 1, far);
        let cop = r.ai.actors[&0].class.cop().expect("cop");
        let assault = cop.assault.as_ref().unwrap();
        assert_eq!(assault.received, super::super::assault::Cue::EntityInput);
        assert!(!assault.active());
        assert!(r.ai.rally_locks.is_empty());
    }
    #[test]
    fn friend_and_foe_cops_target_each_other_and_relationships_override_classes() {
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "npc_metropolice"),
                    ("targetname", "friend"),
                    ("origin", "0 0 0"),
                    ("angles", "0 0 0"),
                    ("additionalequipment", "weapon_pistol"),
                ]),
                entity(&[
                    ("classname", "npc_metropolice"),
                    ("targetname", "foe"),
                    ("origin", "300 0 0"),
                    ("angles", "0 180 0"),
                    ("additionalequipment", "weapon_pistol"),
                ]),
                entity(&[
                    ("classname", "npc_citizen"),
                    ("targetname", "bystander"),
                    ("origin", "0 300 0"),
                ]),
                entity(&[
                    ("classname", "ai_relationship"),
                    ("subject", "npc_metropolice"),
                    ("target", "bystander"),
                    ("disposition", "1"),
                    ("StartActive", "1"),
                ]),
            ],
            ..Default::default()
        };
        let scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let ai = AiRuntime::new(&world, None);
        let mut r = Rig {
            world,
            scene,
            physics,
            controller: Controller::new(None, Default::default()),
            ai,
        };
        // Same class, no allegiances: the SDK table says neutral.
        assert_eq!(r.ai.npc_disposition(&r.world, 0, 1, true), Disposition::NU);
        r.ai.set_player_disposition(0, Some(Disposition::LI));
        r.ai.set_player_disposition(1, Some(Disposition::HT));
        assert_eq!(r.ai.npc_disposition(&r.world, 0, 1, true), Disposition::HT);
        assert_eq!(r.ai.npc_disposition(&r.world, 1, 0, true), Disposition::HT);
        r.ai.enabled = true;
        run(&mut r, 20, player_at(-3000.));
        assert_eq!(r.ai.actors[&0].npc.enemy, Some(1));
        assert_eq!(r.ai.actors[&1].npc.enemy, Some(0));
        // StartActive ai_relationship: cops hate the passive citizen (class rule NU).
        assert_eq!(r.ai.npc_disposition(&r.world, 1, 2, true), Disposition::HT);
        // The AI report (pair-keyed overrides included) serializes.
        assert!(serde_json::to_string(&r.ai).is_ok());
        r.scene
            .ai_inputs
            .push((3, "revertrelationship".into(), String::new(), usize::MAX));
        run(&mut r, 1, player_at(-3000.));
        assert_eq!(r.ai.npc_disposition(&r.world, 1, 2, true), Disposition::NU);
        // A killed enemy is dropped (COND_ENEMY_DEAD) for the next one.
        r.scene.states[1].killed = true;
        run(&mut r, 10, player_at(-3000.));
        assert_eq!(r.ai.actors[&0].npc.enemy, None);
    }
    #[test]
    fn hurt_and_killed_cops_speak_their_pain_and_death_lines() {
        let mut r = rig();
        r.ai.enabled = true;
        let health = crate::gameplay::npc_spawn_health(&r.scene, &r.world.entities[0]);
        // A first light hit (> 0.8 health): PAIN_LIGHT; another within a second: none.
        r.scene.states[0].value = health * 0.85;
        r.scene.npc_damage.push((0, true, health * 0.15));
        r.scene.npc_damage.push((0, true, 1.));
        run(&mut r, 1, player_at(-3000.));
        let spoken = |r: &Rig| -> Vec<String> {
            r.scene
                .sounds
                .iter()
                .filter(|s| s.name.starts_with('#'))
                .map(|s| s.name.clone())
                .collect()
        };
        assert_eq!(spoken(&r), ["#METROPOLICE_PAIN_LIGHT"]);
        // Killed: DeathSound once.
        r.scene.sounds.clear();
        r.scene.states[0].killed = true;
        r.scene.npc_damage.push((0, true, 50.));
        r.scene.npc_damage.push((0, true, 50.));
        run(&mut r, 1, player_at(-3000.));
        assert_eq!(spoken(&r), ["#METROPOLICE_DIE"]);
        assert_eq!(
            sentence_parameters("#COMBINE_DIE"),
            Some(COMBINE_SENTENCE_PARAMETERS)
        );
    }
    #[test]
    fn squads_share_enemies_gate_lines_and_mourn_their_dead() {
        // Three soldiers of squad "s" (the last two face away from the player) and a
        // squadless one far away.
        let soldier = |name: &str, origin: &str, yaw: &str, squad: &str| {
            let angles = format!("0 {yaw} 0");
            let mut pairs = vec![
                ("classname", "npc_combine_s"),
                ("targetname", name),
                ("origin", origin),
                ("angles", angles.as_str()),
            ];
            if !squad.is_empty() {
                pairs.push(("squadname", squad));
            }
            entity(&pairs)
        };
        let world = World {
            entities: vec![
                soldier("a", "0 0 0", "0", "s"),
                soldier("b", "0 200 0", "180", "s"),
                soldier("c", "0 -200 0", "180", "s"),
                soldier("lone", "0 3000 0", "180", ""),
            ],
            ..Default::default()
        };
        let mut r = Rig {
            scene: Scene::new(&world),
            physics: Physics::new(&world),
            controller: Controller::new(None, Default::default()),
            ai: AiRuntime::new(&world, None),
            world,
        };
        r.ai.enabled = true;
        let far = player_at(-5000.);
        run(&mut r, 1, far);
        let squad = &r.ai.squads["s"];
        assert_eq!(
            (squad.members.clone(), squad.leader()),
            (vec![0, 1, 2], Some(0))
        );
        assert!(r.ai.actors[&3].squad.is_none());
        // IN_SQUAD lines need a squad of two; NORMAL ones do not.
        let world = r.world.clone();
        assert!(!r.ai.say(3, "COMBINE_IDLE", &world, &mut r.scene));
        assert!(r.ai.say(3, "COMBINE_DANGER", &world, &mut r.scene));
        assert!(r.ai.say(0, "COMBINE_IDLE", &world, &mut r.scene));
        // The squad's sound gate: another member's normal line waits.
        assert!(!r.ai.say(1, "COMBINE_IDLE", &world, &mut r.scene));
        assert!(r.ai.say(1, "COMBINE_ALERT", &world, &mut r.scene));
        // Only the leader sees the player; SquadNewEnemy gives him to the others.
        run(&mut r, 12, player_at(300.));
        assert_eq!(r.ai.actors[&0].npc.enemy, Some(PLAYER));
        for member in [1, 2] {
            assert_eq!(
                r.ai.actors[&member].npc.enemy,
                Some(PLAYER),
                "member {member}"
            );
            assert!(r.ai.actors[&member]
                .npc
                .enemies
                .find(Some(PLAYER))
                .is_some());
        }
        assert!(r.ai.diagnostics.squad_events["new_enemy"] >= 2);
        assert!(r.ai.diagnostics.squad_events["memory"] >= 2);
        // A death: MAN_DOWN while two remain (one member speaks; the squad gate culls the
        // other's), LAST_OF_SQUAD for the last one. The earlier lines' waits are over.
        r.ai.squads.get_mut("s").expect("squad").sound_wait = 0.;
        r.ai.sound_wait.clear();
        r.scene.sounds.clear();
        r.scene.states[2].killed = true;
        run(&mut r, 1, far);
        assert_eq!(r.ai.squads["s"].members, [0, 1]);
        let spoken = |r: &Rig| -> Vec<String> {
            r.scene
                .sounds
                .iter()
                .filter(|s| {
                    s.name.starts_with("#COMBINE_MAN_DOWN") || s.name.starts_with("#COMBINE_LAST")
                })
                .map(|s| s.name.clone())
                .collect()
        };
        assert_eq!(spoken(&r), ["#COMBINE_MAN_DOWN"]);
        r.scene.sounds.clear();
        r.scene.states[1].killed = true;
        run(&mut r, 1, far);
        assert_eq!(spoken(&r), ["#COMBINE_LAST_OF_SQUAD"]);
        assert_eq!(r.ai.squads["s"].leader(), None);
    }
    #[test]
    fn squad_cops_ask_and_answer_idle_chatter_and_harass_a_near_player_once() {
        let cop = |name: &str, x: &str| {
            entity(&[
                ("classname", "npc_metropolice"),
                ("targetname", name),
                ("origin", x),
                ("squadname", "pair"),
            ])
        };
        let world = World {
            entities: vec![
                cop("a", "0 0 0"),
                cop("b", "0 300 0"),
                entity(&[
                    ("classname", "env_global"),
                    ("globalstate", "gordon_precriminal"),
                    ("initialstate", "1"),
                    ("spawnflags", "1"),
                ]),
            ],
            ..Default::default()
        };
        let mut r = Rig {
            scene: Scene::new(&world),
            physics: Physics::new(&world),
            controller: Controller::new(None, Default::default()),
            ai: AiRuntime::new(&world, None),
            world,
        };
        r.ai.enabled = true;
        run(&mut r, 1, player_at(-5000.));
        let world = r.world.clone();
        // A spoken question makes the asker wait and his squadmate answer it.
        r.ai.idle_lines.push(IdleLine {
            id: 0,
            group: "METROPOLICE_IDLE_QUEST",
            then: IdleThen::CopAsked(1),
        });
        r.ai.speak_idle_lines(&world, &mut r.scene);
        assert_eq!(r.ai.actors[&0].idle_chatter, COP_CHATTER_WAIT_FOR_RESPONSE);
        assert_eq!(r.ai.actors[&1].idle_chatter, COP_CHATTER_RESPONSE + 1);
        // The answer waits for the squad's sound gate, then comes on the next thinks.
        r.scene.sounds.clear();
        run(&mut r, 400, player_at(-5000.));
        assert!(r
            .scene
            .sounds
            .iter()
            .any(|s| s.name == "#METROPOLICE_IDLE_ANSWER"));
        assert_eq!(r.ai.actors[&0].idle_chatter, COP_CHATTER_ASK_QUESTION);
        assert_eq!(r.ai.actors[&1].idle_chatter, COP_CHATTER_ASK_QUESTION);
        // Harassment is remembered by the whole squad.
        r.ai.idle_lines.push(IdleLine {
            id: 1,
            group: "METROPOLICE_IDLE_HARASS_PLAYER",
            then: IdleThen::CopHarassed,
        });
        r.ai.squads.get_mut("pair").expect("squad").sound_wait = 0.;
        r.ai.sound_wait.clear();
        r.ai.speak_idle_lines(&world, &mut r.scene);
        assert!(r.ai.actors[&0].harassed && r.ai.actors[&1].harassed);
    }
    #[test]
    fn an_npc_spawned_after_the_first_think_is_seen() {
        // A friend cop thinks alone (the sight candidate list is built), then a foe cop
        // is created at runtime in front of him (DESIGN 14r part A).
        let world = World {
            entities: vec![entity(&[
                ("classname", "npc_metropolice"),
                ("origin", "0 0 0"),
                ("angles", "0 0 0"),
                ("additionalequipment", "weapon_pistol"),
            ])],
            ..Default::default()
        };
        let scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let ai = AiRuntime::new(&world, None);
        let mut r = Rig {
            world,
            scene,
            physics,
            controller: Controller::new(None, Default::default()),
            ai,
        };
        r.ai.enabled = true;
        r.ai.set_player_disposition(0, Some(Disposition::LI));
        run(&mut r, 10, player_at(-3000.));
        assert_eq!(r.ai.actors[&0].npc.enemy, None);
        let foe = entity(&[
            ("classname", "npc_metropolice"),
            ("origin", "300 0 0"),
            ("angles", "0 180 0"),
            ("additionalequipment", "weapon_pistol"),
        ]);
        let id = r.scene.spawn_entity(&mut r.world, foe.clone());
        assert!(r.ai.add_actor(id, &foe));
        r.ai.set_player_disposition(id, Some(Disposition::HT));
        run(&mut r, 20, player_at(-3000.));
        assert_eq!(r.ai.actors[&0].npc.enemy, Some(id));
        assert_eq!(r.ai.actors[&id].npc.enemy, Some(0));
    }
    #[test]
    fn leaping_headcrabs_bite_hated_npcs_they_touch() {
        let world = World {
            entities: vec![
                entity(&[("classname", "npc_headcrab"), ("origin", "0 0 0")]),
                entity(&[("classname", "npc_metropolice"), ("origin", "60 0 0")]),
            ],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let mut ai = AiRuntime::new(&world, None);
        scene.time = 1.;
        let leap = |ai: &mut AiRuntime| {
            let actor = ai.actors.get_mut(&0).unwrap();
            actor.fall = Some((Vec3::new(400., 0., 100.), 0.9));
            let AiClass::Headcrab(crab) = &mut actor.class else {
                panic!("not a crab");
            };
            crab.leap_touch = true;
        };
        let bites = |ai: &AiRuntime| match &ai.actors[&0].class {
            AiClass::Headcrab(crab) => (crab.bites, crab.leap_touch),
            _ => unreachable!(),
        };
        // A 40-unit step toward the cop 60 ahead: its box is touched, one bite.
        leap(&mut ai);
        ai.step_falls(&world, &mut scene, &physics, player_at(-3000.));
        assert_eq!(ai.melee_hits.len(), 1);
        assert_eq!(ai.melee_hits[0].victim, 1);
        assert_eq!(bites(&ai), (1, false));
        let fall = ai.actors[&0].fall.unwrap().0;
        assert_eq!((fall.x, fall.y), (0., 0.));
        // Out of reach: no bite, the touch stays armed.
        ai.melee_hits.clear();
        scene.states[1].origin = Vec3::new(300., 0., 0.);
        scene.states[0].origin = Vec3::ZERO;
        leap(&mut ai);
        ai.step_falls(&world, &mut scene, &physics, player_at(-3000.));
        assert!(ai.melee_hits.is_empty());
        assert_eq!(bites(&ai), (1, true));
    }
    #[test]
    fn leaping_headcrabs_stop_at_the_player_box_instead_of_entering_it() {
        // Owner playtest (session 38): a crab that reached the player landed inside his
        // box and he could not move. PhysicsTryMove's trace stops it at his box.
        let world = World {
            entities: vec![entity(&[
                ("classname", "npc_headcrab"),
                ("origin", "0 0 0"),
            ])],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let mut ai = AiRuntime::new(&world, None);
        scene.time = 1.;
        {
            let actor = ai.actors.get_mut(&0).unwrap();
            actor.npc.enemy = Some(PLAYER);
            actor.fall = Some((Vec3::new(400., 0., 100.), 0.9));
            let AiClass::Headcrab(crab) = &mut actor.class else {
                panic!("not a crab");
            };
            crab.leap_touch = true;
        }
        // The player's box spans x 34..66: the 40-unit step stops at x 22 and bites once.
        ai.step_falls(&world, &mut scene, &physics, player_at(50.));
        let crab = scene.states[0].origin;
        assert!(crab.x > 21.9 && crab.x + 12. <= 34., "{crab}");
        assert_eq!(ai.player_hits.len(), 1);
        // ClipVelocity: no motion into the box remains; it slides down beside him.
        assert_eq!(ai.actors[&0].fall.unwrap().0.x, 0.);
        for _ in 0..30 {
            scene.time += 0.015;
            ai.step_falls(&world, &mut scene, &physics, player_at(50.));
            assert!(scene.states[0].origin.x + 12. <= 34.);
        }
        assert_eq!(ai.player_hits.len(), 1);
        // A crab a tick-order overlap left 3 units inside the box is pushed out sideways.
        scene.states[0].origin = Vec3::new(25., 0., 10.);
        ai.actors.get_mut(&0).unwrap().fall = Some((Vec3::ZERO, scene.time));
        scene.time += 0.015;
        ai.step_falls(&world, &mut scene, &physics, player_at(50.));
        assert!(
            scene.states[0].origin.x + 12. < 34.,
            "{}",
            scene.states[0].origin
        );
        assert_eq!(ai.diagnostics.player_box_pushouts, 1);
        // Dropped onto his head it is moved off his nearest side and keeps falling.
        scene.states[0].origin = Vec3::new(40., 0., 80.);
        ai.actors.get_mut(&0).unwrap().fall = Some((Vec3::new(0., 0., -200.), scene.time));
        scene.time += 0.05;
        ai.step_falls(&world, &mut scene, &physics, player_at(50.));
        let off = scene.states[0].origin;
        assert!((off.z - 72.).abs() < 0.1 && off.x + 12. < 34., "{off}");
        assert!(ai.actors[&0].fall.is_some());
        assert_eq!(ai.diagnostics.player_box_pushouts, 2);
        scene.time += 0.05;
        ai.step_falls(&world, &mut scene, &physics, player_at(50.));
        assert!(scene.states[0].origin.z < 71.9);
    }
    #[test]
    fn stunstick_swings_strike_only_hated_npcs_in_the_swept_box() {
        let cop = |name: &str, origin: &str, angles: &str| {
            entity(&[
                ("classname", "npc_metropolice"),
                ("targetname", name),
                ("origin", origin),
                ("angles", angles),
                ("additionalequipment", "weapon_stunstick"),
            ])
        };
        let world = World {
            entities: vec![
                cop("friend", "0 0 0", "0 0 0"),
                cop("foe", "40 0 0", "0 180 0"),
                cop("ally", "0 40 0", "0 0 0"),
            ],
            ..Default::default()
        };
        let scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let ai = AiRuntime::new(&world, None);
        let mut r = Rig {
            world,
            scene,
            physics,
            controller: Controller::new(None, Default::default()),
            ai,
        };
        r.ai.set_player_disposition(0, Some(Disposition::LI));
        r.ai.set_player_disposition(1, Some(Disposition::HT));
        r.ai.set_player_disposition(2, Some(Disposition::LI));
        let swing = |end: Vec3| CopRequest::Swing {
            start: Vec3::new(0., 0., 60.),
            end,
            mins: Vec3::new(-16., -16., -40.),
            maxs: Vec3::splat(16.),
            damage: 40.,
        };
        let player = player_at(-3000.);
        // Toward the hated foe 40 units ahead: a hit with a combat sound.
        r.ai.actors
            .get_mut(&0)
            .unwrap()
            .class
            .requests_mut()
            .push(swing(Vec3::new(32., 0., 60.)));
        r.ai.apply_requests(0, &r.world, &mut r.scene, &r.physics, player);
        assert_eq!(r.ai.melee_hits.len(), 1);
        assert_eq!(r.ai.melee_hits[0].victim, 1);
        assert_eq!(r.ai.melee_hits[0].damage, 40.);
        // Toward the liked ally: CTraceFilterMelee passes it (no damage).
        r.ai.melee_hits.clear();
        r.ai.actors
            .get_mut(&0)
            .unwrap()
            .class
            .requests_mut()
            .push(swing(Vec3::new(0., 32., 60.)));
        r.ai.apply_requests(0, &r.world, &mut r.scene, &r.physics, player);
        assert!(r.ai.melee_hits.is_empty());
    }
    #[test]
    fn combine_kick_strikes_the_player_enemy_or_a_hated_npc() {
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "npc_combine_s"),
                    ("origin", "0 0 0"),
                    ("angles", "0 0 0"),
                    ("additionalequipment", "weapon_smg1"),
                ]),
                entity(&[
                    ("classname", "npc_citizen"),
                    ("origin", "50 0 0"),
                    ("angles", "0 180 0"),
                ]),
            ],
            ..Default::default()
        };
        let scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let mut ai = AiRuntime::new(&world, None);
        ai.combine_kick_damage = 10.;
        let mut r = Rig {
            world,
            scene,
            physics,
            controller: Controller::new(None, Default::default()),
            ai,
        };
        let kick = CopRequest::Kick {
            start: Vec3::new(0., 0., 36.),
            forward: Vec3::X,
        };
        // The player as the enemy 40 units ahead: punch (-12, -7, 0), 100 forward + 50 up.
        r.ai.actors.get_mut(&0).unwrap().npc.enemy = Some(PLAYER);
        r.ai.actors
            .get_mut(&0)
            .unwrap()
            .class
            .requests_mut()
            .push(kick.clone());
        r.ai.apply_requests(0, &r.world, &mut r.scene, &r.physics, player_at(40.));
        assert_eq!(r.ai.player_hits.len(), 1);
        assert_eq!(r.ai.player_hits[0].damage, 10.);
        assert_eq!(r.ai.player_hits[0].punch, Vec3::new(-12., -7., 0.));
        assert_eq!(r.ai.player_hits[0].impulse, Vec3::new(100., 0., 50.));
        // A hated rebel in reach takes the kick through the melee hit path.
        r.ai.actors.get_mut(&0).unwrap().npc.enemy = Some(1);
        r.ai.set_player_disposition(0, Some(Disposition::HT));
        r.ai.set_player_disposition(1, Some(Disposition::LI));
        r.ai.actors
            .get_mut(&0)
            .unwrap()
            .class
            .requests_mut()
            .push(kick);
        r.ai.apply_requests(0, &r.world, &mut r.scene, &r.physics, player_at(-3000.));
        assert_eq!(r.ai.melee_hits.len(), 1);
        assert_eq!(
            (r.ai.melee_hits[0].victim, r.ai.melee_hits[0].damage),
            (1, 10.)
        );
    }
    #[test]
    fn player_hits_give_damage_conditions_and_make_the_player_the_enemy() {
        let world = World {
            entities: vec![entity(&[
                ("classname", "npc_combine_s"),
                ("origin", "0 0 0"),
                ("angles", "0 0 0"),
                ("additionalequipment", "weapon_smg1"),
            ])],
            ..Default::default()
        };
        let scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let ai = AiRuntime::new(&world, None);
        let mut r = Rig {
            world,
            scene,
            physics,
            controller: Controller::new(None, Default::default()),
            ai,
        };
        r.ai.enabled = true;
        // The player stands behind the soldier (outside his view cone).
        let behind = player_at(-300.);
        run(&mut r, 10, behind);
        assert_eq!(r.ai.actors[&0].npc.enemy, None);
        // A 357 round (more than 20): light and heavy damage, the player remembered.
        r.scene.npc_damage.push((0, true, 40.));
        run(&mut r, 1, behind);
        for _ in 0..10 {
            if r.ai.actors[&0].npc.enemy.is_some() {
                break;
            }
            run(&mut r, 1, behind);
        }
        assert_eq!(r.ai.actors[&0].npc.enemy, Some(PLAYER));
        assert!(r.scene.npc_damage.is_empty());
    }
    #[test]
    fn grenade_throw_and_toss_velocities_land_on_the_target() {
        let world = World::default();
        let physics = Physics::new(&world);
        let start = Vec3::new(0., 0., 64.);
        let target = Vec3::new(400., 100., 0.);
        let land = |v: Vec3| {
            // Time to reach the target's horizontal distance, then the height there.
            let t = (target - start).truncate().length() / v.truncate().length();
            start + v * t - Vec3::Z * 0.5 * GRAVITY * t * t
        };
        let throw = vec_check_throw(&physics, 0, start, target, 650.).expect("throw");
        assert!((land(throw) - target).length() < 0.5);
        // Unjittered high toss (draws 0.5 = no offset): apex 300 above the midpoint.
        let toss = vec_check_toss(&physics, 0, start, target, Vec3::X, [0.5; 4]).expect("toss");
        assert!((land(toss) - target).length() < 0.5);
        assert!((toss.z - GRAVITY * ((300. - 32.) / (0.5 * GRAVITY)).sqrt()).abs() < 0.5);
        // Targets 500 units above the thrower are refused.
        assert!(vec_check_toss(
            &physics,
            0,
            start,
            target + Vec3::Z * 600.,
            Vec3::X,
            [0.5; 4]
        )
        .is_none());
    }
    #[test]
    fn a_soldier_flushes_an_enemy_that_just_dropped_out_of_sight() {
        let world = World {
            entities: vec![entity(&[
                ("classname", "npc_combine_s"),
                ("origin", "0 0 0"),
                ("angles", "0 0 0"),
                ("additionalequipment", "weapon_smg1"),
                ("NumGrenades", "5"),
            ])],
            ..Default::default()
        };
        let scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let ai = AiRuntime::new(&world, None);
        let mut r = Rig {
            world,
            scene,
            physics,
            controller: Controller::new(None, Default::default()),
            ai,
        };
        r.ai.enabled = true;
        assert!(matches!(&r.ai.actors[&0].class, AiClass::Humanoid(h) if h.grenades == 5));
        // The player was seen 1 s ago at (-300, 0) and now stands behind the soldier,
        // 100 units from there: out of sight, so the soldier tosses at the last seen spot.
        let behind = player_at(-400.);
        r.scene.time = 2.;
        {
            let actor = r.ai.actors.get_mut(&0).unwrap();
            actor.npc.state = NpcState::Combat;
            actor.npc.ideal_state = NpcState::Combat;
            actor.npc.enemy = Some(PLAYER);
            actor
                .npc
                .enemies
                .update(Some(PLAYER), Vec3::new(-300., 0., 0.), 0., true, 1.);
        }
        let mut schedules = Vec::new();
        for _ in 0..20 {
            run(&mut r, 1, behind);
            schedules.push(r.ai.actors[&0].npc.schedule.clone().unwrap_or_default());
        }
        assert!(
            schedules.iter().any(|s| s == "SCHED_COMBINE_RANGE_ATTACK2"),
            "{schedules:?}"
        );
        // COMBINE_AE_GREN_TOSS: a frag with the toss velocity, one grenade fewer.
        r.ai.actors
            .get_mut(&0)
            .unwrap()
            .class
            .requests_mut()
            .push(CopRequest::GrenadeToss);
        r.ai.apply_requests(0, &r.world, &mut r.scene, &r.physics, behind);
        assert_eq!(r.ai.grenade_tosses.len(), 1);
        assert!(r.ai.grenade_tosses[0].2.x < 0.);
        assert!(matches!(&r.ai.actors[&0].class, AiClass::Humanoid(h) if h.grenades == 4));
    }
    #[test]
    fn set_police_goal_enables_policing_warnings_and_goal_outputs() {
        let mut r = rig();
        // logic_auto-style input before the AI is switched on still enables the goal.
        r.scene
            .ai_inputs
            .push((0, "setpolicegoal".into(), "post".into(), usize::MAX));
        run(&mut r, 1, player_at(500.));
        let cop = r.ai.actors[&0].class.cop().expect("cop");
        assert!(cop.policing.is_some());
        r.ai.enabled = true;
        // Inside 2 x PoliceRadius (200) but outside the radius and the shove range.
        run(&mut r, 40, player_at(150.));
        let cop = r.ai.actors[&0].class.cop().expect("cop");
        let policing = cop.policing.as_ref().unwrap();
        assert!(policing.warnings >= 1, "{:?}", r.ai.actors[&0].npc.schedule);
        assert!(r
            .scene
            .sounds
            .iter()
            .any(|s| s.name == "#METROPOLICE_MOVE_ALONG_A"));
        // An unknown goal name is counted, not panicking.
        r.scene
            .ai_inputs
            .push((0, "setpolicegoal".into(), "nowhere".into(), usize::MAX));
        run(&mut r, 1, player_at(150.));
        assert_eq!(r.ai.diagnostics.unknown_goals["nowhere"], 1);
    }
    #[test]
    fn soldier_and_rebel_spawns_fight_each_other_with_their_class_schedules() {
        // Synthetic open ground: an F4-style foe soldier at the origin facing +X and a
        // friendly armed citizen 300 units ahead facing it (owner report: only cops
        // fought; the other classes stood idle with their weapons).
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "npc_combine_s"),
                    ("origin", "0 0 0"),
                    ("angles", "0 0 0"),
                    ("additionalequipment", "weapon_smg1"),
                ]),
                entity(&[
                    ("classname", "npc_citizen"),
                    ("origin", "300 0 0"),
                    ("angles", "0 180 0"),
                    ("additionalequipment", "weapon_smg1"),
                ]),
            ],
            ..Default::default()
        };
        let scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let ai = AiRuntime::new(&world, None);
        let mut r = Rig {
            world,
            scene,
            physics,
            controller: Controller::new(None, Default::default()),
            ai,
        };
        assert!(r.ai.actors[&0].class.humanoid().is_some());
        assert!(r.ai.actors[&1].class.humanoid().is_some());
        r.ai.set_player_disposition(0, Some(Disposition::HT));
        r.ai.set_player_disposition(1, Some(Disposition::LI));
        r.ai.enabled = true;
        run(&mut r, 30, player_at(-3000.));
        assert_eq!(r.ai.actors[&0].npc.enemy, Some(1));
        assert_eq!(r.ai.actors[&1].npc.enemy, Some(0));
        assert_eq!(r.ai.actors[&0].npc.state, NpcState::Combat);
        let soldier = r.ai.actors[&0].npc.schedule.clone().unwrap_or_default();
        let rebel = r.ai.actors[&1].npc.schedule.clone().unwrap_or_default();
        assert!(soldier.starts_with("SCHED_COMBINE_"), "{soldier}");
        assert!(
            matches!(
                rebel.as_str(),
                "SCHED_RANGE_ATTACK1" | "SCHED_COMBAT_FACE" | "SCHED_WAKE_ANGRY"
            ),
            "{rebel}"
        );
        // Their ammo checks and the weapon's attack conditions run for every class.
        assert!(r.ai.actors[&1]
            .npc
            .unsupported_tasks
            .keys()
            .all(|k| !k.starts_with("schedule ")));
    }
}
