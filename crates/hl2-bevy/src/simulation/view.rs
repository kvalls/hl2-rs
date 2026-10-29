//! Resolve authored cameras without advancing the fixed simulation clock.
use super::Simulation;
use crate::gameplay::Gameplay;

impl Simulation {
    pub(crate) fn prepare_background_view(&mut self, game: &mut Gameplay) {
        game.scene.tick(&game.world, self.player.feet, 0.);
        self.refresh_scripted_view(game);
    }

    pub(crate) fn refresh_scripted_view(&mut self, game: &Gameplay) {
        let states = &game.scene.states;
        self.view_control = game
            .scene
            .view_control
            .view(
                &game.world,
                |id| states.get(id).filter(|s| !s.killed).map(|s| s.origin),
                self.eye,
            )
            .or_else(|| game.scene.commentary.view_control_override());
        self.zoom_fov = game
            .scene
            .view_control
            .zoom
            .map(|zoom| zoom.fov_at(self.fov, game.scene.time));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::{Entity, World};

    fn background(delay: f32) -> (Simulation, Gameplay) {
        let entity = |pairs: &[(&str, &str)]| Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        };
        let output = format!("menu_camera,Enable,,{delay},-1");
        let world = World {
            entities: vec![
                entity(&[("classname", "logic_auto"), ("OnMapSpawn", &output)]),
                entity(&[
                    ("classname", "point_viewcontrol"),
                    ("targetname", "menu_camera"),
                    ("origin", "100 200 300"),
                    ("angles", "0 90 0"),
                ]),
            ],
            ..Default::default()
        };
        let sim = Simulation::new(&world, glam::Vec3::Z * 64., 0., 0., false, vec![]);
        (sim, Gameplay::synthetic(world))
    }

    #[test]
    fn initial_background_frame_uses_map_spawn_camera_without_advancing_clocks() {
        let (mut sim, mut game) = background(0.);
        sim.refresh_scripted_view(&game);
        assert!(sim.view_control.is_none(), "spawn I/O has not run yet");
        sim.prepare_background_view(&mut game);
        assert_eq!(sim.view_origin().to_array(), [100., 200., 300.]);
        assert_eq!(sim.view_angles().0, 90f32.to_radians());
        assert_eq!(game.scene.time, 0.);
        assert_eq!(sim.player.ticks, 0);
        assert_eq!(sim.host_tick, 0);
        sim.prepare_background_view(&mut game);
        assert_eq!(sim.view_origin().to_array(), [100., 200., 300.]);
    }

    #[test]
    fn background_projection_is_stable_across_player_fov_initialization() {
        for fov in [75., 87.3083, 90.] {
            let (mut sim, mut game) = background(0.);
            sim.fov = fov;
            sim.prepare_background_view(&mut game);
            let projection = |game: &Gameplay| {
                crate::camera::world_fov(
                    game.inventory.view_fov(game.scene.time),
                    fov,
                    16. / 9.,
                    90.,
                    true,
                )
            };
            let first = projection(&game);
            game.inventory.fov.default_fov = sim.fov;
            assert_eq!(projection(&game), first);
            assert_eq!(first, crate::video::vertical_fov_for(75., 4. / 3.));
            assert_eq!(game.scene.time, 0.);
            assert_eq!(sim.host_tick, 0);
        }
    }

    #[test]
    fn background_preparation_preserves_authored_delays() {
        let (mut sim, mut game) = background(0.1);
        sim.prepare_background_view(&mut game);
        assert!(sim.view_control.is_none());
        assert_eq!(game.scene.time, 0.);
    }
}
