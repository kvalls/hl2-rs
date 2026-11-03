//! CNPC_BaseZombie's physics-object swats: FindNearestPhysicsObject's choice, the swat
//! activity and AE_ZOMBIE_SWATITEM's throw.
use glam::Vec3;

/// ZOMBIE_MAX_PHYSOBJ_MASS: the heaviest object a zombie swats (kg).
pub const MAX_PHYSOBJ_MASS: f32 = 60.;
/// ZOMBIE_PHYSOBJ_SWATDIST: swat from this close (UTIL_DistApprox2D to the object center).
pub const PHYSOBJ_SWATDIST: f32 = 80.;
/// ZOMBIE_PLAYER_MAX_SWAT_DIST: no swats while the enemy is farther than this.
pub const PLAYER_MAX_SWAT_DIST: f32 = 1000.;
/// ZOMBIE_FARTHEST_PHYSICS_OBJECT (40 feet); the search box is half of it at most.
pub const FARTHEST_PHYSICS_OBJECT: f32 = 480.;
/// ZOMBIE_SWAT_DELAY: no new object scan this long after a swat.
pub const SWAT_DELAY: f64 = 5.;
/// CNPC_BaseZombie::SelectFailSchedule swats an obstructor lighter than this (kg).
pub const ATTACKITEM_MAX_MASS: f32 = 100.;
/// ClawAttack's push on the swatted obstructor (5 x 24, the damage force).
pub const ATTACKITEM_FORCE: f32 = 120.;
/// GatherConditions rescans this often while it has no object.
pub const SWAT_SCAN_INTERVAL: f64 = 2.;
pub const AE_SWATITEM: &str = "AE_ZOMBIE_SWATITEM";
/// A VPhysics object near a zombie (CZombieSwatEntitiesEnum already required a body <= the
/// mass limit, asleep and moveable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwatCandidate {
    pub entity: usize,
    /// WorldSpaceCenter.
    pub center: Vec3,
    /// BoundingRadius: half the diagonal of its collision bounds.
    pub radius: f32,
    /// The top of its first solid's world AABB (CollideGetAABB).
    pub top: f32,
    /// FVisible from the zombie's eyes.
    pub visible: bool,
    /// physics_prop_ragdoll / prop_ragdoll (never swatted).
    pub ragdoll: bool,
}
/// UTIL_DistApprox2D: |dx| + |dy|.
pub fn dist_approx_2d(a: Vec3, b: Vec3) -> f32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}
/// The search box half extents around the zombie (UTIL_EntitiesInBox in FindNearestPhysicsObject).
pub fn swat_search_box(origin: Vec3, enemy: Vec3, hull_height: f32) -> Option<Vec3> {
    let dist = (enemy - origin).length();
    (dist <= PLAYER_MAX_SWAT_DIST).then(|| {
        let near = dist.min(FARTHEST_PHYSICS_OBJECT * 0.5);
        Vec3::new(near, near, hull_height * 2.)
    })
}
/// CNPC_BaseZombie::FindNearestPhysicsObject's choice among the objects in the box: nearest
/// (UTIL_DistApprox2D) below min(enemy distance, 240), toward the enemy (2D dot >= 0.8), nearer
/// the zombie than the enemy, reaching above the knees, center below the eyes, visible, not a
/// ragdoll (HL2 is not hl2_episodic: the enemy need not see it).
pub fn find_nearest_physics_object(
    origin: Vec3,
    enemy: Vec3,
    knees_z: f32,
    eye_z: f32,
    candidates: &[SwatCandidate],
) -> Option<usize> {
    let to_enemy = enemy - origin;
    let dist = to_enemy.length();
    if dist > PLAYER_MAX_SWAT_DIST {
        return None;
    }
    let dir_enemy = (to_enemy / dist.max(1e-6)).truncate().extend(0.);
    let mut nearest = dist.min(FARTHEST_PHYSICS_OBJECT * 0.5);
    let mut best = None;
    for c in candidates {
        let d = dist_approx_2d(origin, c.center);
        if d >= nearest {
            continue;
        }
        let dir_object = (c.center - origin)
            .normalize_or_zero()
            .truncate()
            .extend(0.);
        if dir_enemy.dot(dir_object) < 0.8 {
            continue;
        }
        if d >= dist_approx_2d(c.center, enemy) {
            continue;
        }
        if c.center.z + c.radius < knees_z || c.center.z > eye_z || c.top < knees_z {
            continue;
        }
        if !c.visible || c.ragdoll {
            continue;
        }
        best = Some(c.entity);
        nearest = d;
    }
    best
}
/// CNPC_BaseZombie::GetSwatActivity: the arm on the object's side of the facing (yaw in
/// degrees), MID when its center is above the zombie's center, else LOW.
pub fn swat_activity(
    yaw: f32,
    origin: Vec3,
    object_origin: Vec3,
    center_z: f32,
    object_center_z: f32,
) -> &'static str {
    let y = yaw.to_radians();
    let right = Vec3::new(y.sin(), -y.cos(), 0.);
    let to = (object_origin - origin)
        .normalize_or_zero()
        .truncate()
        .extend(0.);
    let mid = center_z - object_center_z < 0.;
    match (right.dot(to) >= 0., mid) {
        (true, true) => "ACT_ZOM_SWATRIGHTMID",
        (true, false) => "ACT_ZOM_SWATRIGHTLOW",
        (false, true) => "ACT_ZOM_SWATLEFTMID",
        (false, false) => "ACT_ZOM_SWATLEFTLOW",
    }
}
/// AE_ZOMBIE_SWATITEM's AddVelocity: 800 in/s from the object center toward the enemy center
/// plus 200 up, and the local angular velocity (deg/s) from two rolls in [0, 1).
pub fn swat_velocity(object_center: Vec3, enemy_center: Vec3, rolls: (f32, f32)) -> (Vec3, Vec3) {
    let v = (enemy_center - object_center).normalize_or_zero() * 800. + Vec3::Z * 200.;
    (
        v,
        Vec3::new(-180. + 360. * rolls.0, 20., -360. + 720. * rolls.1),
    )
}
