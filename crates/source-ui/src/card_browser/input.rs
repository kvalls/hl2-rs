//! Shared toolkit focus, key activation, pointer capture and placement.
use super::{CardBrowser, Outcome, Target};
use crate::{
    input::{Input, Key},
    widgets::focus_step,
};
use glam::Vec2;

impl CardBrowser {
    /// Another layer owns input; preserve selection while cancelling hidden interactions.
    pub fn occlude(&mut self, down: bool) {
        self.tracker.occlude(down);
        self.keys.reset();
        self.hover = None;
        self.escape_held = false;
        self.placement.cancel();
    }

    pub fn take_sounds(&mut self) -> Vec<String> {
        std::mem::take(&mut self.sounds)
    }

    pub fn input(&mut self, input: &Input, canvas: Vec2) -> Outcome {
        let frame = self.frame(canvas);
        if self
            .placement
            .input(&frame, input.pointer, input.click, input.held, input.time)
        {
            self.tracker.occlude(input.click || input.held);
            self.keys.reset();
            return Outcome::None;
        }
        let origin = self.placement.place(&frame).origin;
        let pointer = input.pointer.map(|p| (p - origin) / frame.scale);
        self.hover =
            pointer.and_then(|p| self.targets().find(|(_, r)| r.contains(p)).map(|(t, _)| t));
        let clicked = self.tracker.update(
            self.hover,
            input.click || input.held,
            input.click,
            input.released,
        );
        if input.click {
            self.focus = self.hover;
            self.focus_visible = false;
        }
        if input.pressed(Key::Tab) {
            let mut order = [Target::Close; 10];
            let mut count = 0;
            for (target, _) in self.targets() {
                order[count] = target;
                count += 1;
            }
            self.focus = focus_step(self.focus, &order[..count], !input.shift);
            self.focus_visible = true;
        }
        if matches!(self.focus, Some(Target::Card(_))) {
            let delta = i32::from(input.pressed(Key::Right)) - i32::from(input.pressed(Key::Left));
            if delta != 0 && !self.spec.cards.is_empty() {
                self.selected = (self.selected as i32 + delta)
                    .clamp(0, self.spec.cards.len() as i32 - 1)
                    as usize;
                self.first = self.selected / 3 * 3;
                self.focus = Some(Target::Card(self.selected));
                self.focus_visible = true;
            }
        }
        let pressed = [Key::Enter, Key::Space]
            .into_iter()
            .find(|k| input.pressed(*k));
        let key = self
            .keys
            .update(self.focus, pressed, |k| input.held_keys.contains(&k));
        if let Some(target) = clicked.or(key) {
            if let Some(sound) = &self.spec.release_sound {
                self.sounds.push(sound.clone());
            }
            return self.activate(target, key.is_some());
        }
        Outcome::None
    }

    fn activate(&mut self, target: Target, keyboard: bool) -> Outcome {
        match target {
            Target::Close | Target::Cancel => return Outcome::Close,
            Target::Card(index) => {
                self.selected = index;
                if keyboard {
                    return self.start();
                }
            }
            Target::Next => {
                self.first += 3;
                self.selected = self.first;
                self.focus = Some(Target::Card(self.selected));
            }
            Target::Previous => {
                self.first = self.first.saturating_sub(3);
                self.selected = self.first;
                self.focus = Some(Target::Card(self.selected));
            }
            Target::Toggle => {
                self.toggle_checked = !self.toggle_checked;
            }
            Target::Accept => return self.start(),
        }
        Outcome::None
    }

    fn start(&self) -> Outcome {
        if self.selected < self.spec.cards.len() {
            Outcome::Accept(self.selected)
        } else {
            Outcome::None
        }
    }
}
