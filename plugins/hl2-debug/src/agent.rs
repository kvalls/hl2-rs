//! Agent link: what a debugging agent following the game from outside needs beyond the retail
//! console log file and remote console. Separate from the F1 overlay.
//!
//! `ui_dump` prints the UI's state as one JSON line (`ui_dump {...}`), so an agent reading the
//! console log or the remote console's output parses it without scraping the screen.

/// The prefix of the `ui_dump` line.
pub const UI_DUMP_PREFIX: &str = "ui_dump ";

/// The prefix of the `npc_dump` line.
pub const NPC_DUMP_PREFIX: &str = "npc_dump ";

/// Formats an `npc_dump` payload as one JSON line.
pub fn npc_dump_line(payload: serde_json::Value) -> String {
    format!("{NPC_DUMP_PREFIX}{payload}")
}

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

/// The prefix of the `agent_resume` line.
pub const RESUME_PREFIX: &str = "agent_resume ";

/// Where the player is and which menus are open, for a restart that comes back to it.
pub struct Resume<'a> {
    /// The gameplay map; `None` (or a menu background level) restarts at the main menu.
    pub map: Option<&'a str>,
    pub background_level: bool,
    pub eye: [f32; 3],
    /// Yaw and pitch in degrees as `--yaw` and `--pitch` take them (pitch positive looks up).
    pub yaw: f32,
    pub pitch: f32,
    pub mode: hl2_ui::console::Mode,
    /// Open windows, bottom first.
    pub windows: Vec<hl2_ui::console::Window>,
}

/// The command line that restarts into `resume`: the map and view, then the console commands
/// that reopen its menus (retail `gameui_activate`, `gamemenucommand`, `toggleconsole`).
pub fn resume_args(resume: &Resume<'_>) -> Vec<String> {
    use hl2_ui::console::{Mode, Window};
    let mut args = Vec::new();
    if let Some(map) = resume.map.filter(|_| !resume.background_level) {
        let [x, y, z] = resume.eye;
        args.extend([
            "--map".to_owned(),
            map.to_owned(),
            "--position".to_owned(),
            format!("{x:.3}"),
            format!("{y:.3}"),
            format!("{z:.3}"),
            "--yaw".to_owned(),
            format!("{:.3}", resume.yaw),
            "--pitch".to_owned(),
            format!("{:.3}", resume.pitch),
        ]);
        match resume.mode {
            Mode::MainMenu => args.push("--main-menu".to_owned()),
            Mode::Pause => args.push("+gameui_activate".to_owned()),
            _ => {}
        }
    }
    for window in &resume.windows {
        match window {
            Window::Options => args.extend(["+gamemenucommand".into(), "OpenOptionsDialog".into()]),
            Window::NewGame => args.extend(["+gamemenucommand".into(), "OpenNewGameDialog".into()]),
            Window::Console => args.push("+toggleconsole".to_owned()),
            Window::SubDialog | Window::Confirmation => {}
        }
    }
    args
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

#[cfg(test)]
mod resume_tests {
    use super::{Resume, resume_args};
    use hl2_ui::console::{Mode, Window};

    fn resume(mode: Mode, windows: Vec<Window>, background_level: bool) -> Resume<'static> {
        Resume {
            map: Some("d1_trainstation_02"),
            background_level,
            eye: [1.0, -2.5, 64.0],
            yaw: 90.0,
            pitch: -10.0,
            mode,
            windows,
        }
    }

    #[test]
    fn a_game_restarts_at_the_same_view_with_its_menus_reopened() {
        let args = resume_args(&resume(
            Mode::Console,
            vec![Window::Options, Window::Console],
            false,
        ));
        assert_eq!(
            args.join(" "),
            "--map d1_trainstation_02 --position 1.000 -2.500 64.000 --yaw 90.000 --pitch -10.000 \
             +gamemenucommand OpenOptionsDialog +toggleconsole"
        );
        let paused = resume_args(&resume(Mode::Pause, vec![], false));
        assert_eq!(paused.last().map(String::as_str), Some("+gameui_activate"));
    }

    #[test]
    fn a_menu_background_restarts_at_the_main_menu() {
        let args = resume_args(&resume(Mode::Options, vec![Window::Options], true));
        assert_eq!(args, ["+gamemenucommand", "OpenOptionsDialog"]);
    }
}
