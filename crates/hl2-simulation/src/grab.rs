//! CGrabController (SDK weapon_physcannon.cpp), shared by the gravity gun and the +USE
//! carry (CPlayerPickupController). The VPhysics shadow controller is in vphysics.dll,
//! so Simulate is a velocity-to-target approximation ("native fit needed").
use crate::physics::Physics;
use glam::{Quat, Vec3};
use rapier3d::prelude::*;
use serde::Serialize;

/// CGrabController: maxSpeed 1000, DEFAULT_MAX_ANGULAR 360 x 10 deg/s.
pub(crate) const HOLD_MAX_SPEED: f32 = 1000.;
const HOLD_MAX_ANGULAR: f32 = 3600.;
/// UpdateObject's flError from both callers.
pub(crate) const HOLD_MAX_ERROR: f32 = 12.;
/// Player hull radius in 2D (OBBMaxs 16,16 -> Length2D).
pub(crate) const PLAYER_RADIUS: f32 = 22.627417;
/// hl2_normspeed: a drop clamps linear speed to 1.5x this.
const NORM_SPEED: f32 = 190.;
/// AttachEntity's rotational damping while held.
const HELD_ROT_DAMPING: f32 = 10.;
/// REDUCED_CARRY_MASS: the controlled body's mass while held (retail attach 0x10415e20).
const REDUCED_CARRY_MASS: f32 = 1.;
/// Inches to Rapier metres (physics.rs SCALE).
pub(crate) const SCALE: f32 = 1. / 39.37;
/// DOT_30DEGREE (player.h): the carry's angle alignment.
pub const DOT_30DEGREE: f32 = 0.866_025_4;

pub(crate) fn body_mass(physics: &Physics, id: usize) -> Option<f32> {
    let handle = physics.dynamic.get(&id)?;
    Some(physics.bodies.get(*handle)?.mass())
}
pub(crate) fn center(physics: &Physics, id: usize) -> Option<Vec3> {
    let body = physics.bodies.get(*physics.dynamic.get(&id)?)?;
    let c = body.center_of_mass();
    Some(Vec3::new(c.x, c.y, c.z) / SCALE)
}
pub(crate) fn rotation(physics: &Physics, id: usize) -> Option<Quat> {
    let body = physics.bodies.get(*physics.dynamic.get(&id)?)?;
    let q = body.rotation();
    Some(Quat::from_xyzw(q.i, q.j, q.k, q.w))
}
/// Half extents of the body's colliders (world AABB), in inches.
pub(crate) fn half_extents(physics: &Physics, id: usize) -> Vec3 {
    let Some(body) = physics
        .dynamic
        .get(&id)
        .and_then(|h| physics.bodies.get(*h))
    else {
        return Vec3::ZERO;
    };
    let mut aabb: Option<Aabb> = None;
    for handle in body.colliders() {
        if let Some(collider) = physics.colliders.get(*handle) {
            let next = collider.compute_aabb();
            aabb = Some(aabb.map_or(next, |a| a.merged(&next)));
        }
    }
    aabb.map_or(Vec3::ZERO, |a| {
        let h = a.half_extents();
        Vec3::new(h.x, h.y, h.z) / SCALE
    })
}
/// The eye frame (yaw, then pitch); `ignore_pitch` drops the pitch like
/// TransformAnglesFromPlayerSpace with m_bIgnoreRelativePitch.
pub(crate) fn eye_rotation(forward: Vec3, ignore_pitch: bool) -> Quat {
    let yaw = forward.y.atan2(forward.x);
    if ignore_pitch {
        return Quat::from_rotation_z(yaw);
    }
    let pitch = (-forward.z).atan2(forward.truncate().length());
    Quat::from_rotation_z(yaw) * Quat::from_rotation_y(pitch)
}
/// AlignAngles: snap each axis (z first) of the player-space rotation that lies within
/// acos(`cosine`) of a player axis onto it, re-orthogonalizing the rest.
pub fn align(rotation: Quat, cosine: f32) -> Quat {
    let mut columns = [rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z];
    for j in (0..3).rev() {
        let v = columns[j];
        for i in 0..3 {
            if v[i].abs() > cosine {
                let mut snapped = Vec3::ZERO;
                snapped[i] = v[i].signum();
                columns[j] = snapped;
                // MatrixOrthogonalize(matrix, j) in weapon_physcannon.cpp.
                let (i1, i2) = ((j + 1) % 3, (j + 2) % 3);
                columns[i2] = columns[j].cross(columns[i1]).normalize_or_zero();
                columns[i1] = columns[i2].cross(columns[j]).normalize_or_zero();
                break;
            }
        }
    }
    let matrix = glam::Mat3::from_cols(columns[0], columns[1], columns[2]);
    if (matrix.determinant() - 1.).abs() > 1e-3 {
        // A degenerate column (snapped onto its neighbour) keeps the input.
        return rotation;
    }
    Quat::from_mat3(&matrix).normalize()
}

/// A QAngle (pitch, yaw, roll in degrees) as a rotation (AngleMatrix: yaw about z,
/// then pitch about y with positive pitch down, then roll about x).
pub fn source_angles(angles: [f32; 3]) -> Quat {
    let [p, y, r] = angles.map(f32::to_radians);
    Quat::from_rotation_z(y) * Quat::from_rotation_y(p) * Quat::from_rotation_x(r)
}
/// A carried prop's fixed orientation: Pickup_GetPreferredCarryAngles (a spin_zaxis
/// prop: CPhysicsProp m_preferredCarryAngles (-5, 0, 0) in the player's entity frame,
/// pitch and yaw) or CGrabController's m_vecPreferredCarryAngles (the model's
/// physgun_interactions preferred_carryangles in ComputePlayerMatrix's frame, pitch 0;
/// applied last, so it wins).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum CarryAngles {
    Eye(Quat),
    Level(Quat),
}
impl CarryAngles {
    pub fn for_entity(world: &modkit_core::World, entity: usize) -> Option<Self> {
        if !world
            .entities
            .get(entity)
            .is_some_and(|e| e.class().starts_with("prop_physics"))
        {
            return None;
        }
        let data = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(entity))
            .and_then(|i| world.model_prop_data.get(&i.asset_key()))?;
        if let Some(angles) = data.carry_angles {
            return Some(Self::Level(source_angles(angles)));
        }
        data.spin_z.then(|| Self::Eye(source_angles([-5., 0., 0.])))
    }
    fn target(self, forward: Vec3) -> Quat {
        match self {
            Self::Eye(q) => eye_rotation(forward, false) * q,
            Self::Level(q) => eye_rotation(forward, true) * q,
        }
    }
}

/// CGrabController state for one attached prop.
#[derive(Clone, Debug, Serialize)]
pub struct GrabController {
    pub entity: usize,
    /// Object rotation in player space (m_attachedAnglesPlayerSpace).
    relative: Quat,
    /// m_flLoadWeight / m_savedMass.
    pub saved_mass: f32,
    saved_gravity: f32,
    saved_rot_damping: f32,
    /// The body's mass properties before the carry-mass reduction (authored bodies).
    #[serde(skip)]
    saved_props: Option<MassProperties>,
    /// m_bIgnoreRelativePitch (the +USE carry).
    ignore_pitch: bool,
    pub error: f32,
    error_time: f32,
    pub target: Vec3,
    target_rotation: Quat,
    /// The prop's preferred carry angles, which replace the kept player-space rotation.
    pub carry_angles: Option<CarryAngles>,
}
impl GrabController {
    /// AttachEntity: no gravity (shadow control), rotational damping 10, the player-space
    /// rotation kept (aligned within `alignment` when set), error after 1 s. Retail
    /// (0x10415e20) also sets the controlled body's mass to REDUCED_CARRY_MASS and
    /// disables its drag; the mass reduction applies to authored bodies (mass from
    /// additional mass properties), not to legacy density-mass hulls.
    pub fn attach(
        physics: &mut Physics,
        id: usize,
        forward: Vec3,
        ignore_pitch: bool,
        alignment: Option<f32>,
    ) -> Option<Self> {
        let handle = physics.dynamic.get(&id).copied()?;
        // CPlayerPickupController::Init / Physgun_OnPhysGunPickup: held debris becomes
        // interactive debris.
        physics.make_interactive_debris(id);
        let current = rotation(physics, id).unwrap_or_default();
        let target = center(physics, id)?;
        let authored = physics.bodies.get(handle)?.colliders().iter().all(|c| {
            physics
                .colliders
                .get(*c)
                .is_none_or(|collider| collider.density() == 0.)
        });
        physics.drag_disabled.insert(handle);
        let body = physics.bodies.get_mut(handle)?;
        let saved_mass = body.mass();
        let saved_props = authored.then(|| body.mass_properties().local_mprops);
        if let Some(props) = saved_props.filter(|p| p.inv_mass > 0.) {
            // Scale mass and inertia together so the shape's inertia stays consistent.
            let ratio = (1. / props.inv_mass) / REDUCED_CARRY_MASS;
            let mut reduced = props;
            reduced.inv_mass = 1. / REDUCED_CARRY_MASS;
            reduced.inv_principal_inertia_sqrt *= ratio.sqrt();
            body.set_additional_mass_properties(reduced, true);
        }
        let mut relative = eye_rotation(forward, ignore_pitch).inverse() * current;
        if let Some(cosine) = alignment {
            relative = align(relative, cosine);
        }
        let grab = Self {
            entity: id,
            relative,
            saved_mass,
            saved_gravity: body.gravity_scale(),
            saved_rot_damping: body.angular_damping(),
            saved_props,
            ignore_pitch,
            error: 0.,
            // m_errorTime = -1: one second before error accumulates.
            error_time: -1.,
            target,
            target_rotation: current,
            carry_angles: None,
        };
        body.set_gravity_scale(0., true);
        body.set_angular_damping(HELD_ROT_DAMPING);
        body.wake_up(true);
        Some(grab)
    }
    /// DetachEntity: restore gravity, mass and damping, enable drag (retail 0x104172d0
    /// forces it on rather than restoring it); clear the velocity (`clear`) or clamp it
    /// to 1.5 x hl2_normspeed and 720 deg/s.
    pub fn detach(&self, physics: &mut Physics, clear: bool) {
        let Some(&handle) = physics.dynamic.get(&self.entity) else {
            return;
        };
        physics.drag_disabled.remove(&handle);
        let colliders = &physics.colliders;
        let Some(body) = physics.bodies.get_mut(handle) else {
            return;
        };
        if let Some(props) = self.saved_props {
            body.set_additional_mass_properties(props, true);
            // Apply the restored mass now: Rapier defers it to the next step, and the drag
            // controller runs before that step; with the 1 kg carry mass a launched pallet
            // took 20x its drag and stopped dead (owner report, session 13).
            body.recompute_mass_properties_from_colliders(colliders);
        }
        body.set_gravity_scale(self.saved_gravity, true);
        body.set_angular_damping(self.saved_rot_damping);
        if clear {
            body.set_linvel(vector![0., 0., 0.], true);
            body.set_angvel(vector![0., 0., 0.], true);
            return;
        }
        let v = *body.linvel() / SCALE;
        let limit = NORM_SPEED * 1.5;
        if v.norm() > limit {
            body.set_linvel(v.normalize() * limit * SCALE, true);
        }
        let w = *body.angvel();
        let limit = 720f32.to_radians();
        if w.norm() > limit {
            body.set_angvel(w.normalize() * limit, true);
        }
    }
    /// UpdateObject + Simulate: ComputeError, then the target in front of the eye
    /// (radius from the player and object extents, pulled in by a brush trace, pushed
    /// off the player's vertical axis) and a velocity toward it (max 1000 u/s,
    /// 3600 deg/s). False when the error exceeds `max_error` (the caller lets go).
    /// SDK note: AngleVectors runs before the +/-75 pitch clamp, so the aim is not
    /// clamped (the clamp is a dead store in UpdateObject).
    pub fn update(
        &mut self,
        physics: &mut Physics,
        eye: Vec3,
        forward: Vec3,
        feet: Vec3,
        crouched: bool,
        max_error: f32,
    ) -> bool {
        let id = self.entity;
        let (Some(position), Some(current)) = (center(physics, id), rotation(physics, id)) else {
            return false;
        };
        if self.error_time > 0. {
            self.error_time = self.error_time.min(1.);
            let mut error = (self.target - position).length();
            if error / self.error_time > HOLD_MAX_SPEED {
                error *= 0.5;
            }
            self.error = (1. - self.error_time) * self.error + error * self.error_time;
            self.error_time = 0.;
        }
        if self.error > max_error {
            return false;
        }
        let aim = forward.normalize_or(Vec3::X);
        let extents = half_extents(physics, id);
        let radial =
            (aim.x.abs() * extents.x + aim.y.abs() * extents.y + aim.z.abs() * extents.z).abs();
        let radius = PLAYER_RADIUS + radial;
        let distance = 24. + radius * 2.;
        // MASK_SOLID_BRUSHONLY approximated by a ray against world geometry.
        let fraction = physics
            .projectile_ray(eye, eye + aim * distance, &[id])
            .filter(|hit| hit.entity == usize::MAX)
            .map_or(1., |hit| eye.distance(hit.position) / distance);
        let mut end = if fraction < 0.5 {
            eye + aim * radius * 0.5
        } else {
            eye + aim * (distance - radius)
        };
        let height = if crouched { 36. } else { 72. };
        let nearest = Vec3::new(feet.x, feet.y, end.z.clamp(feet.z, feet.z + height));
        let delta = end - nearest;
        if delta.length() < radius {
            end = nearest + delta.normalize_or(aim) * radius;
        }
        self.target = end;
        self.target_rotation = match self.carry_angles {
            Some(preferred) => preferred.target(forward),
            None => eye_rotation(forward, self.ignore_pitch) * self.relative,
        };
        let dt = 0.015;
        let mut v = (self.target - position) / dt;
        if v.length() > HOLD_MAX_SPEED {
            v = v.normalize() * HOLD_MAX_SPEED;
        }
        let (axis, mut angle) = (self.target_rotation * current.inverse()).to_axis_angle();
        if angle > std::f32::consts::PI {
            angle -= std::f32::consts::TAU;
        }
        let w = (axis * angle / dt).clamp_length_max(HOLD_MAX_ANGULAR.to_radians());
        if let Some(body) = physics.bodies.get_mut(physics.dynamic[&id]) {
            let v = v * SCALE;
            body.set_linvel(vector![v.x, v.y, v.z], true);
            body.set_angvel(vector![w.x, w.y, w.z], true);
        }
        self.error_time += dt;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_spinning_blade_is_held_level_with_the_view_tilted_five_degrees_up() {
        let level = CarryAngles::Eye(source_angles([-5., 0., 0.]));
        // Looking along +y: the blade's z axis stays near up, tilted back 5 degrees, and
        // its x axis points where the player looks.
        let held = level.target(Vec3::Y);
        assert!((held * Vec3::X).angle_between(Vec3::new(0., 1., 0.)) < 5.1f32.to_radians());
        assert!((held * Vec3::Z).angle_between(Vec3::Z) < 5.1f32.to_radians());
        assert!((held * Vec3::X).z > 0., "pitch -5 is up");
        // Looking 45 degrees down tilts it with the view (the player's entity angles
        // carry the pitch); the level frame does not.
        let down = Vec3::new(1., 0., -1.).normalize();
        assert!(
            ((level.target(down) * Vec3::Z).angle_between(Vec3::Z) - 40f32.to_radians()).abs()
                < 1e-3
        );
        let flat = CarryAngles::Level(Quat::IDENTITY).target(down);
        assert!((flat * Vec3::Z).angle_between(Vec3::Z) < 1e-4);
    }
    #[test]
    fn alignment_snaps_axes_within_thirty_degrees_and_keeps_others() {
        let near = Quat::from_rotation_z(20f32.to_radians());
        let snapped = align(near, DOT_30DEGREE);
        assert!(snapped.angle_between(Quat::IDENTITY) < 1e-3, "{snapped}");
        let far = Quat::from_rotation_z(45f32.to_radians());
        let kept = align(far, DOT_30DEGREE);
        // z stays up (aligned), the 45 degree yaw cannot snap.
        assert!((kept * Vec3::Z).distance(Vec3::Z) < 1e-3);
        assert!(kept.angle_between(far) < 1e-3, "{kept}");
    }
}
