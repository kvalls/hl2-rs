//! The health and suit events of the HUD's animation state.
use super::{Animate, Entry, EventRule, Inventory, NumericHudEffects};

impl NumericHudEffects {
    /// CHudHealth and CHudBattery: their animation events as the values change.
    pub(super) fn observe_vitals(
        &mut self,
        inv: &Inventory,
        time: f64,
        rules: &[Animate],
        events: &[EventRule],
        settings: &Entry,
    ) {
        let health = (inv.health as i32).max(0);
        if health != self.health {
            if health >= 20 {
                self.start("HealthIncreasedAbove20", time, rules, events, settings);
            } else if health > 0 {
                self.start("HealthIncreasedBelow20", time, rules, events, settings);
                self.start("HealthLow", time, rules, events, settings);
            }
            // CHudHealth starts nothing at zero; HudPlayerDeath comes from the
            // damage indicator's message handler.
            self.health = health;
        }
        let armor = inv.armor as i32;
        if armor != self.armor {
            if armor == 0 {
                self.start("SuitPowerZero", time, rules, events, settings);
            } else if armor < self.armor {
                self.start("SuitDamageTaken", time, rules, events, settings);
                if armor < 20 {
                    self.start("SuitArmorLow", time, rules, events, settings);
                }
            } else {
                let event = if self.armor == -1 || self.armor == 0 || armor >= 20 {
                    "SuitPowerIncreasedAbove20"
                } else {
                    "SuitPowerIncreasedBelow20"
                };
                self.start(event, time, rules, events, settings);
            }
            self.armor = armor;
        }
    }
}
