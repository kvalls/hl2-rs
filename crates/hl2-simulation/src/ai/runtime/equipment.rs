//! Actors and their weapons: creating an actor for an entity, equipping it, shot regulators
//! and weapon definitions.

use super::{
    humanoid, is_elite_model, zombie, AiActor, AiClass, AiNpc, AiRuntime, Humanoid, Metropolice,
    PlayerState, Zombie, COP_CHATTER_ASK_QUESTION, PLAYER, SF_NPC_GAG, THINK_INTERVAL,
};
use crate::entities::Scene;
use crate::npc_weapons;
use crate::npc_weapons::NpcWeapon;
use crate::npc_weapons::NpcWeaponDef;
use glam::Vec3;
use std::collections::BTreeMap;

impl AiRuntime {
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

    /// The AI actor of an entity whose class has a behavior (npc_metropolice and the
    /// humanoids).
    pub(super) fn actor_for(
        id: usize,
        e: &modkit_core::Entity,
        defs: &BTreeMap<String, NpcWeaponDef>,
    ) -> Option<AiActor> {
        let kind = humanoid::Kind::for_class(e.class());
        let zombie = zombie::Kind::for_class(e.class());
        if e.class() != "npc_metropolice" && kind.is_none() && zombie.is_none() {
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
            _ if zombie.is_some() => AiClass::Zombie(Box::new(Zombie::new(
                zombie.unwrap_or(zombie::Kind::Classic),
                flags & SF_NPC_GAG != 0,
                0x3c6e_f372_fe94_f82b ^ id as u64,
            ))),
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
        })
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
    pub(super) fn equip(
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

    pub(super) fn weapon_def(&self, class: &str) -> Option<NpcWeaponDef> {
        self.weapon_defs
            .get(class)
            .cloned()
            .or_else(|| npc_weapons::builtin(class))
    }

    /// Weapon_ShootPosition (m_HackedGunPos (0, 0, 55) of npc_metropolice) and the
    /// GetActualShootPosition target: the enemy's last known position plus its noisy
    /// BodyTarget offset (player: view offset x U(0.7, 1); NPC: the bell curve from the
    /// low body point to the eyes), and the seconds since the enemy became valid.
    pub(super) fn weapon_aim(
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
                AiClass::Zombie(_) => {}
            }
            actor.weapon = weapon;
            actor.npc.shot_regulator = regulator;
        } else if let Some(weapon) = weapon {
            self.held_weapons.insert(id, weapon);
        } else {
            self.held_weapons.remove(&id);
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
}
