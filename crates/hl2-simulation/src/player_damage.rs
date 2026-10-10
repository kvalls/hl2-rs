//! Local player damage intake in SDK order: CHL2_Player::OnTakeDamage (skill scale),
//! CBasePlayer::OnTakeDamage (armor, DamageEffect, suit diagnosis) and
//! CBaseCombatCharacter::OnTakeDamage_Alive (integer health with a fractional
//! accumulator), plus the per-tick client "Damage" message of
//! CHL2_Player::UpdateClientData. Knockback impulses, punch angles, time-based damage,
//! drowning and explosion ear ringing are not modeled here.
use crate::gameplay::Inventory;
use glam::Vec3;
use serde::Serialize;

pub const DMG_CRUSH: u32 = 1 << 0;
pub const DMG_BULLET: u32 = 1 << 1;
pub const DMG_SLASH: u32 = 1 << 2;
pub const DMG_BURN: u32 = 1 << 3;
pub const DMG_FALL: u32 = 1 << 5;
pub const DMG_BLAST: u32 = 1 << 6;
pub const DMG_CLUB: u32 = 1 << 7;
pub const DMG_SHOCK: u32 = 1 << 8;
pub const DMG_SONIC: u32 = 1 << 9;
pub const DMG_DROWN: u32 = 1 << 14;
pub const DMG_PARALYZE: u32 = 1 << 15;
pub const DMG_NERVEGAS: u32 = 1 << 16;
pub const DMG_POISON: u32 = 1 << 17;
pub const DMG_RADIATION: u32 = 1 << 18;
pub const DMG_DROWNRECOVER: u32 = 1 << 19;
pub const DMG_ACID: u32 = 1 << 20;
pub const DMG_SLOWBURN: u32 = 1 << 21;
pub const DMG_PLASMA: u32 = 1 << 24;
/// hl2_shareddefs.h DMG_SNIPER (DMG_LASTGENERICFLAG << 1).
pub const DMG_SNIPER: u32 = 1 << 30;
/// CSingleplayRules::Damage_GetTimeBased.
pub const TIME_BASED: u32 = DMG_PARALYZE
    | DMG_NERVEGAS
    | DMG_POISON
    | DMG_RADIATION
    | DMG_DROWNRECOVER
    | DMG_ACID
    | DMG_SLOWBURN;
/// CSingleplayRules::Damage_GetShowOnHud.
pub const SHOW_ON_HUD: u32 = DMG_POISON
    | DMG_ACID
    | DMG_DROWN
    | DMG_BURN
    | DMG_SLOWBURN
    | DMG_NERVEGAS
    | DMG_RADIATION
    | DMG_SHOCK;

pub const FFADE_IN: u32 = 0x1;
pub const FFADE_OUT: u32 = 0x2;
pub const FFADE_MODULATE: u32 = 0x4;
pub const FFADE_STAYOUT: u32 = 0x8;
pub const FFADE_PURGE: u32 = 0x10;

/// One CTakeDamageInfo aimed at the local player.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DamageInfo {
    pub amount: f32,
    pub kind: u32,
    /// The inflictor's GetAbsOrigin; the world (falls) is at the origin.
    pub inflictor: Vec3,
}

/// A ScreenFade message (UTIL_ScreenFade or env_fade).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ScreenFade {
    pub color: [u8; 4],
    pub duration: f32,
    pub hold: f32,
    pub flags: u32,
}
impl ScreenFade {
    /// The message carries both times as unsigned 16-bit fixed point with 9 fraction
    /// bits (SCREENFADE_FRACBITS; FixedUnsigned16 truncates and clamps).
    pub fn new(color: [u8; 4], duration: f32, hold: f32, flags: u32) -> Self {
        let fixed = |seconds: f32| {
            let value = if seconds.is_finite() {
                (seconds * 512.).clamp(0., 65535.) as u16
            } else {
                0
            };
            f32::from(value) / 512.
        };
        Self {
            color,
            duration: fixed(duration),
            hold: fixed(hold),
            flags,
        }
    }
}

/// CHL2_Player::UpdateClientData "Damage" user message.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DamageMessage {
    pub armor: u8,
    pub taken: u8,
    pub bits: u32,
    pub from: Vec3,
}

/// A SetSuitUpdate request (sentence name without '!', no-repeat seconds).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SuitUpdate {
    pub sentence: &'static str,
    pub no_repeat: f32,
}
const SUIT_NEXT_IN_30SEC: f32 = 30.;
const SUIT_NEXT_IN_1MIN: f32 = 60.;
const SUIT_NEXT_IN_5MIN: f32 = 300.;
const SUIT_NEXT_IN_10MIN: f32 = 600.;
const SUIT_NEXT_IN_30MIN: f32 = 1800.;

#[derive(Clone, Debug, Serialize)]
pub struct PlayerDamage {
    /// CBaseCombatCharacter::m_flDamageAccumulator.
    accumulator: f32,
    /// m_DmgTake / m_DmgSave since the last client message.
    take: f32,
    save: f32,
    bits_damage_type: u32,
    /// m_bitsHUDDamage; None is Spawn's -1, which forces one message.
    bits_hud: Option<u32>,
    origin: Vec3,
    last_damage: i32,
    rng: u64,
    /// sk_dmg_take_scale for the current skill (CHalfLife2::AdjustPlayerDamageTaken).
    pub take_scale: f32,
    /// DamageEffect output since the last drain.
    #[serde(skip)]
    pub fades: Vec<ScreenFade>,
    #[serde(skip)]
    pub sounds: Vec<&'static str>,
    #[serde(skip)]
    pub suit: Vec<SuitUpdate>,
}
impl Default for PlayerDamage {
    fn default() -> Self {
        Self {
            accumulator: 0.,
            take: 0.,
            save: 0.,
            bits_damage_type: 0,
            bits_hud: None,
            origin: Vec3::ZERO,
            last_damage: 0,
            rng: 0x9e37_79b9_7f4a_7c15,
            take_scale: 1.,
            fades: Vec::new(),
            sounds: Vec::new(),
            suit: Vec::new(),
        }
    }
}
impl PlayerDamage {
    /// skill.cfg's sk_dmg_take_scale<skill>, else the CHalfLife2 ConVar default.
    pub fn with_skill(skill: u32, config: Option<&str>) -> Self {
        let name = format!("sk_dmg_take_scale{skill}");
        let configured = config.and_then(|text| {
            text.lines().find_map(|line| {
                let line = line.split("//").next()?;
                let mut tokens = line.split_whitespace();
                (tokens.next()?.eq_ignore_ascii_case(&name))
                    .then(|| tokens.next()?.trim_matches('"').parse::<f32>().ok())
                    .flatten()
            })
        });
        let default = match skill {
            1 => 0.5,
            3 => 1.5,
            _ => 1.,
        };
        Self {
            take_scale: configured
                .filter(|v| v.is_finite() && *v >= 0.)
                .unwrap_or(default),
            ..Self::default()
        }
    }
    fn random_int(&mut self, low: i32, high: i32) -> i32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        low + (self.rng % (high - low + 1) as u64) as i32
    }
    fn suit_update(&mut self, sentence: &'static str, no_repeat: f32) {
        self.suit.push(SuitUpdate {
            sentence,
            no_repeat,
        });
    }
    /// TakeDamage for the local player. Returns the health removed.
    pub fn take(&mut self, inv: &mut Inventory, info: DamageInfo) -> f32 {
        let bits = info.kind;
        let mut amount = info.amount;
        // CHL2_Player::OnTakeDamage -> CHalfLife2::AdjustPlayerDamageTaken.
        if bits & (DMG_DROWN | DMG_CRUSH | DMG_FALL | DMG_POISON | DMG_SNIPER) == 0 {
            amount *= self.take_scale;
        }
        // CBasePlayer::OnTakeDamage: no damage, or already dead.
        if !amount.is_finite() || amount <= 0. || inv.health <= 0. {
            return 0.;
        }
        let health_before = inv.health;
        self.last_damage = amount as i32;
        // Armor takes 80% (ARMOR_RATIO 0.2, ARMOR_BONUS 1), at least one point;
        // m_ArmorValue is an integer.
        if inv.armor > 0. && bits & (DMG_FALL | DMG_DROWN | DMG_POISON | DMG_RADIATION) == 0 {
            let mut health_part = amount * 0.2;
            let armor_part = (amount - health_part).max(1.);
            if armor_part > inv.armor {
                health_part = amount - inv.armor;
                self.save = inv.armor;
                inv.armor = 0.;
            } else {
                self.save = armor_part;
                inv.armor = (inv.armor - armor_part).trunc();
            }
            amount = health_part;
        }
        // CHL2_Player::OnTakeDamage_Alive (drowning sounds need the drown state).
        if bits & DMG_BURN != 0 {
            self.sounds.push("HL2Player.BurnPain");
        }
        // CBasePlayer::OnTakeDamage_Alive, then the base accumulator.
        self.bits_damage_type |= bits;
        let fraction = amount - amount.floor();
        let mut whole = amount - fraction;
        self.accumulator += fraction;
        if self.accumulator >= 1. {
            whole += 1.;
            self.accumulator -= 1.;
        }
        if whole <= 0. {
            return 0.;
        }
        inv.health = (inv.health - whole).max(0.);
        self.origin = info.inflictor;
        self.take += amount.trunc();
        self.damage_effect(amount, bits);
        self.diagnose(inv, bits, health_before);
        health_before - inv.health
    }
    /// CBasePlayer::DamageEffect (slash blood is a separate effect).
    fn damage_effect(&mut self, _amount: f32, bits: u32) {
        if bits & DMG_CRUSH != 0 {
            self.fades
                .push(ScreenFade::new([128, 0, 0, 128], 1., 0.1, FFADE_IN));
        } else if bits & DMG_DROWN != 0 {
            self.fades
                .push(ScreenFade::new([0, 0, 128, 128], 1., 0.1, FFADE_IN));
        } else if bits & DMG_SLASH != 0 {
        } else if bits & DMG_PLASMA != 0 {
            self.fades
                .push(ScreenFade::new([0, 0, 255, 100], 0.2, 0.4, FFADE_MODULATE));
            self.sounds.push("Player.PlasmaDamage");
        } else if bits & DMG_SONIC != 0 {
            self.sounds.push("Player.SonicDamage");
        } else if bits & DMG_BULLET != 0 {
            self.sounds.push("Flesh.BulletImpact");
        }
    }
    /// The suit diagnosis loop and health warnings of CBasePlayer::OnTakeDamage.
    fn diagnose(&mut self, inv: &Inventory, damage_bits: u32, health_before: f32) {
        let health = inv.health as i32;
        let last = self.last_damage;
        let trivial = health > 75 || last < 5;
        let major = last > 25;
        let critical = health < 30;
        let time_based = damage_bits & TIME_BASED != 0;
        let mut bits = damage_bits;
        let mut found = true;
        while (!trivial || time_based) && found && bits != 0 {
            found = false;
            if bits & DMG_CLUB != 0 {
                if major {
                    self.suit_update("HEV_DMG4", SUIT_NEXT_IN_30SEC);
                }
                bits &= !DMG_CLUB;
                found = true;
            }
            if bits & (DMG_FALL | DMG_CRUSH) != 0 {
                self.suit_update(
                    if major { "HEV_DMG5" } else { "HEV_DMG4" },
                    SUIT_NEXT_IN_30SEC,
                );
                bits &= !(DMG_FALL | DMG_CRUSH);
                found = true;
            }
            if bits & DMG_BULLET != 0 {
                if last > 5 {
                    self.suit_update("HEV_DMG6", SUIT_NEXT_IN_30SEC);
                }
                bits &= !DMG_BULLET;
                found = true;
            }
            if bits & DMG_SLASH != 0 {
                self.suit_update(
                    if major { "HEV_DMG1" } else { "HEV_DMG0" },
                    SUIT_NEXT_IN_30SEC,
                );
                bits &= !DMG_SLASH;
                found = true;
            }
            if bits & DMG_SONIC != 0 {
                if major {
                    self.suit_update("HEV_DMG2", SUIT_NEXT_IN_1MIN);
                }
                bits &= !DMG_SONIC;
                found = true;
            }
            if bits & (DMG_POISON | DMG_PARALYZE) != 0 {
                self.suit_update("HEV_DMG3", SUIT_NEXT_IN_1MIN);
                bits &= !(DMG_POISON | DMG_PARALYZE);
                found = true;
            }
            if bits & DMG_ACID != 0 {
                self.suit_update("HEV_DET1", SUIT_NEXT_IN_1MIN);
                bits &= !DMG_ACID;
                found = true;
            }
            if bits & DMG_NERVEGAS != 0 {
                self.suit_update("HEV_DET0", SUIT_NEXT_IN_1MIN);
                bits &= !DMG_NERVEGAS;
                found = true;
            }
            if bits & DMG_RADIATION != 0 {
                self.suit_update("HEV_DET2", SUIT_NEXT_IN_1MIN);
                bits &= !DMG_RADIATION;
                found = true;
            }
            if bits & DMG_SHOCK != 0 {
                bits &= !DMG_SHOCK;
                found = true;
            }
        }
        if !trivial && major && health_before >= 75. {
            self.suit_update("HEV_MED1", SUIT_NEXT_IN_30MIN);
            self.suit_update("HEV_HEAL7", SUIT_NEXT_IN_30MIN);
        }
        if !trivial && critical && health_before < 75. {
            if health < 6 {
                self.suit_update("HEV_HLTH3", SUIT_NEXT_IN_10MIN);
            } else if health < 20 {
                self.suit_update("HEV_HLTH2", SUIT_NEXT_IN_10MIN);
            }
            if self.random_int(0, 3) == 0 && health_before < 50. {
                self.suit_update("HEV_DMG7", SUIT_NEXT_IN_5MIN);
            }
        }
        if time_based && health_before < 75. {
            if health_before < 50. {
                if self.random_int(0, 3) == 0 {
                    self.suit_update("HEV_DMG7", SUIT_NEXT_IN_5MIN);
                }
            } else {
                self.suit_update("HEV_HLTH1", SUIT_NEXT_IN_10MIN);
            }
        }
    }
    /// m_bitsDamageType: damage types sustained since the last client message.
    pub fn bits(&self) -> u32 {
        self.bits_damage_type
    }
    /// CHL2_Player::UpdateClientData: the "Damage" message when anything changed.
    pub fn client_message(&mut self) -> Option<DamageMessage> {
        if self.take == 0. && self.save == 0. && self.bits_hud == Some(self.bits_damage_type) {
            return None;
        }
        let message = DamageMessage {
            armor: self.save.clamp(0., 255.) as u8,
            taken: self.take.clamp(0., 255.) as u8,
            bits: self.bits_damage_type & SHOW_ON_HUD,
            from: self.origin,
        };
        self.take = 0.;
        self.save = 0.;
        self.bits_hud = Some(self.bits_damage_type);
        self.bits_damage_type &= TIME_BASED;
        Some(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(health: f32, armor: f32) -> Inventory {
        let mut inv = Inventory::default();
        inv.health = health;
        inv.armor = armor;
        inv.suit = true;
        inv
    }
    fn hit(amount: f32, kind: u32) -> DamageInfo {
        DamageInfo {
            amount,
            kind,
            inflictor: Vec3::new(10., 0., 0.),
        }
    }

    #[test]
    fn skill_scale_skips_exempt_damage_types() {
        let mut damage = PlayerDamage::with_skill(3, Some("sk_dmg_take_scale3 \"2.0\"\n"));
        assert_eq!(damage.take_scale, 2.);
        let mut inv = player(100., 0.);
        assert_eq!(damage.take(&mut inv, hit(10., DMG_BULLET)), 20.);
        assert_eq!(damage.take(&mut inv, hit(10., DMG_FALL)), 10.);
        assert_eq!(damage.take(&mut inv, hit(10., DMG_CRUSH)), 10.);
        // The owned skill.cfg has no override: skill 2 keeps the ConVar default.
        assert_eq!(
            PlayerDamage::with_skill(2, Some("sk_plr_dmg_crowbar 10")).take_scale,
            1.
        );
        assert_eq!(PlayerDamage::with_skill(1, None).take_scale, 0.5);
    }

    #[test]
    fn armor_and_fractional_damage_follow_integer_fields() {
        let mut damage = PlayerDamage::default();
        let mut inv = player(100., 50.);
        // 7 damage: armor takes 5.6 (int field truncates to 44), health 1.4 -> 1 now.
        assert_eq!(damage.take(&mut inv, hit(7., DMG_BULLET)), 1.);
        assert_eq!((inv.health, inv.armor), (99., 44.));
        // The 0.4 fraction accumulates: 0.4 + 0.4 -> no hit, then 1.2 -> +1.
        assert_eq!(damage.take(&mut inv, hit(0.4, DMG_FALL)), 0.);
        assert_eq!(damage.take(&mut inv, hit(0.4, DMG_FALL)), 1.);
        // Armor smaller than its share: health takes the rest.
        let mut inv = player(100., 2.);
        assert_eq!(damage.take(&mut inv, hit(20., DMG_BULLET)), 18.);
        assert_eq!(inv.armor, 0.);
        // The dead take nothing.
        let mut inv = player(0., 0.);
        assert_eq!(damage.take(&mut inv, hit(20., DMG_BULLET)), 0.);
    }

    #[test]
    fn client_message_reports_once_and_keeps_time_based_bits() {
        let mut damage = PlayerDamage::default();
        // Spawn's m_bitsHUDDamage -1 forces one empty message.
        assert_eq!(damage.client_message().map(|m| m.taken), Some(0));
        assert_eq!(damage.client_message(), None);
        let mut inv = player(100., 10.);
        damage.take(&mut inv, hit(30., DMG_BURN));
        let message = damage.client_message().unwrap();
        assert_eq!(
            (message.armor, message.taken, message.bits),
            (10, 20, DMG_BURN)
        );
        assert_eq!(message.from, Vec3::new(10., 0., 0.));
        // Burn is not time based: the bits clear, which sends one more message.
        assert_eq!(damage.client_message().map(|m| m.bits), Some(0));
        assert_eq!(damage.client_message(), None);
    }

    #[test]
    fn damage_effects_and_suit_diagnosis_follow_the_sdk() {
        let mut damage = PlayerDamage::default();
        let mut inv = player(100., 0.);
        damage.take(&mut inv, hit(30., DMG_CRUSH));
        assert_eq!(
            damage.fades,
            vec![ScreenFade::new([128, 0, 0, 128], 1., 0.1, FFADE_IN)]
        );
        // Major (> 25) crush from >= 75 health: major fracture, automedic, morphine.
        let names: Vec<_> = damage.suit.iter().map(|s| s.sentence).collect();
        assert_eq!(names, ["HEV_DMG5", "HEV_MED1", "HEV_HEAL7"]);
        // Trivial hits (health still above 75 or under 5 damage) are not diagnosed.
        let mut damage = PlayerDamage::default();
        let mut inv = player(100., 0.);
        damage.take(&mut inv, hit(10., DMG_BULLET));
        assert!(damage.suit.is_empty());
        assert_eq!(damage.sounds, ["Flesh.BulletImpact"]);
        // Critical: health 15 after a hit from below 75.
        let mut damage = PlayerDamage::default();
        let mut inv = player(25., 0.);
        damage.take(&mut inv, hit(10., DMG_FALL));
        let names: Vec<_> = damage.suit.iter().map(|s| s.sentence).collect();
        assert_eq!(&names[..2], ["HEV_DMG4", "HEV_HLTH2"]);
    }

    #[test]
    fn screen_fade_times_use_fixed_point() {
        let fade = ScreenFade::new([0; 4], 0.1, 1000., 0);
        assert_eq!(fade.duration, 51. / 512.);
        assert_eq!(fade.hold, 65535. / 512.);
    }
}
