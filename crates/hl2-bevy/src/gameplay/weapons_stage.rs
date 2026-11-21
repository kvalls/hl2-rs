//! The weapons stage of `Gameplay::tick`: projectile fire, carrying, the active weapon
//! and the gravity gun, in CBasePlayer::PostThink order.
use super::Gameplay;
use glam::Vec3;
use hl2_simulation::physics::Physics;
use modkit_core::movement::{Player, TICK};

impl Gameplay {
    pub(super) fn weapons_stage(
        &mut self,
        physics: &mut Physics,
        player: &Player,
        eye: Vec3,
        direction: Vec3,
    ) {
        let _span = hl2_simulation::profile_span!("weapons", active = %self.inventory.active);
        self.inventory.advance_projectile_fire(
            &self.world,
            &mut self.scene,
            &self.weapons,
            eye,
            direction,
        );
        // CBasePlayer::PostThink -> CPlayerPickupController::Use(USE_SET): held attack
        // throws, held attack2 drops; the holstered weapon does not fire meanwhile.
        let carrying = self.inventory.carrying();
        if carrying {
            self.inventory.carry_tick(
                &self.weapons,
                &self.world,
                &mut self.scene,
                physics,
                eye,
                direction,
                player.feet,
                player.crouched,
                self.primary,
                self.secondary,
            );
            self.primary_consumed |= self.primary;
            self.secondary_consumed |= self.secondary;
        }
        self.attack_stage(carrying, physics, player, eye, direction);
        // weapon_physcannon: ItemPreFrame/ItemPostFrame on the Rapier props (weapons stage).
        self.inventory.physcannon_frame(
            &self.weapons,
            &self.world,
            &mut self.scene,
            physics,
            eye,
            direction,
            player.feet,
            player.crouched,
        );
        self.queued_primary = false;
        self.queued_secondary = false;
    }
    /// The active weapon: attack input, its frame, then the secondary or primary attack.
    fn attack_stage(
        &mut self,
        carrying: bool,
        physics: &mut Physics,
        player: &Player,
        eye: Vec3,
        direction: Vec3,
    ) {
        let allowed = self.selection.pending.is_none() && !carrying;
        let primary = allowed && (self.primary || self.queued_primary) && !self.primary_consumed;
        let secondary = allowed
            && (self.secondary || self.queued_secondary)
            && !self.secondary_consumed
            && matches!(
                self.inventory.active.as_str(),
                "weapon_shotgun"
                    | "weapon_smg1"
                    | "weapon_ar2"
                    | "weapon_frag"
                    | "weapon_crossbow"
                    | "weapon_bugbait"
                    | "weapon_physcannon"
            );
        self.inventory.set_attack_input(primary, secondary);
        self.inventory.ducking = player.crouched;
        let attacking = primary
            || secondary
            || allowed
                && (!self.primary_consumed && self.inventory.delayed_attack
                    || !self.secondary_consumed && self.inventory.delayed_secondary_attack);
        self.inventory.tick(
            &self.world,
            &mut self.scene,
            &self.weapons,
            player.feet,
            attacking,
            TICK,
        );
        if allowed
            && !self.secondary_consumed
            && (secondary || self.inventory.delayed_secondary_attack)
        {
            self.inventory.secondary_attack(
                &self.weapons,
                &self.world,
                &mut self.scene,
                physics,
                eye,
                direction,
            );
        } else if allowed && !self.primary_consumed && (primary || self.inventory.delayed_attack) {
            let active = self.inventory.active.clone();
            let clip = self.inventory.owned.get(&active).copied();
            self.inventory.attack(
                &self.weapons,
                &self.world,
                &mut self.scene,
                physics,
                eye,
                direction,
            );
            // The weapons' PrimaryAttack: CSoundEnt::InsertSound(SOUND_COMBAT, origin,
            // SOUNDENT_VOLUME_PISTOL/MACHINEGUN/SHOTGUN 1500 (the 357: 600), 0.2).
            let volume = match active.as_str() {
                "weapon_pistol" | "weapon_smg1" | "weapon_ar2" | "weapon_shotgun" => Some(1500.),
                "weapon_357" => Some(600.),
                _ => None,
            };
            if let Some(volume) =
                volume.filter(|_| self.inventory.owned.get(&active).copied() < clip)
            {
                self.ai
                    .insert_combat_sound(player.feet, volume, self.scene.time);
            }
        }
    }
}
