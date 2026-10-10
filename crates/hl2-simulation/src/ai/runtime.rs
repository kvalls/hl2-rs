//! The AI think loop in the NPC stage (CAI_BaseNPC::NPCThink every 0.1 s): per-NPC
//! GatherConditions (sight of the player and hated NPCs, hearing, enemy memory and
//! COND_NEW_ENEMY/SEE_ENEMY/ENEMY_OCCLUDED/LOST_ENEMY), SelectIdealState and
//! MaintainSchedule through the class behavior. Off by default (`enabled`; the host's
//! `ai_enable` switch) until validated against native. NPCs owned by a
//! scripted_sequence or a choreographed scene are NPC_STATE_SCRIPT and the AI leaves
//! them alone. Simplified from the SDK: every NPC thinks every 0.1 s (no PVS/efficiency
//! throttling, no think staggering), only npc_metropolice has a class behavior.
use super::conditions::*;
use super::host::{yaw_of, PhysicsSight, SceneMotor};
use super::metropolice::{CopRequest, Metropolice, PlayerView, PLAYER};
use super::police::{PoliceGoal, Policing};
use super::relationships::{classify, Class, Disposition, Relationships};
use super::schedule::{AiNpc, Context, Schedules};
use super::senses::{self, SightWorld, SoundEnt, Viewer};
use super::state::{select_ideal_state, NpcState, StateInputs};
use crate::{
    entities::Scene,
    npc::{Controller, GoalKey},
    npc_probe::ActorHull,
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

/// The sound script entry holding an NPC sentence's volume/soundlevel/pitch
/// (CAI_Sentence::Init), by sentence or group name ("!NAME" / "#GROUP").
pub fn sentence_parameters(sentence: &str) -> Option<&'static str> {
    let name = sentence.trim_start_matches(['!', '#']).to_ascii_uppercase();
    name.starts_with("METROPOLICE_")
        .then_some(METROPOLICE_SENTENCE_PARAMETERS)
}
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
#[derive(Clone, Debug, Serialize)]
pub enum AiClass {
    Metropolice(Box<Metropolice>),
}
#[derive(Clone, Debug, Serialize)]
pub struct AiActor {
    pub npc: AiNpc,
    pub class: AiClass,
    pub next_think: f64,
    pub last_think: f64,
    #[serde(skip)]
    goal: Option<GoalKey>,
    /// Animation events up to this time were handled.
    events_until: f64,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Diagnostics {
    pub thinks: u64,
    pub scripted_skips: u64,
    pub sentences: u64,
    pub player_hits: u64,
    pub unknown_goals: BTreeMap<String, u32>,
}
#[derive(Serialize)]
pub struct AiRuntime {
    /// The `ai_enable` switch (off by default on this branch).
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
    /// Hits on the player for the host to apply.
    pub player_hits: Vec<PlayerHit>,
    pub diagnostics: Diagnostics,
    random: u64,
}
impl AiRuntime {
    /// The AI for the map's NPCs (metrocops) and ai_goal_police entities. `vfs` reads
    /// skill.cfg (sk_npc_dmg_stunstick) and the stunstick's sound script.
    pub fn new(world: &World, vfs: Option<&Vfs>) -> Self {
        let mut schedules = Schedules::default();
        super::police::add_schedules(&mut schedules);
        super::metropolice::add_schedules(&mut schedules);
        let mut stunstick_sounds = (None, None);
        if let Some(vfs) = vfs {
            if let Ok(Some(skill)) = vfs.read("cfg/skill.cfg") {
                if let Ok(tokens) = keyvalues::tokens(&String::from_utf8_lossy(&skill)) {
                    if let Some(value) = tokens
                        .windows(2)
                        .find(|p| p[0].eq_ignore_ascii_case("sk_npc_dmg_stunstick"))
                        .and_then(|p| p[1].parse::<f32>().ok())
                    {
                        super::metropolice::STUNSTICK_DAMAGE.with(|d| d.set(value));
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
        let mut actors = BTreeMap::new();
        let mut goals = BTreeMap::new();
        for (id, e) in world.entities.iter().enumerate() {
            match e.class() {
                "npc_metropolice" => {
                    let flags = e
                        .get("spawnflags")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let baton = e
                        .get("additionalequipment")
                        .is_some_and(|w| w.eq_ignore_ascii_case("weapon_stunstick"));
                    actors.insert(
                        id,
                        AiActor {
                            npc: AiNpc::new(id),
                            class: AiClass::Metropolice(Box::new(Metropolice::new(flags, baton))),
                            next_think: 0.,
                            last_think: 0.,
                            goal: None,
                            events_until: 0.,
                        },
                    );
                }
                "ai_goal_police" => {
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
                _ => {}
            }
        }
        Self {
            enabled: false,
            schedules,
            relationships: Relationships::default(),
            sounds: SoundEnt::default(),
            actors,
            goals,
            serial: 0,
            stunstick_sounds,
            player_hits: Vec::new(),
            diagnostics: Diagnostics::default(),
            random: 0x9e37_79b9_7f4a_7c15,
        }
    }
    fn random(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 40) as f32 / (1u32 << 24) as f32
    }
    /// SetPoliceGoal / ActivateBaton on cops and EnableKnockOut / DisableKnockOut on
    /// ai_goal_police (processed even while the AI is off, so map-spawn inputs stick).
    fn inputs(&mut self, world: &World, scene: &mut Scene) {
        for (target, input, parameter, _) in std::mem::take(&mut scene.ai_inputs) {
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
                        (Some(goal), Some(actor)) => {
                            let AiClass::Metropolice(cop) = &mut actor.class;
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
                        let AiClass::Metropolice(cop) = &mut actor.class;
                        let on = !matches!(parameter.trim(), "" | "0");
                        cop.set_baton_state(&mut actor.npc, on);
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
        self.inputs(world, scene);
        if !self.enabled {
            return;
        }
        let now = scene.time;
        self.sounds.purge(now);
        let precriminal = scene.globals.is_on("gordon_precriminal");
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
                let AiClass::Metropolice(cop) = &mut actor.class;
                cop.scripted = true;
                self.diagnostics.scripted_skips += 1;
                continue;
            }
            let think_at = self.actors[&id].next_think;
            // Animation events of the AI's sequence since the last tick.
            let since = self.actors[&id].events_until.min(now);
            self.actors.get_mut(&id).expect("actor").events_until = now;
            let events = scene.ai_events(world, id, since, now);
            if !events.is_empty() {
                let knock_out = self.knock_out_player(id, world, scene, physics, player);
                let ctx = self.context(id, world, scene, player, now, 0.);
                let actor = self.actors.get_mut(&id).expect("actor");
                let AiClass::Metropolice(cop) = &mut actor.class;
                for (name, event) in events {
                    cop.anim_event(&name, event, &ctx, knock_out);
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
        let AiClass::Metropolice(cop) = &self.actors[&id].class;
        let Some(policing) = &cop.policing else {
            return false;
        };
        let visible = PhysicsSight { physics, world }.visible(
            scene.states[id].origin + Vec3::Z * 64.,
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
        let random = self.random();
        Context {
            now,
            origin: scene.states[id].origin,
            enemy,
            // SetTarget(player): the cop's target is the player while precriminal.
            target: (!player.noclip).then_some(player.feet),
            best_sound: None,
            player: (!player.noclip).then_some(player.feet),
            yaw: yaw_of(scene, id),
            random,
        }
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
        let eye = origin + Vec3::Z * 64.;
        let facing = scene.states[id].rotation * Vec3::X;
        let sight = PhysicsSight { physics, world };
        let player_visible = !player.noclip
            && !player.dead
            && senses::in_view_cone(
                &Viewer {
                    entity: id,
                    class: Class::Metropolice,
                    eye,
                    facing,
                    field_of_view: METROPOLICE_FOV,
                    look_distance: senses::LOOK_DISTANCE,
                },
                player.eye(),
            )
            && player.eye().distance(eye) <= senses::LOOK_DISTANCE
            && sight.visible(eye, player.eye(), &[id]);
        let fvisible = !player.noclip && sight.visible(eye, player.center(), &[id]);
        // Policing goal state (the goal entity can change through inputs).
        let goal_entity = {
            let AiClass::Metropolice(cop) = &self.actors[&id].class;
            cop.policing.as_ref().map(|p| p.goal.entity)
        };
        let goal = goal_entity.and_then(|g| {
            let mut goal = self.goals.get(&g)?.clone();
            let target = Self::goal_target(world, scene, g);
            goal.target = target.map(|t| t.0);
            Some((goal, target))
        });
        let relationships = &self.relationships;
        let precriminal_hate = relationships
            .get(Class::Metropolice, classify("player", precriminal))
            .0
            == Disposition::HT;
        let actor = self.actors.get_mut(&id).expect("actor");
        actor.npc.conditions = Conditions::default();
        let AiClass::Metropolice(cop) = &mut actor.class;
        cop.scripted = false;
        cop.precriminal = precriminal;
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
        }
        if actor.npc.state == NpcState::Script || actor.npc.state == NpcState::None {
            actor.npc.state = NpcState::Idle;
        }
        // Look: the player when hated (IRelationType), never while precriminal unless
        // the cop is chasing him.
        if player_visible {
            actor.npc.conditions.set(COND_SEE_PLAYER);
        }
        let hates = cop.hates_player(precriminal_hate || !precriminal, now);
        let previous_enemy = actor.npc.enemy;
        if hates && player_visible {
            actor.npc.conditions.set(COND_SEE_HATE);
            actor
                .npc
                .enemies
                .update(Some(PLAYER), player.feet, 0., true, now);
            if actor.npc.enemy.is_none() {
                actor.npc.enemy = Some(PLAYER);
            }
        }
        if !hates && actor.npc.enemy == Some(PLAYER) && cop.chase_until <= now {
            actor.npc.enemy = None;
        }
        actor.npc.enemies.refresh(
            now,
            |e| (e == PLAYER).then_some(player.feet),
            |e| e == PLAYER && player.dead,
        );
        if let Some(enemy) = actor.npc.enemy {
            if previous_enemy != Some(enemy) {
                actor.npc.conditions.set(COND_NEW_ENEMY);
            }
            let seen = enemy == PLAYER && player_visible;
            if seen {
                actor.npc.conditions.set(COND_SEE_ENEMY);
                // CWeaponStunStick::WeaponMeleeAttack1Condition (player in 48 units in
                // 0.35 s, within 70 of height and facing within 0.7).
                let delta = player.center() - (origin + Vec3::Z * 36.);
                let forward = facing.truncate().normalize_or_zero();
                let projected = (player.feet + player.velocity * 0.35 - origin).truncate();
                if cop.has_baton {
                    if delta.z.abs() > 70. {
                        actor.npc.conditions.set(COND_TOO_FAR_TO_ATTACK);
                    } else if delta.truncate().normalize_or_zero().dot(forward) < 0.7 {
                        actor.npc.conditions.set(COND_NOT_FACING_ATTACK);
                    } else if projected.length() <= 48. || delta.truncate().length() <= 48. {
                        actor.npc.conditions.set(COND_CAN_MELEE_ATTACK1);
                    } else {
                        actor.npc.conditions.set(COND_TOO_FAR_TO_ATTACK);
                    }
                }
            } else {
                actor.npc.conditions.set(COND_ENEMY_OCCLUDED);
                if now - actor.npc.enemies.last_time_seen(Some(enemy)) > 10. {
                    actor.npc.conditions.set(COND_LOST_ENEMY);
                }
            }
        }
        // Listen.
        let (heard, best) = senses::listen(eye, id, METROPOLICE_SOUNDS, &self.sounds);
        actor.npc.conditions = actor.npc.conditions.union(heard);
        ctx.best_sound = best.map(|s| s.origin);
        // SelectIdealState.
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
                go_idle: actor.npc.enemy.is_none(),
                ideal: match actor.npc.ideal_state {
                    NpcState::None | NpcState::Script => NpcState::Idle,
                    other => other,
                },
            },
        );
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
        cop.gather_conditions(&mut actor.npc, &ctx);
        let dt = (now - actor.last_think).clamp(0., 0.2) as f32;
        actor.last_think = now;
        actor.next_think = now + THINK_INTERVAL;
        let yaw_speed = if actor.npc.activity.starts_with("ACT_WALK") {
            25.
        } else if actor.npc.activity.starts_with("ACT_RUN") {
            15.
        } else {
            45.
        };
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
        };
        actor
            .npc
            .maintain(&self.schedules, cop.as_mut(), &ctx, &mut motor);
        self.diagnostics.thinks += 1;
        // Policing outputs on the goal entity.
        if let Some(policing) = &mut cop.policing {
            let goal = policing.goal.entity;
            for output in std::mem::take(&mut policing.outputs) {
                scene.fire(goal, output, id);
            }
        }
        self.apply_requests(id, world, scene, physics, player);
    }
    /// Carry out the cop's requests (sentences, look targets, outputs, sounds, hits).
    fn apply_requests(
        &mut self,
        id: usize,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        _player: PlayerState,
    ) {
        let origin = scene.states[id].origin;
        let requests = {
            let AiClass::Metropolice(cop) = &mut self.actors.get_mut(&id).expect("actor").class;
            std::mem::take(&mut cop.requests)
        };
        for request in requests {
            match request {
                CopRequest::Sentence(group) => {
                    self.diagnostics.sentences += 1;
                    scene.sounds.push(crate::sounds::SoundRequest {
                        origin: Some(origin + Vec3::Z * 64.),
                        actor: Some(crate::sounds::SoundActor {
                            name: world.entities[id].get("targetname").unwrap_or("").into(),
                            model: String::new(),
                            entity: Some(id),
                        }),
                        ..format!("#{group}").into()
                    });
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
                        let AiClass::Metropolice(cop) = &self.actors[&id].class;
                        if let Some(goal) = cop.policing.as_ref().map(|p| p.goal.entity) {
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
            }
        }
    }
    /// AdministerJustice's search: the first other allowed-to-respond cop within 512
    /// units that sees this cop and the player.
    fn call_for_justice(&mut self, id: usize, world: &World, scene: &Scene, physics: &Physics) {
        let origin = scene.states[id].origin + Vec3::Z * 36.;
        let sight = PhysicsSight { physics, world };
        let helper = self.actors.iter().find_map(|(other, actor)| {
            let AiClass::Metropolice(cop) = &actor.class;
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
            let AiClass::Metropolice(cop) = &mut actor.class;
            cop.administer_justice(&mut actor.npc, &ctx);
        }
    }
    /// g_interactionHitByPlayerThrownPhysObj for an NPC struck by a prop the player
    /// launched or threw (the host's physics-collision hook).
    pub fn hit_by_player_thrown_object(&mut self, id: usize, scene: &Scene) {
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
        let AiClass::Metropolice(cop) = &mut actor.class;
        cop.hit_by_player_thrown_object(&mut actor.npc, &ctx);
    }
    /// +USE on an AI NPC (npc_metropolice's SetUse(PrecriminalUse)). False when the AI
    /// is off or `id` is not an AI actor, so the host falls back to the ordinary use.
    pub fn player_used(
        &mut self,
        id: usize,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        player: PlayerState,
    ) -> bool {
        if !self.enabled || !self.actors.contains_key(&id) {
            return false;
        }
        if !scene.ai_scripted(id) {
            let ctx = self.context(id, world, scene, player, scene.time, 0.);
            let actor = self.actors.get_mut(&id).expect("actor");
            let AiClass::Metropolice(cop) = &mut actor.class;
            cop.precriminal_use(&mut actor.npc, &ctx);
            self.apply_requests(id, world, scene, physics, player);
        }
        true
    }
    /// CSoundEnt::InsertSound from gameplay (gunfire, impacts, explosions, footsteps).
    pub fn insert_sound(&mut self, kind: u32, origin: Vec3, volume: f32, duration: f64, now: f64) {
        self.sounds
            .insert(kind, origin, volume, duration, None, now);
    }
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
                    ("origin", "0 0 0"),
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
        let AiClass::Metropolice(cop) = &r.ai.actors[&0].class;
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
        let AiClass::Metropolice(cop) = &r.ai.actors[&0].class;
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
    fn set_police_goal_enables_policing_warnings_and_goal_outputs() {
        let mut r = rig();
        // logic_auto-style input before the AI is switched on still enables the goal.
        r.scene
            .ai_inputs
            .push((0, "setpolicegoal".into(), "post".into(), usize::MAX));
        run(&mut r, 1, player_at(500.));
        let AiClass::Metropolice(cop) = &r.ai.actors[&0].class;
        assert!(cop.policing.is_some());
        r.ai.enabled = true;
        // Inside 2 x PoliceRadius (200) but outside the radius and the shove range.
        run(&mut r, 40, player_at(150.));
        let AiClass::Metropolice(cop) = &r.ai.actors[&0].class;
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
}
