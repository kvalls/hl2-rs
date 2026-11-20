//! Debug commands use the shared console dispatcher and a bounded host interface.
use source_ui::console::command::{Arg, Context, Outcome, Spec, boolean, parse};
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Request {
    ViewmodelDebugMode(bool),
    ViewmodelTweakReset,
    BackgroundDebugMode(bool),
    BackgroundBlurOpacity(u8),
    BackgroundMapFov(Option<f32>),
    UiDump,
    AgentResume,
    AiEnable(Option<bool>),
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
pub const fn specs<C: Host>() -> [Spec<C>; 9] {
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
        command(
            "agent_resume",
            "agent_resume: print the launch arguments that restart at this map, view and open menus",
            &[],
            agent_resume,
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

fn agent_resume<C: Host>(_console: &mut C, spec: &Spec<C>, args: &[String]) -> Outcome<C::Effect> {
    if args.is_empty() {
        Ok(Some(C::effect(Request::AgentResume)))
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
    fn focus_background(&mut self) {
        self.focus_base();
    }
}
