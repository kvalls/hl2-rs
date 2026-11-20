//! The retail engine's sound channels: 64 dynamic channels for entity sounds and 64 static ones
//! (ambient_generic, soundscapes, `CHAN_STATIC` scripts), so it never mixes more than 128 sounds.
//! A static sound that finds no free channel does not play. A dynamic sound takes over the
//! channel its emitter already plays on (same entity, same script channel other than
//! `CHAN_AUTO`), else the lowest free channel, else steals the lowest busy channel that is not
//! a voice or stream and not the player's own (unless the new sound is the player's too).
//! The engine also has an optional duplicate-sound limit behind a console variable; it is not
//! reproduced (its variable and default were not identified).
use bevy::prelude::{Commands, Entity};
use hl2_simulation::sounds::SoundRequest;

pub const DYNAMIC: usize = 64;
pub const STATIC: usize = 64;
/// SDK soundflags.h channels the picking rules name.
pub const CHAN_AUTO: i32 = 0;
pub const CHAN_VOICE: i32 = 2;
pub const CHAN_STREAM: i32 = 5;
pub const CHAN_STATIC: i32 = 6;
pub const CHAN_VOICE2: i32 = 7;

/// Who plays a dynamic sound, for the picking rules.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Voice {
    /// The emitting entity or handle, when the request names one.
    pub source: Option<u64>,
    /// The script channel (`CHAN_*`).
    pub channel: i32,
    /// The player's own sound (unpositioned): other sounds never steal its channel.
    pub player: bool,
}

/// Where a dynamic sound goes.
#[derive(Debug, PartialEq)]
pub enum Pick {
    Free(usize),
    /// Stops this channel's sound first.
    Replace(usize, Entity),
}

#[derive(Default)]
pub struct Channels {
    dynamic: Vec<Option<(Entity, Voice)>>,
    statics: Vec<Entity>,
}

impl Channels {
    /// Frees the channels whose sound has ended (its player is gone).
    pub fn retain(&mut self, live: impl Fn(Entity) -> bool) {
        for slot in &mut self.dynamic {
            if slot.is_some_and(|(entity, _)| !live(entity)) {
                *slot = None;
            }
        }
        self.statics.retain(|entity| live(*entity));
    }
    pub fn pick_dynamic(&self, voice: &Voice) -> Option<Pick> {
        let busy = || (self.dynamic.iter().enumerate()).filter_map(|(i, s)| s.map(|s| (i, s)));
        if voice.channel != CHAN_AUTO && voice.source.is_some() {
            let same = |(_, (_, v)): &(usize, (Entity, Voice))| {
                v.source == voice.source && v.channel == voice.channel
            };
            if let Some((i, (entity, _))) = busy().find(same) {
                return Some(Pick::Replace(i, entity));
            }
        }
        let free = (0..DYNAMIC).find(|&i| self.dynamic.get(i).is_none_or(Option::is_none));
        if let Some(i) = free {
            return Some(Pick::Free(i));
        }
        let stealable = |(_, (_, v)): &(usize, (Entity, Voice))| {
            !matches!(v.channel, CHAN_VOICE | CHAN_STREAM | CHAN_VOICE2)
                && (!v.player || voice.player)
        };
        busy()
            .find(stealable)
            .map(|(i, (entity, _))| Pick::Replace(i, entity))
    }
    /// Records the sound now playing on dynamic channel `slot`.
    pub fn take_dynamic(&mut self, slot: usize, entity: Entity, voice: Voice) {
        if self.dynamic.len() <= slot {
            self.dynamic.resize(slot + 1, None);
        }
        self.dynamic[slot] = Some((entity, voice));
    }
    pub fn static_free(&self) -> bool {
        self.statics.len() < STATIC
    }
    pub fn take_static(&mut self, entity: Entity) {
        self.statics.push(entity);
    }
    /// Sounds on dynamic and static channels.
    pub fn playing(&self) -> (usize, usize) {
        (self.dynamic.iter().flatten().count(), self.statics.len())
    }
}

/// The channel a starting sound was given.
pub(super) enum Claim {
    /// UI sounds stay outside the world's channels.
    Ui,
    Static,
    Dynamic(usize, Voice),
}

impl super::Audio {
    /// The retail channel for a sound about to start (this module), stopping the sound it
    /// takes over; None when no channel takes it.
    pub(super) fn claim(
        &mut self,
        commands: &mut Commands,
        request: &SoundRequest,
        statics: bool,
        channel: i32,
    ) -> Option<Claim> {
        if request.ui {
            return Some(Claim::Ui);
        }
        if statics {
            if self.channels.static_free() {
                return Some(Claim::Static);
            }
            self.capacity_rejections += 1;
            return None;
        }
        let speaker = request
            .actor
            .as_ref()
            .and_then(|a| a.entity)
            .map(|e| e as u64);
        let voice = Voice {
            source: request.emitter.or(speaker),
            channel,
            player: request.origin.is_none(),
        };
        match self.channels.pick_dynamic(&voice) {
            Some(Pick::Free(slot)) => Some(Claim::Dynamic(slot, voice)),
            Some(Pick::Replace(slot, stopped)) => {
                commands.entity(stopped).try_despawn();
                Some(Claim::Dynamic(slot, voice))
            }
            None => {
                self.capacity_rejections += 1;
                None
            }
        }
    }
    pub(super) fn record(&mut self, claim: Claim, player: Entity) {
        match claim {
            Claim::Ui => {}
            Claim::Static => self.channels.take_static(player),
            Claim::Dynamic(slot, voice) => self.channels.take_dynamic(slot, player, voice),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(source: Option<u64>, channel: i32, player: bool) -> Voice {
        Voice {
            source,
            channel,
            player,
        }
    }
    fn entity(i: u32) -> Entity {
        Entity::from_raw_u32(i + 1).unwrap()
    }

    #[test]
    fn an_emitter_replaces_its_own_channel_then_sounds_fill_free_channels() {
        let mut channels = Channels::default();
        let gun = voice(Some(7), 1, false);
        assert_eq!(channels.pick_dynamic(&gun), Some(Pick::Free(0)));
        channels.take_dynamic(0, entity(0), gun);
        // The same entity on the same channel replaces it; CHAN_AUTO never does.
        assert_eq!(
            channels.pick_dynamic(&gun),
            Some(Pick::Replace(0, entity(0)))
        );
        let auto = voice(Some(7), CHAN_AUTO, false);
        assert_eq!(channels.pick_dynamic(&auto), Some(Pick::Free(1)));
        // A finished sound frees its channel.
        channels.retain(|e| e != entity(0));
        assert_eq!(channels.pick_dynamic(&gun), Some(Pick::Free(0)));
    }

    #[test]
    fn full_channels_steal_the_lowest_ordinary_sound_but_never_voices_or_the_player() {
        let mut channels = Channels::default();
        for i in 0..DYNAMIC {
            let v = match i {
                0 => voice(None, CHAN_VOICE, false),
                1 => voice(None, 1, true),
                2 => voice(None, CHAN_STREAM, false),
                _ => voice(None, 4, false),
            };
            channels.take_dynamic(i, entity(i as u32), v);
        }
        let npc = voice(Some(3), 4, false);
        assert_eq!(
            channels.pick_dynamic(&npc),
            Some(Pick::Replace(3, entity(3)))
        );
        // The player's own sound may take another player sound's channel.
        let player = voice(None, 1, true);
        assert_eq!(
            channels.pick_dynamic(&player),
            Some(Pick::Replace(1, entity(1)))
        );
        // With only voices, streams and the player's sounds left, nothing is stolen.
        let mut voices = Channels::default();
        for i in 0..DYNAMIC {
            voices.take_dynamic(i, entity(i as u32), voice(None, CHAN_VOICE, false));
        }
        assert_eq!(voices.pick_dynamic(&npc), None);
    }

    #[test]
    fn static_channels_refuse_the_sixty_fifth_sound() {
        let mut channels = Channels::default();
        for i in 0..STATIC {
            assert!(channels.static_free());
            channels.take_static(entity(i as u32));
        }
        assert!(!channels.static_free());
        channels.retain(|e| e != entity(5));
        assert!(channels.static_free());
        assert_eq!(channels.playing(), (0, STATIC - 1));
    }
}
