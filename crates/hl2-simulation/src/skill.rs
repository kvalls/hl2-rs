//! skill.cfg convars (sk_*) with the SDK 2013 ConVar defaults for values the installed
//! file leaves out, NPC health per class (each class's Spawn) and hitgroup damage
//! (CAI_BaseNPC::TraceAttack / GetHitgroupDamageMultiplier).
use std::collections::BTreeMap;

/// shareddefs.h hitgroups.
pub const HITGROUP_GENERIC: i32 = 0;
pub const HITGROUP_HEAD: i32 = 1;
pub const HITGROUP_CHEST: i32 = 2;
pub const HITGROUP_STOMACH: i32 = 3;
pub const HITGROUP_LEFTARM: i32 = 4;
pub const HITGROUP_RIGHTARM: i32 = 5;
pub const HITGROUP_LEFTLEG: i32 = 6;
pub const HITGROUP_RIGHTLEG: i32 = 7;
pub const HITGROUP_GEAR: i32 = 10;

#[derive(Clone, Debug)]
pub struct Skill {
    /// 1 easy, 2 medium, 3 hard.
    pub level: u32,
    values: BTreeMap<String, f32>,
}
impl Default for Skill {
    fn default() -> Self {
        Self {
            level: 2,
            values: BTreeMap::new(),
        }
    }
}
impl Skill {
    /// `name "value"` lines; comments after `//`.
    pub fn parse(text: &str, level: u32) -> Self {
        let mut values = BTreeMap::new();
        for line in text.lines() {
            let Some(line) = line.split("//").next() else {
                continue;
            };
            let mut tokens = line.split_whitespace();
            let (Some(name), Some(value)) = (tokens.next(), tokens.next()) else {
                continue;
            };
            if let Ok(v) = value.trim_matches('"').parse::<f32>() {
                if v.is_finite() {
                    values.insert(name.to_lowercase(), v);
                }
            }
        }
        Self { level, values }
    }
    pub fn get(&self, name: &str, default: f32) -> f32 {
        self.values.get(name).copied().unwrap_or(default)
    }
    /// CHalfLife2::AdjustPlayerDamageInflicted (player bullets/melee on NPCs).
    pub fn player_inflict_scale(&self) -> f32 {
        match self.level {
            1 => self.get("sk_dmg_inflict_scale1", 1.5),
            3 => self.get("sk_dmg_inflict_scale3", 0.75),
            _ => self.get("sk_dmg_inflict_scale2", 1.),
        }
    }
    /// npc_health for an entity: CNPC_CombineS::Spawn gives elites (the super soldier
    /// model) sk_combine_guard_health.
    pub fn npc_entity_health(&self, entity: &modkit_core::Entity) -> Option<f32> {
        if entity.class() == "npc_combine_s"
            && crate::ai::runtime::is_elite_model(entity.get("model"))
        {
            return Some(self.get("sk_combine_guard_health", 70.));
        }
        self.npc_health(entity.class())
    }
    /// m_iHealth set by each class's Spawn (SDK 2013 hl2/npc_*.cpp); None for classes
    /// not reviewed yet (the map's health keyvalue, else 40, as before).
    pub fn npc_health(&self, class: &str) -> Option<f32> {
        let sk = |name: &str, default: f32| self.get(name, default);
        Some(match class {
            "npc_metropolice" => sk("sk_metropolice_health", 40.),
            "npc_citizen" => sk("sk_citizen_health", 40.),
            "npc_combine_s" => sk("sk_combine_s_health", 50.),
            "npc_barney" | "npc_alyx" | "npc_fisherman" => 80.,
            "npc_kleiner" | "npc_eli" | "npc_breen" | "npc_mossman" | "npc_gman" => 8.,
            "npc_monk" => 100.,
            "npc_dog" => 999.,
            "npc_vortigaunt" => sk("sk_vortigaunt_health", 100.),
            // CZombie::Spawn sets the same health for npc_zombie_torso.
            "npc_zombie" | "npc_zombie_torso" => sk("sk_zombie_health", 50.),
            "npc_fastzombie" => 50.,
            "npc_poisonzombie" => sk("sk_zombie_poison_health", 175.),
            "npc_zombine" => sk("sk_zombie_soldier_health", 100.),
            "npc_headcrab" | "npc_headcrab_fast" => sk("sk_headcrab_health", 10.),
            "npc_headcrab_black" | "npc_headcrab_poison" => sk("sk_headcrab_poison_health", 35.),
            "npc_manhack" => sk("sk_manhack_health", 25.),
            "npc_cscanner" | "npc_clawscanner" => sk("sk_scanner_health", 30.),
            "npc_stalker" => sk("sk_stalker_health", 50.),
            "npc_antlion" => sk("sk_antlion_health", 30.),
            "npc_antlionguard" => sk("sk_antlionguard_health", 500.),
            "npc_crow" | "npc_pigeon" | "npc_seagull" => sk("sk_crow_health", 1.),
            "npc_bullseye" => sk("sk_bullseye_health", 35.),
            "npc_strider" => sk("sk_strider_health", 350.),
            _ => return None,
        })
    }
    /// GetHitgroupDamageMultiplier with the per-class overrides (npc_combine_s head x2;
    /// zombies head x2, buckshot within ZOMBIE_BUCKSHOT_TRIPLE_DAMAGE_DIST x3). GEAR
    /// hits are handled by `npc_hit_damage`.
    pub fn npc_hitgroup_multiplier(
        &self,
        class: &str,
        hitgroup: i32,
        buckshot: bool,
        attacker_distance: f32,
    ) -> f32 {
        if hitgroup == HITGROUP_HEAD {
            match class {
                "npc_combine_s" => return 2.,
                "npc_zombie"
                | "npc_zombie_torso"
                | "npc_fastzombie"
                | "npc_fastzombie_torso"
                | "npc_poisonzombie"
                | "npc_zombine" => {
                    if !buckshot {
                        return 2.;
                    }
                    // ZOMBIE_BUCKSHOT_TRIPLE_DAMAGE_DIST (npc_BaseZombie.h).
                    if attacker_distance <= 96. {
                        return 3.;
                    }
                }
                _ => {}
            }
        }
        match hitgroup {
            HITGROUP_HEAD => self.get("sk_npc_head", 3.),
            HITGROUP_CHEST => self.get("sk_npc_chest", 1.),
            HITGROUP_STOMACH => self.get("sk_npc_stomach", 1.),
            HITGROUP_LEFTARM | HITGROUP_RIGHTARM => self.get("sk_npc_arm", 1.),
            HITGROUP_LEFTLEG | HITGROUP_RIGHTLEG => self.get("sk_npc_leg", 1.),
            _ => 1.,
        }
    }
    /// CAI_BaseNPC::TraceAttack damage for one hit: gear hits deal 0.01.
    pub fn npc_hit_damage(
        &self,
        class: &str,
        damage: f32,
        hitgroup: i32,
        buckshot: bool,
        attacker_distance: f32,
    ) -> f32 {
        if hitgroup == HITGROUP_GEAR {
            return 0.01;
        }
        damage * self.npc_hitgroup_multiplier(class, hitgroup, buckshot, attacker_distance)
    }
    /// CAI_BaseHumanoid subclasses (CAI_BaseActor, CNPC_PlayerCompanion): the classes
    /// whose TraceAttack applies the sneak-attack rule.
    pub fn is_humanoid(class: &str) -> bool {
        matches!(
            class,
            "npc_metropolice"
                | "npc_citizen"
                | "npc_combine_s"
                | "npc_barney"
                | "npc_alyx"
                | "npc_kleiner"
                | "npc_eli"
                | "npc_breen"
                | "npc_mossman"
                | "npc_magnusson"
                | "npc_gman"
                | "npc_monk"
                | "npc_vortigaunt"
                | "npc_dog"
                | "npc_fisherman"
        )
    }
    /// CBasePlayer::TraceAttack hitgroup scale (sk_player_*).
    pub fn player_hitgroup_multiplier(&self, hitgroup: i32) -> f32 {
        match hitgroup {
            HITGROUP_HEAD => self.get("sk_player_head", 3.),
            HITGROUP_CHEST => self.get("sk_player_chest", 1.),
            HITGROUP_STOMACH => self.get("sk_player_stomach", 1.),
            HITGROUP_LEFTARM | HITGROUP_RIGHTARM => self.get("sk_player_arm", 1.),
            HITGROUP_LEFTLEG | HITGROUP_RIGHTLEG => self.get("sk_player_leg", 1.),
            _ => 1.,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_values_health_and_hitgroups() {
        let skill = Skill::parse(
            "sk_metropolice_health\t\t\"40\" // cop\nsk_npc_head \"3\"\n// sk_npc_leg \"9\"\n",
            2,
        );
        assert_eq!(skill.npc_health("npc_metropolice"), Some(40.));
        assert_eq!(skill.npc_health("npc_kleiner"), Some(8.));
        assert_eq!(skill.npc_health("npc_unknown"), None);
        // Pistol headshot at skill 2: 5 x 3 = 15 (no range falloff in the SDK).
        assert_eq!(
            skill.npc_hit_damage("npc_metropolice", 5., HITGROUP_HEAD, false, 1000.),
            15.
        );
        assert_eq!(
            skill.npc_hit_damage("npc_combine_s", 5., HITGROUP_HEAD, false, 0.),
            10.
        );
        assert_eq!(
            skill.npc_hit_damage("npc_zombie", 8., HITGROUP_HEAD, true, 50.),
            24.
        );
        assert_eq!(
            skill.npc_hit_damage("npc_zombie", 8., HITGROUP_HEAD, true, 500.),
            24.
        );
        assert_eq!(
            skill.npc_hit_damage("npc_citizen", 5., HITGROUP_GEAR, false, 0.),
            0.01
        );
        assert_eq!(
            skill.npc_hit_damage("npc_citizen", 5., HITGROUP_LEFTLEG, false, 0.),
            5.
        );
        assert_eq!(skill.player_inflict_scale(), 1.);
    }
}
