//! Advanced-only world motion vectors: camera turns and opaque/masked moving models.
use super::weapon_blur::WeaponBlur;
use crate::{FlyCamera, gameplay::Gameplay, simulation::Simulation, ui_host::Console};
use bevy::{
    core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass, NoBackgroundMotionVectors},
    prelude::{Commands, Entity, Local, Projection, Query, Res, Transform, Vec4, With},
};

#[derive(Default)]
pub(super) struct History {
    map: usize,
    lens: Vec4,
    time: f64,
    origin: Option<bevy::math::Vec3>,
    warm: u8,
    view: Option<(Transform, Vec4, usize)>,
}

impl History {
    fn advance(&mut self, map: usize, lens: Vec4, time: f64, origin: bevy::math::Vec3) -> bool {
        if self.map != map
            || self.lens != lens
            || time < self.time
            || time - self.time > 0.1
            || self.origin.is_none_or(|old| old.distance(origin) > 256.)
        {
            self.warm = 0;
        }
        self.map = map;
        self.lens = lens;
        self.time = time;
        self.origin = Some(origin);
        self.warm = self.warm.saturating_add(1).min(3);
        self.warm == 3
    }
}

pub(super) fn update(
    mut commands: Commands,
    ui: Option<Res<Console>>,
    game: Option<Res<Gameplay>>,
    sim: Option<Res<Simulation>>,
    mut history: Local<History>,
    cameras: Query<(Entity, &Transform, &Projection, Option<&WeaponBlur>), With<FlyCamera>>,
) {
    let enabled = ui.is_some_and(|ui| {
        let a = &ui.source.config.video.advanced;
        a.motion_blur_mode().world()
    }) && sim.is_some_and(|sim| !sim.paused() && !sim.background_level)
        && game.is_some();
    if !enabled {
        history.warm = 0;
    }
    for (entity, transform, projection, current) in &cameras {
        if !enabled {
            if current.is_some() {
                commands.entity(entity).remove::<(
                    WeaponBlur,
                    MotionVectorPrepass,
                    DepthPrepass,
                    NoBackgroundMotionVectors,
                )>();
            }
            continue;
        }
        let game = game.as_ref().expect("enabled gameplay");
        let lens = match projection {
            Projection::Perspective(p) => Vec4::new(p.fov, p.aspect_ratio, p.near, p.far),
            _ => Vec4::ZERO,
        };
        let view = (
            *transform,
            lens,
            std::sync::Arc::as_ptr(&game.world) as usize,
        );
        let camera_moving = history.view != Some(view);
        history.view = Some(view);
        let ready = history.advance(
            std::sync::Arc::as_ptr(&game.world) as usize,
            lens,
            game.scene.time,
            transform.translation,
        );
        let blur = WeaponBlur {
            max: if ready { 0.12 } else { 0. },
            world: true,
            camera_moving,
        };
        if current != Some(&blur) {
            commands
                .entity(entity)
                .insert((blur, MotionVectorPrepass, NoBackgroundMotionVectors));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

    #[test]
    fn turns_keep_history_but_discontinuities_warm_again() {
        let mut h = History::default();
        let lens = Vec4::ONE;
        assert!(!h.advance(1, lens, 1., Vec3::ZERO));
        assert!(!h.advance(1, lens, 1.015, Vec3::X));
        assert!(h.advance(1, lens, 1.03, Vec3::X));
        assert!(!h.advance(2, lens, 1.045, Vec3::X));
        assert!(!h.advance(2, lens * 2., 1.06, Vec3::X));
        assert!(!h.advance(2, lens * 2., 1.075, Vec3::X * 1024.));
        assert!(!h.advance(2, lens * 2., 0., Vec3::X * 1024.));
    }

    #[test]
    fn missing_gameplay_removes_prepasses_and_reconstruction() {
        let mut app = App::new();
        app.add_systems(Update, update);
        let camera = app
            .world_mut()
            .spawn((
                FlyCamera,
                Transform::IDENTITY,
                Projection::default(),
                WeaponBlur {
                    max: 0.12,
                    world: true,
                    camera_moving: false,
                },
                MotionVectorPrepass,
                DepthPrepass,
                NoBackgroundMotionVectors,
            ))
            .id();
        app.update();
        assert!(app.world().get::<WeaponBlur>(camera).is_none());
        assert!(app.world().get::<MotionVectorPrepass>(camera).is_none());
        assert!(app.world().get::<DepthPrepass>(camera).is_none());
        assert!(
            app.world()
                .get::<NoBackgroundMotionVectors>(camera)
                .is_none()
        );
    }
}
