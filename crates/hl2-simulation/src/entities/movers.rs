//! Doors and linear movers advancing each tick: the delayed close and the travel toward the
//! target fraction.

use super::Scene;
use glam::Quat;
use modkit_core::World;

/// Classes whose pose advances every tick: doors, linear movers and trains.
pub(super) fn moves(class: &str) -> bool {
    matches!(
        class,
        "func_door"
            | "func_door_rotating"
            | "prop_door_rotating"
            | "func_movelinear"
            | "func_tracktrain"
            | "func_rotating"
    )
}

/// Doors and linear movers whose travel is an offset from their closed pose: a parented one
/// travels in its parent's frame (SDK CBaseDoor::Spawn keeps local origins).
pub(super) fn travels(class: &str) -> bool {
    matches!(
        class,
        "func_door" | "func_door_rotating" | "func_movelinear"
    )
}

impl Scene {
    /// func_trains move; corners they reach fire OnPass (path_corner InPass).
    pub(super) fn advance_func_trains(&mut self, world: &World) {
        let mut passed = Vec::new();
        let states = &mut self.states;
        self.func_trains.advance(
            world,
            self.time,
            |id, origin| states[id].origin = origin,
            &mut passed,
        );
        for corner in passed {
            self.fire(corner, "OnPass", usize::MAX);
        }
    }

    /// A func_rotating's state, spawned from its keyvalues on first use.
    pub(super) fn rotating_state(
        &mut self,
        world: &World,
        id: usize,
    ) -> &mut crate::rotating::Rotating {
        let time = self.time;
        self.trains.rotating.entry(id).or_insert_with(|| {
            let e = &world.entities[id];
            let number = |key: &str| super::number(e, key, 0.);
            let angles = modkit_core::parse_vec3(e.get("angles").unwrap_or("0 0 0"));
            crate::rotating::Rotating::spawn(
                number("spawnflags") as u32,
                number("maxspeed"),
                number("fanfriction"),
                angles.unwrap_or_default(),
                time,
            )
        })
    }

    /// One tick of a `func_door`, `func_door_rotating`, `prop_door_rotating` or
    /// `func_movelinear`: a door that waited long enough closes, the pose follows the target.
    pub(super) fn advance_mover(&mut self, world: &World, id: usize, dt: f32) {
        match world.entities[id].class() {
            "func_tracktrain" => return self.advance_train(world, id, dt),
            "func_rotating" => {
                let time = self.time;
                let rotating = self.rotating_state(world, id);
                rotating.advance(time, dt);
                let angles = rotating.angles;
                self.states[id].rotation = crate::physics::angles(angles);
                return;
            }
            _ => {}
        }
        if self.states[id].return_at.is_some_and(|t| t <= self.time) {
            self.send(id, "Close", "");
            self.states[id].return_at = None;
        }
        let s = &mut self.states[id];
        if (s.target - s.fraction).abs() > 0.00001 {
            s.fraction += (s.target - s.fraction).clamp(-s.rate * dt, s.rate * dt);
            s.origin = s.base_origin + s.translation * s.fraction;
            s.rotation = s.base_rotation * Quat::from_axis_angle(s.axis, s.angle * s.fraction);
            if (s.target - s.fraction).abs() < 0.00001 {
                let open = s.target > 0.;
                if open && s.wait >= 0. {
                    s.return_at = Some(self.time + s.wait);
                }
                self.diagnostics.door_completions += 1;
                self.fire(
                    id,
                    if open { "OnFullyOpen" } else { "OnFullyClosed" },
                    usize::MAX,
                );
            }
        }
    }
}
