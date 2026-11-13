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
/// `SF_CAMERA_PLAYER_SNAP_TO`: the first FollowTarget think snaps to the goal angles.
const SNAP_TO: u32 = 16;

/// A following camera's own angles between FollowTarget thinks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turn {
    camera: usize,
    enabled: f64,
    time: f64,
    pitch: f32,
    yaw: f32,
}

/// The view of an enabled camera, in Source units and degrees (pitch positive looks down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub origin: Vec3,
    pub pitch: f32,
    pub yaw: f32,
    /// The player takes no input while this camera is active.
    pub freeze_player: bool,
}

/// `env_zoom` `Zoom`: CBasePlayer::SetFOV(this, FOV, Rate) on the player's field of view,
/// from the current one (FOV 0: back to the default).
pub type Zoom = crate::weapon_crossbow::FovRamp;

#[derive(Clone, Debug, Default)]
pub struct ViewControl {
    /// The enabled `point_viewcontrol` and when it was enabled.
    camera: Option<(usize, f64)>,
    pub zoom: Option<Zoom>,
}

/// An entity's `angles` keyvalue as (pitch, yaw).
fn own_angles(e: &modkit_core::Entity) -> (f32, f32) {
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

fn flags(world: &World, id: usize) -> u32 {
    world.entities[id]
        .get("spawnflags")
        .and_then(|f| f.trim().parse().ok())
        .unwrap_or(0)
}

/// SDK `VectorAngles`: (pitch, yaw) in degrees looking along `direction`.
pub fn angles_toward(direction: Vec3) -> (f32, f32) {
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
    /// `SF_CAMERA_PLAYER_TARGET`), at the target's `targetattachment` when it names one,
    /// else along its own `angles`. `origin_of` gives an entity's current position (so
    /// parented cameras and targets move); `attachment_of` an entity's animated attachment.
    /// These are the goal angles; `follow` turns a following camera toward them.
    pub fn view(
        &self,
        world: &World,
        origin_of: impl Fn(usize) -> Option<Vec3>,
        attachment_of: impl Fn(usize, &str) -> Option<Vec3>,
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
                .and_then(|target| {
                    e.get("targetattachment")
                        .filter(|a| !a.is_empty())
                        .and_then(|a| attachment_of(target, a))
                        .or_else(|| origin_of(target))
                })
        };
        let (pitch, yaw) = match target {
            Some(target) => angles_toward(target - origin),
            None => own_angles(e),
        };
        Some(View {
            origin,
            pitch,
            yaw,
            freeze_player: flags & TAKE_CONTROL != 0,
        })
    }

    /// CTriggerCamera::FollowTarget for a camera with a target: the first think after Enable
    /// snaps to the goal with SF_CAMERA_PLAYER_SNAP_TO (else the camera starts from its own
    /// angles); every later think sets the angular velocity to the angle left x 40 x
    /// frametime, which that frame's movement applies. `turn` keeps the camera's angles;
    /// `time` is the scene time of this think.
    pub fn follow(&self, world: &World, turn: &mut Option<Turn>, goal: View, time: f64) -> View {
        let Some((id, enabled)) = self.camera else {
            *turn = None;
            return goal;
        };
        let e = &world.entities[id];
        let flags = flags(world, id);
        if flags & TARGET_PLAYER == 0 && e.get("target").is_none_or(|t| t.is_empty()) {
            *turn = None;
            return goal;
        }
        let current = match turn {
            Some(t) if t.camera == id && t.enabled == enabled => t,
            _ => {
                let (pitch, yaw) = if flags & SNAP_TO != 0 {
                    (goal.pitch, goal.yaw)
                } else {
                    own_angles(e)
                };
                turn.insert(Turn {
                    camera: id,
                    enabled,
                    time,
                    pitch,
                    yaw,
                })
            }
        };
        let dt = (time - current.time).clamp(0., 0.1) as f32;
        current.time = time;
        let left = |goal: f32, now: f32| (goal - now + 180.).rem_euclid(360.) - 180.;
        current.pitch += left(goal.pitch, current.pitch) * 40. * dt * dt;
        current.yaw += left(goal.yaw, current.yaw) * 40. * dt * dt;
        View {
            pitch: current.pitch,
            yaw: current.yaw,
            ..goal
        }
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
        assert_eq!(
            control.view(&world, origin_of, |_, _| None, Vec3::ZERO),
            None
        );

        assert_eq!(control.enable(0, 1.), None);
        let view = control
            .view(&world, origin_of, |_, _| None, Vec3::ZERO)
            .unwrap();
        assert!(
            (view.yaw - 45.).abs() < 1e-3 && (view.pitch - 45.).abs() < 1e-3,
            "{view:?}"
        );
        assert!(view.freeze_player);
        // Infinite hold: never returns to the player.
        assert_eq!(control.expired(&world, 1000.), None);

        assert_eq!(control.enable(2, 2.), Some(0));
        let view = control
            .view(&world, origin_of, |_, _| None, Vec3::ZERO)
            .unwrap();
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
            .view(&world, |_| Some(Vec3::ZERO), |_, _| None, Vec3::ZERO)
            .unwrap();
        assert!(!view.freeze_player);

        // env_zoom: an instant zoom, then eased ramps from the current FOV (SimpleSpline);
        // FOV 0 returns to the default (d1_trainstation_01's zoom_final).
        let mut zoom = Zoom::default();
        zoom.set(75, 0., 0.);
        assert_eq!(zoom.get(0.), 75.);
        zoom.set(50, 2., 1.);
        assert_eq!(zoom.get(2.), 62.5);
        assert_eq!(zoom.get(9.), 50.);
        zoom.default_fov = 90.;
        zoom.set(0, 14., 10.);
        assert_eq!(zoom.get(17.), 70.);
        assert_eq!(zoom.get(30.), 90.);
    }

    #[test]
    fn following_cameras_snap_once_then_ease_toward_the_goal() {
        let camera = |flags: &str| {
            entity(&[
                ("classname", "point_viewcontrol"),
                ("target", "gman"),
                ("spawnflags", flags),
                ("angles", "0 90 0"),
            ])
        };
        let world = World {
            entities: vec![camera("28"), camera("12")],
            ..Default::default()
        };
        let goal = |yaw: f32| View {
            origin: Vec3::ZERO,
            pitch: 0.,
            yaw,
            freeze_player: true,
        };
        let mut control = ViewControl::default();
        control.enable(0, 1.);
        let mut turn = None;
        // SF_CAMERA_PLAYER_SNAP_TO: the first think takes the goal.
        assert_eq!(control.follow(&world, &mut turn, goal(30.), 1.).yaw, 30.);
        // Then 40 x frametime of the angle left per second, for one 15 ms frame.
        let eased = control.follow(&world, &mut turn, goal(40.), 1.015).yaw;
        assert!(
            (eased - (30. + 10. * 40. * 0.015 * 0.015)).abs() < 1e-4,
            "{eased}"
        );
        // Across 360: the short way round.
        let mut turn = None;
        control.follow(&world, &mut turn, goal(350.), 2.);
        assert!(control.follow(&world, &mut turn, goal(10.), 2.015).yaw > 350.);
        // Without SNAP_TO the camera starts from its own angles.
        control.enable(1, 3.);
        assert_eq!(control.follow(&world, &mut turn, goal(30.), 3.).yaw, 90.);
    }
}
