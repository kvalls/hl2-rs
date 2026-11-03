//! Video playback host resolution and console effect execution.

use std::path::{Path, PathBuf};

/// Normalizes a video path:
/// - Replaces backslashes with forward slashes.
/// - Converts `.avi` or unadorned extensions to `.bik`.
/// - Prepends `media/` if not present.
pub fn normalize_video_path(filename: &str) -> String {
    let trimmed = filename.trim().replace('\\', "/");
    let without_known_ext = if let Some(base) = trimmed
        .strip_suffix(".avi")
        .or_else(|| trimmed.strip_suffix(".AVI"))
    {
        base
    } else if let Some(base) = trimmed
        .strip_suffix(".bik")
        .or_else(|| trimmed.strip_suffix(".BIK"))
    {
        base
    } else {
        &trimmed
    };
    let with_ext = format!("{without_known_ext}.bik");
    if with_ext.to_ascii_lowercase().starts_with("media/") {
        format!("media/{}", &with_ext[6..])
    } else {
        format!("media/{with_ext}")
    }
}

/// Reads the first video path from `media/StartupVids.txt` if present in the game installation.
pub fn startup_video(root: &Path) -> Option<String> {
    let candidates = [
        root.join("hl2_complete/media/StartupVids.txt"),
        root.join("hl2/media/StartupVids.txt"),
        root.join("ep2/media/StartupVids.txt"),
        root.join("episodic/media/StartupVids.txt"),
        root.join("media/StartupVids.txt"),
    ];
    for path in candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with("//") && !trimmed.starts_with('#') {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

/// Plays the startup video sequence if appropriate for interactive launch without `--novid`.
pub fn startup_video_sequence(
    options: &crate::Options,
    is_background_level: bool,
    console_ui: &mut crate::ui_host::Console,
) {
    if !source_assets::bink::HAS_VIDEO_DECODER {
        return;
    }
    let unattended = options.capture.is_some()
        || options.monitor_capture.is_some()
        || options.frames.is_some()
        || options.movement_script.is_some()
        || options.background;
    if !options.novid
        && !unattended
        && !options.no_focus
        && is_background_level
        && options.map.is_none()
    {
        let video = startup_video(&options.game).unwrap_or_else(|| "media/valve.bik".into());
        if let Some((_path, header)) = resolve_video_file(&video) {
            console_ui
                .source
                .start_video_playback(video, None, Some(&header), None, None);
        }
    }
}

/// Resolves a video file in the owned installation or filesystem, parsing its Bink container header.
pub fn resolve_video_file(filename: &str) -> Option<(PathBuf, source_assets::bink::BinkHeader)> {
    let rel_path = normalize_video_path(filename);
    let mut candidates = Vec::new();

    // Check direct / local filesystem path first.
    let direct = Path::new(filename);
    if direct.is_file() {
        candidates.push(direct.to_path_buf());
    }
    let direct_rel = Path::new(&rel_path);
    if direct_rel.is_file() {
        candidates.push(direct_rel.to_path_buf());
    }

    // Check owned game installation.
    if let Ok(root) = source_assets::install::discover() {
        candidates.push(root.join(&rel_path));
        candidates.push(root.join("hl2_complete").join(&rel_path));
        candidates.push(root.join("hl2").join(&rel_path));
        for game in source_assets::install::games(&root) {
            candidates.push(root.join(&game).join(&rel_path));
        }
        candidates.push(root.join("platform").join(&rel_path));
    }

    for path in candidates {
        if path.is_file()
            && let Ok(bytes) = std::fs::read(&path)
            && let Ok(header) = source_assets::bink::BinkHeader::read(&bytes)
        {
            return Some((path, header));
        }
    }
    None
}

/// Handles `Effect::PlayVideo`: resolves the video file and starts playback, or logs a warning
/// and executes the exit command immediately if the video cannot be opened.
pub fn play_video(
    sim: &mut super::Simulation,
    ui: &mut crate::ui_host::Console,
    game: &mut crate::gameplay::Gameplay,
    filename: String,
    exit_command: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
) {
    if let Some((_path, header)) = resolve_video_file(&filename) {
        ui.source
            .start_video_playback(filename, exit_command, Some(&header), width, height);
    } else {
        ui.source.log(format!("Unable to play video: {filename}"));
        if let Some(cmd) = exit_command {
            let effects = ui.source.submit(&cmd);
            sim.console_effects(ui, game, effects);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_video_paths() {
        assert_eq!(normalize_video_path("valve"), "media/valve.bik");
        assert_eq!(normalize_video_path("valve.bik"), "media/valve.bik");
        assert_eq!(normalize_video_path("media/valve"), "media/valve.bik");
        assert_eq!(normalize_video_path("media/valve.bik"), "media/valve.bik");
        assert_eq!(normalize_video_path("media/valve.avi"), "media/valve.bik");
        assert_eq!(normalize_video_path("MEDIA\\VALVE.BIK"), "media/VALVE.bik");
        assert_eq!(normalize_video_path("ep1_recap"), "media/ep1_recap.bik");
    }

    #[test]
    fn missing_video_logs_warning_and_fires_exit_command() {
        let mut sim = super::super::Simulation::new(
            &modkit_core::World::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            false,
            vec![],
        );
        let mut game = crate::gameplay::Gameplay::synthetic(modkit_core::World::default());
        let mut ui = crate::ui_host::Console::new(Default::default());

        play_video(
            &mut sim,
            &mut ui,
            &mut game,
            "nonexistent_video".into(),
            Some("map ep2_outland_01".into()),
            None,
            None,
        );

        assert_eq!(ui.source.video_report(), serde_json::Value::Null);
        assert!(
            ui.source
                .shell
                .panel
                .output
                .iter()
                .any(|line| line.contains("Unable to play video: nonexistent_video")),
            "expected warning in console log"
        );
        assert_eq!(ui.map_request.as_deref(), Some("ep2_outland_01"));
        assert!(sim.loading);
    }

    #[test]
    #[ignore]
    fn resolves_owned_installed_videos() {
        let valve = resolve_video_file("valve");
        assert!(valve.is_some(), "valve video found");
        let (_path, header) = valve.unwrap();
        assert!(header.width > 0 && header.height > 0);
        assert!(header.duration() > 0.0);

        let ep1_recap = resolve_video_file("ep1_recap");
        assert!(ep1_recap.is_some(), "ep1_recap video found");
        let (_path, recap_header) = ep1_recap.unwrap();
        assert_eq!(recap_header.width, 1280);
        assert_eq!(recap_header.height, 720);
    }
}
