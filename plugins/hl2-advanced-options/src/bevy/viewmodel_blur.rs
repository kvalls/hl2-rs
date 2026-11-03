//! Owner-requested animation blur, independent of the retail world-camera blur.
use super::weapon_blur::WeaponBlur;
use super::{BlurFrame, WeaponCamera as ViewmodelCamera};
use bevy::{
    core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass},
    prelude::{Commands, Entity, Local, Projection, Query, Res, Transform, Vec4, With},
};

#[derive(Default)]
pub(super) struct History {
    weapon: String,
    time: f64,
    warm: u8,
    view: Option<(Transform, Vec4, usize)>,
}

impl History {
    fn advance(&mut self, weapon: &str, time: f64, enabled: bool) -> bool {
        if !enabled {
            self.warm = 0;
            return false;
        }
        if self.weapon != weapon || time < self.time || time - self.time > 0.1 {
            self.weapon.clear();
            self.weapon.push_str(weapon);
            self.warm = 0;
        }
        self.time = time;
        self.warm = self.warm.saturating_add(1).min(3);
        self.warm >= 3
    }
}

pub(super) fn update(
    mut commands: Commands,
    frame: Option<Res<BlurFrame>>,
    mut history: Local<History>,
    cameras: Query<(Entity, &Transform, &Projection, Option<&WeaponBlur>), With<ViewmodelCamera>>,
) {
    let frame = frame.as_deref();
    let active = frame.map_or("", |f| f.weapon.as_str());
    let scene_time = frame.map_or(0., |f| f.time);
    let enabled = frame.is_some_and(|f| f.viewmodel);
    for (entity, transform, projection, current) in &cameras {
        let lens = match projection {
            Projection::Perspective(p) => Vec4::new(p.fov, p.aspect_ratio, p.near, p.far),
            _ => Vec4::ZERO,
        };
        let map = frame.map_or(0, |f| f.map);
        let view = (*transform, lens, map);
        if history.view != Some(view) {
            history.warm = 0;
            history.view = Some(view);
        }
        let ready = history.advance(active, scene_time, enabled);
        if !enabled {
            if current.is_some() {
                commands
                    .entity(entity)
                    .remove::<(WeaponBlur, MotionVectorPrepass, DepthPrepass)>();
            }
            continue;
        }
        let blur = WeaponBlur {
            max: if ready { 0.12 } else { 0. },
            // Ultra softens the world's camera blur; the weapon keeps its full animation blur.
            scale: 1.,
            world: false,
            camera_moving: false,
        };
        if current != Some(&blur) {
            commands.entity(entity).insert((blur, MotionVectorPrepass));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::History;

    #[test]
    fn losing_gameplay_removes_existing_weapon_blur_and_prepass() {
        use super::{ViewmodelCamera, WeaponBlur, update};
        use bevy::{
            core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass},
            prelude::*,
        };
        let mut app = App::new();
        app.add_systems(Update, update);
        let camera = app
            .world_mut()
            .spawn((
                ViewmodelCamera,
                Transform::IDENTITY,
                Projection::default(),
                WeaponBlur {
                    max: 0.12,
                    scale: 1.,
                    world: false,
                    camera_moving: false,
                },
                MotionVectorPrepass,
                DepthPrepass,
            ))
            .id();
        app.update();
        assert!(app.world().get::<WeaponBlur>(camera).is_none());
        assert!(app.world().get::<MotionVectorPrepass>(camera).is_none());
        assert!(app.world().get::<DepthPrepass>(camera).is_none());
    }

    #[test]
    fn history_warms_and_resets_on_switch_pause_disable_and_time_jump() {
        let mut h = History::default();
        assert!(!h.advance("crowbar", 1., true));
        assert!(!h.advance("crowbar", 1.015, true));
        assert!(h.advance("crowbar", 1.03, true));
        assert!(!h.advance("pistol", 1.045, true));
        assert!(!h.advance("pistol", 1.06, false));
        assert!(!h.advance("pistol", 1.075, true));
        assert!(!h.advance("pistol", 1.075, true));
        assert!(!h.advance("pistol", 0., true));
        assert!(!h.advance("pistol", 4., true));
    }
}
