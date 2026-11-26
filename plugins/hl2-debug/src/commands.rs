//! Debug commands use the shared console dispatcher and a bounded host interface.
use source_ui::console::command::{Arg, Context, Outcome, Spec, boolean, parse};
/// Default radius (Source units) for `npc_dump` when called with no arguments.
pub const DEFAULT_NPC_DUMP_RADIUS: f32 = 512.0;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum NpcDumpQuery {
    Nearby(f32),
    Entity(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MapDumpQuery {
    Page(usize),
    Near { point: [f32; 3], radius: f32 },
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Request {
    ViewmodelDebugMode(bool),
    ViewmodelTweakReset,
    BackgroundDebugMode(bool),
    BackgroundBlurOpacity(u8),
    BackgroundMapFov(Option<f32>),
    UiDump,
    NpcDump(NpcDumpQuery),
    MapDump(MapDumpQuery),
    AgentResume,
    AgentCaption(String),
    AgentRcon,
    AiEnable(Option<bool>),
    SpawnAt(SpawnAt),
}
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpawnAt {
    pub class: String,
    pub origin: [f32; 3],
    pub name: Option<String>,
    pub model: Option<String>,
    pub equipment: Option<String>,
}

impl SpawnAt {
    pub(crate) fn valid(&self) -> bool {
        !self.class.is_empty()
            && self
                .origin
                .iter()
                .all(|v| v.is_finite() && v.abs() < 65536.)
    }
}

fn parse_spawn_at(args: &[String]) -> Option<SpawnAt> {
    let [class, pos, x, y, z, pairs @ ..] = args else {
        return None;
    };
    if !pos.eq_ignore_ascii_case("pos") || !pairs.len().is_multiple_of(2) {
        return None;
    }
    let mut request = SpawnAt {
        class: class.to_ascii_lowercase(),
        origin: [x.parse().ok()?, y.parse().ok()?, z.parse().ok()?],
        name: None,
        model: None,
        equipment: None,
    };
    for [key, value] in pairs.as_chunks::<2>().0 {
        let field = match key.to_ascii_lowercase().as_str() {
            "targetname" => &mut request.name,
            "model" => &mut request.model,
            "additionalequipment" => &mut request.equipment,
            _ => return None,
        };
        if field.replace(value.clone()).is_some() {
            return None;
        }
    }
    request.valid().then_some(request)
}

fn spawn_at<C: Host>(_console: &mut C, spec: &Spec<C>, args: &[String]) -> Outcome<C::Effect> {
    let request = parse_spawn_at(args).ok_or_else(|| spec.usage())?;
    Ok(Some(C::effect(Request::SpawnAt(request))))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    Clear,
    Token(&'static str),
    Text(String),
}

pub trait Host: Context {
    fn effect(request: Request) -> Self::Effect;
    fn log(&mut self, message: impl Into<String>);
    fn has_background_context(&self) -> bool;
    fn background_mode(&self) -> bool;
    fn set_background_mode(&mut self, on: bool);
    /// `background_force_default_blur`: the saved choice and setting it (the effect applies it).
    fn standard_background(&self) -> bool;
    fn set_standard_background(&mut self, on: bool) -> Self::Effect;
    fn background_opacity(&self) -> u8;
    fn set_background_opacity(&mut self, opacity: u8);
    fn focus_background(&mut self);
    fn agent_status(&mut self, status: AgentStatus) -> Option<String>;
    fn chat_mode(&mut self, enabled: Option<bool>) -> Result<(), String>;
}
const fn command<C: Host>(
    name: &'static str,
    help: &'static str,
    args: &'static [Arg<C::Names>],
    run: source_ui::console::command::Handler<C>,
) -> Spec<C> {
    Spec {
        name,
        help,
        args,
        run,
        value: None,
        setting: None,
    }
}
const fn map_dump_spec<C: Host>() -> Spec<C> {
    command(
        "map_dump",
        "map_dump [page <first_id>|near <x> <y> <z> <radius>]: list map entities by ID or nearest live origin",
        &[
            Arg::Values(&["page", "near"]),
            Arg::Free,
            Arg::Free,
            Arg::Free,
            Arg::Free,
        ],
        map_dump,
    )
}

pub const fn specs<C: Host>() -> [Spec<C>; 15] {
    [
        command(
            "viewmodel_debug_mode",
            "viewmodel_debug_mode <0|1>: toggle the runtime viewmodel tuning overlay (gameplay map only)",
            &[Arg::Values(&["0", "1"])],
            viewmodel_debug_mode,
        ),
        command(
            "viewmodel_tweak_reset",
            "viewmodel_tweak_reset: zero the selected weapon's framing offsets (viewmodel_debug_mode 1 only)",
            &[],
            viewmodel_tweak_reset,
        ),
        command(
            "background_debug_mode",
            "background_debug_mode <0|1>: toggle background visual debug mode (background map/blur only)",
            &[Arg::Values(&["0", "1"])],
            background_debug_mode,
        ),
        command(
            "background_blur_opacity",
            "background_blur_opacity [0..100]: query or set blur background overlay opacity (background_debug_mode 1 only)",
            &[Arg::Free],
            background_blur_opacity,
        ),
        command(
            "background_map_fov",
            "background_map_fov [degrees]: query or set background map field of view",
            &[Arg::Free],
            background_map_fov,
        ),
        command(
            "ui_dump",
            "ui_dump: print the UI state (windows, focus, menus, map, position) as one JSON line",
            &[],
            ui_dump,
        ),
        map_dump_spec::<C>(),
        command(
            "npc_dump",
            "npc_dump [radius|id <entity>]: print nearby (default 512 u) or one entity's NPC AI and goal state as one JSON line",
            &[Arg::Free, Arg::Free],
            npc_dump,
        ),
        command(
            "spawn_at",
            "spawn_at <class> pos <x> <y> <z> [targetname name] [model path] [additionalequipment weapon]",
            &[
                Arg::Free,
                Arg::Values(&["pos"]),
                Arg::Free,
                Arg::Free,
                Arg::Free,
            ],
            spawn_at,
        ),
        command(
            "chat_mode",
            "chat_mode <0|1>: route local console submissions to chat",
            &[Arg::Values(&["0", "1"])],
            chat_mode,
        ),
        command(
            "agent_status",
            "agent_status <thinking|waiting|clear|text>: set the temporary remote chat status",
            &[Arg::Values(&["thinking", "waiting", "clear"])],
            agent_status,
        ),
        command(
            "agent_resume",
            "agent_resume: print the launch arguments that restart at this map, view and open menus",
            &[],
            agent_resume,
        ),
        command(
            "agent_rcon",
            "agent_rcon: allow a local agent to connect to this running game",
            &[],
            agent_rcon,
        ),
        command(
            "ai_enable",
            "ai_enable [0|1]: switch the NPC AI think loop explicitly (toggles without an argument; retail has only ai_disable)",
            &[Arg::Values(&["0", "1"])],
            ai_enable,
        ),
        command(
            crate::standard_background::SETTING,
            "background_force_default_blur [0|1]: query or force the standard background blur artwork and field of view on wide frames",
            &[Arg::Values(&["0", "1"])],
            background_force_default_blur,
        ),
    ]
}
fn viewmodel_debug_mode<C: Host>(
    _console: &mut C,
    spec: &Spec<C>,
    args: &[String],
) -> Outcome<C::Effect> {
    match args {
        [value] => Ok(Some(C::effect(Request::ViewmodelDebugMode(
            boolean(value).ok_or_else(|| spec.usage())?,
        )))),
        _ => Err(spec.usage()),
    }
}

fn viewmodel_tweak_reset<C: Host>(
    _console: &mut C,
    spec: &Spec<C>,
    args: &[String],
) -> Outcome<C::Effect> {
    if args.is_empty() {
        Ok(Some(C::effect(Request::ViewmodelTweakReset)))
    } else {
        Err(spec.usage())
    }
}

fn background_debug_mode<C: Host>(
    console: &mut C,
    spec: &Spec<C>,
    args: &[String],
) -> Outcome<C::Effect> {
    let [value] = args else {
        return Err(spec.usage());
    };
    let Some(on) = boolean(value) else {
        return Err(spec.usage());
    };
    if on {
        if !console.has_background_context() {
            console.log("background_debug_mode needs a background map or loading blur; start or return to the main menu.");
            return Ok(None);
        }
        console.set_background_mode(true);
        console.log("background_debug_mode 1");
        console.focus_background();
        Ok(Some(C::effect(Request::BackgroundDebugMode(true))))
    } else {
        console.set_background_mode(false);
        console.log("background_debug_mode 0");
        Ok(Some(C::effect(Request::BackgroundDebugMode(false))))
    }
}

fn background_blur_opacity<C: Host>(
    console: &mut C,
    spec: &Spec<C>,
    args: &[String],
) -> Outcome<C::Effect> {
    if args.is_empty() {
        console.log(format!(
            "\"background_blur_opacity\" = \"{}\"",
            console.background_opacity()
        ));
        return Ok(None);
    }
    let [value] = args else {
        return Err(spec.usage());
    };
    if !console.background_mode() {
        console.log("background_blur_opacity requires background_debug_mode 1.");
        return Ok(None);
    }
    let opacity: u8 = parse(value).map_err(|_| spec.usage())?;
    if opacity > 100 {
        return Err(spec.usage());
    }
    console.set_background_opacity(opacity);
    console.log(format!("background_blur_opacity {opacity}"));
    Ok(Some(C::effect(Request::BackgroundBlurOpacity(opacity))))
}

fn background_map_fov<C: Host>(
    _console: &mut C,
    spec: &Spec<C>,
    args: &[String],
) -> Outcome<C::Effect> {
    if args.is_empty() {
        return Ok(Some(C::effect(Request::BackgroundMapFov(None))));
    }
    let [value] = args else {
        return Err(spec.usage());
    };
    let deg: f32 = parse(value).map_err(|_| spec.usage())?;
    if !(1.0..=170.0).contains(&deg) {
        return Err(spec.usage());
    }
    Ok(Some(C::effect(Request::BackgroundMapFov(Some(deg)))))
}

fn ui_dump<C: Host>(_console: &mut C, spec: &Spec<C>, args: &[String]) -> Outcome<C::Effect> {
    if args.is_empty() {
        Ok(Some(C::effect(Request::UiDump)))
    } else {
        Err(spec.usage())
    }
}

fn map_dump<C: Host>(_console: &mut C, spec: &Spec<C>, args: &[String]) -> Outcome<C::Effect> {
    let query = match args {
        [] => MapDumpQuery::Page(0),
        [page, id] if page.eq_ignore_ascii_case("page") => {
            MapDumpQuery::Page(parse(id).map_err(|_| spec.usage())?)
        }
        [near, x, y, z, radius] if near.eq_ignore_ascii_case("near") => {
            let number = |v: &str| parse::<f32>(v).map_err(|_| spec.usage());
            let point = [number(x)?, number(y)?, number(z)?];
            let radius = number(radius)?;
            if !point.iter().all(|v| v.is_finite()) || !radius.is_finite() || radius <= 0. {
                return Err(spec.usage());
            }
            MapDumpQuery::Near { point, radius }
        }
        _ => return Err(spec.usage()),
    };
    Ok(Some(C::effect(Request::MapDump(query))))
}

fn npc_dump<C: Host>(_console: &mut C, spec: &Spec<C>, args: &[String]) -> Outcome<C::Effect> {
    let query = match args {
        [] => NpcDumpQuery::Nearby(DEFAULT_NPC_DUMP_RADIUS),
        [radius] => {
            let r: f32 = radius.parse().map_err(|_| spec.usage())?;
            if !r.is_finite() || r <= 0.0 {
                return Err(spec.usage());
            }
            NpcDumpQuery::Nearby(r)
        }
        [kw, id] if kw.eq_ignore_ascii_case("id") => {
            let entity: usize = id.parse().map_err(|_| spec.usage())?;
            NpcDumpQuery::Entity(entity)
        }
        _ => return Err(spec.usage()),
    };
    Ok(Some(C::effect(Request::NpcDump(query))))
}

fn chat_mode<C: Host>(console: &mut C, spec: &Spec<C>, args: &[String]) -> Outcome<C::Effect> {
    let enabled = match args {
        [] => None,
        [value] if value == "0" => Some(false),
        [value] if value == "1" => Some(true),
        _ => return Err(spec.usage()),
    };
    console.chat_mode(enabled)?;
    Ok(None)
}

fn agent_status<C: Host>(console: &mut C, spec: &Spec<C>, args: &[String]) -> Outcome<C::Effect> {
    if args.is_empty() {
        return Err(spec.usage());
    }
    let status = match args {
        [value] if value == "thinking" => AgentStatus::Token("#HL2RS_AgentThinking"),
        [value] if value == "waiting" => AgentStatus::Token("#HL2RS_AgentWaiting"),
        [value] if value == "clear" => AgentStatus::Clear,
        _ => {
            let text = args.join(" ");
            if text.trim().is_empty() {
                return Err(spec.usage());
            }
            AgentStatus::Text(text)
        }
    };
    Ok(console
        .agent_status(status)
        .map(|line| C::effect(Request::AgentCaption(line))))
}

fn agent_resume<C: Host>(_console: &mut C, spec: &Spec<C>, args: &[String]) -> Outcome<C::Effect> {
    if args.is_empty() {
        Ok(Some(C::effect(Request::AgentResume)))
    } else {
        Err(spec.usage())
    }
}

fn agent_rcon<C: Host>(_console: &mut C, spec: &Spec<C>, args: &[String]) -> Outcome<C::Effect> {
    if args.is_empty() {
        Ok(Some(C::effect(Request::AgentRcon)))
    } else {
        Err(spec.usage())
    }
}

fn ai_enable<C: Host>(_console: &mut C, spec: &Spec<C>, args: &[String]) -> Outcome<C::Effect> {
    let value = match args {
        [] => None,
        [value] => Some(boolean(value).ok_or_else(|| spec.usage())?),
        _ => return Err(spec.usage()),
    };
    Ok(Some(C::effect(Request::AiEnable(value))))
}

fn background_force_default_blur<C: Host>(
    console: &mut C,
    spec: &Spec<C>,
    args: &[String],
) -> Outcome<C::Effect> {
    match args {
        [] => {
            let value = u8::from(console.standard_background());
            console.log(format!("\"{}\" = \"{value}\"", spec.name));
            Ok(None)
        }
        [value] => {
            let on = boolean(value).ok_or_else(|| spec.usage())?;
            console.log(format!("{} {}", spec.name, u8::from(on)));
            Ok(Some(console.set_standard_background(on)))
        }
        _ => Err(spec.usage()),
    }
}

impl Host for hl2_ui::console::Console {
    fn effect(request: Request) -> hl2_ui::console::Effect {
        hl2_ui::console::Effect::Extension(serde_json::to_value(request).expect("debug request"))
    }
    fn log(&mut self, message: impl Into<String>) {
        self.log(message);
    }
    fn has_background_context(&self) -> bool {
        self.has_background_context()
    }
    fn background_mode(&self) -> bool {
        self.presentation.capture_menu_navigation
    }
    fn standard_background(&self) -> bool {
        crate::standard_background::forced(&self.config.video)
    }
    fn set_standard_background(&mut self, on: bool) -> hl2_ui::console::Effect {
        crate::standard_background::set_forced(self, on)
    }
    fn set_background_mode(&mut self, on: bool) {
        self.presentation.capture_menu_navigation = on;
        if on && self.presentation.background_opacity.is_none() {
            self.presentation.background_opacity = Some(50);
        }
    }
    fn background_opacity(&self) -> u8 {
        self.presentation.background_opacity.unwrap_or(50)
    }
    fn set_background_opacity(&mut self, opacity: u8) {
        self.presentation.background_opacity = Some(opacity);
    }
    fn chat_mode(&mut self, enabled: Option<bool>) -> Result<(), String> {
        crate::chat_mode::configure(self, enabled)
    }

    fn agent_status(&mut self, status: AgentStatus) -> Option<String> {
        let text = match status {
            AgentStatus::Clear => {
                self.clear_snapshot();
                return None;
            }
            AgentStatus::Token(token) => self.resources().localization.text(token),
            AgentStatus::Text(text) => text,
        };
        let line = format!("{}: {text}", hl2_ui::console::remote::RCON_SPEAKER);
        self.log_snapshot_with_level(source_ui::console::panel::Level::ChatRemote, line.clone());
        self.remote.from_rcon.then_some(line)
    }
    fn focus_background(&mut self) {
        self.focus_base();
    }
}
