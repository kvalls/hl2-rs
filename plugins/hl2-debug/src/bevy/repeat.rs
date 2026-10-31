use bevy::prelude::{ButtonInput, KeyCode};

#[derive(Default)]
pub struct KeyRepeat {
    key: Option<KeyCode>,
    wait: f32,
    pub overlay_cursor: bool,
}
const REPEAT_DELAY: f32 = 0.35;
const REPEAT_INTERVAL: f32 = 0.04;
impl KeyRepeat {
    pub fn update(
        &mut self,
        input: &ButtonInput<KeyCode>,
        keys: &[KeyCode],
        dt: f32,
    ) -> Option<(KeyCode, usize)> {
        if let Some(&key) = keys.iter().find(|k| input.just_pressed(**k)) {
            (self.key, self.wait) = (Some(key), REPEAT_DELAY);
            return Some((key, 1));
        }
        let key = self.key.filter(|k| input.pressed(*k))?;
        self.wait -= dt;
        let mut presses = 0;
        while self.wait <= 0. {
            presses += 1;
            self.wait += REPEAT_INTERVAL;
        }
        (presses > 0).then_some((key, presses))
    }
}
