//! Map inputs to AI goals: ai_goal_actbusy, ai_goal_assault, ai_relationship,
//! aiscripted_schedule and the other inputs the runtime takes.

use super::{assault, yaw_keyvalue, AiClass, AiRuntime, Disposition, NpcState, Policing, PLAYER};
use crate::entities::Scene;
use crate::npc::Controller;
use glam::Vec3;
use modkit_core::World;
use std::collections::BTreeMap;

/// The pairs each active ai_relationship applied.
#[derive(Clone, Debug, Default)]
pub(super) struct AppliedRelationships {
    pairs: BTreeMap<usize, Vec<(usize, usize)>>,
    /// An NPC spawned: CAI_Relationship::OnEntitySpawned reapplies them.
    pub stale: bool,
}

impl AiRuntime {
    /// CAI_GoalEntity::Activate at spawn for StartActive goals.
    fn start_goals(&mut self, world: &World, scene: &mut Scene) {
        if self.goals_started || self.actors.is_empty() {
            return;
        }
        self.goals_started = true;
        for (id, e) in world.entities.iter().enumerate() {
            if e.get("StartActive").is_none_or(|v| v.trim() != "1") {
                continue;
            }
            if e.class() == "ai_relationship" {
                self.apply_relationship(world, scene, id, true);
            }
            if matches!(e.class(), "ai_goal_assault" | "ai_goal_actbusy") {
                scene
                    .ai_inputs
                    .push((id, "activate".into(), String::new(), usize::MAX));
            }
        }
    }
    fn refresh_relationships(&mut self, world: &World, scene: &Scene) {
        if std::mem::take(&mut self.applied_relationships.stale) {
            let active: Vec<usize> = self.applied_relationships.pairs.keys().copied().collect();
            for rel in active {
                self.apply_relationship(world, scene, rel, true);
            }
        }
    }

    /// CAI_DynamicLink(Controller)::InputTurnOn/Off: the graph links' state.
    fn dynamic_link_input(&mut self, controller: &mut Controller, target: usize, input: &str) {
        if !controller.set_dynamic_link(target, input == "turnon") {
            *self
                .diagnostics
                .unknown_goals
                .entry(format!("dynamic link #{target}"))
                .or_default() += 1;
        }
    }

    /// SetPoliceGoal / ActivateBaton on cops and EnableKnockOut / DisableKnockOut on
    /// ai_goal_police (processed even while the AI is off, so map-spawn inputs stick).
    pub(super) fn inputs(&mut self, world: &World, scene: &mut Scene, controller: &mut Controller) {
        self.start_goals(world, scene);
        self.refresh_relationships(world, scene);
        for (target, input, parameter, activator) in std::mem::take(&mut scene.ai_inputs) {
            let class = world.entities.get(target).map_or("", |e| e.class());
            if class == "ai_goal_assault" {
                self.assault_goal_input(world, scene, target, &input);
                continue;
            }
            if class == "npc_barnacle" {
                self.barnacle_input(target, &input, parameter.trim().parse().unwrap_or(0.));
                continue;
            }
            if class == "npc_cscanner" {
                let message = (input.as_str(), parameter.as_str(), activator);
                self.scanner_input(world, scene.time, target, message);
                continue;
            }
            if crate::ai::bird::Kind::for_class(class).is_some() {
                self.bird_input(target, &parameter);
                continue;
            }
            if class == "npc_manhack" {
                self.manhack_input(target, &input);
                continue;
            }
            if class == "aiscripted_schedule" {
                self.scripted_schedule_input(world, scene, target, &input);
                continue;
            }
            if class == "ai_goal_actbusy" {
                if matches!(
                    input.as_str(),
                    "forcethisnpctoactbusy" | "forcethisnpctoleave"
                ) {
                    self.force_this_npc(world, scene.time, target, &input, (&parameter, activator));
                } else {
                    self.actbusy_goal_input(world, scene, target, &input, &parameter);
                }
                continue;
            }
            if class == "ai_relationship" {
                self.apply_relationship(world, scene, target, input == "applyrelationship");
                continue;
            }
            if matches!(class, "info_node_link" | "info_node_link_controller") {
                self.dynamic_link_input(controller, target, &input);
                continue;
            }
            match input.as_str() {
                "giveweapon" => self.give_weapon(target, class, &parameter),
                // A Kill of `<npc>_weapon` (entities::io): the weapon is gone.
                "killweapon" => self.set_equipment(target, class, ""),
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

    /// ForceThisNPCToActBusy / ForceThisNPCToLeave: an EHANDLE input, the named entity or,
    /// without a parameter, the firing output's value (OnSpawnNPC: the child, its activator).
    fn force_this_npc(
        &mut self,
        world: &World,
        now: f64,
        goal: usize,
        input: &str,
        (parameter, activator): (&str, usize),
    ) {
        let name = parameter.trim().to_lowercase();
        let target = if name.is_empty() {
            Some(activator)
        } else {
            self.actors.keys().copied().find(|&id| {
                world.entities.get(id).is_some_and(|e| {
                    crate::entities::glob(&name, &e.get("targetname").unwrap_or("").to_lowercase())
                })
            })
        };
        let range = self.actbusy_ranges.get(&goal).copied().unwrap_or(2048.);
        let Some(actor) = target.and_then(|id| self.actors.get_mut(&id)) else {
            return;
        };
        let Some(busy) = actor.class.actbusy_mut() else {
            return;
        };
        if input == "forcethisnpctoleave" {
            busy.force_leave(range, &mut actor.npc, now);
        } else {
            busy.force(Some(goal), range, None, false, &mut actor.npc, now);
        }
    }

    /// ai_goal_actbusy inputs: Activate (EnableGoal on its actors), Deactivate,
    /// ForceNPCToActBusy ("<npc> [hint] [teleport] [nearest] [$activity] [max time]";
    /// a named hint, teleports and forced activities are not reconstructed) and
    /// SetBusySearchRange. Metrocops and player companions host the behavior so far.
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
                    let Some(busy) = actor.class.actbusy_mut() else {
                        continue;
                    };
                    if input == "activate" {
                        if !busy.enabled {
                            busy.enable(Some(goal), range, now);
                        }
                    } else {
                        busy.disable(&mut actor.npc, now);
                    }
                }
            }
            "forcenpctoactbusy" => {
                let mut tokens = parameter.split_whitespace();
                let name = tokens.next().unwrap_or("").to_lowercase();
                let mut nearest = false;
                let mut max_time = None;
                let mut hint = None;
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
                            // A hint node name (FindEntityByName; a non-hint is refused).
                            Err(_) => {
                                hint = world.entities.iter().position(|e| {
                                    e.class() == "info_node_hint"
                                        && e.get("targetname")
                                            .is_some_and(|n| n.eq_ignore_ascii_case(t))
                                });
                                if hint.is_none() {
                                    *self
                                        .diagnostics
                                        .unknown_goals
                                        .entry(format!("actbusy hint {t}"))
                                        .or_default() += 1;
                                }
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
                if let Some(busy) = actor.class.actbusy_mut() {
                    busy.force(Some(goal), range, max_time, nearest, &mut actor.npc, now);
                    busy.forced_hint = hint;
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
        if let Some(pairs) = self.applied_relationships.pairs.remove(&rel) {
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
            let mut matches: Vec<usize> = world
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
                .collect();
            // ChangeRelationships searches the players too: ClassMatches on "player".
            if crate::entities::glob(&pattern, "player") {
                matches.push(PLAYER);
            }
            matches
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
        self.applied_relationships.pairs.insert(rel, pairs);
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
    pub(super) fn goal_target(world: &World, scene: &Scene, goal: usize) -> Option<(usize, Vec3)> {
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
}
