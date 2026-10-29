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
    fade_start: f64,
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

/// Bias(x, amount) from mathlib.
fn bias(x: f32, amount: f32) -> f32 {
    x.powf(amount.ln() / 0.5f32.ln())
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
    /// AddWorkUnit: an empty chunk adds nothing and keeps the position and newline flag.
    fn add(&mut self, item: &mut Item, tall: f32) {
        if self.out.is_empty() {
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
            fade_start: 0.,
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

    /// Paint for a panel of `size` pixels at proportional `scale` (screen height / 480) on
    /// the game clock `time`.
    pub fn paint(
        &mut self,
        size: (f32, f32),
        scale: f32,
        layout: &Layout,
        measure: &dyn Measure,
        background: [f32; 4],
        time: f64,
    ) -> Paint {
        let (w, h) = size;
        let window_top = layout.top_offset;
        let window_bottom = h;
        let inset = (INSET * scale).floor();
        let available_width = w - 2. * inset;
        let available_height = h - window_top - 2. * inset;
        let line_height = measure.tall().max(6.);
        let mut total = 0.;
        let mut visible: Vec<usize> = Vec::new();
        for index in 0..self.items.len() {
            if self.items[index].predisplay > 0. {
                continue;
            }
            if self.items[index].work.is_none() {
                compute_work(&mut self.items[index], available_width, measure);
            }
            let height = self.items[index].height;
            total += height;
            visible.push(index);
            // Start popping really old items off the stack when out of space.
            while height <= available_height
                && total > available_height
                && visible.len() > MIN_VISIBLE_ITEMS
            {
                let popped = visible.remove(0);
                total -= self.items[popped].height;
                self.items[popped].time_to_live = 0.;
            }
        }
        let desired_alpha = if visible.is_empty() { 0. } else { 1. };
        let total = f32::max(total, line_height);
        if total != self.goal_height {
            self.goal_height = total;
            self.goal_start = time;
            self.goal_finish = time + f64::from(layout.grow_time);
        }
        if desired_alpha != self.goal_alpha {
            self.goal_alpha = desired_alpha;
            self.goal_start = time;
            self.goal_finish = time + f64::from(layout.grow_time);
        }
        if visible.is_empty()
            && self.goal_height == self.current_height
            && self.goal_alpha == self.current_alpha
        {
            self.goal_start = 0.;
            self.goal_finish = 0.;
            return Paint::default();
        }
        let mut growing_down = false;
        if self.goal_finish != 0. && self.goal_start != 0. && self.goal_finish > self.goal_start {
            let to_go = self.goal_height - self.current_height;
            let alpha_to_go = self.goal_alpha - self.current_alpha;
            growing_down = to_go < 0.;
            let frac = ((time - self.goal_start) / (self.goal_finish - self.goal_start))
                .clamp(0., 1.) as f32;
            self.current_height += (frac * to_go).trunc();
            self.current_alpha = (self.current_alpha + frac * alpha_to_go).clamp(0., 1.);
        } else {
            self.current_height = self.goal_height;
            self.current_alpha = self.goal_alpha;
        }
        let mut paint = Paint::default();
        let bottom = h;
        let top = bottom - self.current_height - 2. * inset;
        let box_top = top.max(0.);
        let mut color = background;
        color[3] = layout.bg_alpha / 255. * self.current_alpha;
        paint.background = Some(([0., box_top, w, bottom - box_top], color));
        if visible.is_empty() {
            return paint;
        }
        let left = inset;
        let text_height = if growing_down {
            total
        } else {
            self.current_height
        };
        let mut text_top = bottom - text_height - inset;
        for (position, &index) in visible.iter().enumerate() {
            let item = &mut self.items[index];
            // The oldest item drops the extra time it was created with.
            if position == 0 && item.added_time > 0. {
                item.time_to_live = (item.time_to_live - item.added_time).max(0.);
                item.added_time = 0.;
            }
            let height = item.height;
            let mut fade_line = None;
            let mut fade_line_alpha = 1.;
            if height > available_height {
                // Pan slowly over an item taller than the panel.
                let work = item.work.as_mut().unwrap();
                let mut units = work.len();
                let mut moved = 0.;
                for (j, unit) in work.iter().enumerate() {
                    moved += unit.height;
                    if moved >= height - available_height {
                        units = j + 1;
                        break;
                    }
                }
                let move_delta = item.initial_life_span / units as f32;
                let current_move = item.initial_life_span - item.time_to_live;
                let mut height_to_move = 0.;
                let lines =
                    ((current_move / move_delta).floor() as i64).clamp(0, units as i64) as usize;
                if lines > 0 {
                    let mut current_line_height = 0.;
                    for unit in work.iter().take(lines) {
                        height_to_move = current_line_height;
                        current_line_height += unit.height;
                    }
                    let mut post_move = current_move - move_delta * lines as f32;
                    if post_move < PAN_FADE_TIME {
                        fade_line = Some(lines - 1);
                        let unit = &mut work[lines - 1];
                        if unit.fade_start == 0. {
                            unit.fade_start = time;
                        }
                        let fade = ((time - unit.fade_start) as f32) / PAN_FADE_TIME;
                        fade_line_alpha = (1. - fade).clamp(0., 1.);
                    } else if post_move < PAN_FADE_TIME + PAN_SLIDE_TIME {
                        post_move -= PAN_FADE_TIME;
                        let slide = (post_move / 0.25).clamp(0., 1.);
                        height_to_move += ((current_line_height - height_to_move) * slide).ceil();
                    } else {
                        height_to_move = current_line_height;
                    }
                }
                text_top = -height_to_move + 2.;
            }
            let alpha = item.alpha(
                layout.item_hidden_time,
                layout.item_fade_in_time,
                layout.item_fade_out_time,
            );
            for (j, unit) in item.work.as_ref().unwrap().iter().enumerate() {
                let x = left + unit.x + 3.;
                let y = text_top + unit.y;
                let mut line_alpha = alpha;
                if fade_line == Some(j) {
                    line_alpha *= fade_line_alpha;
                } else if y < window_top {
                    continue;
                } else if y > window_bottom {
                    let distance = bias((y - window_bottom) / (unit.height * 0.25), 0.2);
                    if distance > 1. {
                        continue;
                    }
                    line_alpha *= 1. - distance;
                }
                let rgba = |c: [u8; 4], a: f32| {
                    [
                        f32::from(c[0]) / 255.,
                        f32::from(c[1]) / 255.,
                        f32::from(c[2]) / 255.,
                        f32::from(c[3]) / 255. * a,
                    ]
                };
                let (color, highlight) = if item.valid {
                    (rgba(unit.color, line_alpha), None)
                } else {
                    (
                        rgba([255; 4], line_alpha),
                        Some(rgba([100, 100, 40, 255], line_alpha)),
                    )
                };
                paint.runs.push(Run {
                    text: unit.text.clone(),
                    x,
                    y,
                    width: unit.width + if item.valid { 0. } else { 2. },
                    height: unit.height,
                    style: unit.style,
                    color,
                    highlight,
                });
            }
            text_top += height;
            if text_top >= window_bottom {
                break;
            }
        }
        paint
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
mod tests {
    use super::*;

    /// Every character is 6 pixels wide (bold 7), lines are 12 tall.
    struct Fixed;
    impl Measure for Fixed {
        fn tall(&self) -> f32 {
            12.
        }
        fn width(&self, style: Style, text: &str) -> f32 {
            let w = if matches!(style, Style::Bold | Style::BoldItalic) {
                7.
            } else {
                6.
            };
            text.chars().count() as f32 * w
        }
    }

    fn on() -> Settings {
        Settings {
            closecaption: true,
            ..Settings::default()
        }
    }

    fn item(stream: &str, from_player: bool) -> Item {
        Item {
            stream: stream.into(),
            time_to_live: 1.,
            initial_life_span: 1.,
            added_time: 0.,
            predisplay: 0.,
            valid: true,
            from_player,
            work: None,
            width: 0.,
            height: 0.,
        }
    }

    #[test]
    fn options_setting_maps_to_the_caption_cvars() {
        use crate::config::Captions;
        let none = Settings::from_config(Captions::None);
        assert!(!none.closecaption);
        let all = Settings::from_config(Captions::ClosedCaptions);
        assert!(all.closecaption && !all.subtitles);
        let subtitles = Settings::from_config(Captions::Subtitles);
        assert!(subtitles.closecaption && subtitles.subtitles);
        assert_eq!((all.linger_time, all.predisplay_time), (1., 0.25));
    }

    #[test]
    fn commands_split_like_the_sdk() {
        let chars: Vec<char> = "<clr:255,255,200>Hi".chars().collect();
        let mut i = 0;
        assert_eq!(
            split_command(&chars, &mut i),
            Some(("clr".into(), "255,255,200".into()))
        );
        assert_eq!(chars[i], '>');
        assert_eq!(
            commands("<sfx><norepeat:30>[Gunfire]<len:2.5>"),
            [
                ("sfx".to_string(), String::new()),
                ("norepeat".into(), "30".into()),
                ("len".into(), "2.5".into())
            ]
        );
        assert_eq!(float_command("<len:2.5s>", "len"), Some(2.5));
        assert_eq!(float_command("x", "len"), None);
        assert_eq!(leading_float("-3"), -3.);
    }

    #[test]
    fn items_get_duration_plus_linger_and_wait_for_the_predisplay_delay() {
        let mut cc = CloseCaption::new();
        cc.process("<clr:255,255,200><I>Ahem...is this on?", 4., false, &on());
        assert_eq!(cc.items.len(), 1);
        let item = &cc.items[0];
        assert_eq!(item.time_to_live, 5.);
        assert_eq!(item.predisplay, 0.25);
        cc.tick(0.2);
        assert_eq!(cc.items[0].time_to_live, 5., "still waiting");
        cc.tick(0.1);
        cc.tick(1.);
        assert!((cc.items[0].time_to_live - 4.).abs() < 1e-5);
        for _ in 0..5 {
            cc.tick(1.);
        }
        assert!(cc.is_empty(), "expired");
    }

    #[test]
    fn delay_splits_phrases_and_len_overrides_the_life() {
        let mut cc = CloseCaption::new();
        cc.process("First.<delay:2>Second.<len:3>", 1., false, &on());
        let streams: Vec<_> = cc.streams().collect();
        assert_eq!(streams, ["First.", "Second.<len:3>"]);
        assert_eq!(cc.items[0].predisplay, 0.25);
        assert_eq!(cc.items[1].predisplay, 2.25);
        assert_eq!(cc.items[1].time_to_live, 3.);
        assert_eq!(cc.items[1].initial_life_span, 3.);
        // A later, shorter caption lives as long as the previous one with the difference
        // marked as added time.
        cc.process("Third.", 0.5, false, &on());
        assert_eq!(cc.items[2].time_to_live, 3.);
        assert_eq!(cc.items[2].added_time, 1.5);
    }

    #[test]
    fn subtitles_drop_sound_effects_and_disabled_captions_clear_the_queue() {
        let mut cc = CloseCaption::new();
        cc.process("<sfx>[Explosion]", 1., false, &on());
        assert_eq!(cc.items[0].predisplay, 0., "sfx show instantly");
        let subtitles = Settings {
            subtitles: true,
            ..on()
        };
        cc.process("<sfx>[Gunfire]", 1., false, &subtitles);
        assert_eq!(cc.len(), 1);
        cc.process("Voice.", 1., false, &subtitles);
        assert_eq!(cc.len(), 2);
        cc.process("Voice.", 1., false, &Settings::default());
        assert!(cc.is_empty());
        cc.process("-->untranslated", 1., false, &on());
        cc.process("   ", 1., false, &on());
        assert!(cc.is_empty());
        cc.process("!!!pending", 1., false, &on());
        assert!(!cc.items[0].valid);
    }

    #[test]
    fn norepeat_and_same_tick_repeats_are_suppressed() {
        let mut cc = CloseCaption::new();
        let lookup = |token: &str| match token {
            "barney.yell" => Some("<norepeat:10>Hey!".to_owned()),
            "alyx.hi" => Some("Hi.".to_owned()),
            "alyx.hi_female" => Some("Hi there.".to_owned()),
            _ => None,
        };
        let request = |token: &str| Request {
            token: token.into(),
            duration: 1.,
            from_player: false,
            gender: None,
        };
        assert!(cc.emit(&lookup, &request("Barney.Yell"), &on(), 1.));
        assert!(cc.emit(&lookup, &request("barney.yell"), &on(), 5.));
        assert_eq!(cc.len(), 1, "within the norepeat interval");
        cc.tick(0.);
        assert!(cc.emit(&lookup, &request("barney.yell"), &on(), 11.5));
        assert_eq!(cc.len(), 2);
        cc.emit(&lookup, &request("alyx.hi"), &on(), 12.);
        cc.emit(&lookup, &request("alyx.hi"), &on(), 12.);
        assert_eq!(cc.len(), 3, "same tick");
        cc.tick(0.);
        cc.emit(
            &lookup,
            &Request {
                gender: Some("female"),
                ..request("alyx.hi")
            },
            &on(),
            12.1,
        );
        assert_eq!(cc.streams().last(), Some("Hi there."));
        assert!(!cc.emit(&lookup, &request("missing.token"), &on(), 13.));
        assert_eq!(cc.missing, ["missing.token"]);
    }

    #[test]
    fn work_units_follow_color_style_and_wrap_at_spaces() {
        let mut unit = item("<clr:255,255,200><I>aaa bbb ccc<B>dd", false);
        compute_work(&mut unit, 50., &Fixed);
        let work = unit.work.unwrap();
        let texts: Vec<_> = work.iter().map(|w| w.text.as_str()).collect();
        // 50 pixels hold eight 6-pixel characters: "aaa bbb " then wrap at the space.
        assert_eq!(texts, ["aaa bbb", "ccc", "dd"]);
        assert_eq!(work[0].color, [255, 255, 200, 255]);
        assert_eq!(work[0].style, Style::Italic);
        assert_eq!((work[0].y, work[1].y), (0., 12.));
        assert_eq!(work[0].width, 48., "the trailing space keeps its width");
        assert_eq!(work[2].style, Style::BoldItalic);
        assert_eq!(work[2].x, 18.);
        assert_eq!(unit.height, 24.);
    }

    #[test]
    fn color_stack_pops_and_player_colors_pick_the_speaker() {
        let mut unit = item("<clr:1,2,3>a<clr:4,5,6>b<clr>c<cr>d", false);
        compute_work(&mut unit, 500., &Fixed);
        let work = unit.work.unwrap();
        let colors: Vec<_> = work.iter().map(|w| (w.text.as_str(), w.color)).collect();
        assert_eq!(
            colors,
            [
                ("a", [1, 2, 3, 255]),
                ("b", [4, 5, 6, 255]),
                ("c", [1, 2, 3, 255]),
                ("d", [1, 2, 3, 255])
            ]
        );
        assert_eq!(work[3].y, 12., "<cr> starts a new line");
        for (from_player, expected) in [(true, [9, 9, 9, 255]), (false, [7, 7, 7, 255])] {
            let mut unit = item("<playerclr:9,9,9:7,7,7>x", from_player);
            compute_work(&mut unit, 500., &Fixed);
            assert_eq!(unit.work.unwrap()[0].color, expected);
        }
    }

    #[test]
    fn paint_grows_the_box_and_fades_the_item_in() {
        let layout = Layout {
            top_offset: 0.,
            bg_alpha: 128.,
            ..Layout::default()
        };
        let mut cc = CloseCaption::new();
        cc.process("<clr:255,255,200>Hello", 2., false, &on());
        cc.tick(0.3);
        let background = [0., 0., 0., 76. / 255.];
        let paint = cc.paint((500., 136.), 1., &layout, &Fixed, background, 10.);
        // Growth starts at the old height (0); the box starts opaque (current alpha 1).
        let (rect, color) = paint.background.unwrap();
        assert_eq!(rect, [0., 124., 500., 12.]);
        assert!((color[3] - 128. / 255.).abs() < 1e-6);
        let paint = cc.paint((500., 136.), 1., &layout, &Fixed, background, 10.25);
        let (rect, color) = paint.background.unwrap();
        assert_eq!(rect, [0., 112., 500., 24.]);
        assert!((color[3] - 128. / 255.).abs() < 1e-6);
        let run = &paint.runs[0];
        assert_eq!(run.text, "Hello");
        assert_eq!((run.x, run.y), (9., 118.));
        assert_eq!(run.color[3], 0., "hidden during ItemHiddenTime");
        cc.tick(0.3);
        let paint = cc.paint((500., 136.), 1., &layout, &Fixed, background, 10.55);
        let run = &paint.runs[0];
        assert!((run.color[3] - 0.6667).abs() < 1e-3, "{}", run.color[3]);
        assert_eq!(&run.color[..3], &[1., 1., 200. / 255.]);
    }

    #[test]
    fn old_items_are_popped_when_the_panel_is_full() {
        let layout = Layout {
            top_offset: 0.,
            ..Layout::default()
        };
        let mut cc = CloseCaption::new();
        for n in 0..12 {
            cc.process(&format!("line {n}"), 5., false, &on());
        }
        cc.tick(0.3);
        let paint = cc.paint((500., 136.), 1., &layout, &Fixed, [0.; 4], 1.);
        // 136 - 2 * 6 = 124 pixels hold ten 12-pixel lines.
        assert_eq!(cc.items.iter().filter(|i| i.time_to_live == 0.).count(), 2);
        assert!(paint.runs.iter().all(|r| r.text != "line 0"));
        cc.tick(0.01);
        assert_eq!(cc.len(), 10);
    }

    #[test]
    fn caption_faces_come_from_registered_files_with_the_requested_slant() {
        let file = |family: &str, weight: u16, italic: bool| FontFile {
            bytes: Vec::new(),
            info: source_ui::scheme_fonts::FontFileInfo {
                families: vec![family.into()],
                weight,
            },
            italic,
        };
        let files = [
            file("HalfLife2", 400, false),
            file("DejaVu Sans", 400, false),
            file("DejaVu Sans", 700, false),
            file("DejaVu Sans", 700, true),
            file("DejaVu Sans", 400, true),
            file("GorDIN", 600, false),
        ];
        let font = |name: &str, weight, italic| FontDefinition {
            name: name.into(),
            tall: 12.,
            weight,
            italic,
        };
        let picked = |definition| {
            let f = pick_file(&files, &definition).unwrap();
            (f.info.families[0].clone(), f.info.weight, f.italic)
        };
        // Tahoma is not installed: the registered substitute family, by slant then weight.
        assert_eq!(
            picked(font("Tahoma", 500, false)),
            ("DejaVu Sans".into(), 400, false)
        );
        assert_eq!(
            picked(font("Tahoma", 500, true)),
            ("DejaVu Sans".into(), 400, true)
        );
        assert_eq!(
            picked(font("Tahoma", 900, false)),
            ("DejaVu Sans".into(), 700, false)
        );
        assert_eq!(
            picked(font("Tahoma", 900, true)),
            ("DejaVu Sans".into(), 700, true)
        );
        // An installed family is used as named, even without an italic file.
        assert_eq!(
            picked(font("GorDIN", 600, true)),
            ("GorDIN".into(), 600, false)
        );
    }

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn owned_episode_one_caption_panel_reads_layout_scheme_and_fonts() {
        let root = source_assets::install::discover().expect("installed game");
        let vfs = source_assets::vpk::Vfs::mount_game(&root, "episodic").unwrap();
        let parse = |path: &str| {
            source_assets::keyvalues::parse_resource(&vfs.read(path).unwrap().unwrap())
                .unwrap()
                .remove(0)
        };
        let scheme = parse("resource/ClientScheme.res");
        let layout = parse("scripts/HudLayout.res");
        let hud = CaptionHud::load(&vfs, &scheme, &layout, Default::default(), LANGUAGE);
        assert!(hud.errors.is_empty(), "{:?}", hud.errors);
        let lengths: Vec<_> = hud.captions.dictionaries.iter().map(|d| d.len()).collect();
        assert_eq!(lengths, [3153, 3970]);
        assert_eq!(
            hud.layout,
            Layout {
                xpos: "c-250".into(),
                ypos: 276.,
                wide: 500.,
                tall: 136.,
                bg_alpha: 128.,
                grow_time: 0.25,
                item_hidden_time: 0.2,
                item_fade_in_time: 0.15,
                item_fade_out_time: 0.3,
                top_offset: 0.,
            }
        );
        assert_eq!(hud.background, [0., 0., 0., 76. / 255.]);
        assert_eq!(hud.fonts.len(), 4);
        assert!(hud.fonts.iter().all(|(_, _, tall)| *tall == 12.));
        // 768 lines: 12 * 1.6 = 19 pixel caption text.
        let (_, tall) = hud.face(Style::Italic, 768. / 480.).unwrap();
        assert_eq!(tall, 19.);
    }

    #[test]
    fn durations_are_quantized_like_the_message() {
        assert!((message_duration(4.36) - 4.3).abs() < 1e-5);
        assert!((message_duration(36.8) - 25.5).abs() < 1e-5);
    }
}
