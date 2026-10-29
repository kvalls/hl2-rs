//! Image-space motion blur (Source SDK `game/client/viewpostprocess.cpp` `DoImageSpaceMotionBlur`):
//! the four blur values (yaw, pitch, falling, roll) derived each frame from how the view moved
//! since the previous frame. The host draws them with the `dev/motion_blur` shader
//! (`materialsystem/stdshaders/motion_blur_ps2x.fxc`).

use glam::Vec3;

/// The `mat_motion_blur_*` cvars the SDK reads (its defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// `mat_motion_blur_forward_enabled`: forward motion blurs too, not only falling.
    pub forward_enabled: bool,
    pub falling_min: f32,
    pub falling_max: f32,
    pub falling_intensity: f32,
    pub rotation_intensity: f32,
    pub strength: f32,
    /// `mat_motion_blur_percent_of_screen_max`: the longest blur, in percent of the screen.
    pub percent_of_screen_max: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            forward_enabled: false,
            falling_min: 10.,
            falling_max: 20.,
            falling_intensity: 1.,
            rotation_intensity: 1.,
            strength: 1.,
            percent_of_screen_max: 4.,
        }
    }
}

/// The view of one frame (`CViewSetup`): Source angles in degrees, pitch positive looking down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub origin: Vec3,
    pub pitch: f32,
    pub yaw: f32,
    /// Horizontal field of view in degrees (the 4:3 setting).
    pub fov: f32,
    /// Width over height of the frame.
    pub aspect: f32,
}

/// The values of the previous frame and the ones last produced (the SDK keeps both in statics).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionBlur {
    last_time: f32,
    origin: Vec3,
    pitch: f32,
    yaw: f32,
    no_rotation_until: f32,
    values: [f32; 4],
}
impl Default for MotionBlur {
    fn default() -> Self {
        Self {
            last_time: 0.,
            origin: Vec3::ZERO,
            pitch: 0.,
            yaw: 0.,
            no_rotation_until: 0.,
            values: [0.; 4],
        }
    }
}

/// Angle wrapped to [-180, 180].
fn wrap(mut degrees: f32) -> f32 {
    while degrees > 180. {
        degrees -= 360.;
    }
    while degrees < -180. {
        degrees += 360.;
    }
    degrees
}

impl MotionBlur {
    /// The blur values of this frame for `view` at `now` (real time in seconds): the yaw and pitch
    /// vector (fractions of the screen), the falling (zoom) intensity and the roll intensity.
    pub fn update(&mut self, view: &View, now: f32, settings: &Settings) -> [f32; 4] {
        let elapsed = now - self.last_time;
        let (pitch, yaw) = (wrap(view.pitch), wrap(view.yaw));
        let (sp, cp) = pitch.to_radians().sin_cos();
        let (sy, cy) = yaw.to_radians().sin_cos();
        let forward = Vec3::new(cp * cy, cp * sy, -sp);
        let side = Vec3::new(-sy, cy, 0.);
        let change = self.origin - view.origin;
        if change.length() > 30. && elapsed >= 0.5 {
            // Moved far after a long gap (a level start or teleport).
            self.values = [0.; 4];
        } else if elapsed > 1. / 15. {
            // Slower than 15 fps: no blur.
            self.values = [0.; 4];
        } else if change.length() > 50. {
            // Moved far in one frame (a portal): keep the values, hold the rotation blur off.
            self.no_rotation_until = now + 1.;
        } else {
            self.values = self.normal(
                view,
                (pitch, yaw),
                (forward, side, change),
                elapsed,
                settings,
            );
        }
        if now < self.no_rotation_until {
            self.values[0] = 0.;
            self.values[1] = 0.;
            self.values[3] = 0.;
        } else {
            self.no_rotation_until = 0.;
        }
        self.origin = view.origin;
        self.pitch = pitch;
        self.yaw = yaw;
        self.last_time = now;
        self.values
    }

    /// The normal update path.
    fn normal(
        &self,
        view: &View,
        (pitch, yaw): (f32, f32),
        (forward, side, change): (Vec3, Vec3, Vec3),
        elapsed: f32,
        s: &Settings,
    ) -> [f32; 4] {
        let horizontal_fov = view.fov;
        let vertical_fov = if view.aspect <= 0. {
            view.fov
        } else {
            view.fov / view.aspect
        };
        // Forward motion: only falling (looking up or down) unless forward blur is enabled.
        let view_dot_motion = forward.dot(change);
        let mut falling = if s.forward_enabled {
            view_dot_motion
        } else {
            view_dot_motion * forward.z.abs()
        };
        // Yaw, compensated for circle strafing so the adjustment only lessens the blur.
        let side_dot_motion = side.dot(change);
        let mut yaw_diff = self.yaw - yaw;
        if (self.yaw - yaw > 180. || self.yaw - yaw < -180.)
            && (self.yaw + yaw > -180. && self.yaw + yaw < 180.)
        {
            yaw_diff = self.yaw + yaw;
        }
        let yaw_adjusted = lessen(yaw_diff + side_dot_motion / 3., yaw_diff);
        let undampened_yaw = yaw_adjusted / horizontal_fov;
        let pitch_ratio = pitch.abs() / 90.;
        let mut values = [undampened_yaw * (1. - pitch_ratio), 0., 0., 0.];
        // Pitch, compensated for forward motion.
        let compensate = 1. - (1. - forward.z.abs()) * (1. - forward.z.abs());
        let pitch_diff = self.pitch - pitch;
        let pitch_adjusted = if pitch > 0. {
            pitch_diff - (view_dot_motion / 2.) * compensate
        } else {
            pitch_diff + (view_dot_motion / 2.) * compensate
        };
        values[1] = lessen(pitch_adjusted, pitch_diff) / vertical_fov;
        // Roll: the yaw blur while looking down, by pitch^3.
        values[3] = undampened_yaw * pitch_ratio * pitch_ratio * pitch_ratio;
        // Falling, adjusted to 1/30 s until the SDK does something smarter.
        falling = if elapsed > 0. {
            falling / (elapsed * 30.)
        } else {
            0.
        };
        let ramp =
            ((falling.abs() - s.falling_min) / (s.falling_max - s.falling_min)).clamp(0., 1.);
        values[2] = ramp * if falling >= 0. { 1. } else { -1. } / 30.;
        // Intensity: rotation blur never passes 15% of the range, roll 30%.
        values[0] *= s.rotation_intensity * 0.15 * s.strength;
        values[1] *= s.rotation_intensity * 0.15 * s.strength;
        values[2] *= s.falling_intensity * s.strength;
        values[3] *= 0.3 * s.strength;
        // Fade out from 50 to 30 fps.
        let fps = if elapsed > 0. { 1. / elapsed } else { 0. };
        let damp = ((fps - 30.) / (50. - 30.)).clamp(0., 1.);
        values.map(|v| v * damp)
    }
}

/// `adjusted` clamped so it only lessens `original`: between 0 and `original`.
fn lessen(adjusted: f32, original: f32) -> f32 {
    if original < 0. {
        adjusted.clamp(original, 0.)
    } else {
        adjusted.clamp(0., original)
    }
}

/// Samples of the blur by frame height (`motion_blur_dx9.cpp`): 15 from 1080 lines, 11 from 720,
/// else 7; 0 when no blur is needed this frame.
pub fn samples(frame_height: u32, values: [f32; 4]) -> u32 {
    if values.iter().map(|v| v.abs()).sum::<f32>() == 0. {
        0
    } else if frame_height >= 1080 {
        15
    } else if frame_height >= 720 {
        11
    } else {
        7
    }
}

#[cfg(test)]
mod tests;
