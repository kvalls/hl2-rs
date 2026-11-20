//! Agent link: what a debugging agent following the game from outside needs beyond the retail
//! console log file and remote console. Separate from the F1 overlay.
//!
//! `ui_dump` prints the UI's state as one JSON line (`ui_dump {...}`), so an agent reading the
//! console log or the remote console's output parses it without scraping the screen.

/// The prefix of the `ui_dump` line.
pub const UI_DUMP_PREFIX: &str = "ui_dump ";

/// Where the player is and what the game is doing, beside the UI report.
pub struct World<'a> {
    pub map: Option<&'a str>,
    pub eye: [f32; 3],
    /// Source pitch and yaw in degrees.
    pub angles: [f32; 2],
    pub paused: bool,
    pub loading: bool,
    pub background_level: bool,
}

/// The `ui_dump` line: the console's UI report without its output history (the log has it),
/// plus `world`.
pub fn ui_dump_line(mut report: serde_json::Value, world: &World<'_>) -> String {
    if let Some(fields) = report.as_object_mut() {
        fields.remove("output");
        fields.insert(
            "world".into(),
            serde_json::json!({
                "map": world.map,
                "eye": world.eye,
                "angles": world.angles,
                "paused": world.paused,
                "loading": world.loading,
                "background_level": world.background_level,
            }),
        );
    }
    format!("{UI_DUMP_PREFIX}{report}")
}

#[cfg(test)]
mod tests {
    use super::{UI_DUMP_PREFIX, World, ui_dump_line};

    #[test]
    fn the_dump_is_one_json_line_without_the_history() {
        let report = serde_json::json!({"mode": "Console", "output": ["a\nb"], "windows": []});
        let world = World {
            map: Some("d1_trainstation_01"),
            eye: [1., 2., 3.],
            angles: [-10., 90.],
            paused: true,
            loading: false,
            background_level: false,
        };
        let line = ui_dump_line(report, &world);
        assert!(!line.contains('\n'));
        let json: serde_json::Value =
            serde_json::from_str(line.strip_prefix(UI_DUMP_PREFIX).expect("prefix")).expect("json");
        assert_eq!(json["mode"], "Console");
        assert!(json.get("output").is_none());
        assert_eq!(json["world"]["map"], "d1_trainstation_01");
        assert_eq!(json["world"]["paused"], true);
    }
}
