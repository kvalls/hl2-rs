//! npc_metropolice (hl2/npc_metropolice.cpp) for the precriminal campaign: the player's
//! criminal status and relationship, look-at-near-player, the baton (SetBatonState,
//! ACT_ACTIVATE_BATON/DEACTIVATE), shoving a player who comes too close
//! (IncrementPlayerCriminalStatus, METROPOLICE_MAX_WARNINGS 3, ACT_PUSH_PLAYER becoming
//! ACT_MELEE_ATTACK1 after the last warning), AdministerJustice (allowed-to-respond cops
//! chase; others call a nearby one), the prechase return, the physics-object assault
//! reaction (g_interactionHitByPlayerThrownPhysObj) and the stunstick melee
//! (weapon_stunstick.cpp), with CAI_PolicingBehavior hosted when SetPoliceGoal enables it.
//! Out of scope: pistol/SMG combat, stitching, manhacks, arrests, fire, airboats, squads.
//! SDK behavior, retail not compared.
use super::conditions::*;
use super::police::Policing;
use super::schedule::{
    default_select_schedule, AiNpc, Behavior, Context, Motor, Schedule, Schedules, TaskArg,
    TaskStatus,
};
use super::state::NpcState;
use glam::Vec3;
use serde::Serialize;

pub const METROPOLICE_MAX_WARNINGS: u32 = 3;
pub const SF_METROPOLICE_ALLOWED_TO_RESPOND: u32 = 0x0100_0000;
/// Class conditions after the shared ones and the policing behavior's two.
pub const COND_METROPOLICE_PLAYER_TOO_CLOSE: Cond = Cond(LAST_SHARED_CONDITION + 2);
pub const COND_METROPOLICE_CHANGE_BATON_STATE: Cond = Cond(LAST_SHARED_CONDITION + 3);
pub const COND_METROPOLICE_PHYSOBJECT_ASSAULT: Cond = Cond(LAST_SHARED_CONDITION + 4);
/// AE_METROPOLICE_* animation event names and the weapon melee hit (npcevent.h).
pub const AE_SHOVE: &str = "AE_METROPOLICE_SHOVE";
pub const AE_BATON_ON: &str = "AE_METROPOLICE_BATON_ON";
pub const AE_BATON_OFF: &str = "AE_METROPOLICE_BATON_OFF";
pub const EVENT_WEAPON_MELEE_HIT: i32 = 3001;

/// The class schedules used here, in the SDK text format (npc_metropolice.cpp).
const SCHEDULES: &[(&str, &str)] = &[
    (
        "SCHED_METROPOLICE_SHOVE",
        "Tasks TASK_STOP_MOVING 0 TASK_FACE_PLAYER 0.1 TASK_METROPOLICE_ACTIVATE_BATON 1 \
         TASK_PLAY_SEQUENCE ACTIVITY:ACT_PUSH_PLAYER Interrupts",
    ),
    (
        "SCHED_METROPOLICE_ACTIVATE_BATON",
        "Tasks TASK_STOP_MOVING 0 TASK_FACE_TARGET 0 TASK_METROPOLICE_ACTIVATE_BATON 1 Interrupts",
    ),
    (
        "SCHED_METROPOLICE_DEACTIVATE_BATON",
        "Tasks TASK_STOP_MOVING 0 TASK_METROPOLICE_ACTIVATE_BATON 0 Interrupts",
    ),
    (
        "SCHED_METROPOLICE_RETURN_TO_PRECHASE",
        "Tasks TASK_WAIT_RANDOM 1 TASK_METROPOLICE_GET_PATH_TO_PRECHASE 0 TASK_WALK_PATH 0 \
         TASK_WAIT_FOR_MOVEMENT 0 TASK_STOP_MOVING 0 TASK_METROPOLICE_CLEAR_PRECHASE 0 \
         Interrupts COND_NEW_ENEMY COND_CAN_MELEE_ATTACK1 COND_CAN_MELEE_ATTACK2 COND_TASK_FAILED \
         COND_LOST_ENEMY COND_HEAR_DANGER",
    ),
    (
        "SCHED_METROPOLICE_CHASE_ENEMY",
        "Tasks TASK_STOP_MOVING 0 TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_COMBAT_FACE \
         TASK_SET_TOLERANCE_DISTANCE 24 TASK_GET_CHASE_PATH_TO_ENEMY 300 TASK_SPEAK_SENTENCE 6 \
         TASK_RUN_PATH 0 TASK_METROPOLICE_RESET_LEDGE_CHECK_TIME 0 TASK_WAIT_FOR_MOVEMENT 0 \
         TASK_FACE_ENEMY 0 Interrupts COND_NEW_ENEMY COND_ENEMY_DEAD COND_ENEMY_UNREACHABLE \
         COND_CAN_RANGE_ATTACK1 COND_CAN_MELEE_ATTACK1 COND_CAN_RANGE_ATTACK2 COND_CAN_MELEE_ATTACK2 \
         COND_TOO_CLOSE_TO_ATTACK COND_TASK_FAILED COND_LOST_ENEMY COND_HEAR_DANGER",
    ),
    (
        "SCHED_METROPOLICE_WAKE_ANGRY",
        "Tasks TASK_STOP_MOVING 0 TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE TASK_FACE_ENEMY 0 Interrupts",
    ),
];
/// Add the class schedules (SCHED_METROPOLICE_CHASE_ENEMY's fail schedule
/// ESTABLISH_LINE_OF_FIRE is a pistol schedule: SCHED_COMBAT_FACE stands in).
pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in SCHEDULES {
        let schedule = Schedule::parse(name, text).expect("metropolice schedule");
        schedules.0.insert((*name).into(), schedule);
    }
}

/// What the runtime tells the cop about the player each think.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerView {
    pub feet: Vec3,
    pub center: Vec3,
    pub velocity: Vec3,
    /// FVisible from the cop.
    pub visible: bool,
    /// The player stands on this cop (GetGroundEntity() == this).
    pub on_me: bool,
    pub on_ground: bool,
    pub suit: bool,
}
/// Something the runtime must do for the cop.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum CopRequest {
    /// m_Sentences.Speak(group).
    Sentence(&'static str),
    /// AddLookTarget(player, importance, duration).
    LookAtPlayer { importance: f32, duration: f32 },
    /// An output of the cop (OnStunnedPlayer).
    Output(&'static str),
    /// A sound cue at the cop (NPC_Metropolice.Shove, stunstick hit/miss).
    Sound(&'static str),
    /// The player was hit: damage (DMG_CLUB), view punch (Source degrees), velocity
    /// impulse, fades (color, duration, hold, flags) and the knock-out output.
    PlayerHit {
        damage: f32,
        punch: Vec3,
        impulse: Vec3,
        fade: Option<([u8; 4], f32, f32, u32)>,
        knock_out: bool,
    },
    /// AdministerJustice handed to a nearby allowed-to-respond cop.
    CallForJustice,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Metropolice {
    pub spawnflags: u32,
    /// A weapon_stunstick (additionalequipment).
    pub has_baton: bool,
    pub baton_active: bool,
    pub should_activate_baton: bool,
    pub warnings: u32,
    pub baton_debounce: f64,
    pub chase_until: f64,
    pub pre_chase: Option<(Vec3, f32)>,
    pub player_hits: u32,
    pub player_too_close: bool,
    pub keep_facing_player: bool,
    pub player_near: bool,
    pub last_attack: f64,
    pub baton_swing_ready: f64,
    pub last_physics_flinch: f64,
    /// CAI_PolicingBehavior when SetPoliceGoal enabled it.
    pub policing: Option<Policing>,
    /// The policing behavior selected the current schedule (IsRunningBehavior).
    pub running_behavior: bool,
    /// The cop's own pre-translated activity for TASK_METROPOLICE_ACTIVATE_BATON.
    baton_task: Option<bool>,
    #[serde(skip)]
    pub requests: Vec<CopRequest>,
    /// Set by the runtime each think.
    #[serde(skip)]
    pub player: Option<PlayerView>,
    #[serde(skip)]
    pub precriminal: bool,
    #[serde(skip)]
    pub scripted: bool,
}
impl Metropolice {
    pub fn new(spawnflags: u32, has_baton: bool) -> Self {
        Self {
            spawnflags,
            has_baton,
            precriminal: true,
            ..Default::default()
        }
    }
    /// PlayerIsCriminal.
    pub fn player_is_criminal(&self) -> bool {
        if self.policing.as_ref().is_some_and(|p| p.hostile) {
            return true;
        }
        !self.precriminal
    }
    /// IRelationType toward the player given the table's disposition: a precriminal
    /// player is neutral unless the cop is chasing him.
    pub fn hates_player(&self, table_hates: bool, now: f64) -> bool {
        if !table_hates {
            return false;
        }
        self.player_is_criminal() || self.chase_until > now
    }
    /// SetBatonState.
    pub fn set_baton_state(&mut self, npc: &mut AiNpc, state: bool) {
        if !self.has_baton {
            return;
        }
        if self.should_activate_baton != state {
            self.should_activate_baton = state;
            npc.conditions.set(COND_METROPOLICE_CHANGE_BATON_STATE);
        }
    }
    /// GatherConditions + PrescheduleThink (player proximity, look target, baton).
    pub fn gather_conditions(&mut self, npc: &mut AiNpc, ctx: &Context) {
        if !self.player_too_close {
            npc.conditions.clear(COND_METROPOLICE_PLAYER_TOO_CLOSE);
        }
        let Some(player) = self.player else {
            return;
        };
        let distance_sq = (player.feet - ctx.origin).length_squared();
        if player.on_me {
            self.player_hits = 0;
            npc.conditions.set(COND_METROPOLICE_PLAYER_TOO_CLOSE);
        } else if distance_sq < 42. * 42. && player.visible {
            if self.player_hits < 3 || self.pre_chase.is_none() {
                npc.conditions.set(COND_METROPOLICE_PLAYER_TOO_CLOSE);
            }
        } else {
            npc.conditions.clear(COND_METROPOLICE_PLAYER_TOO_CLOSE);
            if ctx.now - self.last_attack > 3. {
                self.player_hits = 0;
            }
            self.player_too_close = false;
        }
        // PrescheduleThink: look at a near player (128 units between centers).
        self.player_near = false;
        if !self.player_is_criminal() {
            let center = ctx.origin + Vec3::Z * 36.;
            if (player.center - center).length_squared() < 128. * 128. {
                self.player_near = true;
                self.requests.push(CopRequest::LookAtPlayer {
                    importance: 0.75,
                    duration: 5.,
                });
                if self.policing.is_none() && self.warnings >= METROPOLICE_MAX_WARNINGS {
                    self.baton_debounce = ctx.now + 2.5 + 1.5 * f64::from(ctx.random);
                    self.set_baton_state(npc, true);
                }
            } else {
                if self.policing.is_none() && ctx.now > self.baton_debounce {
                    self.set_baton_state(npc, false);
                }
                self.keep_facing_player = false;
            }
        }
        // The policing behavior's own conditions and its baton requests.
        if let Some(policing) = &self.policing {
            policing.gather_conditions(npc);
            let want = policing.baton;
            self.set_baton_state(npc, want);
        }
    }
    /// IncrementPlayerCriminalStatus.
    fn increment_criminal_status(&mut self, npc: &mut AiNpc, ctx: &Context) {
        if self.player.is_some() {
            self.requests.push(CopRequest::LookAtPlayer {
                importance: 0.8,
                duration: 5.,
            });
            if self.warnings < METROPOLICE_MAX_WARNINGS {
                self.warnings += 1;
            }
            if self.warnings >= METROPOLICE_MAX_WARNINGS - 1 {
                self.set_baton_state(npc, true);
            }
        }
        self.baton_debounce = ctx.now + 2. + 2. * f64::from(ctx.random);
        // AnnounceHarrassment: BACK_UP_A/B/C at random.
        const BACK_UP: [&str; 3] = [
            "METROPOLICE_BACK_UP_A",
            "METROPOLICE_BACK_UP_B",
            "METROPOLICE_BACK_UP_C",
        ];
        self.requests.push(CopRequest::Sentence(
            BACK_UP[((ctx.random * 3.) as usize).min(2)],
        ));
        self.keep_facing_player = true;
    }
    /// AdministerJustice: allowed-to-respond cops chase the player for 3-7 s (the
    /// player becomes the enemy); others watch him and call a nearby cop.
    pub fn administer_justice(&mut self, npc: &mut AiNpc, ctx: &Context) {
        if !self.scripted
            && npc.state != NpcState::Script
            && self.spawnflags & SF_METROPOLICE_ALLOWED_TO_RESPOND != 0
        {
            if self.pre_chase.is_none() {
                self.pre_chase = Some((ctx.origin, ctx.yaw));
            }
            self.chase_until = ctx.now + 3. + 4. * f64::from(ctx.random);
            npc.enemy = Some(PLAYER);
            npc.state = NpcState::Combat;
            if let Some(player) = self.player {
                npc.enemies
                    .update(Some(PLAYER), player.feet, 0., true, ctx.now);
            }
        } else {
            self.keep_facing_player = true;
            self.requests.push(CopRequest::CallForJustice);
        }
    }
    /// HandleInteraction(g_interactionHitByPlayerThrownPhysObj).
    pub fn hit_by_player_thrown_object(&mut self, npc: &mut AiNpc, ctx: &Context) {
        if !self.scripted && npc.state != NpcState::Script {
            npc.conditions.set(COND_METROPOLICE_PHYSOBJECT_ASSAULT);
        } else {
            self.administer_justice(npc, ctx);
        }
    }
    /// PrecriminalUse: +USE on a calm cop counts as bothering him.
    pub fn precriminal_use(&mut self, npc: &mut AiNpc, ctx: &Context) {
        if self.scripted
            || !matches!(npc.state, NpcState::Alert | NpcState::Idle)
            || self.player_is_criminal()
        {
            return;
        }
        self.increment_criminal_status(npc, ctx);
        if self.warnings == METROPOLICE_MAX_WARNINGS {
            self.administer_justice(npc, ctx);
        }
    }
    /// Animation events of the cop's current sequence (HandleAnimEvent and the
    /// stunstick's Operator_HandleAnimEvent). `knock_out`: the policing goal's
    /// ShouldKnockOutTarget for the player.
    pub fn anim_event(&mut self, name: &str, id: i32, ctx: &Context, knock_out: bool) {
        match (name, id) {
            (AE_BATON_ON, _) => self.baton_active = true,
            (AE_BATON_OFF, _) => self.baton_active = false,
            (AE_SHOVE, _) => self.shove(ctx),
            (_, EVENT_WEAPON_MELEE_HIT) => self.stunstick_hit(ctx, knock_out),
            _ => {}
        }
    }
    /// The swept melee box hits the player's hull (CheckTraceHullAttack without the
    /// world trace; the runtime checks the line of sight).
    fn hull_hits_player(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3) -> bool {
        let Some(player) = self.player else {
            return false;
        };
        let box_min = player.feet + Vec3::new(-16., -16., 0.) - maxs;
        let box_max = player.feet + Vec3::new(16., 16., 72.) - mins;
        segment_hits_box(start, end, box_min, box_max)
    }
    /// OnAnimEventShove: 16 units ahead from the body center, 15 DMG_CLUB, view punch
    /// (8, 14, 0) and a 250 u/s push; damage only once the player is a criminal.
    fn shove(&mut self, ctx: &Context) {
        let forward = forward(ctx.yaw);
        let start = ctx.origin + Vec3::Z * 36.;
        if !self.hull_hits_player(
            start,
            start + forward * 16.,
            Vec3::splat(-16.),
            Vec3::splat(16.),
        ) {
            return;
        }
        let player = self.player.expect("hit implies a player");
        let mut push = (player.feet - ctx.origin).normalize_or_zero();
        if !player.on_ground {
            push.z = 0.;
        }
        self.requests.push(CopRequest::PlayerHit {
            damage: self.player_damage(15., player.suit),
            punch: Vec3::new(8., 14., 0.),
            impulse: push * 250.,
            fade: None,
            knock_out: false,
        });
        self.requests
            .push(CopRequest::Sound("NPC_Metropolice.Shove"));
    }
    /// The CTraceFilterMetroPolice rule: a precriminal player takes no damage; with the
    /// suit, a quarter.
    fn player_damage(&self, damage: f32, suit: bool) -> f32 {
        if self.precriminal {
            0.
        } else if suit {
            damage * 0.25
        } else {
            damage
        }
    }
    /// CWeaponStunStick EVENT_WEAPON_MELEE_HIT: a 32-unit swing from the shoot position
    /// (toward the enemy when within 0.8 of facing), box (-16,-16,-40)..(16,16,16).
    fn stunstick_hit(&mut self, ctx: &Context, knock_out: bool) {
        let mut direction = forward(ctx.yaw);
        let start = ctx.origin + Vec3::Z * 60.;
        if let Some(player) = self.player {
            let delta = (player.center - start).normalize_or_zero();
            if delta
                .truncate()
                .normalize_or_zero()
                .dot(direction.truncate())
                > 0.8
            {
                direction = delta;
            }
        }
        if !self.hull_hits_player(
            start,
            start + direction * 32.,
            Vec3::new(-16., -16., -40.),
            Vec3::splat(16.),
        ) {
            self.requests.push(CopRequest::Sound("melee_miss"));
            return;
        }
        let player = self.player.expect("hit implies a player");
        self.requests.push(CopRequest::Sound("melee_hit"));
        let yaw_kick = -48. + 24. * ctx.random;
        if knock_out {
            self.requests.push(CopRequest::PlayerHit {
                damage: self.player_damage(self.stunstick_damage(), player.suit),
                punch: Vec3::new(-16., yaw_kick, 2.),
                impulse: Vec3::ZERO,
                fade: Some((
                    [255, 255, 255, 255],
                    0.2,
                    1.,
                    crate::player_damage::FFADE_OUT
                        | crate::player_damage::FFADE_PURGE
                        | crate::player_damage::FFADE_STAYOUT,
                )),
                knock_out: true,
            });
            return;
        }
        // StunnedTarget.
        self.last_attack = ctx.now;
        self.player_hits += 1;
        self.requests.push(CopRequest::Output("OnStunnedPlayer"));
        let mut push = if player.on_me {
            Vec3::new(direction.x, direction.y, 0.)
        } else {
            player.feet - ctx.origin
        }
        .normalize_or_zero()
            * 500.;
        if !player.on_ground {
            push.z = 0.;
        }
        self.requests.push(CopRequest::PlayerHit {
            damage: self.player_damage(self.stunstick_damage(), player.suit),
            punch: Vec3::new(-16., yaw_kick, 2.),
            impulse: push,
            fade: Some(([128, 0, 0, 128], 0.5, 0.1, crate::player_damage::FFADE_IN)),
            knock_out: false,
        });
    }
    /// sk_npc_dmg_stunstick, set by the runtime from skill.cfg.
    fn stunstick_damage(&self) -> f32 {
        STUNSTICK_DAMAGE.with(|d| d.get())
    }
    /// SelectSchedule (precriminal subset).
    fn select(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        self.running_behavior = false;
        if npc.conditions.has(COND_METROPOLICE_PHYSOBJECT_ASSAULT) {
            npc.conditions.clear(COND_METROPOLICE_PHYSOBJECT_ASSAULT);
            if !self.player_is_criminal() {
                self.requests
                    .push(CopRequest::Sentence("METROPOLICE_HIT_BY_PHYSOBJECT"));
                self.warnings = METROPOLICE_MAX_WARNINGS;
                self.administer_justice(npc, ctx);
            } else if self.precriminal {
                self.requests
                    .push(CopRequest::Sentence("METROPOLICE_IDLE_HARASS_PLAYER"));
            }
        }
        let current = npc.schedule.clone().unwrap_or_default();
        if self.has_baton {
            if self.should_activate_baton
                && !self.baton_active
                && current != "SCHED_METROPOLICE_ACTIVATE_BATON"
            {
                return "SCHED_METROPOLICE_ACTIVATE_BATON".into();
            }
            if !self.should_activate_baton
                && self.baton_active
                && current != "SCHED_METROPOLICE_DEACTIVATE_BATON"
            {
                return "SCHED_METROPOLICE_DEACTIVATE_BATON".into();
            }
        }
        if !self.player_is_criminal() {
            if !self.scripted
                && (npc.conditions.has(COND_METROPOLICE_PLAYER_TOO_CLOSE) || self.player_too_close)
            {
                if self.player_hits < 3 || self.pre_chase.is_none() {
                    npc.conditions.clear(COND_METROPOLICE_PLAYER_TOO_CLOSE);
                    self.player_too_close = false;
                    // SelectShoveSchedule.
                    self.increment_criminal_status(npc, ctx);
                    self.chase_until = 0.;
                    return "SCHED_METROPOLICE_SHOVE".into();
                }
            } else if self.player_hits > 0
                && npc.state != NpcState::Combat
                && self.pre_chase.is_some()
                && self.chase_until < ctx.now
            {
                return "SCHED_METROPOLICE_RETURN_TO_PRECHASE".into();
            }
        }
        if npc.conditions.has(COND_HEAR_PHYSICS_DANGER) && self.last_physics_flinch + 4. <= ctx.now
        {
            self.last_physics_flinch = ctx.now;
            return "SCHED_FLINCH_PHYSICS".into();
        }
        if npc.conditions.has(COND_HEAR_DANGER) {
            return "SCHED_TAKE_COVER_FROM_BEST_SOUND".into();
        }
        if npc.state == NpcState::Combat && self.chase_until > ctx.now {
            return "SCHED_CHASE_ENEMY".into();
        }
        // BehaviorSelectSchedule: the policing behavior when it can select.
        if let Some(policing) = &mut self.policing {
            if policing.goal.target.is_some() {
                self.running_behavior = true;
                return policing.select(npc, ctx).into();
            }
        }
        if self.keep_facing_player && !self.player_is_criminal() {
            return "SCHED_TARGET_FACE".into();
        }
        if npc.state == NpcState::Combat {
            // SelectCombatSchedule (stunstick cops): the baton first, then melee when
            // the swing timer allows, else face/chase.
            if self.has_baton && !self.baton_active {
                self.should_activate_baton = true;
                return "SCHED_METROPOLICE_ACTIVATE_BATON".into();
            }
            if npc.conditions.has(COND_CAN_MELEE_ATTACK1) {
                if self.baton_swing_ready <= ctx.now {
                    self.chase_until = 0.;
                    self.baton_swing_ready = ctx.now + 1. + 0.75 * f64::from(ctx.random);
                    return "SCHED_MELEE_ATTACK1".into();
                }
                return "SCHED_COMBAT_FACE".into();
            }
        }
        if npc.state != NpcState::Combat && self.pre_chase.is_some() && self.chase_until < ctx.now {
            return "SCHED_METROPOLICE_RETURN_TO_PRECHASE".into();
        }
        default_select_schedule(npc, ctx)
    }
}
thread_local! {
    /// sk_npc_dmg_stunstick (skill.cfg; the ConVar default is 0).
    pub static STUNSTICK_DAMAGE: std::cell::Cell<f32> = const { std::cell::Cell::new(0.) };
}
/// The player's entity id in AI tables (player memories use it).
pub const PLAYER: usize = usize::MAX;
fn forward(yaw: f32) -> Vec3 {
    let yaw = yaw.to_radians();
    Vec3::new(yaw.cos(), yaw.sin(), 0.)
}
/// Segment-AABB intersection (slab test).
pub fn segment_hits_box(start: Vec3, end: Vec3, min: Vec3, max: Vec3) -> bool {
    let d = end - start;
    let (mut t0, mut t1) = (0f32, 1f32);
    for axis in 0..3 {
        if d[axis].abs() < 1e-6 {
            if start[axis] < min[axis] || start[axis] > max[axis] {
                return false;
            }
            continue;
        }
        let (mut a, mut b) = (
            (min[axis] - start[axis]) / d[axis],
            (max[axis] - start[axis]) / d[axis],
        );
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        t0 = t0.max(a);
        t1 = t1.min(b);
        if t0 > t1 {
            return false;
        }
    }
    true
}
impl Behavior for Metropolice {
    fn select_schedule(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        self.select(npc, ctx)
    }
    /// TranslateSchedule.
    fn translate_schedule(&mut self, name: &str) -> String {
        match name {
            "SCHED_CHASE_ENEMY" if !self.running_behavior => "SCHED_METROPOLICE_CHASE_ENEMY",
            "SCHED_WAKE_ANGRY" => "SCHED_METROPOLICE_WAKE_ANGRY",
            other => other,
        }
        .into()
    }
    /// BuildScheduleTestBits.
    fn custom_interrupts(&self, npc: &AiNpc) -> Conditions {
        let mut c = Conditions::default();
        let current = npc.schedule.as_deref().unwrap_or("");
        if !self.player_is_criminal() {
            c.set(COND_METROPOLICE_PHYSOBJECT_ASSAULT);
        }
        if !self.scripted
            && !matches!(
                current,
                "SCHED_METROPOLICE_SHOVE"
                    | "SCHED_MELEE_ATTACK1"
                    | "SCHED_RELOAD"
                    | "SCHED_METROPOLICE_ACTIVATE_BATON"
            )
        {
            c.set(COND_METROPOLICE_PLAYER_TOO_CLOSE);
        }
        if !matches!(
            current,
            "SCHED_CHASE_ENEMY"
                | "SCHED_METROPOLICE_CHASE_ENEMY"
                | "SCHED_METROPOLICE_ACTIVATE_BATON"
                | "SCHED_METROPOLICE_DEACTIVATE_BATON"
                | "SCHED_METROPOLICE_SHOVE"
                | "SCHED_METROPOLICE_RETURN_TO_PRECHASE"
        ) {
            c.set(COND_METROPOLICE_CHANGE_BATON_STATE);
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
        if self.running_behavior {
            if let Some(policing) = &mut self.policing {
                if let Some(status) = policing.start_task(npc, task, arg, ctx, motor) {
                    self.drain_policing();
                    return Some(status);
                }
            }
        }
        Some(match task {
            "TASK_METROPOLICE_ACTIVATE_BATON" => {
                let activate = arg.number() != 0.;
                if !self.has_baton
                    || activate && (self.baton_active || !self.should_activate_baton)
                    || !activate && (!self.baton_active || self.should_activate_baton)
                {
                    return Some(TaskStatus::Complete);
                }
                self.requests.push(CopRequest::Sentence(if activate {
                    "METROPOLICE_ACTIVATE_BATON"
                } else {
                    "METROPOLICE_DEACTIVATE_BATON"
                }));
                motor.set_activity(if activate {
                    "ACT_ACTIVATE_BATON"
                } else {
                    "ACT_DEACTIVATE_BATON"
                });
                self.baton_task = Some(activate);
                npc.conditions.clear(COND_METROPOLICE_CHANGE_BATON_STATE);
                TaskStatus::Running
            }
            "TASK_METROPOLICE_GET_PATH_TO_PRECHASE" => match self.pre_chase {
                Some((origin, _)) if motor.path_to(origin, false) => TaskStatus::Complete,
                _ => TaskStatus::Failed,
            },
            "TASK_METROPOLICE_CLEAR_PRECHASE" => {
                self.pre_chase = None;
                TaskStatus::Complete
            }
            "TASK_METROPOLICE_RESET_LEDGE_CHECK_TIME" => TaskStatus::Complete,
            // NPC_TranslateActivity: the shove becomes a baton strike after the last
            // warning.
            "TASK_PLAY_SEQUENCE" if matches!(arg, TaskArg::Activity(a) if a == "ACT_PUSH_PLAYER") =>
            {
                motor.set_activity(if self.warnings >= METROPOLICE_MAX_WARNINGS {
                    "ACT_MELEE_ATTACK1"
                } else {
                    "ACT_PUSH_PLAYER"
                });
                TaskStatus::Running
            }
            "TASK_MELEE_ATTACK1" => {
                self.last_attack = ctx.now;
                motor.set_activity("ACT_MELEE_ATTACK1");
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
        if self.running_behavior {
            if let Some(policing) = &mut self.policing {
                if let Some(status) = policing.run_task(npc, task, arg, ctx, motor) {
                    return Some(status);
                }
            }
        }
        match task {
            "TASK_METROPOLICE_ACTIVATE_BATON" => Some(if motor.activity_finished() {
                // The baton's AE_METROPOLICE_BATON_ON/OFF event sets the state during the
                // sequence; without one the finished sequence settles it.
                if let Some(activate) = self.baton_task.take() {
                    self.baton_active = activate;
                }
                TaskStatus::Complete
            } else {
                TaskStatus::Running
            }),
            _ => None,
        }
    }
}
impl Metropolice {
    /// Move the policing behavior's sentences and outputs into the cop's requests;
    /// goal outputs are returned to the runtime separately.
    fn drain_policing(&mut self) {
        let Some(policing) = &mut self.policing else {
            return;
        };
        for sentence in policing.sentences.drain(..) {
            let group: &'static str = match sentence.as_str() {
                "METROPOLICE_MOVE_ALONG_A" => "METROPOLICE_MOVE_ALONG_A",
                "METROPOLICE_MOVE_ALONG_B" => "METROPOLICE_MOVE_ALONG_B",
                "METROPOLICE_MOVE_ALONG_C" => "METROPOLICE_MOVE_ALONG_C",
                _ => continue,
            };
            self.requests.push(CopRequest::Sentence(group));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::schedule::tests::FakeMotor;
    use super::*;

    fn cop(flags: u32) -> (Metropolice, AiNpc, Schedules) {
        let mut schedules = Schedules::default();
        add_schedules(&mut schedules);
        super::super::police::add_schedules(&mut schedules);
        let mut c = Metropolice::new(flags, true);
        c.player = Some(PlayerView {
            feet: Vec3::new(30., 0., 0.),
            center: Vec3::new(30., 0., 36.),
            visible: true,
            on_ground: true,
            suit: false,
            ..Default::default()
        });
        (c, AiNpc::new(1), schedules)
    }
    fn ctx(now: f64) -> Context {
        Context {
            now,
            player: Some(Vec3::new(30., 0., 0.)),
            random: 0.5,
            ..Default::default()
        }
    }
    fn think(c: &mut Metropolice, npc: &mut AiNpc, s: &Schedules, m: &mut FakeMotor, now: f64) {
        let ctx = ctx(now);
        c.gather_conditions(npc, &ctx);
        npc.maintain(s, c, &ctx, m);
    }

    #[test]
    fn a_player_too_close_is_shoved_with_warnings_then_struck() {
        let (mut c, mut npc, s) = cop(0);
        let mut m = FakeMotor::default();
        think(&mut c, &mut npc, &s, &mut m, 0.);
        assert_eq!(npc.schedule.as_deref(), Some("SCHED_METROPOLICE_SHOVE"));
        assert_eq!(c.warnings, 1);
        assert!(c
            .requests
            .iter()
            .any(|r| matches!(r, CopRequest::Sentence(s) if s.starts_with("METROPOLICE_BACK_UP"))));
        assert!(c
            .requests
            .iter()
            .any(|r| matches!(r, CopRequest::LookAtPlayer { .. })));
        // Repeat the shoves: the second warning asks for the baton, the third shove is
        // a baton strike (ACT_PUSH_PLAYER -> ACT_MELEE_ATTACK1).
        for i in 1..40 {
            m.finished = true;
            think(&mut c, &mut npc, &s, &mut m, f64::from(i) * 0.1);
        }
        assert_eq!(c.warnings, METROPOLICE_MAX_WARNINGS);
        assert!(c.should_activate_baton);
        assert!(m.activities.iter().any(|a| a == "ACT_ACTIVATE_BATON"));
        assert!(m.activities.iter().any(|a| a == "ACT_MELEE_ATTACK1"));
    }

    #[test]
    fn precriminal_player_is_neutral_and_shoves_do_no_damage() {
        let (mut c, _, _) = cop(0);
        assert!(!c.hates_player(true, 0.));
        c.chase_until = 5.;
        assert!(c.hates_player(true, 1.));
        c.anim_event(AE_SHOVE, 0, &ctx(0.), false);
        let hit = c
            .requests
            .iter()
            .find_map(|r| match r {
                CopRequest::PlayerHit {
                    damage,
                    punch,
                    impulse,
                    ..
                } => Some((*damage, *punch, *impulse)),
                _ => None,
            })
            .expect("shove hits the player 30 units ahead");
        assert_eq!(hit.0, 0.);
        assert_eq!(hit.1, Vec3::new(8., 14., 0.));
        assert!((hit.2 - Vec3::X * 250.).length() < 1e-3);
        c.precriminal = false;
        c.requests.clear();
        c.anim_event(AE_SHOVE, 0, &ctx(0.), false);
        assert!(c
            .requests
            .iter()
            .any(|r| matches!(r, CopRequest::PlayerHit { damage, .. } if *damage == 15.)));
    }

    #[test]
    fn thrown_object_makes_an_allowed_cop_chase_and_others_call_for_help() {
        let (mut c, mut npc, s) = cop(SF_METROPOLICE_ALLOWED_TO_RESPOND);
        c.player.as_mut().unwrap().feet = Vec3::new(400., 0., 0.);
        c.player.as_mut().unwrap().center = Vec3::new(400., 0., 36.);
        let mut m = FakeMotor::default();
        c.hit_by_player_thrown_object(&mut npc, &ctx(0.));
        think(&mut c, &mut npc, &s, &mut m, 0.);
        assert_eq!(c.warnings, METROPOLICE_MAX_WARNINGS);
        assert_eq!(npc.enemy, Some(PLAYER));
        assert!(c.chase_until > 3.);
        assert!(c.pre_chase.is_some());
        assert!(c
            .requests
            .contains(&CopRequest::Sentence("METROPOLICE_HIT_BY_PHYSOBJECT")));
        let (mut other, mut npc2, _) = cop(0);
        other.administer_justice(&mut npc2, &ctx(0.));
        assert!(other.requests.contains(&CopRequest::CallForJustice));
        assert!(npc2.enemy.is_none());
    }

    #[test]
    fn melee_box_reaches_the_player_only_in_front_and_in_range() {
        let player_min = Vec3::new(14., -16., 0.);
        let player_max = Vec3::new(46., 16., 72.);
        // Shove: 16 ahead from (0,0,36) with a 16-unit box reaches x <= 32 + ...
        assert!(segment_hits_box(
            Vec3::new(0., 0., 36.),
            Vec3::new(16., 0., 36.),
            player_min - Vec3::splat(16.),
            player_max + Vec3::splat(16.)
        ));
        assert!(!segment_hits_box(
            Vec3::new(0., 0., 36.),
            Vec3::new(-16., 0., 36.),
            player_min + Vec3::X * 40. - Vec3::splat(16.),
            player_max + Vec3::X * 40. + Vec3::splat(16.)
        ));
    }
}
