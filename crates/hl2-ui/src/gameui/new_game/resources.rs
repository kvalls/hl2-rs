//! Campaign resources are prepared once, outside frame systems.
use crate::gameui::resources::{
    background_map, blur, chapter_background_maps, image, load_vtf_file, menu_games,
    GameUiResources, Reader,
};
use source_assets::{keyvalues::decode_text, vpk::Vfs};
use source_ui::layout::Layout;

impl GameUiResources {
    pub(crate) fn load_campaigns(&mut self, vfs: &Vfs, reader: &Reader<'_>) {
        self.games = menu_games(&self.game_menu);
        for game in self.games.clone() {
            let game_vfs = vfs
                .game_root()
                .and_then(|root| Vfs::mount_game(root, &game).ok());
            let own = game_vfs.as_ref().unwrap_or(vfs);
            let chapter_reader = Reader::new(own);
            let catalog = super::Catalog::load(own, &game, &mut self.localization);
            if let Some(first) = catalog.entries.first() {
                self.new_game_maps.insert(game.clone(), first.map.clone());
            }
            self.new_games.insert(game.clone(), catalog);
            let Some(text) = chapter_reader.read_text("scripts/chapterbackgrounds.txt") else {
                self.missing
                    .push(format!("{game}/scripts/chapterbackgrounds.txt"));
                continue;
            };
            if let Some(first_map) = background_map(&text) {
                self.background_maps.insert(game.clone(), first_map);
            }
            self.chapters.insert(
                game.clone(),
                crate::gameui::chapters::list(&text, |path| chapter_reader.read_text(path)),
            );

            // Pre-blurred background images for every chapter background of this game. Maps
            // are keyed by game: Half-Life 2 and Lost Coast both have a `background01`.
            for map_name in chapter_background_maps(&text) {
                let map = blur(&map_name);
                for image_name in [map.to_owned(), format!("{map}_widescreen")] {
                    if let Some(img) = load_vtf_file(reader, vfs, &game, &image_name) {
                        self.background_images
                            .insert((game.clone(), image_name), img);
                    }
                }
            }
        }
        // The main category shows the first game's background (Half-Life 2 in the menu).
        self.background_map = self.background_maps.get(self.main_game()).cloned();
    }
}

impl super::Catalog {
    pub(super) fn load(
        vfs: &Vfs,
        game: &str,
        loc: &mut crate::gameui::localize::Localization,
    ) -> Self {
        let reader = Reader::new(vfs);
        let read = |path: &str| reader.read(path);
        let file = format!("resource/{game}_english.txt");
        if let Some(bytes) = read(&file) {
            loc.add_resource(&file, &bytes);
        }
        let files = vfs.list_game_files(game, "cfg/", ".cfg");
        let entries = super::catalog(
            files.iter().map(String::as_str),
            |path| decode_text(&vfs.read_game_file(game, path).ok()??).ok(),
            game,
            loc,
        );
        let thumbnails = entries
            .iter()
            .map(|entry| image(vfs, &format!("vgui/chapters/chapter{}", entry.key)))
            .collect();
        let layout = |name: &str| {
            read(name)
                .as_deref()
                .and_then(Layout::parse)
                .unwrap_or_default()
        };
        let dialog = layout("resource/newgamedialog.res");
        let panel = layout("resource/newgamechapterpanel.res");
        let commentary_icon = dialog
            .get("CommentaryIcon")
            .or_else(|| panel.get("CommentaryIcon"))
            .and_then(|c| {
                let name = c.image.trim_start_matches('/');
                (!name.is_empty()).then(|| image(vfs, &format!("vgui/{name}")))?
            })
            .or_else(|| image(vfs, "vgui/resource/commentary_hl2c"))
            .or_else(|| image(vfs, "vgui/hud/icon_commentary_small"));
        let scheme = reader
            .read("resource/sourcescheme.res")
            .and_then(|b| source_assets::keyvalues::parse_resource(&b).ok())
            .and_then(|roots| roots.into_iter().next());
        let color = |name| {
            scheme
                .as_ref()
                .and_then(|root| source_ui::widgets::scheme_color(root, name))
        };
        let title = read("resource/newgamedialog.res")
            .and_then(|bytes| source_assets::keyvalues::parse_resource(&bytes).ok())
            .and_then(|roots| roots.into_iter().next())
            .and_then(|root| {
                root.get("NewGameDialog")?
                    .get("title")?
                    .text()
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let font_tall = scheme
            .as_ref()
            .and_then(|root| {
                source_ui::scheme_fonts::scheme_font_tall(root, "ChapterTitle")
                    .or_else(|| source_ui::scheme_fonts::scheme_font_tall(root, "UiBold"))
            })
            .map(|tall| tall.clamp(14., 15.));
        Self {
            title,
            selection: color("NewGame.SelectionColor"),
            fill: color("NewGame.FillColor"),
            text: color("NewGame.TextColor"),
            font_tall,
            entries,
            thumbnails,
            commentary_icon,
            dialog,
            panel,
        }
    }
}
