//! Installed GameUI scheme setup.
use super::{scheme_color, Console, ConsoleStyle, Style, Theme};
use source_assets::{keyvalues, vpk::Vfs};
impl Console {
    pub(super) fn load_scheme(&mut self, vfs: &Vfs) {
        // Installed desktop GameUI colors. Layout/window controls remain an initial implementation.
        let scheme_reader = crate::gameui::resources::Reader::new(vfs);
        if let Some(root) = scheme_reader
            .read("resource/SourceSchemeBase.res")
            .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
            .and_then(|roots| roots.into_iter().next())
        {
            self.theme = Theme::from_scheme(&root);
            if let Some(time) = root
                .get("BaseSettings")
                .and_then(|e| e.get("Frame.FocusTransitionEffectTime"))
                .and_then(|e| e.text())
                .and_then(|t| t.trim().parse::<f32>().ok())
            {
                self.shell.windows.fade_time = time;
            }
            self.style = Style {
                menu: scheme_color(&root, "MainMenu.TextColor").unwrap_or(self.style.menu),
                menu_armed: scheme_color(&root, "MainMenu.ArmedTextColor")
                    .unwrap_or(self.style.menu_armed),
                panel: scheme_color(&root, "Frame.BgColor").unwrap_or(self.style.panel),
            };
            let style = &mut self.shell.panel.style;
            *style = ConsoleStyle {
                entry: scheme_color(&root, "RichText.BgColor").unwrap_or(style.entry),
                text: scheme_color(&root, "Console.TextColor").unwrap_or(style.text),
                dev_text: scheme_color(&root, "Console.DevTextColor").unwrap_or(style.dev_text),
                ..*style
            };
            self.menu_layout.apply_source_scheme(&root);
        }
        // The console and history colors from the installed source scheme (the base scheme it
        // reads above is empty in the installed game). GameUI loads this scheme, so its
        // MainMenu.MenuItemHeight and MenuLarge also size the menu (hl2_complete: 22 and a
        // proportional GorDIN 12, where the platform base has a fixed 16 up to 1080 lines).
        if let Some(root) = scheme_reader
            .read("resource/SourceScheme.res")
            .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
            .and_then(|roots| roots.into_iter().next())
        {
            self.menu_layout.apply_source_scheme(&root);
            for (name, color) in [
                ("Console.TextColor", &mut self.shell.panel.style.text),
                ("Console.DevTextColor", &mut self.shell.panel.style.dev_text),
                ("RichText.BgColor", &mut self.shell.panel.style.entry),
                (
                    "RichText.SelectedBgColor",
                    &mut self.shell.panel.style.selected_bg,
                ),
                (
                    "RichText.SelectedTextColor",
                    &mut self.shell.panel.style.selected_text,
                ),
            ] {
                if let Some(value) = scheme_color(&root, name) {
                    *color = value;
                }
            }
        }
        if let Some(root) = scheme_reader
            .read("resource/ClientScheme.res")
            .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
            .and_then(|roots| roots.into_iter().next())
        {
            self.menu_layout.apply_client_scheme(&root);
        }
    }
}
