//! Owner-requested animation blur, independent of the retail world-camera blur.
use super::weapon_blur::WeaponBlur;
use crate::{
    camera::ViewmodelCamera, gameplay::Gameplay, simulation::Simulation, ui_host::Console,
};
use bevy::{
    core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass},
    mesh::skinning::SkinnedMesh,
    prelude::{Commands, Entity, Local, Projection, Query, Res, Transform, Vec4, Visibility, With},
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
    ui: Option<Res<Console>>,
    game: Option<Res<Gameplay>>,
    sim: Option<Res<Simulation>>,
    mut history: Local<History>,
    cameras: Query<(Entity, &Transform, &Projection, Option<&WeaponBlur>), With<ViewmodelCamera>>,
    draws: Query<(&crate::rendering::WeaponMesh, &Visibility), With<SkinnedMesh>>,
) {
    let active = game.as_ref().map_or("", |g| g.inventory.active.as_str());
    let scene_time = game.as_ref().map_or(0., |g| g.scene.time);
    let enabled = ui.is_some_and(|ui| {
        let a = &ui.source.config.video.advanced;
        a.motion_blur_mode().viewmodel()
    }) && sim.is_some_and(|sim| !sim.paused() && !sim.background_level)
        && game.as_ref().is_some_and(|g| !g.inventory.carrying())
        && draws
            .iter()
            .any(|(weapon, visibility)| weapon.0 == active && *visibility != Visibility::Hidden);
    for (entity, transform, projection, current) in &cameras {
        let lens = match projection {
            Projection::Perspective(p) => Vec4::new(p.fov, p.aspect_ratio, p.near, p.far),
            _ => Vec4::ZERO,
        };
        let map = game
            .as_ref()
            .map_or(0, |g| std::sync::Arc::as_ptr(&g.world) as usize);
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
