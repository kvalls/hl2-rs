//! Doors and linear movers advancing each tick: the delayed close and the travel toward the
//! target fraction.

use super::Scene;
use glam::Quat;

impl Scene {
    /// One tick of a `func_door`, `func_door_rotating`, `prop_door_rotating` or
    /// `func_movelinear`: a door that waited long enough closes, the pose follows the target.
    pub(super) fn advance_mover(&mut self, id: usize, dt: f32) {
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
