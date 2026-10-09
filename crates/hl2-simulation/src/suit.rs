//! HEV suit voice scheduling: SDK CBasePlayer::SetSuitUpdate / CheckSuitUpdate (a play
//! list of four sentences, 32 no-repeat slots, 3.5 s between sentences, 0.1 s before the
//! first) and the immediate suit sounds (UTIL_EmitSoundSuit, UTIL_EmitGroupnameSuit).
//! Nothing is scheduled or played without the suit. Sentence playback is the host's.
use serde::Serialize;

const CSUITPLAYLIST: usize = 4;
const CSUITNOREPEAT: usize = 32;
const SUITUPDATETIME: f64 = 3.5;
const SUITFIRSTUPDATETIME: f64 = 0.1;

#[derive(Clone, Debug, Serialize)]
pub struct Suit {
    play_list: [Option<String>; CSUITPLAYLIST],
    play_next: usize,
    no_repeat: [Option<(String, f64)>; CSUITNOREPEAT],
    /// m_flSuitUpdate; 0 when the queue is empty.
    update_at: f64,
    rng: u64,
    /// Sentences (or `#group`) to start now, drained by the host.
    #[serde(skip)]
    pub play: Vec<String>,
}
impl Default for Suit {
    fn default() -> Self {
        Self {
            play_list: Default::default(),
            play_next: 0,
            no_repeat: Default::default(),
            update_at: 0.,
            rng: 0x2545_f491_4f6c_dd1d,
            play: Vec::new(),
        }
    }
}
impl Suit {
    fn random_int(&mut self, low: usize, high: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        low + (self.rng % (high - low + 1) as u64) as usize
    }
    /// SetSuitUpdate for a sentence name (without '!'), with no-repeat seconds.
    pub fn update(&mut self, suit: bool, sentence: &str, no_repeat: f32, now: f64) {
        if !suit {
            return;
        }
        let mut empty = None;
        for i in 0..CSUITNOREPEAT {
            match &self.no_repeat[i] {
                Some((name, until)) if name.eq_ignore_ascii_case(sentence) => {
                    if *until < now {
                        self.no_repeat[i] = None;
                        empty = Some(i);
                        break;
                    }
                    return;
                }
                None => empty = Some(i),
                _ => {}
            }
        }
        if no_repeat > 0. {
            let slot = match empty {
                Some(slot) => slot,
                None => self.random_int(0, CSUITNOREPEAT - 1),
            };
            self.no_repeat[slot] = Some((sentence.into(), now + f64::from(no_repeat)));
        }
        self.play_list[self.play_next] = Some(sentence.into());
        self.play_next = (self.play_next + 1) % CSUITPLAYLIST;
        if self.update_at <= now {
            self.update_at = now
                + if self.update_at == 0. {
                    SUITFIRSTUPDATETIME
                } else {
                    SUITUPDATETIME
                };
        }
    }
    /// CheckSuitUpdate, every player think: play the next queued sentence when due.
    pub fn think(&mut self, suit: bool, now: f64) {
        if !suit || self.update_at <= 0. || now < self.update_at {
            return;
        }
        let mut search = self.play_next;
        for _ in 0..CSUITPLAYLIST {
            if let Some(sentence) = self.play_list[search].take() {
                self.play.push(sentence);
                self.update_at = now + SUITUPDATETIME;
                return;
            }
            search = (search + 1) % CSUITPLAYLIST;
        }
        self.update_at = 0.;
    }
    /// Moves pending timers to a new map's clock (level change).
    pub fn rebase_clock(&mut self, previous: f64, next: f64) {
        let shift = next - previous;
        if self.update_at > 0. {
            self.update_at = (self.update_at + shift).max(f64::MIN_POSITIVE);
        }
        for (_, until) in self.no_repeat.iter_mut().flatten() {
            *until += shift;
        }
    }
    /// UTIL_EmitSoundSuit / UTIL_EmitGroupnameSuit: play at once (`#` marks a group).
    pub fn emit(&mut self, suit: bool, sentence_or_group: &str) {
        if suit {
            self.play.push(sentence_or_group.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_spacing_and_no_repeat_follow_the_sdk() {
        let mut suit = Suit::default();
        suit.update(true, "HEV_DMG5", 30., 10.);
        suit.update(true, "HEV_MED1", 1800., 10.);
        suit.update(true, "HEV_HEAL7", 1800., 10.);
        // A repeat inside its window is dropped.
        suit.update(true, "HEV_DMG5", 30., 11.);
        suit.think(true, 10.05);
        assert!(suit.play.is_empty());
        suit.think(true, 10.1);
        assert_eq!(suit.play, ["HEV_DMG5"]);
        suit.think(true, 13.5);
        assert_eq!(suit.play.len(), 1);
        suit.think(true, 13.6);
        suit.think(true, 17.1);
        assert_eq!(suit.play, ["HEV_DMG5", "HEV_MED1", "HEV_HEAL7"]);
        // The queue empties, and after 30 s the fracture can be reported again.
        suit.think(true, 20.6);
        suit.update(true, "HEV_DMG5", 30., 41.);
        suit.think(true, 41.1);
        assert_eq!(suit.play.last().map(String::as_str), Some("HEV_DMG5"));
    }

    #[test]
    fn nothing_without_the_suit() {
        let mut suit = Suit::default();
        suit.update(false, "HEV_DMG5", 30., 0.);
        suit.emit(false, "#HEV_DEAD");
        suit.think(false, 1.);
        suit.think(true, 1.);
        assert!(suit.play.is_empty());
    }
}
