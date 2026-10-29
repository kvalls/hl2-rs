//! Loads every installed resource the GameUI needs, once, outside frame systems.
//!
//! Search order mirrors the anniversary install: loose files under `hl2_complete/` first (its
//! `gamemenu.res` and `merged_loc_all.txt` define the combined HL2/Episode menu), then the mounted
//! `hl2`/`platform` VFS. The VFS mount itself is not changed, so world/HUD content is unaffected.
//! Missing files leave the matching field empty; callers fall back to token names and built-in
//! menu structure rather than failing.

use super::localize::Localization;
use source_assets::{keyvalues, vpk::Vfs};
use source_ui::{
    keys::{self, KeyRow},
    layout::Layout,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Which game state a menu entry is shown in (`OnlyInGame` / `NotInGame` keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Always,
    OnlyInGame,
    NotInGame,
}

/// One `GameMenu.res` entry. An empty label is a spacer row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub command: String,
    /// `MenuCategory`; `None` for the in-game (pause) entries.
    pub category: Option<String>,
    pub order: i32,
    pub visibility: Visibility,
    /// `Caps`: the label is drawn upper-case.
    pub caps: bool,
}

impl MenuItem {
    pub fn is_spacer(&self) -> bool {
        self.label.is_empty()
    }
}

/// Parses `GameMenu.res`, keeping file order.
pub fn parse_game_menu(bytes: &[u8]) -> Option<Vec<MenuItem>> {
    let roots = keyvalues::parse_resource(bytes).ok()?;
    let root = roots.iter().find(|e| !e.children().is_empty())?;
    let items = root
        .children()
        .iter()
        .filter(|e| !e.children().is_empty())
        .map(|entry| {
            let text = |key: &str| entry.get(key).and_then(|e| e.text()).unwrap_or_default();
            let flag = |key: &str| text(key) == "1";
            MenuItem {
                label: text("label").trim().to_owned(),
                command: text("command").trim().to_owned(),
                category: Some(text("MenuCategory").trim().to_ascii_lowercase())
                    .filter(|c| !c.is_empty()),
                order: text("InGameOrder").trim().parse().unwrap_or(0),
                caps: flag("Caps"),
                visibility: if flag("OnlyInGame") {
                    Visibility::OnlyInGame
                } else if flag("NotInGame") {
                    Visibility::NotInGame
                } else {
                    Visibility::Always
                },
            }
        })
        .collect::<Vec<_>>();
    (!items.is_empty()).then_some(items)
}

/// Menu structure used only when no `GameMenu.res` can be read: the same tokens and commands as
/// the anniversary file, so labels still come from localization.
pub fn fallback_game_menu() -> Vec<MenuItem> {
    let item = |label: &str, command: &str, category: Option<&str>, order, visibility| MenuItem {
        label: label.into(),
        command: command.into(),
        category: category.map(Into::into),
        order,
        visibility,
        caps: true,
    };
    use Visibility::*;
    let mut items = vec![
        item(
            "#GameUI_HalfLife2",
            "SwitchExpansion hl2",
            Some("main"),
            30,
            NotInGame,
        ),
        item(
            "#GameUI_HalfLife2EpisodeOne",
            "SwitchExpansion episodic",
            Some("main"),
            40,
            NotInGame,
        ),
        item(
            "#GameUI_HalfLife2EpisodeTwo",
            "SwitchExpansion ep2",
            Some("main"),
            50,
            NotInGame,
        ),
        item("", "", Some("main"), 65, NotInGame),
        item(
            "#GameUI_Extras",
            "SwitchCategory extras",
            Some("main"),
            70,
            NotInGame,
        ),
        item(
            "#GameUI_GameMenu_Options",
            "OpenOptionsDialog",
            Some("main"),
            90,
            NotInGame,
        ),
        item(
            "#GameUI_GameMenu_Quit",
            "Quit",
            Some("main"),
            100,
            NotInGame,
        ),
        item(
            "#GameUI_GameMenu_ResumeGame",
            "ResumeGame",
            None,
            10,
            OnlyInGame,
        ),
        item(
            "#GameUI_GameMenu_LoadGame",
            "OpenLoadGameDialog",
            None,
            30,
            OnlyInGame,
        ),
        item(
            "#GameUI_GameMenu_SaveGame",
            "OpenSaveGameDialog",
            None,
            40,
            OnlyInGame,
        ),
        item(
            "#GameUI_GameMenu_Achievements",
            "OpenAchievementsDialog",
            None,
            70,
            OnlyInGame,
        ),
        item(
            "#GameUI_GameMenu_Options",
            "OpenOptionsDialog",
            None,
            90,
            OnlyInGame,
        ),
        item(
            "#GameUI_GameMenu_ReturnToMainMenu",
            "OpenMainMenuDialog",
            None,
            99,
            OnlyInGame,
        ),
        item("#GameUI_GameMenu_Quit", "Quit", None, 100, OnlyInGame),
    ];
    for game in ["hl2", "episodic", "ep2", "lostcoast"] {
        let category = Some(game);
        items.extend([
            item(
                "#GameUI_GameMenu_NewGame",
                &format!("OpenNewGameDialog {game}"),
                category,
                20,
                NotInGame,
            ),
            item(
                "#GameUI_GameMenu_LoadGame",
                &format!("OpenLoadGameDialog {game}"),
                category,
                30,
                NotInGame,
            ),
            item(
                "#GameUI_GameMenu_Achievements",
                &format!("OpenAchievementsDialog {game}"),
                category,
                70,
                NotInGame,
            ),
            item("", "", category, 85, NotInGame),
            item(
                "#GameUI_GameMenu_Options",
                "OpenOptionsDialog",
                category,
                90,
                NotInGame,
            ),
            item(
                "#GameUI_Back",
                "SwitchCategory main",
                category,
                100,
                NotInGame,
            ),
        ]);
    }
    items.extend([
        item(
            "#GameUI_HalfLife2LostCoast",
            "SwitchExpansion lostcoast",
            Some("extras"),
            70,
            NotInGame,
        ),
        item(
            "#GameUI_Worskhop_MenuName",
            "openworkshopdialog",
            Some("extras"),
            72,
            NotInGame,
        ),
        item("", "", Some("extras"), 85, NotInGame),
        item(
            "#GameUI_Back",
            "SwitchCategory main",
            Some("extras"),
            100,
            NotInGame,
        ),
    ]);
    items
}

/// Normalizes expansion submenus (e.g. Lost Coast and Extras) so they conform to the standard
/// game submenu pattern across both installed GameMenu.res and fallback structures.
pub fn normalize_menu_categories(items: &mut Vec<MenuItem>) {
    items.retain(|i| {
        i.category.as_deref() != Some("extras") && i.category.as_deref() != Some("lostcoast")
    });
    let item = |label: &str, command: &str, category: &str, order, visibility| MenuItem {
        label: label.into(),
        command: command.into(),
        category: Some(category.into()),
        order,
        visibility,
        caps: true,
    };
    use Visibility::NotInGame;
    items.extend([
        item(
            "#GameUI_HalfLife2LostCoast",
            "SwitchExpansion lostcoast",
            "extras",
            70,
            NotInGame,
        ),
        item(
            "#GameUI_Worskhop_MenuName",
            "openworkshopdialog",
            "extras",
            72,
            NotInGame,
        ),
        item("", "", "extras", 85, NotInGame),
        item(
            "#GameUI_Back",
            "SwitchCategory main",
            "extras",
            100,
            NotInGame,
        ),
    ]);
    items.extend([
        item(
            "#GameUI_GameMenu_NewGame",
            "OpenNewGameDialog lostcoast",
            "lostcoast",
            20,
            NotInGame,
        ),
        item(
            "#GameUI_GameMenu_LoadGame",
            "OpenLoadGameDialog lostcoast",
            "lostcoast",
            30,
            NotInGame,
        ),
        item(
            "#GameUI_GameMenu_Achievements",
            "OpenAchievementsDialog lostcoast",
            "lostcoast",
            70,
            NotInGame,
        ),
        item("", "", "lostcoast", 85, NotInGame),
        item(
            "#GameUI_GameMenu_Options",
            "OpenOptionsDialog",
            "lostcoast",
            90,
            NotInGame,
        ),
        item(
            "#GameUI_Back",
            "SwitchCategory main",
            "lostcoast",
            100,
            NotInGame,
        ),
    ]);
}

/// Decoded RGBA image (gamma test pattern, credit logos).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

/// The VGUI layout files the dialogs use, keyed by lower-case file stem.
pub const LAYOUT_FILES: &[&str] = &[
    "optionssubdifficulty",
    "optionssubkeyboard",
    "optionssubmouse",
    "optionssubgamepad",
    "optionssubaudio",
    "optionssubvideo",
    "optionssubvideogammadlg",
    "optionssubaudiothirdpartydlg",
    "optionssubvideothirdpartydlg",
    "optionssubkeyboardadvanceddlg",
    "optionssubvideoadvanceddlg",
];

/// HL2-RS layout additions appended to an installed layout, keyed like [`LAYOUT_FILES`]: extra
/// controls for settings the installed dialogs lack (embedded from `assets/ui/*.res`).
const LAYOUT_ADDENDA: &[(&str, &str)] = &[(
    "optionssubvideoadvanceddlg",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/ui/optionssubvideoadvanceddlg_extra.res"
    )),
)];

/// Appends the [`LAYOUT_ADDENDA`] controls to the layouts they extend.
pub fn apply_layout_addenda(layouts: &mut BTreeMap<String, Layout>) {
    for (name, addendum) in LAYOUT_ADDENDA {
        let extra = Layout::parse(addendum.as_bytes()).expect("embedded layout addendum parses");
        if let Some(layout) = layouts.get_mut(*name) {
            layout.controls.extend(extra.controls);
        }
    }
}

/// Everything the GameUI reads from the installation.
#[derive(Clone, Debug, Default)]
pub struct GameUiResources {
    pub localization: Localization,
    pub game_menu: Vec<MenuItem>,
    pub layouts: BTreeMap<String, Layout>,
    pub key_rows: Vec<KeyRow>,
    /// Command -> default binding name from `cfg/config_default.cfg` (macOS overrides applied on macOS).
    pub default_bindings: BTreeMap<String, String>,
    pub gamma_image: Option<Image>,
    pub bink_image: Option<Image>,
    /// Installed voice languages (`hl2_sound_vo_<language>_dir.vpk`), lower-case, English first.
    pub spoken_languages: Vec<String>,
    /// The installation root, for finding the game that ships a map.
    pub install_root: Option<PathBuf>,
    /// The games the menu switches to, in menu order (see [`menu_games`]).
    pub games: Vec<String>,
    /// Main-menu background map (`scripts/chapterbackgrounds.txt`, chapter 1).
    pub background_map: Option<String>,
    /// Game -> chapter 1 background map of that game (`scripts/chapterbackgrounds.txt`).
    pub background_maps: BTreeMap<String, String>,
    /// `(game, image)` -> pre-blurred `materials/console/<image>.vtf` of each chapter background
    /// map of that game; `<map>_widescreen` images are stored next to `<map>`.
    pub background_images: BTreeMap<(String, String), Image>,
    /// Game -> first selectable chapter map (also enables New Game for that game).
    pub new_game_maps: BTreeMap<String, String>,
    /// Game -> its chapters in play order, with their start maps and backgrounds
    /// (`scripts/chapterbackgrounds.txt`, `cfg/chapter<key>.cfg`).
    pub chapters: BTreeMap<String, Vec<super::chapters::Chapter>>,
    /// Selectable chapters and their installed dialog resources, keyed by game.
    pub new_games: BTreeMap<String, super::new_game::Catalog>,
    /// Category -> menu title (two lines for episodes, e.g. "Half-Life 2\n--- Episode One ---").
    pub category_titles: BTreeMap<String, String>,
    /// Files that could not be read or parsed (reported in the console log).
    pub missing: Vec<String>,
}

impl GameUiResources {
    /// The game whose background a menu category shows: a `SwitchExpansion` game is its own
    /// category, and the root `main` category shows the first game of the menu.
    pub fn category_game<'a>(&'a self, category: &'a str) -> &'a str {
        match category {
            "hl2ep1" => "episodic",
            "hl2ep2" => "ep2",
            "hl2lc" => "lostcoast",
            "main" => self.main_game(),
            other => other,
        }
    }

    /// The first game of the menu (Half-Life 2), shown behind the root menu.
    pub fn main_game(&self) -> &str {
        self.games
            .first()
            .map(String::as_str)
            .unwrap_or(source_assets::install::BASE_GAME)
    }

    /// The chapter 1 background map of the game a menu category shows, if any.
    pub fn background_map_for(&self, category: &str) -> Option<&str> {
        self.background_maps
            .get(self.category_game(category))
            .map(String::as_str)
    }

    /// The pre-blurred image of `game`'s background map `map`, if loaded.
    pub fn background_image(&self, game: &str, map: &str, widescreen: bool) -> Option<&Image> {
        let key = |name: String| (game.to_owned(), name);
        let map = blur(map);
        widescreen
            .then(|| {
                self.background_images
                    .get(&key(format!("{map}_widescreen")))
            })
            .flatten()
            .or_else(|| self.background_images.get(&key(map.to_owned())))
    }
}

impl GameUiResources {
    pub fn load(vfs: &Vfs) -> Self {
        let reader = Reader::new(vfs);
        let mut res = Self {
            install_root: vfs.game_root().map(Path::to_path_buf),
            ..Self::default()
        };
        for file in [
            "resource/merged_loc_all.txt",
            "resource/gameui_english.txt",
            "resource/valve_english.txt",
            "resource/vgui_english.txt",
            "resource/hl2_english.txt",
        ] {
            match reader.read(file) {
                Some(bytes) if res.localization.add_resource(file, &bytes).is_some() => {}
                _ => res.missing.push(file.into()),
            }
        }
        res.game_menu = match reader
            .read("resource/gamemenu.res")
            .as_deref()
            .and_then(parse_game_menu)
        {
            Some(mut items) => {
                normalize_menu_categories(&mut items);
                items
            }
            None => {
                res.missing.push("resource/gamemenu.res".into());
                fallback_game_menu()
            }
        };
        for name in LAYOUT_FILES {
            let file = format!("resource/{name}.res");
            match reader.read(&file).as_deref().and_then(Layout::parse) {
                Some(layout) => {
                    res.layouts.insert((*name).to_owned(), layout);
                }
                None => res.missing.push(file),
            }
        }
        apply_layout_addenda(&mut res.layouts);
        match reader.read_text("scripts/kb_act.lst") {
            Some(text) => res.key_rows = keys::parse_actions(&text),
            None => res.missing.push("scripts/kb_act.lst".into()),
        }
        match reader.read_text("cfg/config_default.cfg") {
            Some(text) => {
                res.default_bindings = keys::parse_default_binds(&text, cfg!(target_os = "macos"))
            }
            None => res.missing.push("cfg/config_default.cfg".into()),
        }
        res.gamma_image = image(vfs, "vgui/gamma");
        if res.gamma_image.is_none() {
            res.missing.push("materials/vgui/gamma.vmt".into());
        }
        res.bink_image = image(vfs, "vgui/techcredits/bink");
        res.spoken_languages = vfs
            .game_root()
            .map(|root| spoken_languages(&root.join("hl2")))
            .unwrap_or_default();
        if res.spoken_languages.is_empty() {
            res.spoken_languages.push("english".into());
        }
        res.load_campaigns(vfs, &reader);
        for category in ["main", "hl2", "episodic", "ep2", "lostcoast"] {
            let gameinfo_path = match category {
                "episodic" => "episodic/gameinfo.txt",
                "ep2" => "ep2/gameinfo.txt",
                "lostcoast" => "lostcoast/gameinfo.txt",
                _ => "gameinfo.txt",
            };
            let title = reader
                .read(gameinfo_path)
                .or_else(|| {
                    vfs.game_root()
                        .and_then(|root| std::fs::read(root.join(gameinfo_path)).ok())
                })
                .as_deref()
                .and_then(parse_gameinfo_title);
            let title =
                title.unwrap_or_else(|| category_title_fallback(category, &res.localization));
            res.category_titles.insert(category.to_owned(), title);
        }
        for (alias, canonical) in [
            ("hl2ep1", "episodic"),
            ("hl2ep2", "ep2"),
            ("hl2lc", "lostcoast"),
        ] {
            if let Some(title) = res.category_titles.get(canonical).cloned() {
                res.category_titles.insert(alias.to_owned(), title);
            }
        }
        res
    }

    pub fn layout(&self, name: &str) -> Option<&Layout> {
        self.layouts.get(name)
    }
}

/// Reads `hl2_complete/<path>` loose first, then the mounted VFS.
pub struct Reader<'a> {
    vfs: &'a Vfs,
    overlay: Option<PathBuf>,
}

impl<'a> Reader<'a> {
    pub fn new(vfs: &'a Vfs) -> Self {
        Self {
            vfs,
            overlay: vfs.game_root().map(|root| root.join("hl2_complete")),
        }
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let normalized = source_assets::vpk::normalize(path).ok()?;
        self.overlay
            .as_ref()
            .and_then(|root| std::fs::read(root.join(&normalized)).ok())
            .or_else(|| self.vfs.read(&normalized).ok().flatten())
    }

    pub fn read_text(&self, path: &str) -> Option<String> {
        self.read(path)
            .and_then(|bytes| keyvalues::decode_text(&bytes).ok())
    }
}

pub(super) fn image(vfs: &Vfs, material: &str) -> Option<Image> {
    let base = vfs.base_texture(material).ok()??;
    let file = format!(
        "materials/{}.vtf",
        base.trim_start_matches("materials/")
            .trim_end_matches(".vtf")
    );
    let decoded = source_assets::vtf::decode(&vfs.read(&file).ok()??, 512).ok()?;
    Some(Image {
        width: decoded.width,
        height: decoded.height,
        rgba: decoded.rgba,
    })
}

pub(super) fn load_vtf_file(
    reader: &Reader,
    vfs: &Vfs,
    expansion: &str,
    vtf_filename: &str,
) -> Option<Image> {
    let rel_path = format!("materials/console/{vtf_filename}.vtf");

    let read_loose = |root: &Path, sub: &str| -> Option<Vec<u8>> {
        std::fs::read(root.join(sub).join(&rel_path)).ok()
    };
    let read_vpk = |root: &Path, vpk_rel: &str| -> Option<Vec<u8>> {
        let p = root.join(vpk_rel);
        if p.exists() {
            source_assets::vpk::Vpk::open(&p)
                .ok()
                .and_then(|vpk| vpk.read(&rel_path).ok().flatten())
        } else {
            None
        }
    };

    let bytes = if let Some(root) = vfs.game_root() {
        match expansion {
            "lostcoast" => read_loose(root, "lostcoast")
                .or_else(|| read_vpk(root, "lostcoast/lostcoast_pak_dir.vpk")),
            "episodic" => read_loose(root, "episodic")
                .or_else(|| read_vpk(root, "episodic/ep1_pak_dir.vpk"))
                .or_else(|| read_loose(root, "hl2_complete"))
                .or_else(|| read_vpk(root, "hl2_complete/hl2_complete_misc_dir.vpk")),
            "ep2" => read_loose(root, "ep2")
                .or_else(|| read_vpk(root, "ep2/ep2_pak_dir.vpk"))
                .or_else(|| read_loose(root, "hl2_complete"))
                .or_else(|| read_vpk(root, "hl2_complete/hl2_complete_misc_dir.vpk")),
            _ => {
                // Base Half-Life 2 / main: strictly check hl2_complete and hl2 archives,
                // never falling through to expansion archives (e.g. Lost Coast's background01.vtf).
                read_loose(root, "hl2_complete")
                    .or_else(|| read_vpk(root, "hl2_complete/hl2_complete_misc_dir.vpk"))
                    .or_else(|| read_loose(root, "hl2"))
                    .or_else(|| read_vpk(root, "hl2/hl2_textures_dir.vpk"))
                    .or_else(|| read_vpk(root, "hl2/hl2_misc_dir.vpk"))
            }
        }
        .or_else(|| reader.read(&rel_path))
        .or_else(|| vfs.read(&rel_path).ok().flatten())
    } else {
        reader
            .read(&rel_path)
            .or_else(|| vfs.read(&rel_path).ok().flatten())
    }?;

    let decoded = source_assets::vtf::decode(&bytes, 512).ok()?;
    Some(Image {
        width: decoded.width,
        height: decoded.height,
        rgba: decoded.rgba,
    })
}

/// Languages with an installed `hl2_sound_vo_<language>_dir.vpk`, English first, then sorted.
pub fn spoken_languages(hl2: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(hl2)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let name = entry
                .ok()?
                .file_name()
                .to_string_lossy()
                .to_ascii_lowercase();
            name.strip_prefix("hl2_sound_vo_")?
                .strip_suffix("_dir.vpk")
                .map(str::to_owned)
        })
        .collect();
    found.sort_by_key(|l| (l != "english", l.clone()));
    found.dedup();
    found
}

/// Chapter 1's background map from `scripts/chapterbackgrounds.txt`.
pub fn background_map(text: &str) -> Option<String> {
    let roots = keyvalues::parse_resource(text.as_bytes()).ok()?;
    roots
        .iter()
        .find_map(|root| root.get("1"))
        .and_then(|e| e.text())
        .map(|map| map.trim().to_owned())
        .filter(|map| !map.is_empty())
}

/// The games the main menu switches to, in menu order: the arguments of the
/// `SwitchExpansion <game>` commands in `gamemenu.res`, which name the game's folder in the
/// installation (`hl2`, `episodic`, `ep2`, `lostcoast`).
pub fn menu_games(items: &[MenuItem]) -> Vec<String> {
    let mut games: Vec<String> = Vec::new();
    for item in items {
        let mut words = item.command.split_whitespace();
        let switch = words
            .next()
            .is_some_and(|w| w.eq_ignore_ascii_case("SwitchExpansion"));
        if let (true, Some(game)) = (switch, words.next()) {
            if !games.iter().any(|g| g.eq_ignore_ascii_case(game)) {
                games.push(game.to_owned());
            }
        }
    }
    games
}

/// Extracts all unique background maps from a `scripts/chapterbackgrounds.txt` file content.
pub fn chapter_background_maps(text: &str) -> Vec<String> {
    let Ok(roots) = keyvalues::parse_resource(text.as_bytes()) else {
        return Vec::new();
    };
    let mut maps = Vec::new();
    for root in roots {
        for child in root.children() {
            if let Some(map) = child.text().map(|m| m.trim()).filter(|m| !m.is_empty()) {
                if !maps
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(map))
                {
                    maps.push(map.to_owned());
                }
            }
        }
    }
    maps
}

/// Returns the pre-blurred background image name for a map.
/// In Source GameUI, the console blur image name in `materials/console/` always matches the background map name.
pub fn blur(map: &str) -> &str {
    map.trim_end_matches(".bsp")
}

/// The map of a chapter config (`map d1_trainstation_01` or `playvideo_exitcommand ep1_recap map ep2_outland_01`).
pub fn chapter_map(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let clean = line.split("//").next()?;
        let mut words = clean.split_whitespace();
        while let Some(w) = words.next() {
            if w.eq_ignore_ascii_case("map") {
                return words.next().map(|m| m.trim_matches('"').to_owned());
            }
        }
        None
    })
}

/// Background map for startup without a `--map`, read through a freshly mounted VFS.
pub fn startup_background_map(vfs: &Vfs) -> Option<String> {
    Reader::new(vfs)
        .read_text("scripts/chapterbackgrounds.txt")
        .and_then(|text| background_map(&text))
}

/// Normalizes subtitle decorations (dashes, en-dashes, em-dashes, hyphens) to the authentic
/// Source GameUI character representations that match the glyph mappings of the title fonts.
pub fn normalize_subtitle_decoration(title2: &str) -> String {
    let trimmed = title2.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower == "== episode one==" || lower == "== episode one ==" || lower.contains("episode one")
    {
        "== episode one==".to_string()
    } else if lower == "== episode two =="
        || lower == "== episode two=="
        || lower.contains("episode two")
    {
        "== episode two ==".to_string()
    } else if lower.contains("lost coast") {
        "lost coasT".to_string()
    } else {
        trimmed
            .replace(['\u{2013}', '\u{2014}'], "=")
            .replace("---", "==")
            .replace("--", "==")
    }
}

/// Extracts the menu title from a `gameinfo.txt` file (`title` + optional `title2` joined by `\n`).
pub fn parse_gameinfo_title(bytes: &[u8]) -> Option<String> {
    let roots = keyvalues::parse_resource(bytes).ok()?;
    let root = roots.into_iter().next()?;
    let title = root.get("title")?.text()?.to_owned();
    if let Some(title2) = root.get("title2").and_then(|t| t.text()) {
        Some(format!(
            "{title}\n{}",
            normalize_subtitle_decoration(title2)
        ))
    } else {
        Some(title)
    }
}

pub fn category_title_fallback(category: &str, loc: &Localization) -> String {
    let canonical = match category {
        "hl2ep1" => "episodic",
        "hl2ep2" => "ep2",
        "hl2lc" => "lostcoast",
        other => other,
    };
    let hl2 = loc.text("#GameUI_HalfLife2");
    match canonical {
        "episodic" => format!("{hl2}\n== episode one=="),
        "ep2" => format!("{hl2}\n== episode two =="),
        "lostcoast" => format!("{hl2}\nlost coasT"),
        _ => hl2,
    }
}

/// Returns fallback titles for all recognized categories.
pub fn fallback_category_titles(loc: &Localization) -> BTreeMap<String, String> {
    let mut titles = BTreeMap::new();
    for category in [
        "main",
        "hl2",
        "episodic",
        "ep2",
        "lostcoast",
        "hl2ep1",
        "hl2ep2",
        "hl2lc",
    ] {
        titles.insert(category.to_owned(), category_title_fallback(category, loc));
    }
    titles
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mean advance of the capital letters (menu labels are upper case) per cap height.
    fn capital_width(bytes: &[u8]) -> f32 {
        let face = ttf_parser::Face::parse(bytes, 0).unwrap();
        let cap = f32::from(face.capital_height().unwrap());
        let advances: Vec<f32> = ('A'..='Z')
            .filter_map(|c| face.glyph_index(c))
            .filter_map(|g| face.glyph_hor_advance(g))
            .map(f32::from)
            .collect();
        advances.iter().sum::<f32>() / advances.len() as f32 / cap
    }

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn owned_menu_font_is_the_schemes_semibold_gordin() {
        let root = source_assets::install::discover().expect("installed game");
        let vfs = source_assets::vpk::Vfs::mount(&root).unwrap();
        let reader = Reader::new(&vfs);
        let scheme = keyvalues::parse_resource(&reader.read("resource/SourceScheme.res").unwrap())
            .unwrap()
            .remove(0);
        let bytes =
            source_ui::scheme_fonts::resolve(&scheme, "MenuLarge", |path| reader.read(path))
                .expect("MenuLarge");
        let info = source_ui::scheme_fonts::FontFileInfo::read(&bytes).unwrap();
        assert!(info.matches("GorDIN"), "{info:?}");
        assert_eq!(info.weight, 600);
        let liberation = reader
            .read("resource/linux_fonts/liberationsans-regular.ttf")
            .unwrap();
        // GorDIN SemiBold capitals are 0.86 cap heights wide on average, Liberation Sans
        // Regular ones 0.98: the former menu face looked stretched.
        let (gordin, liberation) = (capital_width(&bytes), capital_width(&liberation));
        assert!(gordin < 0.9 * liberation, "{gordin} vs {liberation}");
    }

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn owned_marlett_font_resolves_for_dialog_close_buttons() {
        let root = source_assets::install::discover().expect("installed game");
        let vfs = source_assets::vpk::Vfs::mount(&root).unwrap();
        let reader = Reader::new(&vfs);
        let scheme = keyvalues::parse_resource(&reader.read("resource/SourceScheme.res").unwrap())
            .unwrap()
            .remove(0);
        let bytes = source_ui::scheme_fonts::resolve(&scheme, "Marlett", |path| reader.read(path))
            .expect("Marlett font from scheme");
        let canvas = source_ui::canvas::Canvas::default();
        let font_face = source_ui::font::FontFace::load(&bytes, canvas).expect("FontFace load");
        let w = font_face.width_cell("r", source_ui::widgets::CLOSE_GLYPH_TALL);
        assert!(
            (8.0..=12.0).contains(&w),
            "Marlett 'r' width at tall {}: {w}",
            source_ui::widgets::CLOSE_GLYPH_TALL
        );
    }

    #[test]
    fn game_menu_entries_keep_file_order_categories_and_spacers() {
        let menu = parse_game_menu(
            br##""GameMenu" {
                "Main_1" { "label" "#GameUI_HalfLife2" "command" "SwitchExpansion hl2" "InGameOrder" "30" "NotInGame" "1" "MenuCategory" "main" }
                "Main_4_spc" { "label" "" "command" "" "InGameOrder" "65" "NotInGame" "1" "MenuCategory" "Main" }
                "Pause_1" { "label" "#GameUI_GameMenu_ResumeGame" "command" "ResumeGame" "InGameOrder" "10" "OnlyInGame" "1" }
            }"##,
        )
        .unwrap();
        assert_eq!(menu.len(), 3);
        assert_eq!(menu[0].category.as_deref(), Some("main"));
        assert!(menu[1].is_spacer());
        assert_eq!(
            menu[1].category.as_deref(),
            Some("main"),
            "categories are case-insensitive"
        );
        assert_eq!(menu[2].category, None);
        assert_eq!(menu[2].visibility, Visibility::OnlyInGame);
        assert_eq!(menu[0].visibility, Visibility::NotInGame);
        assert!(parse_game_menu(b"\"GameMenu\" { }").is_none());
    }

    #[test]
    fn fallback_menu_has_every_requested_category() {
        let menu = fallback_game_menu();
        for category in ["main", "hl2", "episodic", "ep2", "extras", "lostcoast"] {
            assert!(
                menu.iter().any(|i| i.category.as_deref() == Some(category)),
                "{category}"
            );
        }
        assert!(
            menu.iter()
                .all(|i| i.is_spacer() || i.label.starts_with('#')),
            "labels stay tokens"
        );

        let extras_items: Vec<_> = menu
            .iter()
            .filter(|i| i.category.as_deref() == Some("extras"))
            .collect();
        assert_eq!(extras_items.len(), 4);
        assert_eq!(extras_items[0].label, "#GameUI_HalfLife2LostCoast");
        assert_eq!(extras_items[0].command, "SwitchExpansion lostcoast");
        assert_eq!(extras_items[1].label, "#GameUI_Worskhop_MenuName");
        assert_eq!(extras_items[1].command, "openworkshopdialog");
        assert!(extras_items[2].is_spacer());
        assert_eq!(extras_items[3].label, "#GameUI_Back");
        assert_eq!(extras_items[3].command, "SwitchCategory main");

        let lc_items: Vec<_> = menu
            .iter()
            .filter(|i| i.category.as_deref() == Some("lostcoast"))
            .collect();
        assert_eq!(lc_items.len(), 6);
        assert_eq!(lc_items[0].label, "#GameUI_GameMenu_NewGame");
        assert_eq!(lc_items[0].command, "OpenNewGameDialog lostcoast");
        assert_eq!(lc_items[1].label, "#GameUI_GameMenu_LoadGame");
        assert_eq!(lc_items[1].command, "OpenLoadGameDialog lostcoast");
        assert_eq!(lc_items[2].label, "#GameUI_GameMenu_Achievements");
        assert_eq!(lc_items[2].command, "OpenAchievementsDialog lostcoast");
        assert!(lc_items[3].is_spacer());
        assert_eq!(lc_items[4].label, "#GameUI_GameMenu_Options");
        assert_eq!(lc_items[4].command, "OpenOptionsDialog");
        assert_eq!(lc_items[5].label, "#GameUI_Back");
        assert_eq!(lc_items[5].command, "SwitchCategory main");
    }

    #[test]
    fn chapter_files_name_their_maps() {
        assert_eq!(
            background_map("\"chapters\"\n{\n\t1\t\"background01\"\n\t2\t\"background01\"\n}"),
            Some("background01".into())
        );
        assert_eq!(
            chapter_map("// comment\nmap d1_trainstation_01\n"),
            Some("d1_trainstation_01".into())
        );
        assert_eq!(
            chapter_map("map ep1_citadel_00"),
            Some("ep1_citadel_00".into())
        );
        assert_eq!(
            chapter_map("playvideo_exitcommand ep1_recap map ep2_outland_01"),
            Some("ep2_outland_01".into())
        );
        assert_eq!(
            chapter_map("map \"d2_lostcoast\""),
            Some("d2_lostcoast".into())
        );
        assert_eq!(chapter_map("echo hi"), None);
    }

    #[test]
    fn menu_games_come_from_switch_expansion_commands_in_menu_order() {
        let item = |command: &str| MenuItem {
            label: String::new(),
            command: command.into(),
            category: Some("main".into()),
            order: 0,
            visibility: Visibility::Always,
            caps: false,
        };
        let items = [
            item("SwitchExpansion hl2"),
            item("SwitchCategory extras"),
            item("switchexpansion episodic"),
            item("SwitchExpansion ep2"),
            item("OpenOptionsDialog"),
            item("SwitchExpansion lostcoast"),
            item("SwitchExpansion hl2"),
        ];
        assert_eq!(menu_games(&items), ["hl2", "episodic", "ep2", "lostcoast"]);
        assert_eq!(
            menu_games(&fallback_game_menu())
                .first()
                .map(String::as_str),
            Some("hl2")
        );
    }

    #[test]
    fn background_map_for_resolves_aliases() {
        let res = GameUiResources {
            games: vec!["hl2".into(), "episodic".into()],
            background_maps: [("hl2", "background01"), ("episodic", "ep1_background01")]
                .into_iter()
                .map(|(game, map)| (game.to_owned(), map.to_owned()))
                .collect(),
            ..Default::default()
        };
        assert_eq!(res.background_map_for("hl2"), Some("background01"));
        assert_eq!(res.background_map_for("main"), Some("background01"));
        assert_eq!(res.background_map_for("hl2ep1"), Some("ep1_background01"));
        assert_eq!(res.background_map_for("extras"), None);
        assert_eq!(res.category_game("main"), "hl2");
    }

    #[test]
    fn background_images_are_keyed_by_game_and_map() {
        let image = |width| Image {
            width,
            height: 1,
            rgba: vec![0; width as usize * 4],
        };
        let res = GameUiResources {
            background_images: [
                (("hl2", "background01"), image(1)),
                (("hl2", "background01_widescreen"), image(2)),
                (("lostcoast", "background01"), image(3)),
            ]
            .into_iter()
            .map(|((game, name), img)| ((game.to_owned(), name.to_owned()), img))
            .collect(),
            ..Default::default()
        };
        let width = |game, map, wide| res.background_image(game, map, wide).map(|i| i.width);
        assert_eq!(width("hl2", "background01", false), Some(1));
        assert_eq!(width("hl2", "background01.bsp", true), Some(2));
        assert_eq!(width("lostcoast", "background01", false), Some(3));
        assert_eq!(width("lostcoast", "background01", true), Some(3));
        assert_eq!(width("episodic", "background01", false), None);
    }

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn every_game_reads_its_own_chapter_files_whatever_map_is_loaded() {
        let root = source_assets::install::discover().expect("installed game");
        // The menu's file system follows the loaded background map; an Episode Two background
        // mounts Episode Two first. Half-Life 2 must still get its own chapter files.
        for startup in ["hl2", "ep2", "episodic", "lostcoast"] {
            let vfs = Vfs::mount_game(&root, startup).expect("mount");
            let res = GameUiResources::load(&vfs);
            let map = |game: &str| res.background_maps.get(game).map(String::as_str);
            let new_game = |game: &str| res.new_game_maps.get(game).map(String::as_str);
            assert_eq!(map("hl2"), Some("background01"), "{startup:?}");
            assert_eq!(res.background_map_for("main"), Some("background01"));
            assert_eq!(map("episodic"), Some("ep1_background01"), "{startup:?}");
            assert_eq!(map("ep2"), Some("ep2_background01"), "{startup:?}");
            assert_eq!(map("lostcoast"), Some("background01"), "{startup:?}");
            assert_eq!(new_game("hl2"), Some("d1_trainstation_01"), "{startup:?}");
            assert_eq!(new_game("episodic"), Some("ep1_citadel_00"), "{startup:?}");
            assert_eq!(new_game("ep2"), Some("ep2_outland_01"), "{startup:?}");
            assert_eq!(new_game("lostcoast"), Some("d2_lostcoast"), "{startup:?}");
        }
    }

    #[test]
    fn parse_gameinfo_title_handles_single_and_two_line_titles() {
        let single = b"\"GameInfo\" { \"game\" \"HALF-LIFE 2\" \"title\" \"HALF-LIFE'\" }";
        assert_eq!(parse_gameinfo_title(single), Some("HALF-LIFE'".into()));

        let episode1 = b"\"GameInfo\" { \"game\" \"HALF-LIFE 2\" \"title\" \"HALF-LIFE'\" \"title2\" \"== episode one==\" }";
        assert_eq!(
            parse_gameinfo_title(episode1),
            Some("HALF-LIFE'\n== episode one==".into())
        );

        let episode2 = b"\"GameInfo\" { \"game\" \"HALF-LIFE 2\" \"title\" \"HALF-LIFE'\" \"title2\" \"== episode two ==\" }";
        assert_eq!(
            parse_gameinfo_title(episode2),
            Some("HALF-LIFE'\n== episode two ==".into())
        );

        // UTF-8 with BOM
        let utf8_bom = b"\xef\xbb\xbf\"GameInfo\" { \"game\" \"HALF-LIFE 2\" \"title\" \"HALF-LIFE'\" \"title2\" \"== episode one==\" }";
        assert_eq!(
            parse_gameinfo_title(utf8_bom),
            Some("HALF-LIFE'\n== episode one==".into())
        );

        // UTF-16 LE with BOM
        let utf16_bom = [
            0xff, 0xfe, b'"', 0, b'G', 0, b'a', 0, b'm', 0, b'e', 0, b'I', 0, b'n', 0, b'f', 0,
            b'o', 0, b'"', 0, b' ', 0, b'{', 0, b' ', 0, b'"', 0, b't', 0, b'i', 0, b't', 0, b'l',
            0, b'e', 0, b'"', 0, b' ', 0, b'"', 0, b'H', 0, b'A', 0, b'L', 0, b'F', 0, b'-', 0,
            b'L', 0, b'I', 0, b'F', 0, b'E', 0, b'\'', 0, b'"', 0, b' ', 0, b'}', 0,
        ];
        assert_eq!(parse_gameinfo_title(&utf16_bom), Some("HALF-LIFE'".into()));

        // UTF-16 LE without BOM
        let utf16_nobom = [
            b'"', 0, b'G', 0, b'a', 0, b'm', 0, b'e', 0, b'I', 0, b'n', 0, b'f', 0, b'o', 0, b'"',
            0, b' ', 0, b'{', 0, b' ', 0, b'"', 0, b't', 0, b'i', 0, b't', 0, b'l', 0, b'e', 0,
            b'"', 0, b' ', 0, b'"', 0, b'H', 0, b'A', 0, b'L', 0, b'F', 0, b'-', 0, b'L', 0, b'I',
            0, b'F', 0, b'E', 0, b'\'', 0, b'"', 0, b' ', 0, b'"', 0, b't', 0, b'i', 0, b't', 0,
            b'l', 0, b'e', 0, b'2', 0, b'"', 0, b' ', 0, b'"', 0, b'l', 0, b'o', 0, b's', 0, b't',
            0, b' ', 0, b'c', 0, b'o', 0, b'a', 0, b's', 0, b'T', 0, b'"', 0, b' ', 0, b'}', 0,
        ];
        assert_eq!(
            parse_gameinfo_title(&utf16_nobom),
            Some("HALF-LIFE'\nlost coasT".into())
        );

        // Windows-1252 en-dash byte pattern (\x96)
        let win1252 =
            b"\"GameInfo\" { \"title\" \"HALF-LIFE'\" \"title2\" \"\x96 episode one \x96\" }";
        assert_eq!(
            parse_gameinfo_title(win1252),
            Some("HALF-LIFE'\n== episode one==".into())
        );
    }

    #[test]
    fn test_normalize_subtitle_decoration() {
        assert_eq!(
            normalize_subtitle_decoration("== episode one=="),
            "== episode one=="
        );
        assert_eq!(
            normalize_subtitle_decoration("== episode two =="),
            "== episode two =="
        );
        assert_eq!(normalize_subtitle_decoration("lost coasT"), "lost coasT");
        assert_eq!(
            normalize_subtitle_decoration("– episode one –"),
            "== episode one=="
        );
        assert_eq!(
            normalize_subtitle_decoration("— episode two —"),
            "== episode two =="
        );
        assert_eq!(
            normalize_subtitle_decoration("--- Episode One ---"),
            "== episode one=="
        );
        assert_eq!(
            normalize_subtitle_decoration("--- Episode Two ---"),
            "== episode two =="
        );
        assert_eq!(
            normalize_subtitle_decoration("--- Lost Coast ---"),
            "lost coasT"
        );
    }

    #[test]
    fn category_titles_fallback_provides_two_line_episode_titles() {
        let loc = Localization::default();
        let titles = fallback_category_titles(&loc);
        assert_eq!(titles.get("main").map(String::as_str), Some("Half-Life 2"));
        assert_eq!(titles.get("hl2").map(String::as_str), Some("Half-Life 2"));
        assert_eq!(
            titles.get("episodic").map(String::as_str),
            Some("Half-Life 2\n== episode one==")
        );
        assert_eq!(
            titles.get("ep2").map(String::as_str),
            Some("Half-Life 2\n== episode two ==")
        );
        assert_eq!(
            titles.get("lostcoast").map(String::as_str),
            Some("Half-Life 2\nlost coasT")
        );
        // Expansion aliases
        assert_eq!(
            titles.get("hl2ep1").map(String::as_str),
            Some("Half-Life 2\n== episode one==")
        );
        assert_eq!(
            titles.get("hl2ep2").map(String::as_str),
            Some("Half-Life 2\n== episode two ==")
        );
        assert_eq!(
            titles.get("hl2lc").map(String::as_str),
            Some("Half-Life 2\nlost coasT")
        );

        let empty_loc = Localization::default();
        let fallback_empty = fallback_category_titles(&empty_loc);
        assert_eq!(
            fallback_empty.get("episodic").map(String::as_str),
            Some("Half-Life 2\n== episode one==")
        );
        assert_eq!(
            fallback_empty.get("ep2").map(String::as_str),
            Some("Half-Life 2\n== episode two ==")
        );
        assert_eq!(
            fallback_empty.get("hl2ep1").map(String::as_str),
            Some("Half-Life 2\n== episode one==")
        );
        assert_eq!(
            fallback_empty.get("hl2ep2").map(String::as_str),
            Some("Half-Life 2\n== episode two ==")
        );
        assert_eq!(
            fallback_empty.get("hl2lc").map(String::as_str),
            Some("Half-Life 2\nlost coasT")
        );
    }

    #[test]
    fn spoken_languages_list_installed_voice_packs_english_first() {
        let dir = std::env::temp_dir().join(format!("hl2-rs-vo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "hl2_sound_vo_russian_dir.vpk",
            "hl2_sound_vo_english_dir.vpk",
            "hl2_sound_vo_english_000.vpk",
            "hl2_pak_dir.vpk",
        ] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        assert_eq!(spoken_languages(&dir), ["english", "russian"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Owned-install check: `cargo test -p hl2-ui -- --ignored owned_install`.
    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn owned_install_resources_load() {
        let root = source_assets::install::discover().expect("installed game");
        let vfs = Vfs::mount(&root).unwrap();
        let res = GameUiResources::load(&vfs);
        assert!(
            res.missing.is_empty() || !res.missing.contains(&"resource/gamemenu.res".to_string()),
            "{:?}",
            res.missing
        );
        assert_eq!(
            res.localization.text("#GameUI_QuitConfirmationTitle"),
            "QUIT GAME"
        );
        assert_eq!(
            res.localization.text("#GameUI_NoWindowBorder"),
            "Windowed (No Border)"
        );
        assert_eq!(
            res.localization.text("#HL2RS_Absent_Token"),
            "HL2RS_Absent_Token"
        );
        assert!(res
            .game_menu
            .iter()
            .any(|i| i.command == "SwitchExpansion ep2"));
        assert!(res
            .layout("optionssubmouse")
            .and_then(|l| l.get("Autoaim"))
            .is_some());
        assert!(res.key_rows.len() > 30);
        assert_eq!(
            res.default_bindings.get("+forward").map(String::as_str),
            Some("KeyW")
        );
        assert!(res.gamma_image.is_some(), "{:?}", res.missing);
        assert_eq!(
            res.spoken_languages.first().map(String::as_str),
            Some("english")
        );
        assert_eq!(res.background_map.as_deref(), Some("background01"));
        assert_eq!(
            res.background_maps.get("hl2").map(String::as_str),
            Some("background01")
        );
        assert_eq!(
            res.background_maps.get("episodic").map(String::as_str),
            Some("ep1_background01")
        );
        assert_eq!(
            res.background_maps.get("ep2").map(String::as_str),
            Some("ep2_background01")
        );
        assert_eq!(
            res.background_maps.get("lostcoast").map(String::as_str),
            Some("background01")
        );
        assert_eq!(
            res.new_game_maps.get("hl2").map(String::as_str),
            Some("d1_trainstation_01")
        );
        assert_eq!(
            res.new_game_maps.get("episodic").map(String::as_str),
            Some("ep1_citadel_00")
        );
        assert_eq!(
            res.new_game_maps.get("ep2").map(String::as_str),
            Some("ep2_outland_01")
        );
        assert_eq!(
            res.new_game_maps.get("lostcoast").map(String::as_str),
            Some("d2_lostcoast")
        );
        assert_eq!(
            res.category_titles.get("episodic").map(String::as_str),
            Some("HALF-LIFE'\n== episode one==")
        );
        assert_eq!(
            res.category_titles.get("ep2").map(String::as_str),
            Some("HALF-LIFE'\n== episode two ==")
        );
        assert_eq!(
            res.category_titles.get("lostcoast").map(String::as_str),
            Some("HALF-LIFE'\nlost coasT")
        );
        let image = |game, map, wide| res.background_image(game, map, wide);
        for (game, map) in [
            ("hl2", "background01"),
            ("hl2", "background05"),
            ("episodic", "ep1_background01"),
            ("ep2", "ep2_background01"),
            ("lostcoast", "background01"),
        ] {
            assert!(image(game, map, false).is_some(), "{game}/{map}");
            assert!(image(game, map, true).is_some(), "{game}/{map} widescreen");
        }
        assert_ne!(
            image("hl2", "background01", false).unwrap().rgba,
            image("lostcoast", "background01", false).unwrap().rgba,
            "Half-Life 2 and Lost Coast each show their own background01"
        );
        println!("missing: {:?}", res.missing);
    }

    #[test]
    fn test_blur_and_chapter_background_maps() {
        assert_eq!(blur("background01"), "background01");
        assert_eq!(blur("background01.bsp"), "background01");
        assert_eq!(blur("ep1_background01"), "ep1_background01");
        assert_eq!(blur("ep1_background01.bsp"), "ep1_background01");
        assert_eq!(blur("ep2_background01"), "ep2_background01");
        assert_eq!(blur("ep2_background01.bsp"), "ep2_background01");

        let bgs = chapter_background_maps(
            "\"chapters\"\n{\n\t1\t\"background01\"\n\t2\t\"background01\"\n\t3\t\"background02\"\n\t4\t\"background02\"\n}",
        );
        assert_eq!(bgs, vec!["background01", "background02"]);
    }

    #[test]
    #[ignore]
    fn test_lostcoast_vfs_does_not_shadow_hl2_background() {
        let root = source_assets::install::discover().expect("game install");
        let vfs_clean = source_assets::vpk::Vfs::mount(&root).unwrap();
        let res_clean = GameUiResources::load(&vfs_clean);
        let hl2_clean = res_clean
            .background_image("hl2", "background01", false)
            .unwrap();

        let vfs_lc = source_assets::vpk::Vfs::mount_game(&root, "lostcoast").unwrap();
        let res_lc = GameUiResources::load(&vfs_lc);
        let hl2_lc = res_lc
            .background_image("hl2", "background01", false)
            .unwrap();
        let lc_bg = res_lc
            .background_image("lostcoast", "background01", false)
            .unwrap();

        assert_eq!(
            hl2_lc.rgba, hl2_clean.rgba,
            "background01 under lostcoast VFS must match background01 under clean HL2 VFS"
        );
        assert_ne!(
            hl2_lc.rgba, lc_bg.rgba,
            "HL2 background01 must NEVER be shadowed by Lost Coast cliff under lostcoast VFS"
        );
    }
}
