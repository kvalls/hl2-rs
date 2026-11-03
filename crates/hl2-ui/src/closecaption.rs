//! Closed captions: the HUD caption panel (SDK `CHudCloseCaption`, hud_closecaption.cpp).
//!
//! Caption text comes from the installed compiled dictionaries
//! (`source_assets::captions`); the panel geometry and animation values from the installed
//! `HudLayout.res` `HudCloseCaption`; fonts and the background color from the installed
//! `ClientScheme.res`. The model is pure: [`CloseCaption::emit`] queues items, [`tick`]
//! (`OnTick`) decays them on the game clock and [`paint`] (`Paint`) lays them out into a box
//! and text runs that the HUD draws.
//!
//! [`tick`]: CloseCaption::tick
//! [`paint`]: CloseCaption::paint
use std::collections::HashMap;
mod paint;

/// `CC_INSET`, proportional.
const INSET: f32 = 6.;
/// `MAX_CAPTION_CHARACTERS`.
const MAX_CHARACTERS: usize = 4096;
/// SplitCommand fails on commands or arguments that do not fit 256 characters.
const MAX_COMMAND: usize = 256;
/// `CAPTION_PAN_FADE_TIME` / `CAPTION_PAN_SLIDE_TIME`.
const PAN_FADE_TIME: f32 = 0.5;
const PAN_SLIDE_TIME: f32 = 0.5;
/// The caption language: empty `cc_lang` uses the game UI language, whose resources the HUD
/// loads (`*_english.txt`).
pub const LANGUAGE: &str = "english";
/// `cc_minvisibleitems`.
const MIN_VISIBLE_ITEMS: usize = 1;
/// Retail sends the duration in a user message as tenths of a second, at most 255.
pub fn message_duration(seconds: f32) -> f32 {
    ((seconds * 10.) as i32).min(255) as f32 * 0.1
}

/// The Options > Audio "Closed captions" setting as the `closecaption` / `cc_subtitles`
/// cvars and the timing cvars (`cc_linger_time`, `cc_predisplay_time`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub closecaption: bool,
    /// Voice only: captions with `<sfx>` are not shown.
    pub subtitles: bool,
    pub linger_time: f32,
    pub predisplay_time: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            closecaption: false,
            subtitles: false,
            linger_time: 1.,
            predisplay_time: 0.25,
        }
    }
}

impl Settings {
    /// None = `closecaption 0`; subtitles and sound effects = `closecaption 1, cc_subtitles 0`;
    /// subtitles = `closecaption 1, cc_subtitles 1`.
    pub fn from_config(captions: crate::config::Captions) -> Self {
        use crate::config::Captions;
        Self {
            closecaption: captions != Captions::None,
            subtitles: captions == Captions::Subtitles,
            ..Self::default()
        }
    }
}

/// Font variant of a run (`CCFONT_NORMAL` / `_ITALIC` / `_BOLD` / `_ITALICBOLD`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    Normal,
    Italic,
    Bold,
    BoldItalic,
}

impl Style {
    /// CHudCloseCaption::GetFontNumber.
    pub fn of(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (true, true) => Self::BoldItalic,
            (true, false) => Self::Bold,
            (false, true) => Self::Italic,
            (false, false) => Self::Normal,
        }
    }
}

/// Text measurement in panel pixels: per-character widths (VGUI `GetTextSize` on one
/// character) and the font height (`GetFontTall`), the same for every style.
pub trait Measure {
    fn tall(&self) -> f32;
    fn width(&self, style: Style, text: &str) -> f32;
}

/// `HudCloseCaption` values from `HudLayout.res` (480-line proportional units).
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub xpos: String,
    pub ypos: f32,
    pub wide: f32,
    pub tall: f32,
    pub bg_alpha: f32,
    pub grow_time: f32,
    pub item_hidden_time: f32,
    pub item_fade_in_time: f32,
    pub item_fade_out_time: f32,
    pub top_offset: f32,
}

impl Default for Layout {
    /// The CPanelAnimationVar defaults; the installed layout supplies the position.
    fn default() -> Self {
        Self {
            xpos: "c-250".into(),
            ypos: 276.,
            wide: 500.,
            tall: 136.,
            bg_alpha: 192.,
            grow_time: 0.25,
            item_hidden_time: 0.2,
            item_fade_in_time: 0.15,
            item_fade_out_time: 0.3,
            top_offset: 40.,
        }
    }
}

impl Layout {
    pub fn read(entry: &source_assets::keyvalues::Entry) -> Self {
        let defaults = Self::default();
        let number = |key: &str, fallback: f32| {
            entry
                .get(key)
                .and_then(source_assets::keyvalues::Entry::text)
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(fallback)
        };
        Self {
            xpos: entry
                .get("xpos")
                .and_then(source_assets::keyvalues::Entry::text)
                .map_or(defaults.xpos.clone(), |s| s.trim().to_owned()),
            ypos: number("ypos", defaults.ypos),
            wide: number("wide", defaults.wide),
            tall: number("tall", defaults.tall),
            bg_alpha: number("BgAlpha", defaults.bg_alpha),
            grow_time: number("GrowTime", defaults.grow_time),
            item_hidden_time: number("ItemHiddenTime", defaults.item_hidden_time),
            item_fade_in_time: number("ItemFadeInTime", defaults.item_fade_in_time),
            item_fade_out_time: number("ItemFadeOutTime", defaults.item_fade_out_time),
            top_offset: number("topoffset", defaults.top_offset),
        }
    }
}

/// One pre-processed chunk of caption text (CCloseCaptionWorkUnit).
#[derive(Clone, Debug, PartialEq)]
struct WorkUnit {
    text: String,
    color: [u8; 4],
    style: Style,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// One queued caption phrase (CCloseCaptionItem).
#[derive(Clone, Debug)]
struct Item {
    stream: String,
    time_to_live: f32,
    initial_life_span: f32,
    added_time: f32,
    predisplay: f32,
    valid: bool,
    from_player: bool,
    work: Option<Vec<WorkUnit>>,
    width: f32,
    height: f32,
}

impl Item {
    fn alpha(&self, hidden: f32, fade_in: f32, fade_out: f32) -> f32 {
        let since_start = self.initial_life_span - self.time_to_live;
        let until_end = self.time_to_live;
        let total_fade_in = hidden + fade_in;
        if total_fade_in > 0.001 && since_start < total_fade_in {
            if since_start >= hidden {
                let f = if fade_in > 0.001 {
                    (since_start - hidden) / fade_in
                } else {
                    1.
                };
                return f.clamp(0., 1.);
            }
            return 0.;
        }
        if fade_out > 0.001 && until_end < fade_out {
            return (until_end / fade_out).clamp(0., 1.);
        }
        1.
    }
}

#[derive(Clone, Copy, Debug)]
struct Repeat {
    last_time: f64,
    last_tick: u64,
    interval: f32,
}

/// A `<cmd>` or `<cmd:args>` at `chars[*i]` (SplitCommand). On success `*i` is left on the
/// character that ended the command (`>`, a space or the end), which the caller's loop
/// then steps over, as the SDK does.
fn split_command(chars: &[char], i: &mut usize) -> Option<(String, String)> {
    if chars.get(*i) != Some(&'<') {
        return None;
    }
    let mut at = *i + 1;
    let mut command = String::new();
    while let Some(&c) = chars.get(at) {
        if c == ':' || c == '>' || c.is_whitespace() {
            break;
        }
        if command.chars().count() + 1 == MAX_COMMAND {
            return None;
        }
        command.push(c);
        at += 1;
    }
    let mut args = String::new();
    if chars.get(at) == Some(&':') {
        at += 1;
        while let Some(&c) = chars.get(at) {
            if c == '>' {
                break;
            }
            if args.chars().count() + 1 == MAX_COMMAND {
                return None;
            }
            args.push(c);
            at += 1;
        }
    }
    *i = at;
    Some((command, args))
}

/// Every command of `stream` in order.
fn commands(stream: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = stream.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some(command) = split_command(&chars, &mut i) {
            found.push(command);
        }
        i += 1;
    }
    found
}

fn has_command(stream: &str, name: &str) -> bool {
    commands(stream).iter().any(|(c, _)| c == name)
}

/// The leading number of `text` (`wcstod`), 0 when there is none.
fn leading_float(text: &str) -> f32 {
    let text = text.trim_start();
    let end = text
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && i == 0)))
        .map_or(text.len(), |(i, _)| i);
    text[..end].parse().unwrap_or(0.)
}

fn float_command(stream: &str, name: &str) -> Option<f32> {
    commands(stream)
        .into_iter()
        .find(|(c, _)| c == name)
        .map(|(_, args)| leading_float(&args))
}

/// `%i,%i,%i` color components.
fn components<const N: usize>(text: &str, separators: &[char]) -> Option<[i32; N]> {
    let parts: Vec<i32> = text
        .split(|c| separators.contains(&c))
        .map(|p| p.trim().parse::<i32>().ok())
        .collect::<Option<_>>()?;
    parts.try_into().ok()
}

fn channel(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

/// Layout state while splitting a stream into work units (WorkUnitParams).
struct Params {
    out: Vec<char>,
    x: f32,
    y: f32,
    width: f32,
    bold: bool,
    italic: bool,
    color: [u8; 4],
    newline: bool,
}

impl Params {
    /// Flush the current styled run and honor line breaks even at style boundaries.
    fn add(&mut self, item: &mut Item, tall: f32) {
        if self.out.is_empty() {
            if self.newline {
                self.newline = false;
                self.x = 0.;
                self.y += tall;
            }
            return;
        }
        let unit = WorkUnit {
            text: self.out.iter().collect(),
            color: self.color,
            style: Style::of(self.bold, self.italic),
            x: self.x,
            y: self.y,
            width: self.width,
            height: tall,
        };
        item.height = item.height.max(self.y + tall);
        item.width = item.width.max(self.x + self.width);
        item.work.get_or_insert_with(Vec::new).push(unit);
        self.out.clear();
        self.x += self.width;
        self.width = 0.;
        if self.newline {
            self.newline = false;
            self.x = 0.;
            self.y += tall;
        }
    }
}

/// ComputeStreamWork: color/style commands and wrapping at the last space before
/// `available` pixels.
fn compute_work(item: &mut Item, available: f32, measure: &dyn Measure) {
    let tall = measure.tall();
    let mut params = Params {
        out: Vec::new(),
        x: 0.,
        y: 0.,
        width: 0.,
        bold: false,
        italic: false,
        color: [255; 4],
        newline: false,
    };
    item.work = Some(Vec::new());
    item.width = 0.;
    item.height = 0.;
    let chars: Vec<char> = item.stream.chars().collect();
    let mut stack: Vec<[u8; 4]> = Vec::new();
    let mut space: Option<(usize, f32)> = None;
    let mut i = 0;
    while i < chars.len() {
        if let Some((command, args)) = split_command(&chars, &mut i) {
            match command.as_str() {
                "cr" => {
                    params.newline = true;
                    params.add(item, tall);
                }
                "clr" | "playerclr" => {
                    params.add(item, tall);
                    if args.is_empty() && stack.len() >= 2 {
                        stack.pop();
                        params.color = *stack.last().unwrap();
                    } else if command == "clr" {
                        if let Some([r, g, b]) = components::<3>(&args, &[',']) {
                            stack.push([channel(r), channel(g), channel(b), 255]);
                            params.color = *stack.last().unwrap();
                        }
                    } else if let Some([pr, pg, pb, nr, ng, nb]) =
                        components::<6>(&args, &[',', ':'])
                    {
                        let color = if item.from_player {
                            [channel(pr), channel(pg), channel(pb), 255]
                        } else {
                            [channel(nr), channel(ng), channel(nb), 255]
                        };
                        stack.push(color);
                        params.color = color;
                    }
                }
                "I" => {
                    params.add(item, tall);
                    params.italic = !params.italic;
                }
                "B" => {
                    params.add(item, tall);
                    params.bold = !params.bold;
                }
                _ => {}
            }
            i += 1;
            continue;
        }
        let c = chars[i];
        let style = Style::of(params.bold, params.italic);
        let w = measure.width(style, &c.to_string());
        if params.x + params.width + w > available {
            match space {
                // Roll back to the previous space when it is still in this chunk.
                Some((at, at_width)) if i > at && i - at <= params.out.len() => {
                    let carried: Vec<char> = chars[at + 1..i].to_vec();
                    params.out.truncate(params.out.len() - (i - at));
                    params.width = at_width;
                    params.newline = true;
                    params.add(item, tall);
                    params.width = measure.width(style, &carried.iter().collect::<String>());
                    params.out = carried;
                    space = None;
                }
                _ => {
                    params.newline = true;
                    params.add(item, tall);
                }
            }
        }
        params.out.push(c);
        params.width += w;
        if c.is_whitespace() {
            space = Some((i, params.width));
        }
        i += 1;
    }
    params.newline = true;
    params.add(item, tall);
}

/// One text run to draw, in panel pixels relative to the panel's top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub style: Style,
    pub color: [f32; 4],
    /// Untranslated (`!!!`) captions draw over this box.
    pub highlight: Option<[f32; 4]>,
}

/// What [`CloseCaption::paint`] laid out: the background box and the text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paint {
    /// x, y, width, height relative to the panel, and the RGBA color.
    pub background: Option<([f32; 4], [f32; 4])>,
    pub runs: Vec<Run>,
}

/// The caption queue and box animation (CHudCloseCaption).
#[derive(Clone, Debug, Default)]
pub struct CloseCaption {
    items: Vec<Item>,
    repeats: HashMap<String, Repeat>,
    work_metrics: Option<(f32, f32)>,
    goal_height: f32,
    current_height: f32,
    goal_alpha: f32,
    current_alpha: f32,
    goal_start: f64,
    goal_finish: f64,
    /// Ticks seen (the same-tick repeat rule).
    tick_count: u64,
    /// Tokens with no caption, reported once.
    pub missing: Vec<String>,
    /// Captions processed (shown or queued).
    pub processed: u64,
    /// The latest processed streams, oldest first, for run reports.
    pub recent: Vec<String>,
}

impl CloseCaption {
    pub fn new() -> Self {
        Self {
            goal_alpha: 1.,
            current_alpha: 1.,
            ..Self::default()
        }
    }

    /// LevelShutdown/Reset: drop every item.
    pub fn reset(&mut self) {
        self.items.clear();
    }

    /// LevelInit: forget repeat timers.
    pub fn level_init(&mut self) {
        self.repeats.clear();
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The streams currently queued, oldest first.
    pub fn streams(&self) -> impl Iterator<Item = &str> {
        self.items.iter().map(|i| i.stream.as_str())
    }

    /// A CloseCaption message for `token` (MsgFunc_CloseCaption / ProcessCaption): the
    /// gendered `<token>_male` / `_female` first, then the token itself, looked up in
    /// `lookup` (the first dictionary holding it). Returns whether a caption was found.
    pub fn emit(
        &mut self,
        lookup: &dyn Fn(&str) -> Option<String>,
        caption: &Request,
        settings: &Settings,
        time: f64,
    ) -> bool {
        let token = caption.token.to_ascii_lowercase();
        let gendered = caption.gender.map(|g| format!("{token}_{g}"));
        let found = gendered
            .iter()
            .chain(std::iter::once(&token))
            .find_map(|t| lookup(t).map(|text| (t.clone(), text)));
        let Some((token, text)) = found else {
            if self.missing.len() < 64 && !self.missing.contains(&token) {
                self.missing.push(token);
            }
            return false;
        };
        self.process_caption(
            &text,
            &token,
            caption.duration,
            caption.from_player,
            settings,
            time,
        );
        true
    }

    /// _ProcessCaption: `<norepeat:N>` keeps a token from showing again within N seconds;
    /// without it, a token is not shown twice in one tick.
    fn process_caption(
        &mut self,
        text: &str,
        token: &str,
        duration: f32,
        from_player: bool,
        settings: &Settings,
        time: f64,
    ) {
        let norepeat = float_command(text, "norepeat");
        let tick = self.tick_count;
        match self.repeats.get_mut(token) {
            None => {
                self.repeats.insert(
                    token.to_owned(),
                    Repeat {
                        last_time: time,
                        last_tick: tick,
                        interval: norepeat.unwrap_or(0.),
                    },
                );
            }
            Some(entry) => {
                if entry.interval <= 0. {
                    if tick <= entry.last_tick {
                        return;
                    }
                } else if norepeat.is_some() && time < entry.last_time + f64::from(entry.interval) {
                    return;
                }
                entry.last_time = time;
                entry.last_tick = tick;
            }
        }
        self.process(text, duration, from_player, settings);
    }

    /// Process: split the stream at `<delay>` into items with their lifespans.
    pub fn process(&mut self, stream: &str, duration: f32, from_player: bool, settings: &Settings) {
        if !settings.closecaption {
            self.reset();
            return;
        }
        if stream.trim().is_empty() {
            return;
        }
        if settings.subtitles && has_command(stream, "sfx") {
            return;
        }
        let valid = !stream.starts_with("!!!");
        if stream.starts_with("-->") {
            // In the file but not translated (cc_captiontrace 1 only reports it).
            return;
        }
        self.processed += 1;
        if self.recent.len() == 16 {
            self.recent.remove(0);
        }
        self.recent.push(stream.to_owned());
        let mut lifespan = duration + settings.linger_time;
        let mut added = 0.;
        if let Some(last) = self.items.last() {
            let previous = last.time_to_live;
            if previous > lifespan {
                added = previous - lifespan;
            }
            lifespan = lifespan.max(previous);
        }
        let chars: Vec<char> = stream.chars().take(MAX_CHARACTERS - 1).collect();
        let mut phrase = String::new();
        let mut delay = 0.;
        let mut i = 0;
        while i < chars.len() {
            let start = i;
            if let Some((command, args)) = split_command(&chars, &mut i) {
                if command == "delay" {
                    self.add_phrase(
                        &phrase,
                        lifespan,
                        added,
                        delay,
                        valid,
                        from_player,
                        settings,
                        false,
                    );
                    phrase.clear();
                    delay = leading_float(&args).max(0.);
                    i += 1;
                    continue;
                }
                phrase.extend(&chars[start..i]);
            }
            if let Some(&c) = chars.get(i) {
                phrase.push(c);
            }
            i += 1;
        }
        self.add_phrase(
            &phrase,
            lifespan,
            added,
            delay,
            valid,
            from_player,
            settings,
            true,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn add_phrase(
        &mut self,
        phrase: &str,
        lifespan: f32,
        added: f32,
        delay: f32,
        valid: bool,
        from_player: bool,
        settings: &Settings,
        last: bool,
    ) {
        if phrase.is_empty() {
            return;
        }
        let mut item = Item {
            stream: phrase.to_owned(),
            time_to_live: lifespan,
            initial_life_span: lifespan,
            added_time: added,
            predisplay: settings.predisplay_time + delay,
            valid,
            from_player,
            work: None,
            width: 0.,
            height: 0.,
        };
        if has_command(phrase, "sfx") {
            // SFX show up instantly.
            item.predisplay = 0.;
        }
        if let Some(length) = float_command(phrase, "len") {
            item.time_to_live = length;
            // Only the final phrase also resets its initial life span (SDK).
            if last {
                item.initial_life_span = length;
            }
        }
        self.items.push(item);
    }

    /// OnTick: count down the predisplay delay, then the life of shown items, and remove
    /// expired items from the head until the first one still alive.
    pub fn tick(&mut self, dt: f32) {
        self.tick_count += 1;
        for item in &mut self.items {
            if item.predisplay > 0. {
                item.predisplay = (item.predisplay - dt).max(0.);
            } else {
                item.time_to_live = (item.time_to_live - dt).max(0.);
            }
        }
        let mut alive = false;
        self.items.retain(|item| {
            if item.predisplay > 0. {
                return true;
            }
            if item.time_to_live > 0. {
                alive = true;
                return true;
            }
            alive
        });
    }
}

/// A caption to show: retail's CloseCaption user message (token, duration, flags).
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub token: String,
    /// Seconds, already quantized like the message ([`message_duration`]).
    pub duration: f32,
    pub from_player: bool,
    /// `male` / `female` for gendered actors (CLOSE_CAPTION_GENDER_*).
    pub gender: Option<&'static str>,
}

/// The `ClientScheme.res` caption fonts, in [`Style`] order.
const FONT_NAMES: [(Style, &str); 4] = [
    (Style::Normal, "CloseCaption_Normal"),
    (Style::Italic, "CloseCaption_Italic"),
    (Style::Bold, "CloseCaption_Bold"),
    (Style::BoldItalic, "CloseCaption_BoldItalic"),
];
/// The installed schemes ask for Tahoma (Verdana on macOS), which the game does not ship;
/// they register this family's regular, bold, oblique and bold oblique files, so the faces
/// are drawn from those.
const REGISTERED_SUBSTITUTE: &str = "DejaVu Sans";

/// One scheme font definition (`Fonts/<name>/1`).
#[derive(Clone, Debug, PartialEq)]
struct FontDefinition {
    name: String,
    tall: f32,
    weight: u16,
    italic: bool,
}

fn font_definition(scheme: &source_assets::keyvalues::Entry, name: &str) -> Option<FontDefinition> {
    let definition = scheme.get("Fonts")?.get(name)?.children().first()?;
    let value = |key: &str| {
        definition
            .get(key)
            .and_then(source_assets::keyvalues::Entry::text)
            .map(str::trim)
    };
    Some(FontDefinition {
        name: value("name")?.to_owned(),
        tall: value("tall").and_then(|t| t.parse().ok()).unwrap_or(12.),
        weight: value("weight").and_then(|w| w.parse().ok()).unwrap_or(0),
        italic: value("italic").is_some_and(|v| v != "0"),
    })
}

/// A registered font file: family names, weight and slant.
struct FontFile {
    bytes: Vec<u8>,
    info: source_ui::scheme_fonts::FontFileInfo,
    italic: bool,
}

/// The registered file drawn for `font`: its family (else [`REGISTERED_SUBSTITUTE`]), the
/// matching slant, then the closest weight.
fn pick_file<'a>(files: &'a [FontFile], font: &FontDefinition) -> Option<&'a FontFile> {
    let family = |name: &str| {
        files
            .iter()
            .filter(|f| f.info.families.iter().any(|n| n.eq_ignore_ascii_case(name)))
            .collect::<Vec<_>>()
    };
    let mut candidates = family(&font.name);
    if candidates.is_empty() {
        candidates = family(REGISTERED_SUBSTITUTE);
    }
    let wanted = if font.weight == 0 { 400 } else { font.weight };
    candidates
        .into_iter()
        .min_by_key(|f| (f.italic != font.italic, f.info.weight.abs_diff(wanted)))
}

/// The caption panel of the HUD: dictionaries, layout, fonts and the caption queue.
pub struct CaptionHud {
    pub captions: source_assets::captions::Captions,
    pub layout: Layout,
    /// `Panel.BgColor` of the scheme; its alpha is replaced by `BgAlpha`.
    background: [f32; 4],
    /// Faces in [`Style`] order with their proportional tall (480-line units).
    fonts: Vec<(Style, source_ui::font::FontFace, f32)>,
    pub state: source_ui::canvas::MutexCell<CloseCaption>,
    clock: source_ui::canvas::MutexCell<Option<f64>>,
    pub errors: Vec<String>,
}

impl CaptionHud {
    /// Reads the mounted game's caption dictionaries for `language`, its `HudLayout.res`
    /// `HudCloseCaption` and `ClientScheme.res` fonts.
    pub fn load(
        vfs: &source_assets::vpk::Vfs,
        scheme: &source_assets::keyvalues::Entry,
        layout: &source_assets::keyvalues::Entry,
        canvas: source_ui::canvas::Canvas,
        language: &str,
    ) -> Self {
        let mut errors = Vec::new();
        let captions =
            source_assets::captions::Captions::load(vfs, language).unwrap_or_else(|error| {
                errors.push(format!("caption dictionaries: {error:#}"));
                Default::default()
            });
        errors.extend(captions.errors.iter().cloned());
        let files: Vec<FontFile> = source_ui::scheme_fonts::custom_font_files(scheme)
            .iter()
            .filter_map(|path| vfs.read(path).ok().flatten())
            .filter_map(|bytes| {
                let info = source_ui::scheme_fonts::FontFileInfo::read(&bytes)?;
                let face = ttf_parser::Face::parse(&bytes, 0).ok()?;
                let italic = face.is_italic() || face.is_oblique();
                Some(FontFile {
                    bytes,
                    info,
                    italic,
                })
            })
            .collect();
        let mut fonts = Vec::new();
        for (style, name) in FONT_NAMES {
            let Some(definition) = font_definition(scheme, name) else {
                errors.push(format!("scheme font {name} missing"));
                continue;
            };
            let Some(file) = pick_file(&files, &definition) else {
                errors.push(format!("no registered font file for {name}"));
                continue;
            };
            match source_ui::font::FontFace::load(&file.bytes, canvas.clone()) {
                Ok(face) => fonts.push((style, face, definition.tall)),
                Err(error) => errors.push(format!("{name}: {error:#}")),
            }
        }
        let background = scheme
            .get("Colors")
            .and_then(|c| c.get("Panel.BgColor"))
            .or_else(|| {
                scheme
                    .get("BaseSettings")
                    .and_then(|c| c.get("Panel.BgColor"))
            })
            .and_then(source_assets::keyvalues::Entry::text)
            .and_then(scheme_color)
            .unwrap_or([0., 0., 0., 76. / 255.]);
        Self {
            captions,
            layout: layout
                .get("HudCloseCaption")
                .map(Layout::read)
                .unwrap_or_default(),
            background,
            fonts,
            state: source_ui::canvas::MutexCell::new(CloseCaption::new()),
            clock: source_ui::canvas::MutexCell::new(None),
            errors,
        }
    }

    /// Shows the requested captions when captions are enabled (retail culls the messages of
    /// players without `closecaption`).
    pub fn emit(&self, requests: &[Request], settings: &Settings, time: f64) {
        if !settings.closecaption {
            return;
        }
        let lookup = |token: &str| self.captions.get(token).map(str::to_owned);
        let mut state = self.state.borrow_mut();
        for request in requests {
            state.emit(&lookup, request, settings, time);
        }
    }

    /// Advances the queue on the game clock (so a paused game holds its captions).
    pub fn tick(&self, time: f64) {
        let mut clock = self.clock.borrow_mut();
        let dt = clock.map_or(0., |last| (time - last).max(0.)) as f32;
        *clock = Some(time);
        self.state.borrow_mut().tick(dt);
    }

    /// Lays out the panel for a canvas `size` in pixels; None while there is nothing to draw
    /// or the fonts are missing.
    pub fn paint(&self, size: (f32, f32), time: f64) -> Option<(source_ui::canvas::Rect, Paint)> {
        let canvas = glam::vec2(size.0, size.1);
        let scale = crate::proportional::scale(canvas);
        let measure = self.measure(scale)?;
        let x = crate::hud::panel_axis(&self.layout.xpos, size.0, scale).unwrap_or(0.);
        let rect = source_ui::canvas::Rect::new(
            x.trunc(),
            (self.layout.ypos * crate::proportional::y_scale(canvas)).trunc(),
            (self.layout.wide * scale).trunc(),
            (self.layout.tall * scale).trunc(),
        );
        let paint = self.state.borrow_mut().paint(
            (rect.w, rect.h),
            scale,
            &self.layout,
            &measure,
            self.background,
            time,
        );
        Some((rect, paint))
    }

    fn measure(&self, scale: f32) -> Option<Fonts<'_>> {
        let normal = self.fonts.iter().find(|(s, ..)| *s == Style::Normal)?;
        Some(Fonts {
            fonts: &self.fonts,
            tall: (normal.2 * scale).trunc().max(1.),
        })
    }

    /// The face drawing `style` at the panel's font height.
    pub(crate) fn face(
        &self,
        style: Style,
        scale: f32,
    ) -> Option<(&source_ui::font::FontFace, f32)> {
        let measure = self.measure(scale)?;
        Some((measure.face(style), measure.tall))
    }
}

/// `r g b a` scheme colors.
fn scheme_color(text: &str) -> Option<[f32; 4]> {
    let values: Vec<f32> = text
        .split_whitespace()
        .map(|v| v.parse::<u8>().ok().map(|v| f32::from(v) / 255.))
        .collect::<Option<_>>()?;
    values.try_into().ok()
}

/// The loaded caption faces as a [`Measure`].
struct Fonts<'a> {
    fonts: &'a [(Style, source_ui::font::FontFace, f32)],
    tall: f32,
}

impl<'a> Fonts<'a> {
    fn face(&self, style: Style) -> &'a source_ui::font::FontFace {
        self.fonts
            .iter()
            .find(|(s, ..)| *s == style)
            .or_else(|| self.fonts.iter().find(|(s, ..)| *s == Style::Normal))
            .map(|(_, face, _)| face)
            .expect("normal caption face")
    }
}

impl Measure for Fonts<'_> {
    fn tall(&self) -> f32 {
        self.tall
    }
    fn width(&self, style: Style, text: &str) -> f32 {
        self.face(style).width_cell(text, self.tall)
    }
}

#[cfg(test)]
mod tests;
