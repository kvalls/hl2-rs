//! A bounded standalone console. Commands without a Rust implementation report an error.
use crate::{canvas::*, hud::FontFace};
use glam::{vec2, Vec2};
use serde::Serialize;
use source_assets::{keyvalues, vpk::Vfs};
use std::collections::VecDeque;
use std::sync::Arc;

const COMMANDS: &[(&str, &str)] = &[
    ("help", "help [command]: describe implemented commands"),
    (
        "find",
        "find <text>: search implemented command names and descriptions",
    ),
    (
        "sv_cheats",
        "sv_cheats [0|1]: query or enable local cheat commands",
    ),
    (
        "impulse",
        "impulse 101: grant the six implemented weapons, ammunition and suit (cheat)",
    ),
    (
        "noclip",
        "noclip [0|1]: toggle collision-free flight (cheat)",
    ),
    (
        "getpos",
        "getpos: print camera position and Source pitch/yaw/roll in degrees; getpos 2 is not implemented",
    ),
    (
        "setpos",
        "setpos <x> <y> [z]: move the player origin, preserving height if z is omitted (cheat)",
    ),
    (
        "setang",
        "setang <pitch> <yaw> [roll]: set Source view angles; roll must be zero (cheat)",
    ),
    (
        "map",
        "map <name>: load an installed map and reset the player inventory",
    ),
    (
        "npc_create",
        "npc_create <class> [name]: place an NPC at the crosshair (cheat; spawning not implemented yet)",
    ),
    (
        "npc_create_aimed",
        "npc_create_aimed <class> [name]: like npc_create, facing the aim (cheat)",
    ),
    (
        "npc_create_equipment",
        "npc_create_equipment <weapon>: weapon for the next npc_create",
    ),
    (
        "ai_enable",
        "ai_enable [0|1]: query or switch the NPC AI think loop (development, default 0)",
    ),
    ("clear", "clear: clear console output"),
    ("ent_fire", "ent_fire <targetname> [input=Use] [parameter] [delay]: queue entity input; names/globs supported (cheat)"),
    ("echo", "echo <text>: print text"),
    ("toggleconsole", "toggleconsole: close or open the console"),
    ("quit", "quit: exit and save the runtime report"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Mode {
    #[default]
    Gameplay,
    Pause,
    Console,
}

#[derive(Debug, PartialEq)]
pub enum Effect {
    Loadout,
    /// ai_enable: None toggles.
    AiEnable(Option<bool>),
    /// npc_create / npc_create_aimed / npc_create_equipment, the whole line.
    NpcCommand(String),
    Noclip(Option<bool>),
    Getpos,
    Setpos {
        x: f32,
        y: f32,
        z: Option<f32>,
    },
    Setang(glam::Vec3),
    Map(String),
    Fire {
        target: String,
        input: String,
        parameter: String,
        delay: f64,
    },
    Quit,
}

pub struct Console {
    pub canvas: Canvas,
    pub mode: Mode,
    previous: Mode,
    pub cheats: bool,
    pub input: String,
    pub output: VecDeque<String>,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: String,
    cursor: usize,
    scroll: usize,
    menu_selection: usize,
    font: Option<FontFace>,
    menu_font: Option<FontFace>,
    menu_metrics: CellMetrics,
    title_font: Option<FontFace>,
    title_face: Option<Arc<fontdue::Font>>,
    title_metrics: CellMetrics,
    title: String,
    menu_layout: MenuLayout,
    mono: Option<FontFace>,
    developer_font: Option<FontFace>,
    menu_labels: [String; 3],
    style: Style,
}

/// Positive native font heights describe Windows character cells, not pixels per EM.
#[derive(Clone, Copy)]
struct CellMetrics {
    em_per_cell: f32,
    ascent_per_cell: f32,
}
impl Default for CellMetrics {
    fn default() -> Self {
        Self {
            em_per_cell: 1.,
            ascent_per_cell: 0.8,
        }
    }
}
impl CellMetrics {
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let face = ttf_parser::Face::parse(bytes, 0).ok()?;
        let units = f32::from(face.units_per_em());
        let (ascent, descent) = face.tables().os2.map_or_else(
            || {
                (
                    f32::from(face.ascender()),
                    f32::from(face.descender()).abs(),
                )
            },
            |table| {
                (
                    f32::from(table.windows_ascender()),
                    f32::from(table.windows_descender()).abs(),
                )
            },
        );
        let height = ascent + descent;
        if height <= 0. || units <= 0. {
            return None;
        }
        Some(Self {
            em_per_cell: units / height,
            ascent_per_cell: ascent / height,
        })
    }
}
struct MenuLayout {
    title_x: f32,
    title_y: f32,
    menu_x: f32,
    menu_y: f32,
    title_tall: f32,
    item_tall: f32,
    font_ranges: Vec<(f32, f32, f32)>,
    font_tall: f32,
}
impl Default for MenuLayout {
    fn default() -> Self {
        // Stock desktop defaults, overridden by the mounted schemes below.
        Self {
            title_x: 76.,
            title_y: 145.,
            menu_x: 76.,
            menu_y: 240.,
            title_tall: 32.,
            item_tall: 14.,
            font_ranges: vec![(1., 1080., 16.)],
            font_tall: 11.,
        }
    }
}
#[derive(Debug)]
struct MenuGeometry {
    title: Vec2,
    menu: Vec2,
    title_tall: f32,
    font_tall: f32,
    item_tall: f32,
}
impl MenuLayout {
    fn geometry(&self, height: f32) -> MenuGeometry {
        let scale = height / 480.;
        MenuGeometry {
            title: vec2(
                (self.title_x * scale).trunc(),
                (self.title_y * scale).trunc(),
            ),
            menu: vec2((self.menu_x * scale).trunc(), (self.menu_y * scale).trunc()),
            title_tall: (self.title_tall * scale).round(),
            font_tall: self
                .font_ranges
                .iter()
                .find(|(low, high, _)| height >= *low && height <= *high)
                .map_or_else(|| (self.font_tall * scale).round(), |(_, _, tall)| *tall),
            item_tall: (self.item_tall * scale).trunc().max(1.),
        }
    }
}
fn resource_number(root: &keyvalues::Entry, key: &str) -> Option<f32> {
    root.get("BaseSettings")?
        .get(key)?
        .text()?
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}
type FontRule = (f32, Option<(f32, f32)>);
fn resource_font(root: &keyvalues::Entry, name: &str) -> Option<Vec<FontRule>> {
    let entries = root.get("Fonts")?.get(name)?.children();
    let values: Vec<_> = entries
        .iter()
        .filter_map(|entry| {
            let tall = entry.get("tall")?.text()?.parse::<f32>().ok()?;
            if !tall.is_finite() || tall <= 0. || tall > 256. {
                return None;
            }
            let range = entry
                .get("yres")
                .and_then(|range| range.text())
                .and_then(|range| {
                    let mut parts = range
                        .split_whitespace()
                        .filter_map(|p| p.parse::<f32>().ok());
                    Some((parts.next()?, parts.next()?))
                });
            Some((tall, range))
        })
        .collect();
    (!values.is_empty()).then_some(values)
}
fn draw_cell_text(
    canvas: &Canvas,
    value: &str,
    origin: Vec2,
    tall: f32,
    font: Option<&FontFace>,
    metrics: CellMetrics,
    color: Color,
) {
    let em = (tall * metrics.em_per_cell).max(1.);
    let raster_size = em.round().clamp(1., 1024.);
    text(
        canvas,
        value,
        origin + vec2(0., tall * metrics.ascent_per_cell),
        raster_size,
        em / raster_size,
        font,
        color,
    );
}
fn text(
    _canvas: &Canvas,
    value: &str,
    origin: Vec2,
    em: f32,
    scale: f32,
    font: Option<&FontFace>,
    color: Color,
) {
    if let Some(font) = font {
        font.draw_baseline(value, origin, em, scale, color);
    }
}
fn rectangle_lines(canvas: &Canvas, x: f32, y: f32, w: f32, h: f32, thickness: f32, color: Color) {
    canvas.rectangle(x, y, w, thickness, color);
    canvas.rectangle(x, y + h - thickness, w, thickness, color);
    canvas.rectangle(x, y, thickness, h, color);
    canvas.rectangle(x + w - thickness, y, thickness, h, color);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Up,
    Down,
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
}
#[derive(Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    pub keys: std::collections::BTreeSet<Key>,
    pub text: String,
    pub pointer: Option<Vec2>,
    pub click: bool,
}
impl Input {
    fn pressed(&self, key: Key) -> bool {
        self.keys.contains(&key)
    }
}

struct Style {
    menu: Color,
    menu_armed: Color,
    panel: Color,
    entry: Color,
    text: Color,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            menu: WHITE,
            menu_armed: Color::from_rgba(200, 200, 200, 255),
            panel: Color::from_rgba(132, 132, 132, 212),
            entry: Color::from_rgba(0, 0, 0, 128),
            text: Color::from_rgba(221, 221, 221, 255),
        }
    }
}
fn scheme_color(root: &keyvalues::Entry, name: &str) -> Option<Color> {
    let mut value = name;
    for _ in 0..8 {
        let values: Vec<u8> = value
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .unwrap_or_default();
        if let [r, g, b, a] = values.as_slice() {
            return Some(Color::from_rgba(*r, *g, *b, *a));
        }
        value = root
            .get("BaseSettings")
            .and_then(|e| e.get(value))
            .or_else(|| root.get("Colors").and_then(|e| e.get(value)))?
            .text()?;
    }
    None
}

impl Default for Console {
    fn default() -> Self {
        let mut console = Self {
            canvas: Canvas::default(),
            mode: Mode::Gameplay,
            previous: Mode::Gameplay,
            cheats: false,
            input: String::new(),
            output: VecDeque::new(),
            history: Vec::new(),
            history_index: None,
            history_draft: String::new(),
            cursor: 0,
            scroll: 0,
            menu_selection: 0,
            font: None,
            menu_font: None,
            menu_metrics: CellMetrics::default(),
            title_font: None,
            title_face: None,
            title_metrics: CellMetrics::default(),
            title: "HL2-RS".into(),
            menu_layout: MenuLayout::default(),
            mono: None,
            developer_font: None,
            menu_labels: [
                "RESUME GAME".into(),
                "DEVELOPER CONSOLE".into(),
                "QUIT".into(),
            ],
            style: Style::default(),
        };
        console.log("HL2-RS developer console. Type help for implemented commands.");
        console.log("Other Source commands are not implemented in this runtime.");
        console
    }
}

impl Console {
    pub fn load(vfs: &Vfs, canvas: Canvas) -> Self {
        let windows = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "C:/Windows".into());
        let font = |file| {
            std::fs::read(windows.join("Fonts").join(file))
                .ok()
                .and_then(|bytes| FontFace::load(&bytes, canvas.clone()).ok())
        };
        let mut console = Self {
            canvas: canvas.clone(),
            developer_font: FontFace::load(
                include_bytes!("../assets/fonts/ProggyClean.ttf"),
                canvas.clone(),
            )
            .ok(),
            font: font("tahoma.ttf"),
            menu_font: font("verdanab.ttf"),
            mono: font("lucon.ttf").or_else(|| {
                vfs.read("resource/linux_fonts/DejaVuSansMono.ttf")
                    .ok()
                    .flatten()
                    .and_then(|bytes| FontFace::load(&bytes, canvas.clone()).ok())
            }),
            ..Default::default()
        };
        if let Ok(bytes) = std::fs::read(windows.join("Fonts/verdanab.ttf")) {
            console.menu_metrics = CellMetrics::from_bytes(&bytes).unwrap_or_default();
        }
        if let Some(bytes) = vfs.read("resource/HALFLIFE2.ttf").ok().flatten() {
            console.title_metrics = CellMetrics::from_bytes(&bytes).unwrap_or_default();
            console.title_font = FontFace::load(&bytes, canvas.clone()).ok();
            console.title_face = console.title_font.as_ref().map(FontFace::face);
            if console.title_font.is_some() {
                // Owned 720px retail capture + GDI probe: requested cell48 -> EM41,
                // native title baseline262 relative to ClientScheme panelY217.
                // Other resolutions scale this measured GDI/label relation; font rasterization remains approximate.
                console.title_metrics = CellMetrics {
                    em_per_cell: 41. / 48.,
                    ascent_per_cell: 45. / 48.,
                };
                console.title = vfs
                    .read("gameinfo.txt")
                    .ok()
                    .flatten()
                    .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
                    .and_then(|roots| roots.into_iter().next())
                    .and_then(|root| {
                        root.get("title")
                            .and_then(|title| title.text())
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| "HALF-LIFE'".into());
            } else {
                console.title_face = None;
                console.title_metrics = CellMetrics::default();
                console.log("Owned HalfLife2 title font could not be loaded; using the HL2-RS fallback title.");
            }
        } else {
            console.log("Owned HalfLife2 title font not found; using the HL2-RS fallback title.");
        }
        // Installed desktop GameUI colors. Layout/window controls remain an initial implementation.
        if let Some(root) = vfs
            .read("resource/SourceSchemeBase.res")
            .ok()
            .flatten()
            .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
            .and_then(|roots| roots.into_iter().next())
        {
            console.style = Style {
                menu: scheme_color(&root, "MainMenu.TextColor").unwrap_or(console.style.menu),
                menu_armed: scheme_color(&root, "MainMenu.ArmedTextColor")
                    .unwrap_or(console.style.menu_armed),
                panel: scheme_color(&root, "Frame.BgColor").unwrap_or(console.style.panel),
                entry: scheme_color(&root, "RichText.BgColor").unwrap_or(console.style.entry),
                text: scheme_color(&root, "Console.TextColor").unwrap_or(console.style.text),
            };
            if let Some(tall) = resource_number(&root, "MainMenu.MenuItemHeight")
                .filter(|value| *value > 0. && *value <= 128.)
            {
                console.menu_layout.item_tall = tall;
            }
            if let Some(fonts) =
                resource_font(&root, "MainMenuFont").or_else(|| resource_font(&root, "MenuLarge"))
            {
                console.menu_layout.font_ranges.clear();
                for (tall, range) in fonts {
                    if let Some((low, high)) = range {
                        console.menu_layout.font_ranges.push((low, high, tall));
                    } else {
                        console.menu_layout.font_tall = tall;
                    }
                }
            }
        }
        if let Some(root) = vfs
            .read("resource/ClientScheme.res")
            .ok()
            .flatten()
            .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
            .and_then(|roots| roots.into_iter().next())
        {
            for (key, target) in [
                ("Main.Title1.X", &mut console.menu_layout.title_x),
                ("Main.Title1.Y", &mut console.menu_layout.title_y),
                ("Main.Menu.X", &mut console.menu_layout.menu_x),
                ("Main.Menu.Y", &mut console.menu_layout.menu_y),
            ] {
                if let Some(value) = resource_number(&root, key) {
                    *target = value;
                }
            }
            if let Some(fonts) = resource_font(&root, "ClientTitleFont") {
                console.menu_layout.title_tall = fonts[0].0;
            }
        }
        if let Some(root) = vfs
            .read("resource/gameui_english.txt")
            .ok()
            .flatten()
            .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
            .and_then(|roots| roots.into_iter().next())
        {
            if let Some(tokens) = root.get("Tokens") {
                for (i, key) in [
                    (0, "GameUI_GameMenu_ResumeGame"),
                    (2, "GameUI_GameMenu_Quit"),
                ] {
                    if let Some(label) = tokens.get(key).and_then(|entry| entry.text()) {
                        console.menu_labels[i] = label.to_uppercase();
                    }
                }
            }
        }
        console
    }
    pub fn paused(&self) -> bool {
        self.mode != Mode::Gameplay
    }
    pub fn escape(&mut self) {
        self.mode = match self.mode {
            Mode::Gameplay => Mode::Pause,
            Mode::Pause => Mode::Gameplay,
            Mode::Console => self.previous,
        };
    }
    pub fn toggle(&mut self) {
        if self.mode == Mode::Console {
            self.mode = self.previous;
        } else {
            self.previous = self.mode;
            self.mode = Mode::Console;
        }
    }
    pub fn log(&mut self, text: impl Into<String>) {
        self.output.push_back(text.into());
        if self.output.len() > 256 {
            self.output.pop_front();
        }
        self.scroll = 0;
    }
    pub fn submit(&mut self, line: &str) -> Vec<Effect> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        self.log(format!("] {line}"));
        if self.history.last().is_none_or(|last| last != line) {
            self.history.push(line.to_owned());
            if self.history.len() > 128 {
                self.history.remove(0);
            }
        }
        self.history_index = None;
        self.history_draft.clear();
        let commands = match tokenize(line) {
            Ok(commands) => commands,
            Err(error) => {
                self.log(error);
                return Vec::new();
            }
        };
        let mut effects = Vec::new();
        for tokens in commands {
            match self.interpret(&tokens) {
                Ok(Some(effect)) => effects.push(effect),
                Ok(None) => (),
                Err(error) => self.log(error),
            }
        }
        effects
    }
    fn interpret(&mut self, tokens: &[String]) -> Result<Option<Effect>, String> {
        let name = tokens[0].to_ascii_lowercase();
        let args = &tokens[1..];
        let usage = || {
            COMMANDS.iter().find(|(n, _)| *n == name).map_or_else(
                || format!("Unknown or unimplemented command: {}", tokens[0]),
                |(_, d)| d.to_string(),
            )
        };
        let cheat = || {
            if self.cheats {
                Ok(())
            } else {
                Err(format!("Can't use {name} without sv_cheats 1"))
            }
        };
        match name.as_str() {
            "sv_cheats" => {
                if args.is_empty() {
                    self.log(format!("\"sv_cheats\" = \"{}\"", u8::from(self.cheats)));
                } else if args.len() == 1 {
                    self.cheats = boolean(&args[0]).ok_or_else(usage)?;
                    self.log(format!("sv_cheats {}", u8::from(self.cheats)));
                } else {
                    return Err(usage());
                }
            }
            "impulse" => {
                if args != ["101"] {
                    return Err(usage());
                }
                cheat()?;
                return Ok(Some(Effect::Loadout));
            }
            "noclip" => {
                let value = match args {
                    [] => None,
                    [value] => Some(boolean(value).ok_or_else(usage)?),
                    _ => return Err(usage()),
                };
                cheat()?;
                return Ok(Some(Effect::Noclip(value)));
            }
            "npc_create" | "npc_create_aimed" | "npc_create_equipment" => {
                if name != "npc_create_equipment" {
                    cheat()?;
                }
                return Ok(Some(Effect::NpcCommand(
                    std::iter::once(name.as_str())
                        .chain(args.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" "),
                )));
            }
            "ai_enable" => {
                let value = match args {
                    [] => None,
                    [value] => Some(boolean(value).ok_or_else(usage)?),
                    _ => return Err(usage()),
                };
                return Ok(Some(Effect::AiEnable(value)));
            }
            "getpos" => {
                if !args.is_empty() {
                    return Err(usage());
                }
                return Ok(Some(Effect::Getpos));
            }
            "setpos" | "setang" => {
                let value = vector(args).ok_or_else(usage)?;
                cheat()?;
                if name == "setang" && value.z != 0. {
                    return Err("setang: camera roll is not implemented; use zero roll".into());
                }
                return Ok(Some(if name == "setpos" {
                    Effect::Setpos {
                        x: value.x,
                        y: value.y,
                        z: (args.len() == 3).then_some(value.z),
                    }
                } else {
                    Effect::Setang(value)
                }));
            }
            "map" => {
                if args.len() != 1 {
                    return Err(usage());
                }
                let map = args[0].strip_suffix(".bsp").unwrap_or(&args[0]);
                if map.is_empty()
                    || map.len() > 128
                    || !map.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return Err("map: use a simple installed map name".into());
                }
                return Ok(Some(Effect::Map(map.to_owned())));
            }
            "ent_fire" => {
                cheat()?;
                if args.is_empty() || args.len() > 4 || args[0].is_empty() {
                    return Err(usage());
                }
                let delay = args
                    .get(3)
                    .map_or(Ok(0.), |s| s.parse::<f64>())
                    .map_err(|_| usage())?;
                if !delay.is_finite() || delay < 0. {
                    return Err("ent_fire: delay must be finite and nonnegative".into());
                }
                let input = args.get(1).map_or("Use", String::as_str);
                if input.is_empty() {
                    return Err(usage());
                }
                return Ok(Some(Effect::Fire {
                    target: args[0].clone(),
                    input: input.into(),
                    parameter: args.get(2).cloned().unwrap_or_default(),
                    // Desktop ent_fire parses an integer delay; map I/O keeps fractions.
                    delay: delay.trunc(),
                }));
            }
            "help" | "find" => {
                if args.len() > 1 || name == "find" && args.is_empty() {
                    return Err(usage());
                }
                let query = args.first().map(|s| s.to_ascii_lowercase());
                let found: Vec<_> = COMMANDS
                    .iter()
                    .filter(|(n, description)| {
                        query.as_ref().is_none_or(|q| {
                            if name == "help" {
                                n == q
                            } else {
                                description.to_ascii_lowercase().contains(q)
                            }
                        })
                    })
                    .collect();
                if found.is_empty() {
                    self.log("No implemented command matches.");
                }
                for (_, description) in found {
                    self.log(*description);
                }
            }
            "clear" => {
                if !args.is_empty() {
                    return Err(usage());
                }
                self.output.clear();
            }
            "echo" => self.log(args.join(" ")),
            "toggleconsole" => {
                if !args.is_empty() {
                    return Err(usage());
                }
                self.toggle();
            }
            "quit" => {
                if !args.is_empty() {
                    return Err(usage());
                }
                return Ok(Some(Effect::Quit));
            }
            _ => return Err(usage()),
        }
        Ok(None)
    }
    fn history_move(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        if self.history_index.is_none() {
            self.history_draft = self.input.clone();
        }
        self.history_index = if older {
            Some(
                self.history_index
                    .map_or(self.history.len() - 1, |i| i.saturating_sub(1)),
            )
        } else {
            self.history_index
                .and_then(|i| (i + 1 < self.history.len()).then_some(i + 1))
        };
        self.input = self
            .history_index
            .map_or_else(|| self.history_draft.clone(), |i| self.history[i].clone());
        self.cursor = self.input.len();
    }
    /// Called before gameplay input collection. Opening characters are drained by the host.
    pub fn input(&mut self, input: &Input) -> Vec<Effect> {
        if self.mode == Mode::Pause {
            if input.pressed(Key::Up) {
                self.menu_selection = (self.menu_selection + 2) % 3;
            }
            if input.pressed(Key::Down) {
                self.menu_selection = (self.menu_selection + 1) % 3;
            }
            let pointer = input.pointer;
            let geometry = self.menu_layout.geometry(self.canvas.height());
            for i in 0..3 {
                let rect = Rect::new(
                    geometry.menu.x,
                    geometry.menu.y + i as f32 * geometry.item_tall,
                    250. * self.canvas.height() / 480.,
                    geometry.item_tall,
                );
                if pointer.is_some_and(|p| rect.contains(p)) {
                    self.menu_selection = i;
                    if input.click {
                        return self.activate_menu();
                    }
                }
            }
            if input.pressed(Key::Enter) {
                return self.activate_menu();
            }
            return Vec::new();
        }
        if self.mode != Mode::Console {
            return Vec::new();
        }
        for c in input.text.chars() {
            if !c.is_control() && c != '`' && c != '~' && self.input.len() + c.len_utf8() <= 1024 {
                self.input.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
        }
        if input.pressed(Key::Backspace) && self.cursor > 0 {
            let previous = self.input[..self.cursor].char_indices().last().unwrap().0;
            self.input.replace_range(previous..self.cursor, "");
            self.cursor = previous;
        }
        if input.pressed(Key::Delete) && self.cursor < self.input.len() {
            let length = self.input[self.cursor..].chars().next().unwrap().len_utf8();
            self.input
                .replace_range(self.cursor..self.cursor + length, "");
        }
        if input.pressed(Key::Left) && self.cursor > 0 {
            self.cursor = self.input[..self.cursor].char_indices().last().unwrap().0;
        }
        if input.pressed(Key::Right) && self.cursor < self.input.len() {
            self.cursor += self.input[self.cursor..].chars().next().unwrap().len_utf8();
        }
        if input.pressed(Key::Home) {
            self.cursor = 0;
        }
        if input.pressed(Key::End) {
            self.cursor = self.input.len();
        }
        if input.pressed(Key::Up) {
            self.history_move(true);
        }
        if input.pressed(Key::Down) {
            self.history_move(false);
        }
        if input.pressed(Key::PageUp) {
            self.scroll = (self.scroll + 10).min(self.output.len().saturating_sub(1));
        }
        if input.pressed(Key::PageDown) {
            self.scroll = self.scroll.saturating_sub(10);
        }
        if input.pressed(Key::Tab) {
            let matches: Vec<_> = COMMANDS
                .iter()
                .filter(|(name, _)| name.starts_with(&self.input))
                .collect();
            if matches.len() == 1 {
                self.input = format!("{} ", matches[0].0);
                self.cursor = self.input.len();
            } else if matches.len() > 1 {
                self.log(
                    matches
                        .into_iter()
                        .map(|(n, _)| *n)
                        .collect::<Vec<_>>()
                        .join("  "),
                );
            }
        }
        if input.pressed(Key::Enter) {
            let line = std::mem::take(&mut self.input);
            self.cursor = 0;
            return self.submit(&line);
        }
        Vec::new()
    }
    fn activate_menu(&mut self) -> Vec<Effect> {
        match self.menu_selection {
            0 => self.mode = Mode::Gameplay,
            1 => self.toggle(),
            _ => return vec![Effect::Quit],
        }
        Vec::new()
    }
    /// Four compact header rows and a footer, preserving the retained host's developer layout.
    /// Font data is bundled under its own MIT license, independent of installed game content.
    pub fn draw_debug(&self, lines: &[String]) {
        self.canvas.rectangle(
            0.,
            0.,
            self.canvas.width(),
            105.,
            Color::new(0.025, 0.045, 0.07, 0.88),
        );
        let rows = [
            (vec2(22., 30.), 27., WHITE),
            (vec2(22., 55.), 19., Color::new(0.784, 0.784, 0.784, 1.)),
            (vec2(22., 78.), 17., Color::new(0.784, 0.784, 0.784, 1.)),
            (vec2(22., 98.), 16., Color::from_rgba(240, 180, 90, 255)),
        ];
        let draw = |line: &str, origin: Vec2, size: f32, color| {
            if let Some(font) = &self.developer_font {
                font.draw_baseline(line, origin, size, 1., color);
            } else {
                text(
                    &self.canvas,
                    line,
                    origin,
                    size,
                    1.,
                    self.mono.as_ref().or(self.font.as_ref()),
                    color,
                );
            }
        };
        for (line, (origin, size, color)) in lines.iter().take(4).zip(rows) {
            draw(line, origin, size, color);
        }
        if let Some(footer) = lines.get(4) {
            self.canvas.rectangle(
                0.,
                self.canvas.height() - 38.,
                self.canvas.width(),
                38.,
                Color::new(0.025, 0.045, 0.07, 0.85),
            );
            draw(footer, vec2(20., self.canvas.height() - 13.), 18., WHITE);
        }
    }

    pub fn draw(&self, time: f64) {
        if self.mode == Mode::Gameplay {
            return;
        }
        let scale = self.canvas.height() / 720.;
        let text = |content: &str, x, y, size, color, mono: bool| {
            text(
                &self.canvas,
                content,
                vec2(x, y),
                (size * scale) as u16 as f32,
                1.,
                if mono {
                    self.mono.as_ref()
                } else if self.mode == Mode::Pause {
                    self.menu_font.as_ref()
                } else {
                    self.font.as_ref()
                },
                color,
            );
        };
        if self.mode == Mode::Pause {
            // The accepted retail in-game capture has no global dimming rectangle.
            let geometry = self.menu_layout.geometry(self.canvas.height());
            if let Some(face) = &self.title_face {
                let em = geometry.title_tall * self.title_metrics.em_per_cell;
                let mut pen = geometry.title;
                for character in self.title.chars() {
                    draw_cell_text(
                        &self.canvas,
                        &character.to_string(),
                        pen,
                        geometry.title_tall,
                        self.title_font.as_ref(),
                        self.title_metrics,
                        WHITE,
                    );
                    // GDI supplies integer ABC advances. Fractional accumulation visibly widens this logo.
                    pen.x += face.metrics(character, em).advance_width.round();
                }
            } else {
                draw_cell_text(
                    &self.canvas,
                    &self.title,
                    geometry.title,
                    geometry.title_tall,
                    self.title_font.as_ref(),
                    self.title_metrics,
                    WHITE,
                );
            }
            for (i, label) in self.menu_labels.iter().enumerate() {
                draw_cell_text(
                    &self.canvas,
                    label,
                    geometry.menu
                        + vec2(
                            0.,
                            i as f32 * geometry.item_tall
                                + (geometry.item_tall - geometry.font_tall) * 0.5,
                        ),
                    geometry.font_tall,
                    self.menu_font.as_ref(),
                    self.menu_metrics,
                    if self.menu_selection == i {
                        self.style.menu_armed
                    } else {
                        self.style.menu
                    },
                );
            }
        } else {
            let left = 32. * scale;
            let top = 32. * scale;
            let width = (self.canvas.width() - 64. * scale).min(1000. * scale);
            let height = self.canvas.height() - 64. * scale;
            self.canvas.rectangle(
                0.,
                0.,
                self.canvas.width(),
                self.canvas.height(),
                Color::new(0., 0., 0., 0.3),
            );
            self.canvas
                .rectangle(left, top, width, height, self.style.panel);
            rectangle_lines(
                &self.canvas,
                left,
                top,
                width,
                height,
                1.,
                Color::from_rgba(150, 158, 147, 255),
            );
            text(
                "Console",
                left + 12. * scale,
                top + 24. * scale,
                18.,
                WHITE,
                false,
            );
            self.canvas.rectangle(
                left + 10. * scale,
                top + 36. * scale,
                width - 20. * scale,
                height - 88. * scale,
                self.style.entry,
            );
            let rows = ((height / scale - 108.) / 19.).max(1.) as usize;
            let end = self.output.len().saturating_sub(self.scroll);
            let start = end.saturating_sub(rows);
            for (i, line) in self.output.iter().skip(start).take(end - start).enumerate() {
                // Bound horizontal text to the panel. Full output remains in runtime reports.
                let columns = ((width / scale - 36.) / 9.).max(1.) as usize;
                let mut clipped = String::new();
                for c in line.chars().take(columns) {
                    let mut candidate = clipped.clone();
                    candidate.push(c);
                    if self.mono.as_ref().is_some_and(|font| {
                        font.width_em(&candidate, (16. * scale) as u16 as f32) > width - 36. * scale
                    }) {
                        break;
                    }
                    clipped = candidate;
                }
                text(
                    &clipped,
                    left + 18. * scale,
                    top + (57. + i as f32 * 19.) * scale,
                    16.,
                    self.style.text,
                    true,
                );
            }
            let bottom = top + height - 38. * scale;
            self.canvas.rectangle(
                left + 10. * scale,
                bottom,
                width - 20. * scale,
                28. * scale,
                self.style.entry,
            );
            let columns = ((width / scale - 40.) / 9.).max(1.) as usize;
            let preceding = self.input[..self.cursor].chars().count();
            let start = preceding.saturating_sub(columns.saturating_sub(1));
            let shown: String = self.input.chars().skip(start).take(columns).collect();
            text(
                &format!("] {shown}"),
                left + 15. * scale,
                bottom + 20. * scale,
                16.,
                WHITE,
                true,
            );
            if ((time * 2.) as u64).is_multiple_of(2) {
                let prefix = format!(
                    "] {}",
                    self.input
                        .chars()
                        .skip(start)
                        .take(preceding - start)
                        .collect::<String>()
                );
                let cursor_x = self.mono.as_ref().map_or(0., |font| {
                    font.width_em(&prefix, (16. * scale) as u16 as f32)
                });
                self.canvas.rectangle(
                    left + 15. * scale + cursor_x,
                    bottom + 5. * scale,
                    1.,
                    18. * scale,
                    WHITE,
                );
            }
        }
    }
}

fn boolean(value: &str) -> Option<bool> {
    match value {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}
fn vector(args: &[String]) -> Option<glam::Vec3> {
    if !(2..=3).contains(&args.len()) {
        return None;
    }
    let values: Vec<f32> = args
        .iter()
        .map(|v| v.parse())
        .collect::<Result<_, _>>()
        .ok()?;
    values
        .iter()
        .all(|v| v.is_finite())
        .then(|| glam::Vec3::new(values[0], values[1], values.get(2).copied().unwrap_or(0.)))
}
/// Semicolons outside quotes separate commands; quoted arguments preserve spaces and separators.
fn tokenize(line: &str) -> Result<Vec<Vec<String>>, String> {
    let mut commands = Vec::new();
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            ';' if !quoted => {
                if started {
                    tokens.push(std::mem::take(&mut token));
                    started = false;
                }
                if !tokens.is_empty() {
                    commands.push(std::mem::take(&mut tokens));
                }
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    tokens.push(std::mem::take(&mut token));
                    started = false;
                }
            }
            _ => {
                token.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err("Unterminated quoted argument; no commands executed".into());
    }
    if started {
        tokens.push(token);
    }
    if !tokens.is_empty() {
        commands.push(tokens);
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_menu_uses_480_coordinate_scaling_but_keeps_yres_font_height() {
        let layout = MenuLayout::default();
        let at_720 = layout.geometry(720.);
        assert_eq!(at_720.title, vec2(114., 217.));
        assert_eq!(at_720.menu, vec2(114., 360.));
        assert_eq!(at_720.title_tall, 48.);
        assert_eq!(at_720.item_tall, 21.);
        assert_eq!(at_720.font_tall, 16.);
        let at_480 = layout.geometry(480.);
        assert_eq!(at_480.menu, vec2(76., 240.));
        assert_eq!(at_480.font_tall, 16.);
        assert_eq!(at_480.title_tall, 32.);
        assert_eq!(layout.geometry(1440.).font_tall, 33.);
    }
    #[test]
    fn menu_resources_resolve_desktop_colors_ranges_and_reject_bad_heights() {
        let resource = keyvalues::parse_resource(br#"Scheme {
            Colors { White "255 255 255 255" } BaseSettings { MainMenu.TextColor White Main.Menu.X 76 }
            Fonts { MenuLarge { 1 { tall 16 yres "1 1080" } [$WINDOWS]
                1 { tall 24 yres "1 1080" } [$LINUX] 2 { tall 11 } 3 { tall NaN } } }
        }"#).unwrap();
        let root = &resource[0];
        assert_eq!(scheme_color(root, "MainMenu.TextColor"), Some(WHITE));
        assert_eq!(resource_number(root, "Main.Menu.X"), Some(76.));
        let ranges = resource_font(root, "MenuLarge").unwrap();
        #[cfg(target_os = "windows")]
        assert_eq!(ranges, vec![(16., Some((1., 1080.))), (11., None)]);
        #[cfg(target_os = "linux")]
        assert_eq!(ranges, vec![(24., Some((1., 1080.))), (11., None)]);
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        assert_eq!(ranges, vec![(11., None)]);
        assert!(resource_font(root, "Absent").is_none());
        assert!(CellMetrics::from_bytes(b"missing font").is_none());
    }
    #[test]
    fn cheats_are_gated_in_order_and_queries_do_not_change_them() {
        let mut console = Console::default();
        assert!(console
            .submit("impulse 101; noclip; setpos 1 2 3; setang 0 0 0")
            .is_empty());
        assert_eq!(
            console.submit("sv_cheats 1; impulse 101; noclip 1"),
            vec![Effect::Loadout, Effect::Noclip(Some(true))]
        );
        assert!(console.submit("sv_cheats").is_empty());
        assert!(console.cheats);
        assert!(console.submit("sv_cheats 0; impulse 101").is_empty());
        assert!(!console.cheats);
    }
    #[test]
    fn npc_create_is_a_cheat_but_its_equipment_cvar_is_not() {
        let mut console = Console::default();
        assert_eq!(
            console.submit("npc_create npc_metropolice; npc_create_equipment weapon_stunstick"),
            vec![Effect::NpcCommand(
                "npc_create_equipment weapon_stunstick".into()
            )]
        );
        assert_eq!(
            console.submit("sv_cheats 1; npc_create_aimed npc_metropolice cop1"),
            vec![Effect::NpcCommand(
                "npc_create_aimed npc_metropolice cop1".into()
            )]
        );
    }
    #[test]
    fn quoted_separators_are_text_and_bad_quotes_execute_nothing() {
        let mut console = Console::default();
        console.submit("echo \"hello; world\"; echo done");
        assert!(console.output.iter().any(|line| line == "hello; world"));
        assert!(console.output.iter().any(|line| line == "done"));
        console.submit("sv_cheats 1; echo \"unfinished");
        assert!(!console.cheats);
    }
    #[test]
    fn invalid_numbers_paths_impulses_and_unknown_commands_do_not_succeed() {
        let mut console = Console::default();
        console.submit("sv_cheats 1");
        for line in [
            "setpos NaN 0 0",
            "setpos inf 0 0",
            "setpos 1",
            "setang 0 0 20",
            "map ../other",
            "map a/b",
            "map C:\\foo",
            "impulse 42",
            "god",
            "sv_cheats 2",
        ] {
            assert!(console.submit(line).is_empty(), "{line}");
        }
        assert_eq!(
            console.submit("map d1_trainstation_02.bsp"),
            vec![Effect::Map("d1_trainstation_02".into())]
        );
        assert_eq!(
            console.submit("setpos -10 20 30"),
            vec![Effect::Setpos {
                x: -10.,
                y: 20.,
                z: Some(30.)
            }]
        );
        assert_eq!(
            console.submit("setpos 1 2"),
            vec![Effect::Setpos {
                x: 1.,
                y: 2.,
                z: None
            }]
        );
    }

    #[test]
    fn entity_inputs_preserve_quoted_values_and_enforce_cheat_and_delay_rules() {
        let mut console = Console::default();
        assert!(console.submit("ent_fire security03 Start").is_empty());
        assert_eq!(
            console.submit("sv_cheats 1; ent_fire security_* Start \"value;kept\" 0.1"),
            vec![Effect::Fire {
                target: "security_*".into(),
                input: "Start".into(),
                parameter: "value;kept".into(),
                delay: 0.,
            }]
        );
        assert_eq!(
            console.submit("ent_fire panel"),
            vec![Effect::Fire {
                target: "panel".into(),
                input: "Use".into(),
                parameter: String::new(),
                delay: 0.,
            }]
        );
        for command in [
            "ent_fire",
            "ent_fire door Open x NaN",
            "ent_fire door Open x -1",
            "ent_fire door Open x inf",
        ] {
            assert!(console.submit(command).is_empty());
        }
    }
    #[test]
    fn console_returns_to_its_previous_pause_state_and_history_restores_draft() {
        let mut console = Console::default();
        console.escape();
        assert!(console.paused());
        console.toggle();
        assert_eq!(console.mode, Mode::Console);
        console.escape();
        assert_eq!(console.mode, Mode::Pause);
        console.escape();
        assert!(!console.paused());
        console.submit("echo first");
        console.submit("echo second");
        console.input = "draft".into();
        console.history_move(true);
        assert_eq!(console.input, "echo second");
        console.history_move(true);
        assert_eq!(console.input, "echo first");
        console.history_move(false);
        assert_eq!(console.input, "echo second");
        console.history_move(false);
        assert_eq!(console.input, "draft");
    }
    #[test]
    fn explicit_text_input_edits_unicode_at_boundaries_and_completes_commands() {
        let mut console = Console::default();
        console.toggle();
        console.input(&Input {
            text: "aé🦀".into(),
            ..Default::default()
        });
        console.input(&Input {
            keys: [Key::Left].into(),
            ..Default::default()
        });
        console.input(&Input {
            keys: [Key::Backspace].into(),
            ..Default::default()
        });
        assert_eq!(console.input, "a🦀");
        console.input(&Input {
            keys: [Key::Delete].into(),
            ..Default::default()
        });
        assert_eq!(console.input, "a");
        console.input.clear();
        console.cursor = 0;
        console.input(&Input {
            text: "noc".into(),
            keys: [Key::Tab].into(),
            ..Default::default()
        });
        assert_eq!(console.input, "noclip ");
        console.input(&Input {
            keys: [Key::Enter].into(),
            ..Default::default()
        });
        assert!(console.input.is_empty());
        assert!(console.output.iter().any(|s| s.contains("sv_cheats")));
    }
    #[test]
    fn menu_pointer_hit_regions_and_keyboard_activate_the_same_choices() {
        let mut console = Console::default();
        console.escape();
        let geometry = console.menu_layout.geometry(console.canvas.height());
        console.input(&Input {
            pointer: Some(geometry.menu + vec2(5., geometry.item_tall * 1.5)),
            click: true,
            ..Default::default()
        });
        assert_eq!(console.mode, Mode::Console);
        console.escape();
        assert_eq!(console.mode, Mode::Pause);
        console.input(&Input {
            keys: [Key::Down].into(),
            ..Default::default()
        });
        assert_eq!(
            console.input(&Input {
                keys: [Key::Enter].into(),
                ..Default::default()
            }),
            vec![Effect::Quit]
        );
    }
}
