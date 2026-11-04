//! The player's outcome of a tick.
use super::Gameplay;
use glam::Vec3;

impl Gameplay {
    /// After the player move: this tick's damage (trigger_hurt, blasts, then the
    /// landing's fall damage) through the SDK player path, then death
    /// (Event_Killed/DeathSound). Returns true when the player died this tick.
    pub fn player_outcome(&mut self, player: &mut modkit_core::movement::Player) -> bool {
        use hl2_simulation::player_damage::{DMG_FALL, DamageInfo};
        // CHL2_Player::OnTakeDamage refuses everything while gordon_invulnerable is on
        // (d1_trainstation_01-03 and parts of 04); TakeHealth is separate.
        let invulnerable = self.scene.globals.is_on("gordon_invulnerable");
        let mut infos = std::mem::take(&mut self.scene.player_damage);
        if let Some(speed) = player.landed {
            let damage = hl2_simulation::gameplay::fall_damage(speed);
            if damage > 0. {
                // CMoveHelperServer::PlayerFallingDamage: the world inflicts DMG_FALL.
                infos.push(DamageInfo {
                    amount: damage,
                    kind: DMG_FALL,
                    inflictor: Vec3::ZERO,
                });
            }
        }
        for info in infos {
            if info.amount < 0. {
                if self.inventory.health > 0. {
                    self.inventory.health = (self.inventory.health - info.amount).min(100.);
                }
            } else if !invulnerable
                && self.player_damage.take(&mut self.inventory, info) > 0.
                && info.kind & DMG_FALL != 0
                && player.landed.is_some()
            {
                self.scene.sounds.push("Player.FallDamage".into());
            }
        }
        // CBlackHeadcrab::TouchDamage: the bite (DMG_SLASH) leaves at least one health, then
        // DMG_POISON takes all but one; CheckTimeBasedDamage restores the poison later.
        for (crab, bite) in std::mem::take(&mut self.ai.poison_bites) {
            if !invulnerable {
                let inflictor = self.scene.states[crab].origin;
                self.player_damage
                    .poison_bite(&mut self.inventory, bite, inflictor);
            }
        }
        let now = self.scene.time;
        self.player_damage
            .check_time_based(&mut self.inventory, now);
        // PlayerFallingDamage: a landing that leaves the player dead cuts to black
        // (FlPlayerFallDeathDoesScreenFade, duration 0, hold 9999, OUT | STAYOUT).
        if player
            .landed
            .is_some_and(|speed| hl2_simulation::gameplay::fall_damage(speed) > 0.)
            && self.inventory.health <= 0.
            && !player.dead
        {
            use hl2_simulation::player_damage::{FFADE_OUT, FFADE_STAYOUT, ScreenFade};
            self.player_damage.fades.push(ScreenFade::new(
                [0, 0, 0, 255],
                0.,
                9999.,
                FFADE_OUT | FFADE_STAYOUT,
            ));
        }
        let suit = self.inventory.suit;
        for sound in self.player_damage.sounds.drain(..) {
            self.scene.sounds.push(sound.into());
        }
        let now = self.scene.time;
        for update in std::mem::take(&mut self.player_damage.suit) {
            self.suit
                .update(suit, update.sentence, update.no_repeat, now);
        }
        // Weapon SetSuitUpdate requests (crossbow/RPG out of ammo).
        for (sentence, no_repeat) in std::mem::take(&mut self.inventory.suit_updates) {
            self.suit.update(suit, &sentence, no_repeat, now);
        }
        let died = !player.dead && self.inventory.health <= 0.;
        if died {
            self.player_dies(player, suit);
        }
        self.suit_speech(suit && !player.dead, now);
        if let Some(message) = self.player_damage.client_message() {
            self.damage_messages.push(message);
        }
        died
    }
}
