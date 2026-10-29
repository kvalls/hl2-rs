//! What the local player's view follows: SDK `CTriggerCamera` (`point_viewcontrol`) and
//! `CEnvZoom` (`env_zoom`). Every menu background level enables a `point_viewcontrol` from a
//! `logic_auto` and the Half-Life 2 ones lock the field of view with an `env_zoom`.
use glam::Vec3;
use modkit_core::World;

/// `SF_CAMERA_PLAYER_POSITION`: start at the player's eye (moves; not implemented).
const START_AT_PLAYER: u32 = 1;
/// `SF_CAMERA_PLAYER_TARGET`: look at the player instead of `target`.
const TARGET_PLAYER: u32 = 2;
/// `SF_CAMERA_PLAYER_TAKECONTROL`: the player is frozen (`EnableControl(FALSE)`).
const TAKE_CONTROL: u32 = 4;
/// `SF_CAMERA_PLAYER_INFINITE_WAIT`: never return to the player after `wait` seconds.
const INFINITE_WAIT: u32 = 8;

/// The view of an enabled camera, in Source units and degrees (pitch positive looks down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub origin: Vec3,
    pub pitch: f32,
    pub yaw: f32,
    /// The player takes no input while this camera is active.
    pub freeze_player: bool,
}

/// `env_zoom` `Zoom`: the player's field of view moves to `fov` over `rate` seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    pub fov: f32,
    pub rate: f32,
    pub started: f64,
}

impl Zoom {
    /// The field of view at `time`, blending from `from` (the player's own field of view).
    pub fn fov_at(&self, from: f32, time: f64) -> f32 {
        if self.rate <= 0. {
            return self.fov;
        }
        let t = (((time - self.started) as f32) / self.rate).clamp(0., 1.);
        from + (self.fov - from) * t
    }
}

#[derive(Clone, Debug, Default)]
pub struct ViewControl {
    /// The enabled `point_viewcontrol` and when it was enabled.
    camera: Option<(usize, f64)>,
    pub zoom: Option<Zoom>,
}

fn flags(world: &World, id: usize) -> u32 {
    world.entities[id]
        .get("spawnflags")
        .and_then(|f| f.trim().parse().ok())
        .unwrap_or(0)
}

/// SDK `VectorAngles`: (pitch, yaw) in degrees looking along `direction`.
fn angles_toward(direction: Vec3) -> (f32, f32) {
    let yaw = direction.y.atan2(direction.x).to_degrees();
    let pitch = (-direction.z)
        .atan2(direction.truncate().length())
        .to_degrees();
    (pitch, yaw)
}

impl ViewControl {
    /// `Enable`: this camera takes the view. Returns the camera it replaced, which fires
    /// `OnEndFollow` (SDK: the previous view control is disabled first).
    pub fn enable(&mut self, id: usize, time: f64) -> Option<usize> {
        let previous = self.camera.map(|(c, _)| c).filter(|&c| c != id);
        self.camera = Some((id, time));
        previous
    }

    /// `Disable`: returns whether `id` was the active camera (it then fires `OnEndFollow`).
    pub fn disable(&mut self, id: usize) -> bool {
        let active = self.camera.is_some_and(|(c, _)| c == id);
        if active {
            self.camera = None;
        }
        active
    }

    pub fn active(&self) -> Option<usize> {
        self.camera.map(|(c, _)| c)
    }

    /// Unsupported keys of a camera being enabled, for the diagnostics.
    pub fn unsupported(world: &World, id: usize) -> Vec<&'static str> {
        let e = &world.entities[id];
        let mut unsupported = Vec::new();
        if e.get("moveto").is_some_and(|p| !p.is_empty()) {
            unsupported.push("Enable:moveto");
        }
        if e.get("interpolatepositiontoplayer")
            .is_some_and(|v| v.trim() == "1")
        {
            unsupported.push("Enable:interpolatepositiontoplayer");
        }
        if flags(world, id) & START_AT_PLAYER != 0 {
            unsupported.push("Enable:start-at-player");
        }
        unsupported
    }

    /// The camera to return to the player from: without `INFINITE_WAIT`, a camera that
    /// follows a target gives the view back after `wait` seconds (SDK `FollowTarget`).
    pub fn expired(&self, world: &World, time: f64) -> Option<usize> {
        let (id, enabled) = self.camera?;
        let e = &world.entities[id];
        let follows =
            flags(world, id) & TARGET_PLAYER != 0 || e.get("target").is_some_and(|t| !t.is_empty());
        let wait = e
            .get("wait")
            .and_then(|w| w.trim().parse::<f64>().ok())
            .unwrap_or(0.);
        (follows && flags(world, id) & INFINITE_WAIT == 0 && time >= enabled + wait).then_some(id)
    }

    /// The active camera's view: its position, looking at its `target` (or the player with
    /// `SF_CAMERA_PLAYER_TARGET`), else along its own `angles`. `origin_of` gives an entity's
    /// current position (so parented cameras and targets move). Cameras snap to their goal
    /// angles, as every menu background camera does (`SF_CAMERA_PLAYER_SNAP_TO`); the eased
    /// turn of the other cameras is not modeled.
    pub fn view(
        &self,
        world: &World,
        origin_of: impl Fn(usize) -> Option<Vec3>,
        player_eye: Vec3,
    ) -> Option<View> {
        let (id, _) = self.camera?;
        let origin = origin_of(id)?;
        let e = &world.entities[id];
        let flags = flags(world, id);
        let target = if flags & TARGET_PLAYER != 0 {
            Some(player_eye)
        } else {
            e.get("target")
                .filter(|t| !t.is_empty())
                .and_then(|name| {
                    world.entities.iter().position(|t| {
                        t.get("targetname")
                            .is_some_and(|n| n.eq_ignore_ascii_case(name))
                    })
                })
                .and_then(&origin_of)
        };
        let (pitch, yaw) = match target {
            Some(target) => angles_toward(target - origin),
            None => {
                let angles: Vec<f32> = e
                    .get("angles")
                    .unwrap_or("0 0 0")
                    .split_whitespace()
                    .filter_map(|v| v.parse().ok())
                    .collect();
                (
                    angles.first().copied().unwrap_or(0.),
                    angles.get(1).copied().unwrap_or(0.),
                )
            }
        };
        Some(View {
            origin,
            pitch,
            yaw,
            freeze_player: flags & TAKE_CONTROL != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::Entity;

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        }
    }

    #[test]
    fn cameras_look_at_their_target_or_along_their_angles() {
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "point_viewcontrol"),
                    ("target", "view_target"),
                    ("spawnflags", "28"),
                    ("wait", "100"),
                ]),
                entity(&[("classname", "info_target"), ("targetname", "view_target")]),
                entity(&[
                    ("classname", "point_viewcontrol"),
                    ("angles", "-10 295 0"),
                    ("spawnflags", "28"),
                ]),
            ],
            ..Default::default()
        };
        let origins = [
            Vec3::ZERO,
            Vec3::new(100., 100., -141.42136),
            Vec3::new(1., 2., 3.),
        ];
        let origin_of = |id: usize| origins.get(id).copied();
        let mut control = ViewControl::default();
        assert_eq!(control.view(&world, origin_of, Vec3::ZERO), None);

        assert_eq!(control.enable(0, 1.), None);
        let view = control.view(&world, origin_of, Vec3::ZERO).unwrap();
        assert!(
            (view.yaw - 45.).abs() < 1e-3 && (view.pitch - 45.).abs() < 1e-3,
            "{view:?}"
        );
        assert!(view.freeze_player);
        // Infinite hold: never returns to the player.
        assert_eq!(control.expired(&world, 1000.), None);

        assert_eq!(control.enable(2, 2.), Some(0));
        let view = control.view(&world, origin_of, Vec3::ZERO).unwrap();
        assert_eq!(
            (view.origin, view.pitch, view.yaw),
            (origins[2], -10., 295.)
        );
        assert!(!control.disable(0));
        assert!(control.disable(2));
        assert_eq!(control.active(), None);
    }

    #[test]
    fn cameras_with_a_finite_wait_return_and_zoom_blends_over_its_rate() {
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "point_viewcontrol"),
                    ("target", "t"),
                    ("wait", "3"),
                ]),
                entity(&[("classname", "info_target"), ("targetname", "t")]),
            ],
            ..Default::default()
        };
        let mut control = ViewControl::default();
        control.enable(0, 10.);
        assert_eq!(control.expired(&world, 12.9), None);
        assert_eq!(control.expired(&world, 13.), Some(0));
        let view = control
            .view(&world, |_| Some(Vec3::ZERO), Vec3::ZERO)
            .unwrap();
        assert!(!view.freeze_player);

        let instant = Zoom {
            fov: 75.,
            rate: 0.,
            started: 0.,
        };
        assert_eq!(instant.fov_at(110., 0.), 75.);
        let eased = Zoom {
            fov: 50.,
            rate: 2.,
            started: 1.,
        };
        assert_eq!(eased.fov_at(90., 2.), 70.);
        assert_eq!(eased.fov_at(90., 9.), 50.);
    }
}
