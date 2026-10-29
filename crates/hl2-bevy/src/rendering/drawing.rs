//! Startup-only drawing selection; automatic direct mode is limited to measured hardware.
use bevy::{
    prelude::{Camera3d, World},
    render::{renderer::RenderAdapterInfo, view::NoIndirectDrawing},
};

fn selected(macos: bool, adapter: &str, metal: bool, requested: Option<&str>) -> bool {
    match requested {
        Some("1") => true,
        Some("0") => false,
        _ => macos && metal && adapter == "Apple M1 Pro",
    }
}
fn requested<'a>(
    profile: bool,
    general: Option<&'a str>,
    profiling: Option<&'a str>,
) -> Option<&'a str> {
    if profile {
        profiling.or(general)
    } else {
        general
    }
}

pub(crate) fn configure(world: &mut World) {
    let profile = world
        .get_resource::<crate::Options>()
        .is_some_and(|o| o.profile && o.background);
    let general = std::env::var("HL2RS_DIRECT_DRAW").ok();
    let profiling = profile
        .then(|| std::env::var("HL2RS_PROFILE_DIRECT_DRAW").ok())
        .flatten();
    let adapter = world.get_resource::<RenderAdapterInfo>();
    let direct = selected(
        cfg!(target_os = "macos"),
        adapter.map_or("", |a| a.name.as_str()),
        adapter.is_some_and(|a| format!("{:?}", a.backend) == "Metal"),
        requested(profile, general.as_deref(), profiling.as_deref()),
    );
    if direct {
        world.register_required_components::<Camera3d, NoIndirectDrawing>();
    }
    if let Some(performance) = world.get_resource::<crate::performance::Performance>() {
        performance.set_camera_drawing(direct);
    }
    bevy::log::info!("camera indirect drawing disabled: {direct}");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_policy_is_limited_to_the_measured_metal_adapter() {
        assert!(selected(true, "Apple M1 Pro", true, None));
        assert!(!selected(false, "Apple M1 Pro", true, None));
        assert!(!selected(true, "Apple M1 Pro", false, None));
        for name in ["", "Apple M2", "Intel GPU"] {
            assert!(!selected(true, name, true, None));
        }
    }
    #[test]
    fn startup_overrides_are_explicit_and_profile_specific() {
        assert!(selected(false, "unknown", false, Some("1")));
        assert!(!selected(true, "Apple M1 Pro", true, Some("0")));
        assert_eq!(requested(false, None, Some("1")), None);
        assert_eq!(requested(false, Some("0"), Some("1")), Some("0"));
        assert_eq!(requested(true, Some("0"), Some("1")), Some("1"));
        assert_eq!(requested(true, Some("0"), None), Some("0"));
    }
}
