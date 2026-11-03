//! Resource-driven chapter browser; no map tables or filesystem work during input/drawing.
mod layout;
mod resources;

use super::{localize::Localization, resources::Image};
use source_ui::layout::Layout;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub map: String,
    pub heading: String,
    pub title: String,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub thumbnails: Vec<Option<Image>>,
    pub commentary_icon: Option<Image>,
    pub dialog: Layout,
    pub panel: Layout,
    pub selection: Option<source_ui::canvas::Color>,
    pub fill: Option<source_ui::canvas::Color>,
    pub text: Option<source_ui::canvas::Color>,
    pub font_tall: Option<f32>,
    pub title: String,
}

/// Retail discovers MOD chapter configs, not the chapter background list. Reject malformed
/// keys and configs without a map; numeric order keeps chapter9a before chapter10.
pub fn catalog<'a>(
    files: impl IntoIterator<Item = &'a str>,
    read: impl Fn(&str) -> Option<String>,
    game: &str,
    loc: &Localization,
) -> Vec<Entry> {
    let mut entries: Vec<_> = files
        .into_iter()
        .filter_map(|file| {
            let key = file.strip_prefix("cfg/chapter")?.strip_suffix(".cfg")?;
            let digits = key.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || !key[digits..].bytes().all(|c| c.is_ascii_alphabetic()) {
                return None;
            }
            let map = super::resources::chapter_map(&read(file)?)?;
            Some(Entry {
                key: key.to_owned(),
                map,
                heading: format!("{} {key}", loc.text("#GameUI_Chapter")),
                title: loc.text(&format!("#{game}_Chapter{key}_Title")),
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        let number = |key: &str| {
            key.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u32>()
                .unwrap_or(u32::MAX)
        };
        number(&a.key)
            .cmp(&number(&b.key))
            .then_with(|| a.key.len().cmp(&b.key.len()))
            .then_with(|| a.key.cmp(&b.key))
    });
    entries.dedup_by(|a, b| a.key == b.key);
    entries
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Close,
    Start(crate::console::GameMap),
}

/// HL2 chapter metadata and typed map effects over the reusable toolkit dialog.
pub struct Dialog {
    pub game: String,
    entries: Vec<Entry>,
    pub browser: source_ui::card_browser::CardBrowser,
}
impl Dialog {
    pub fn new(game: &str, catalog: &Catalog, loc: &Localization) -> Self {
        Self {
            game: game.to_owned(),
            entries: catalog.entries.clone(),
            browser: source_ui::card_browser::CardBrowser::new(
                catalog.browser_spec(loc),
                crate::proportional::SCALING,
            ),
        }
    }
    pub fn input(&mut self, input: &source_ui::input::Input, canvas: glam::Vec2) -> Outcome {
        use source_ui::card_browser::Outcome as BrowserOutcome;
        match self.browser.input(input, canvas) {
            BrowserOutcome::None => Outcome::None,
            BrowserOutcome::Close => Outcome::Close,
            BrowserOutcome::Accept(index) => {
                self.entries.get(index).map_or(Outcome::None, |entry| {
                    Outcome::Start(crate::console::GameMap::new(&self.game, &entry.map))
                })
            }
        }
    }
    pub fn commentary_enabled(&self) -> bool {
        self.browser.toggle_checked
    }
    pub fn report(&self, canvas: glam::Vec2) -> serde_json::Value {
        let b = self.browser.bounds(canvas);
        serde_json::json!({"game":self.game,"selected":self.browser.selected,"first":self.browser.first,
            "commentary":self.browser.toggle_checked,
            "frame":[b.x,b.y,b.w,b.h],"entries":self.entries.iter().map(|e| {
                serde_json::json!({"key":e.key,"map":e.map,"heading":e.heading,"title":e.title})
            }).collect::<Vec<_>>(),"thumbnails":self.browser.thumbnail_count()})
    }
}

#[cfg(test)]
mod tests;
