//! Advanced camera policy and tuning, independent of the host and settings storage.
use serde::Deserialize;
use std::sync::LazyLock;
#[derive(Deserialize)]
struct Tuning {
    video_confirm_seconds: f64,
    vertical_fov_range: (f32, f32),
    viewmodel_fov_range: (f32, f32),
    world_fov_offset: Offsets,
}
#[derive(Deserialize)]
struct Offsets {
    portrait: f32,
    ultrawide: f32,
}
static TUNING: LazyLock<Tuning> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/ui/tuning.json")).expect("advanced options tuning")
});
pub fn video_confirm_seconds() -> f64 {
    TUNING.video_confirm_seconds
}
pub fn vertical_fov_range() -> (f32, f32) {
    TUNING.vertical_fov_range
}
pub fn viewmodel_fov_range() -> (f32, f32) {
    TUNING.viewmodel_fov_range
}
pub fn world_fov_offset(aspect: f32) -> f32 {
    if !aspect.is_finite() || aspect <= 0. {
        return 0.;
    }
    let ease = |t: f32| {
        let t = t.clamp(0., 1.);
        t * t * (3. - 2. * t)
    };
    if aspect < 5. / 4. {
        TUNING.world_fov_offset.portrait * ease((5. / 4. - aspect) / (5. / 4. - 9. / 16.))
    } else if aspect > 16. / 9. {
        TUNING.world_fov_offset.ultrawide * ease((aspect - 16. / 9.) / (32. / 9. - 16. / 9.))
    } else {
        0.
    }
}
/// Apply an explicit vertical FOV or the aspect-adaptive profile while preserving zoom.
pub fn vertical_fov(view: f32, default: f32, aspect: f32, vertical: f32) -> f32 {
    let derived =
        |degrees: f32| 2. * ((degrees.to_radians() / 2.).tan() / aspect.min(4. / 3.)).atan();
    let base = derived(default);
    let target = if vertical > 0. {
        vertical.to_radians()
    } else {
        let offset = world_fov_offset(aspect);
        if offset == 0. {
            return derived(view);
        }
        derived((default + offset).clamp(1., 160.))
    };
    2. * ((derived(view) / 2.).tan() * (target / 2.).tan() / (base / 2.).tan()).atan()
}
