//! F4 developer spawn overlay. Core spawn requests remain in simulation.
use hl2_simulation::ai::spawn::{
    Allegiance, SPAWN_CLASSES, SPAWN_PROP_NAMES, SPAWN_PROPS, SpawnRequest, default_weapon,
    entry_count, reads_num_grenades, spawn_class,
};
use serde::Serialize;

/// F4 overlay keys (keyboard or movement-script `spawn_overlay` actions).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayKey {
    Toggle,
    Up,
    Down,
    Left,
    Right,
    Enter,
    /// Switch the NPC / Props tab.
    Tab,
    PageUp,
    PageDown,
    /// Start or stop typing into the Props filter.
    Filter,
    /// Delete the filter's last character (while typing).
    Backspace,
}
/// What an overlay key asks the host to do.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum OverlayCommand {
    Spawn(SpawnRequest),
    /// prop_physics_create with this argument (relative to models/).
    SpawnProp(String),
    /// Toggle notarget (the AI does not sense the player).
    ToggleNotarget,
    /// Toggle the AI think loop (ai_enable).
    ToggleAi,
    ToggleFriendlyFire,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum Column {
    #[default]
    Classes,
    Options,
}
/// One option row of the right column.
#[derive(Clone, Debug, PartialEq, Eq)]
enum OptionRow {
    /// A weapon ("" = none).
    Weapon(String),
    Allegiance,
    /// NumGrenades (classes that read it).
    Grenades,
    Notarget,
    Ai,
    /// Debug: the player's hits hurt friendly-immune NPCs (ours).
    FriendlyFire,
}
/// What a pointer click on a drawn panel row means (`Overlay::click`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum PanelTarget {
    Tab(Tab),
    /// The Props tab's filter box.
    Filter,
    /// An item of a column (its index in that column's list).
    Row(Column, usize),
    /// A "more above/below" note: scroll that column a page (-1 up, 1 down).
    Page(Column, isize),
}
/// A drawn panel row: text, selected (the current choice), under the cursor.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PanelRow {
    pub text: String,
    pub selected: bool,
    pub cursor: bool,
    #[serde(skip)]
    pub target: Option<PanelTarget>,
}
/// The overlay as the host draws it: a title, a tab bar and two columns.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Panel {
    pub title: String,
    /// Tab buttons (NPCs, Props; the filter box on the Props tab).
    pub tabs: Vec<PanelRow>,
    /// Column headings (left, right).
    pub headings: [String; 2],
    pub classes: Vec<PanelRow>,
    pub options: Vec<PanelRow>,
}
/// The F4 grenade counts Enter cycles through (a developer choice: native npc_create
/// spawns soldiers with NumGrenades 0, so they never throw; maps set it per soldier).
pub const GRENADE_CHOICES: &[i32] = &[0, 1, 2, 3, 5, 10, 99];
/// Classes whose spawn reads the NumGrenades keyvalue (CNPC_Combine::KeyValue).

/// Class rows shown at once; the list scrolls to keep the selection visible.
pub const VISIBLE_CLASSES: usize = 14;
/// The overlay's two tabs (Tab switches): NPC classes, or physics props by folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum Tab {
    #[default]
    Npcs,
    Props,
}
/// The rows of `items` drawn around `selected` (at most `visible`, plus "more" notes), so
/// the selection is always visible.
fn scrolled_rows(
    items: &[String],
    selected: usize,
    focused: bool,
    visible: usize,
    column: Column,
) -> Vec<PanelRow> {
    let total = items.len();
    let first = selected
        .saturating_sub(visible / 2)
        .min(total.saturating_sub(visible));
    let last = (first + visible).min(total);
    let note = |text: String, page| PanelRow {
        text,
        selected: false,
        cursor: false,
        target: Some(PanelTarget::Page(column, page)),
    };
    let mut rows = Vec::new();
    if first > 0 {
        rows.push(note(format!("({first} more above)"), -1));
    }
    for (index, text) in items.iter().enumerate().take(last).skip(first) {
        rows.push(PanelRow {
            text: text.clone(),
            selected: index == selected,
            cursor: focused && index == selected,
            target: Some(PanelTarget::Row(column, index)),
        });
    }
    if last < total {
        rows.push(note(format!("({} more below)", total - last), 1));
    }
    rows
}
/// The Props tab's first folder: the common HL2 props (`SPAWN_PROPS`) that are installed.
pub const COMMON_FOLDER: &str = "* common";
/// A common prop's plain name ("explosive barrel"), if it has one.
pub fn prop_name(model: &str) -> Option<&'static str> {
    SPAWN_PROP_NAMES
        .iter()
        .find(|(m, _)| *m == model)
        .map(|(_, name)| *name)
}
/// Characters the Props filter accepts (model paths are lowercase ASCII).
fn filter_char(c: char) -> Option<char> {
    let c = c.to_ascii_lowercase();
    (c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '.' | '-' | ' ')).then_some(c)
}

/// The F4 overlay. NPC tab: a scrolling class column and an options column with the
/// weapons the selected class can use (`set_usable`), its allegiance, notarget and the AI
/// switch. Props tab: model folders (first the installed common props) and the selected
/// folder's physics props, narrowed by a typed filter. Keys, mouse clicks
/// (`click`) and the wheel (`scroll`) drive it.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Overlay {
    pub open: bool,
    /// Shown state of the friendly-fire debug option (the host owns the switch).
    pub friendly_fire: bool,
    pub tab: Tab,
    pub class: usize,
    /// Props tab cursors: folder index and model index inside it.
    pub folder: usize,
    pub model: usize,
    /// Props tab filter (lowercase substring of the model path) and whether typed keys go
    /// into it.
    pub filter: String,
    pub typing: bool,
    /// List rows that fit on screen (host, from the window height; 0 = VISIBLE_CLASSES).
    pub rows_visible: usize,
    /// Physics props by folder (relative to models/), from the installation at map load.
    #[serde(skip)]
    props: Vec<(String, Vec<String>)>,
    pub column: Column,
    /// Cursor row in the options column.
    pub row: usize,
    /// The chosen weapon ("" = none).
    pub weapon: String,
    pub allegiance: Allegiance,
    /// The NumGrenades given to classes that read it (`reads_num_grenades`).
    pub grenades: i32,
    /// Usable weapons per class (bits_CAP_USE_WEAPONS + Weapon_CanUse on the model the
    /// spawn uses); classes missing here have none.
    usable: std::collections::BTreeMap<String, Vec<String>>,
}
impl Overlay {
    /// The weapons each class can be given, from the loaded models (host, map load).
    pub fn set_usable(&mut self, usable: std::collections::BTreeMap<String, Vec<String>>) {
        self.usable = usable;
        self.select_class(self.class);
    }
    pub fn usable(&self, class: &str) -> &[String] {
        self.usable.get(class).map_or(&[], Vec::as_slice)
    }
    /// The Props tab's models by folder (host, map load: every installed model with a .phy).
    pub fn set_props(&mut self, folders: std::collections::BTreeMap<String, Vec<String>>) {
        self.props = folders.into_iter().filter(|(_, m)| !m.is_empty()).collect();
        self.folder = self.folder.min(self.prop_folders().len().saturating_sub(1));
        self.model = 0;
    }
    /// The installed prop folders: the installation listing, else `SPAWN_PROPS` by folder.
    fn installed_folders(&self) -> std::borrow::Cow<'_, [(String, Vec<String>)]> {
        if !self.props.is_empty() {
            return std::borrow::Cow::Borrowed(&self.props);
        }
        let mut folders = std::collections::BTreeMap::<String, Vec<String>>::new();
        for model in SPAWN_PROPS {
            let folder = model.rsplit_once('/').map_or("", |(f, _)| f);
            folders
                .entry(folder.into())
                .or_default()
                .push((*model).into());
        }
        std::borrow::Cow::Owned(folders.into_iter().collect())
    }
    /// The Props tab's folders as listed: without a filter the installed common props
    /// first, then every folder; with one, all matches first, then the folders that
    /// have matches (only their matching models).
    fn prop_folders(&self) -> Vec<(String, Vec<String>)> {
        let installed = self.installed_folders();
        if self.filter.is_empty() {
            let all: std::collections::BTreeSet<&str> = installed
                .iter()
                .flat_map(|(_, m)| m.iter().map(String::as_str))
                .collect();
            let common = SPAWN_PROPS
                .iter()
                .filter(|m| all.contains(**m))
                .map(|m| (*m).to_owned())
                .collect();
            let mut folders = vec![(COMMON_FOLDER.to_owned(), common)];
            folders.extend(installed.iter().cloned());
            folders.retain(|(_, m)| !m.is_empty());
            return folders;
        }
        let matches: Vec<(String, Vec<String>)> = installed
            .iter()
            .map(|(folder, models)| {
                let models = models
                    .iter()
                    .filter(|m| {
                        m.to_ascii_lowercase().contains(&self.filter)
                            || prop_name(m).is_some_and(|n| n.contains(&self.filter))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                (folder.clone(), models)
            })
            .filter(|(_, m)| !m.is_empty())
            .collect();
        let all = matches
            .iter()
            .flat_map(|(_, m)| m.iter().cloned())
            .collect();
        let mut folders = vec![(format!("* matches \"{}\"", self.filter), all)];
        folders.extend(matches);
        folders
    }
    /// Rows per page and per drawn list.
    fn page(&self) -> usize {
        if self.rows_visible == 0 {
            VISIBLE_CLASSES
        } else {
            self.rows_visible
        }
    }
    fn label(&self) -> &'static str {
        SPAWN_CLASSES.get(self.class).copied().unwrap_or("")
    }
    /// The selected prop's model argument (Props tab).
    fn prop(&self) -> Option<String> {
        if self.tab != Tab::Props {
            return None;
        }
        let folders = self.prop_folders();
        folders.get(self.folder)?.1.get(self.model).cloned()
    }
    fn class_name(&self) -> &'static str {
        spawn_class(self.label()).0
    }
    fn rows(&self) -> Vec<OptionRow> {
        let mut rows = vec![OptionRow::Weapon(String::new())];
        rows.extend(
            self.usable(self.label())
                .iter()
                .cloned()
                .map(OptionRow::Weapon),
        );
        rows.push(OptionRow::Allegiance);
        if reads_num_grenades(self.class_name()) {
            rows.push(OptionRow::Grenades);
        }
        rows.extend([OptionRow::Notarget, OptionRow::Ai, OptionRow::FriendlyFire]);
        rows
    }
    fn select_class(&mut self, class: usize) {
        self.class = class;
        let label = self.label();
        let default = default_weapon(label);
        self.weapon = if self.usable(label).iter().any(|w| w == default) {
            default.into()
        } else {
            String::new()
        };
        let weapon = OptionRow::Weapon(self.weapon.clone());
        self.row = self.rows().iter().position(|r| *r == weapon).unwrap_or(0);
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.typing = false;
    }
    fn switch_tab(&mut self, tab: Tab) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        self.column = Column::Classes;
        self.typing = false;
        if self.tab == Tab::Npcs {
            self.select_class(self.class);
        }
    }
    /// Typed text for the Props filter (only while `typing`); the selection restarts at
    /// the matches.
    pub fn type_text(&mut self, text: &str) {
        if !self.open || !self.typing {
            return;
        }
        let before = self.filter.len();
        self.filter.extend(text.chars().filter_map(filter_char));
        if self.filter.len() != before {
            (self.folder, self.model) = (0, 0);
        }
    }
    /// Escape: stop typing, else close the overlay. False when it was closed already.
    pub fn escape(&mut self) -> bool {
        if !self.open {
            return false;
        }
        if self.typing {
            self.typing = false;
        } else {
            self.open = false;
        }
        true
    }
    /// Apply one key: Tab switches NPCs / Props; Up/Down (PageUp/PageDown: a page) move
    /// in the focused column (lists wrap; moving onto a weapon row chooses it), Left/Right
    /// switch columns, Enter toggles the option under the cursor (allegiance, notarget,
    /// AI) or else spawns. Props: Enter on a folder opens its models; Filter starts or
    /// stops typing into the filter, Backspace edits it.
    pub fn key(&mut self, key: OverlayKey) -> Option<OverlayCommand> {
        if key == OverlayKey::Toggle {
            self.toggle();
            return None;
        }
        if !self.open {
            return None;
        }
        match key {
            OverlayKey::Tab => {
                self.switch_tab(match self.tab {
                    Tab::Npcs => Tab::Props,
                    Tab::Props => Tab::Npcs,
                });
                return None;
            }
            OverlayKey::Filter => {
                self.switch_tab(Tab::Props);
                self.typing = !self.typing;
                return None;
            }
            OverlayKey::Backspace => {
                if self.typing && self.filter.pop().is_some() {
                    (self.folder, self.model) = (0, 0);
                }
                return None;
            }
            // Enter finishes typing; the arrows still move while typing.
            OverlayKey::Enter if self.typing => {
                self.typing = false;
                self.column = Column::Options;
                return None;
            }
            _ => {}
        }
        let step = KeyStep::new(key, self.page() as isize);
        if self.tab == Tab::Props {
            return self.key_props(key, step);
        }
        self.key_npcs(key, step)
    }
    fn key_props(&mut self, key: OverlayKey, step: KeyStep) -> Option<OverlayCommand> {
        match key {
            OverlayKey::Left => self.column = Column::Classes,
            OverlayKey::Right => self.column = Column::Options,
            OverlayKey::Up | OverlayKey::Down | OverlayKey::PageUp | OverlayKey::PageDown => {
                let folders = self.prop_folders();
                let models = folders.get(self.folder).map_or(0, |f| f.1.len());
                let count = folders.len();
                if self.column == Column::Classes {
                    self.folder = step.moved(self.folder, count);
                    self.model = 0;
                } else {
                    self.model = step.moved(self.model, models);
                }
            }
            OverlayKey::Enter => {
                if self.column == Column::Classes {
                    self.column = Column::Options;
                    return None;
                }
                return self.prop().map(OverlayCommand::SpawnProp);
            }
            _ => {}
        }
        return None;
    }
    fn key_npcs(&mut self, key: OverlayKey, step: KeyStep) -> Option<OverlayCommand> {
        match key {
            OverlayKey::Left => self.column = Column::Classes,
            OverlayKey::Right => self.column = Column::Options,
            OverlayKey::Up | OverlayKey::Down | OverlayKey::PageUp | OverlayKey::PageDown => {
                match self.column {
                    Column::Classes => {
                        self.select_class(step.moved(self.class, entry_count()));
                    }
                    Column::Options => {
                        let rows = self.rows();
                        self.row = step.moved(self.row, rows.len());
                        if let OptionRow::Weapon(weapon) = &rows[self.row] {
                            self.weapon = weapon.clone();
                        }
                    }
                }
            }
            OverlayKey::Enter => {
                if self.column == Column::Options {
                    match self.rows()[self.row] {
                        OptionRow::Allegiance => {
                            self.allegiance = self.allegiance.next();
                            return None;
                        }
                        OptionRow::Grenades => {
                            let next = GRENADE_CHOICES
                                .iter()
                                .position(|&n| n == self.grenades)
                                .map_or(0, |i| (i + 1) % GRENADE_CHOICES.len());
                            self.grenades = GRENADE_CHOICES[next];
                            return None;
                        }
                        OptionRow::Notarget => return Some(OverlayCommand::ToggleNotarget),
                        OptionRow::Ai => return Some(OverlayCommand::ToggleAi),
                        OptionRow::FriendlyFire => return Some(OverlayCommand::ToggleFriendlyFire),
                        OptionRow::Weapon(_) => {}
                    }
                }
                return Some(OverlayCommand::Spawn(self.request()));
            }
            _ => {}
        }
        None
    }
    /// A pointer click on a drawn row: tabs switch, the filter box starts typing, a "more"
    /// note pages, an item is selected and a click on the selected item acts like Enter
    /// (spawn, open a folder); option toggles act on every click.
    pub fn click(&mut self, target: PanelTarget) -> Option<OverlayCommand> {
        if !self.open {
            return None;
        }
        match target {
            PanelTarget::Tab(tab) => {
                self.switch_tab(tab);
                self.typing = false;
                None
            }
            PanelTarget::Filter => {
                self.switch_tab(Tab::Props);
                self.typing = true;
                None
            }
            PanelTarget::Page(column, direction) => {
                self.typing = false;
                self.column = column;
                self.scroll(Some(column), direction * self.page() as isize);
                None
            }
            PanelTarget::Row(column, index) => {
                self.typing = false;
                let focused = self.column == column;
                self.column = column;
                match (self.tab, column) {
                    (Tab::Props, Column::Classes) => {
                        if focused && self.folder == index {
                            self.column = Column::Options;
                        } else if self.folder != index {
                            (self.folder, self.model) = (index, 0);
                        }
                        None
                    }
                    (Tab::Props, Column::Options) => {
                        if focused && self.model == index {
                            return self.prop().map(OverlayCommand::SpawnProp);
                        }
                        self.model = index;
                        None
                    }
                    (Tab::Npcs, Column::Classes) => {
                        if focused && self.class == index {
                            return Some(OverlayCommand::Spawn(self.request()));
                        }
                        if index < entry_count() {
                            self.select_class(index);
                        }
                        None
                    }
                    (Tab::Npcs, Column::Options) => {
                        let rows = self.rows();
                        let row = rows.get(index)?;
                        let again = focused && self.row == index;
                        self.row = index;
                        match row {
                            OptionRow::Weapon(weapon) if !again => {
                                self.weapon = weapon.clone();
                                None
                            }
                            // Toggles act on every click; the chosen weapon spawns.
                            _ => self.key(OverlayKey::Enter),
                        }
                    }
                }
            }
        }
    }
    /// Mouse wheel over a column (None: the focused one): moves that column's selection
    /// by `rows`, clamped at the ends (a weapon row reached is chosen).
    pub fn scroll(&mut self, column: Option<Column>, rows: isize) {
        if !self.open || rows == 0 {
            return;
        }
        let column = column.unwrap_or(self.column);
        self.column = column;
        let moved =
            |at: usize, n: usize| (at as isize + rows).clamp(0, n.max(1) as isize - 1) as usize;
        match (self.tab, column) {
            (Tab::Props, Column::Classes) => {
                let folder = moved(self.folder, self.prop_folders().len());
                if folder != self.folder {
                    (self.folder, self.model) = (folder, 0);
                }
            }
            (Tab::Props, Column::Options) => {
                let models = self
                    .prop_folders()
                    .get(self.folder)
                    .map_or(0, |f| f.1.len());
                self.model = moved(self.model, models);
            }
            (Tab::Npcs, Column::Classes) => self.select_class(moved(self.class, entry_count())),
            (Tab::Npcs, Column::Options) => {
                let options = self.rows();
                self.row = moved(self.row, options.len());
                if let OptionRow::Weapon(weapon) = &options[self.row] {
                    self.weapon = weapon.clone();
                }
            }
        }
    }
    /// The overlay spawns at the crosshair like npc_create_aimed (facing the aim).
    pub fn request(&self) -> SpawnRequest {
        SpawnRequest {
            classname: self.class_name().into(),
            name: None,
            equipment: self.weapon.clone(),
            aimed: true,
            allegiance: self.allegiance,
            variant: self.label().into(),
            grenades: reads_num_grenades(self.class_name()).then_some(self.grenades),
            ..Default::default()
        }
    }
    /// The panel to draw; `ai` and `notarget` are the host's live switches.
    pub fn panel(&self, ai: bool, notarget: bool) -> Panel {
        let total = entry_count();
        let page = self.page();
        let classes_focused = self.column == Column::Classes;
        let tab = |text: &str, t: Tab| PanelRow {
            text: text.into(),
            selected: self.tab == t,
            cursor: false,
            target: Some(PanelTarget::Tab(t)),
        };
        let mut tabs = vec![tab("NPCs", Tab::Npcs), tab("Props", Tab::Props)];
        if self.tab == Tab::Props {
            let text = if self.typing {
                format!("filter: {}_", self.filter)
            } else if self.filter.is_empty() {
                "filter: (click or /)".into()
            } else {
                format!("filter: {}", self.filter)
            };
            tabs.push(PanelRow {
                text,
                selected: !self.filter.is_empty(),
                cursor: self.typing,
                target: Some(PanelTarget::Filter),
            });
        }
        if self.tab == Tab::Props {
            return self.props_panel(tabs, page, classes_focused);
        }
        let names: Vec<String> = SPAWN_CLASSES.iter().map(|c| (*c).to_owned()).collect();
        let classes = scrolled_rows(&names, self.class, classes_focused, page, Column::Classes);
        let natural = !hl2_simulation::npc_weapons::uses_weapons(self.class_name());
        let options = self
            .rows()
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let (text, selected) = self.npc_option_text(&row, natural, ai, notarget);
                PanelRow {
                    text,
                    selected,
                    cursor: !classes_focused && index == self.row,
                    target: Some(PanelTarget::Row(Column::Options, index)),
                }
            })
            .collect();
        Panel {
            title: format!(
                "F4 spawn: NPCs ({}/{total}): click: select, again: spawn / toggle; Enter spawns",
                self.class + 1
            ),
            tabs,
            headings: ["NPC class".into(), "weapon / options".into()],
            classes,
            options,
        }
    }
    fn props_panel(&self, tabs: Vec<PanelRow>, page: usize, classes_focused: bool) -> Panel {
        let folders = self.prop_folders();
        let names: Vec<String> = folders
            .iter()
            .map(|(folder, models)| {
                let folder = if folder.is_empty() {
                    "(models)"
                } else {
                    folder
                };
                format!("{folder}/ ({})", models.len())
            })
            .collect();
        // The common/matches list mixes folders: it shows each model's folder too.
        let mixed = self.folder == 0;
        let models: Vec<String> = folders.get(self.folder).map_or(Vec::new(), |(_, m)| {
            m.iter()
                .map(|m| {
                    let shown = if mixed {
                        m.as_str()
                    } else {
                        m.rsplit('/').next().unwrap_or(m)
                    };
                    let shown = shown.trim_end_matches(".mdl");
                    match prop_name(m) {
                        Some(name) => format!("{shown}  ({name})"),
                        None => shown.into(),
                    }
                })
                .collect()
        });
        let count: usize = self.installed_folders().iter().map(|f| f.1.len()).sum();
        Panel {
            title: format!(
                "F4 Props ({count} models): folder {}/{}, model {}/{}  click: select, again: spawn",
                self.folder + 1,
                folders.len(),
                self.model + 1,
                models.len()
            ),
            tabs,
            headings: ["folder".into(), "model (Enter spawns)".into()],
            classes: scrolled_rows(&names, self.folder, classes_focused, page, Column::Classes),
            options: scrolled_rows(&models, self.model, !classes_focused, page, Column::Options),
        }
    }
    fn npc_option_text(
        &self,
        row: &OptionRow,
        natural: bool,
        ai: bool,
        notarget: bool,
    ) -> (String, bool) {
        match row {
            OptionRow::Weapon(w) if w.is_empty() => (
                if natural {
                    "no weapons (natural attacks)".into()
                } else {
                    "no weapon".into()
                },
                self.weapon.is_empty(),
            ),
            // Where the choice comes from: the class's HL2 map loadouts or
            // Weapon_CanUse on its model.
            OptionRow::Weapon(w) => (
                if hl2_simulation::npc_weapons::campaign_loadout(self.class_name())
                    .contains(&w.as_str())
                {
                    format!("{w} (HL2 maps)")
                } else {
                    format!("{w} (can use)")
                },
                *w == self.weapon,
            ),
            OptionRow::Allegiance => (format!("allegiance: {}", self.allegiance.label()), false),
            OptionRow::Grenades => (format!("grenades: {} (NumGrenades)", self.grenades), false),
            OptionRow::Notarget => (
                format!("notarget: {}", if notarget { "ON" } else { "off" }),
                false,
            ),
            OptionRow::Ai => (
                format!("AI: {}", if ai { "ON" } else { "OFF (idle)" }),
                false,
            ),
            OptionRow::FriendlyFire => (
                format!(
                    "player hurts friendlies: {} (debug)",
                    if self.friendly_fire { "ON" } else { "off" }
                ),
                false,
            ),
        }
    }
}

/// How far one list key moves: a page clamps at the ends, single steps wrap.
#[derive(Clone, Copy)]
struct KeyStep {
    step: isize,
    paged: bool,
}

impl KeyStep {
    fn new(key: OverlayKey, page: isize) -> Self {
        let step = match key {
            OverlayKey::Up => -1,
            OverlayKey::PageUp => -page,
            OverlayKey::PageDown => page,
            _ => 1,
        };
        let paged = matches!(key, OverlayKey::PageUp | OverlayKey::PageDown);
        Self { step, paged }
    }
    fn moved(self, at: usize, n: usize) -> usize {
        if n == 0 {
            0
        } else if self.paged {
            (at as isize + self.step).clamp(0, n as isize - 1) as usize
        } else {
            (at as isize + self.step).rem_euclid(n as isize) as usize
        }
    }
}

#[cfg(test)]
mod tests;
