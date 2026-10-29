//! Entity I/O: firing outputs, resolving targets, queuing and delivering inputs, and +USE.

use super::{glob, inputs, number, Pending, PhysGunPickup, Scene};
use glam::Vec3;
use modkit_core::World;

impl Scene {
    pub fn fire(&mut self, id: usize, name: &str, activator: usize) {
        let Some(state) = self.states.get_mut(id) else {
            return;
        };
        for output in state
            .outputs
            .iter_mut()
            .filter(|o| o.name.eq_ignore_ascii_case(name) && o.remaining != 0)
        {
            if output.remaining > 0 {
                output.remaining -= 1;
            }
            self.sequence += 1;
            self.diagnostics.outputs_fired += 1;
            self.queue.push(Pending {
                due: self.time + output.delay,
                sequence: self.sequence,
                target: output.target.clone(),
                input: output.input.clone(),
                parameter: output.parameter.clone(),
                caller: id,
                activator,
                opener: None,
            });
        }
    }

    pub(super) fn targets(&self, world: &World, p: &Pending) -> Vec<usize> {
        let target = p.target.to_lowercase();
        if target == "!self" || target == "!caller" {
            return vec![p.caller];
        }
        if target == "!player" {
            return vec![usize::MAX];
        }
        if target == "!activator" {
            return vec![p.activator];
        }
        world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.get("targetname")
                    .is_some_and(|n| glob(&target, &n.to_lowercase()))
            })
            .map(|(id, _)| id)
            .collect()
    }

    pub fn send(&mut self, id: usize, input: &str, parameter: &str) {
        self.send_delayed(id, input, parameter, 0.);
    }

    /// An input to an entity by id after `delay` seconds.
    pub fn send_delayed(&mut self, id: usize, input: &str, parameter: &str, delay: f64) {
        self.sequence += 1;
        self.queue.push(Pending {
            due: self.time + delay,
            sequence: self.sequence,
            target: format!("#{id}"),
            input: input.into(),
            parameter: parameter.into(),
            caller: id,
            activator: usize::MAX,
            opener: None,
        });
    }

    /// Console input uses the normal deferred I/O queue and target-name matching.
    pub fn send_named(&mut self, target: &str, input: &str, parameter: &str, delay: f64) -> bool {
        if target.is_empty()
            || input.is_empty()
            || !delay.is_finite()
            || delay < 0.
            || !(self.time + delay).is_finite()
        {
            return false;
        }
        self.sequence += 1;
        self.queue.push(Pending {
            due: self.time + delay,
            sequence: self.sequence,
            target: target.into(),
            input: input.into(),
            parameter: parameter.into(),
            caller: usize::MAX,
            activator: usize::MAX,
            opener: None,
        });
        true
    }

    /// An NPC blocked by a closed door opens it away from itself; locked doors stay shut.
    pub fn npc_open_door(&mut self, world: &World, id: usize, opener: Vec3) {
        let Some(state) = self.states.get(id) else {
            return;
        };
        if state.killed || state.locked || state.target != 0. {
            return;
        }
        match self.ent(world, id).class() {
            "prop_door_rotating" => self.use_with_opener(world, id, Some(opener)),
            "func_door" | "func_door_rotating" => self.send(id, "Open", ""),
            _ => {}
        }
    }

    pub fn use_entity(&mut self, world: &World, id: usize) {
        self.use_with_opener(world, id, None);
    }

    pub fn use_entity_at(&mut self, world: &World, id: usize, player_origin: Vec3) {
        self.use_with_opener(world, id, Some(player_origin));
    }

    fn use_with_opener(&mut self, world: &World, id: usize, opener: Option<Vec3>) {
        let Some(entity) = world.entities.get(id) else {
            return;
        };
        if entity.class() == "prop_door_rotating" {
            let opening = self.states[id].target == 0.;
            if opening && self.states[id].locked {
                self.send(id, "Open", "");
                return;
            }
            if !opening && number(entity, "spawnflags", 0.) as u32 & 8192 == 0 {
                return;
            }
            let group = entity
                .get("slavename")
                .filter(|s| !s.is_empty())
                .or_else(|| entity.get("targetname").filter(|s| !s.is_empty()));
            let ids: Vec<_> = world
                .entities
                .iter()
                .enumerate()
                .filter(|(other, e)| {
                    *other == id
                        || e.class() == "prop_door_rotating"
                            && group.is_some_and(|name| e.get("targetname") == Some(name))
                })
                .map(|(i, _)| i)
                .collect();
            for door in ids {
                self.send(door, if opening { "Open" } else { "Close" }, "");
                self.queue.last_mut().expect("queued door use").opener = opener;
            }
        } else if matches!(entity.class(), "func_door" | "func_door_rotating") {
            self.send(id, "Toggle", "");
        } else if entity.class() == "func_button" {
            self.fire(id, "OnPressed", usize::MAX);
        } else {
            self.prop_player_used(world, id);
        }
    }

    /// CPhysicsProp::Use by the player: OnPlayerUse with SF_PHYSPROP_ENABLE_PICKUP_OUTPUT
    /// (256), before the pickup attempt.
    pub fn prop_player_used(&mut self, world: &World, id: usize) {
        if world.entities.get(id).is_some_and(|e| {
            e.class().starts_with("prop_physics") && number(e, "spawnflags", 0.) as u32 & 256 != 0
        }) {
            self.fire(id, "OnPlayerUse", usize::MAX);
        }
    }

    /// CPhysicsProp / CPhysBox::OnPhysGunPickup outputs for a player pickup (+USE),
    /// a gravity gun pickup or a punt (activator: the player).
    pub fn prop_physgun_pickup(&mut self, world: &World, id: usize, reason: PhysGunPickup) {
        let class = world.entities.get(id).map_or("", |e| e.class());
        if !class.starts_with("prop_physics") && class != "func_physbox" {
            return;
        }
        self.physics_attackers.insert(id, self.time);
        self.fire(id, "OnPhysGunPickup", usize::MAX);
        match reason {
            PhysGunPickup::Cannon => self.fire(id, "OnPhysGunOnlyPickup", usize::MAX),
            PhysGunPickup::Punt => self.fire(id, "OnPhysGunPunt", usize::MAX),
            PhysGunPickup::Player => {}
        }
        if class != "func_physbox" && reason != PhysGunPickup::Punt {
            self.fire(id, "OnPlayerPickup", usize::MAX);
        }
    }

    /// OnPhysGunDrop (+USE drop or throw, gravity gun drop or launch).
    pub fn prop_physgun_drop(&mut self, world: &World, id: usize) {
        let class = world.entities.get(id).map_or("", |e| e.class());
        if class.starts_with("prop_physics") || class == "func_physbox" {
            self.physics_attackers.insert(id, self.time);
            self.fire(id, "OnPhysGunDrop", usize::MAX);
        }
    }

    /// CBreakableProp::HasPhysicsAttacker(dt): the player while the last physgun/+USE
    /// pickup or drop is at most `window` seconds old.
    pub fn physics_attacker(&self, id: usize, window: f64) -> crate::projectiles::Attacker {
        match self.physics_attackers.get(&id) {
            Some(at) if self.time - window <= *at => crate::projectiles::Attacker::Player,
            _ => crate::projectiles::Attacker::None,
        }
    }

    pub(super) fn deliver(&mut self, world: &World, id: usize, p: &Pending, player_feet: Vec3) {
        if id >= self.states.len() {
            inputs::deliver_player(self, world, id, p, player_feet);
            return;
        }
        if self.states[id].killed {
            return;
        }
        self.diagnostics.inputs_delivered += 1;
        let e = self.ent(world, id);
        let class = e.class();
        let input = p.input.to_lowercase();
        let value = p.parameter.parse::<f32>().unwrap_or(0.);
        if self
            .monitors
            .input(world, &self.states, id, &input, &p.parameter, self.time)
        {
            return;
        }
        if class == "env_global" && self.globals.input(e, &input, &p.parameter).0 {
            return;
        }
        // AI inputs (step 14): npc_metropolice SetPoliceGoal / baton inputs and the
        // ai_goal_police knock-out switches go to the AI runtime.
        if class == "npc_template_maker" {
            self.maker_inputs.push((id, input, p.parameter.clone()));
            return;
        }
        if (class == "npc_metropolice"
            && matches!(
                input.as_str(),
                "setpolicegoal" | "activatebaton" | "enablemanhacktoss" | "disablemanhacktoss"
            ))
            || (class == "ai_goal_police"
                && matches!(input.as_str(), "enableknockout" | "disableknockout"))
            || (class == "ai_goal_assault"
                && matches!(
                    input.as_str(),
                    "activate" | "deactivate" | "beginassault" | "updateactors"
                ))
            || (class == "aiscripted_schedule"
                && matches!(input.as_str(), "startschedule" | "stopschedule"))
            || (class == "ai_goal_actbusy"
                && matches!(
                    input.as_str(),
                    "activate" | "deactivate" | "forcenpctoactbusy" | "setbusysearchrange"
                ))
            || (matches!(class, "info_node_link" | "info_node_link_controller")
                && matches!(input.as_str(), "turnon" | "turnoff"))
            || (class == "ai_relationship"
                && matches!(input.as_str(), "applyrelationship" | "revertrelationship"))
        {
            self.ai_inputs
                .push((id, input, p.parameter.clone(), p.activator));
            return;
        }
        // CBaseAnimating::InputIgnite (Ignite(30)) / InputIgniteLifetime(seconds) on
        // NPCs and physics props; a zombie's Ignite caps its health to about ten
        // seconds of burning (CNPC_BaseZombie::Ignite).
        if matches!(input.as_str(), "ignite" | "ignitelifetime")
            && (class.starts_with("npc_") || class.starts_with("prop_physics"))
        {
            let lifetime = if input == "ignitelifetime" {
                p.parameter.trim().parse::<f32>().unwrap_or(30.)
            } else {
                30.
            };
            let origin = self.states.get(id).map_or(Vec3::ZERO, |s| s.origin);
            let now = self.time;
            if self.fires.ignite(
                id,
                crate::fire::Interactions {
                    flammable: true,
                    ..Default::default()
                },
                lifetime,
                16.,
                origin + Vec3::Z * 36.,
                now,
            ) && crate::ai::zombie::is_zombie_class(class)
            {
                let health = self.skill.npc_entity_health(e).unwrap_or(50.);
                let roll = crate::fire::roll_public(id, now);
                if let Some(state) = self.states.get_mut(id) {
                    if state.value <= 0. {
                        state.value = health;
                    }
                    state.value = state.value.min(5. * (10. + 4. * roll - 2.));
                }
            }
            return;
        }
        let delivery = inputs::Delivery {
            world,
            id,
            pending: p,
            player_feet,
            entity: e,
            class,
            input: &input,
            value,
        };
        if inputs::handler(class).is_some_and(|handle| handle(self, &delivery)) {
            return;
        }
        // Inputs every entity takes.
        match input.as_str() {
            // CBaseEntity::InputAddOutput, keyvalue form ("key value"): KeyValue on the
            // entity. Stored as an override that keyvalue readers consult first (only
            // impact damage's physdamagescale reads it so far); the output-connection
            // form ("Output target:input:param:delay:times") is not supported.
            "addoutput" if !p.parameter.contains(':') => {
                let mut parts = p.parameter.trim().splitn(2, char::is_whitespace);
                match (parts.next(), parts.next()) {
                    (Some(key), Some(v)) if !key.is_empty() => {
                        self.keyvalue_overrides
                            .insert((id, key.to_lowercase()), v.trim().to_string());
                    }
                    _ => self.unsupported_input(class, "AddOutput:malformed"),
                }
            }
            "kill" => {
                self.states[id].killed = true;
                self.states[id].visible = false;
                self.states[id].enabled = false;
            }
            "enable" => {
                self.states[id].enabled = true;
                if matches!(class, "func_brush" | "func_monitor") {
                    self.states[id].visible = true;
                }
                if class == "logic_timer" {
                    self.states[id].timer_at = self.time + self.timer_interval(e);
                } else {
                    self.states[id].timer_at =
                        self.time + number(e, "RefireTime", 1.).max(0.015) as f64;
                }
            }
            "disable" => {
                self.states[id].enabled = false;
                if matches!(class, "func_brush" | "func_monitor") {
                    self.states[id].visible = false;
                }
            }
            "toggleenabled" if class.starts_with("env_soundscape") => {
                self.states[id].enabled = !self.states[id].enabled;
            }
            "toggle" => {
                self.states[id].enabled = !self.states[id].enabled;
                if matches!(class, "func_brush" | "func_monitor") {
                    self.states[id].visible = self.states[id].enabled;
                }
            }
            "turnon" => self.states[id].visible = true,
            "turnoff" => self.states[id].visible = false,
            "setparent" => {
                let mut lookup = p.clone();
                lookup.target = p.parameter.clone();
                match self.targets(world, &lookup).first() {
                    Some(&parent) if !p.parameter.is_empty() && parent != id => {
                        self.set_parent(world, id, parent, None, true)
                    }
                    _ => self.states[id].parent = None,
                }
            }
            "setparentattachment" | "setparentattachmentmaintainoffset" => {
                // CBaseEntity::InputSetParentAttachment needs an existing parent; without
                // MaintainOffset the child moves onto the attachment.
                if let Some(parent) = self.states[id].parent.as_ref().map(|p| p.entity) {
                    let keep = input == "setparentattachmentmaintainoffset";
                    self.set_parent(world, id, parent, Some(p.parameter.clone()), keep);
                }
            }
            "clearparent" => self.states[id].parent = None,
            "lock" => self.states[id].locked = true,
            "unlock" => self.states[id].locked = false,
            "firetimer" => self.fire(id, "OnTimer", p.activator),
            // InputSetHealth lowers health through TakeDamage(DMG_GENERIC): to zero or
            // below, the NPC dies like any other damage (Event_Killed, ragdoll).
            "sethealth" if class.starts_with("npc_") && value <= 0. => {
                let origin = self.states[id].origin;
                self.entity_damage.push(crate::projectiles::Damage {
                    target: crate::projectiles::DamageTarget::Entity(id),
                    amount: 1e9,
                    kind: 0,
                    dissolve: false,
                    direction: Vec3::ZERO,
                    origin,
                    attacker: crate::projectiles::Attacker::None,
                });
            }
            "sethealth" if class.starts_with("npc_") => self.states[id].value = value,
            "setdefaultanimation" if class.starts_with("prop_dynamic") => {
                // CDynamicProp stores the name without restarting or validating the active clip.
                self.states[id].default_animation = p.parameter.clone();
            }
            "setanimation" if class.starts_with("prop_dynamic") || class.starts_with("npc_") => {
                self.animate(world, id, &p.parameter, true);
            }
            _ => self.unsupported_input(class, &p.input),
        }
    }

    pub(super) fn fire_value(&mut self, id: usize, name: &str, value: f32, activator: usize) {
        let before = self.sequence;
        self.fire(id, name, activator);
        for p in self
            .queue
            .iter_mut()
            .filter(|p| p.sequence > before && p.parameter.is_empty())
        {
            p.parameter = value.to_string();
        }
    }

    /// CTimerEntity::ResetTimer: RandomFloat(LowerRandomBound, UpperRandomBound) with
    /// UseRandomTime, else RefireTime (at least LOGIC_TIMER_MIN_INTERVAL 0.01).
    pub(super) fn timer_interval(&mut self, e: &modkit_core::Entity) -> f64 {
        if number(e, "UseRandomTime", 0.) != 0. {
            self.random = self
                .random
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1);
            let unit = (self.random >> 40) as f32 / (1u64 << 24) as f32;
            let (low, high) = (
                number(e, "LowerRandomBound", 0.),
                number(e, "UpperRandomBound", 0.),
            );
            f64::from(low + (high - low) * unit).max(0.015)
        } else {
            number(e, "RefireTime", 1.).max(0.01).max(0.015) as f64
        }
    }

    pub fn unsupported_input(&mut self, class: &str, input: &str) {
        let key = format!("{class}.{input}");
        *self.diagnostics.unsupported.entry(key.clone()).or_default() += 1;
        if self.unsupported.insert(key.clone()) {
            eprintln!("Unimplemented entity input: {key}");
        }
    }
}
