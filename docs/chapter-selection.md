# Chapter selection research and implementation

## Evidence reviewed (2026-10-27)

Installed content is authoritative. The desktop layouts are
`hl2/resource/newgamedialog.res` (600 x 296; Prev, Next, Play, Cancel and two
separators; Lost Coast overrides the frame width to 300) and `hl2/resource/newgamechapterpanel.res` (chapter heading/name,
152 x 86 picture in a 168-wide border, normalized to a uniform 8 px border on all
four sides at 0, 44, 168, 102 with the picture at 8, 52, 152, 86 so the three
168 x 146 cards align flush with the 548 px separators and titles never overhang
the border). SDK `vgui2/vgui_controls/ImagePanel.cpp`
paints unscaled images at native size and clips them to the panel. The chapter
textures contain power-of-two padding: sample their top-left 152 x 86 region
before proportional UI scaling; stretching the entire texture makes the picture
small inside a black padded area and is rejected. The selected game's VFS resolves
`materials/vgui/chapters/chapter<key>.vmt` and its `$basetexture`, rather than
assuming a texture name or using replacement artwork. Scheme colors resolve
`NewGame.TextColor`, `NewGame.FillColor` and `NewGame.SelectionColor`; the
selected card applies `NewGame.SelectionColor` to both `LevelPicBorder` and the
card's heading and title labels (`10071090`), and card labels size from the
scheme's `ChapterTitle` / `UiBold` height.

Each game's own `cfg/chapter*.cfg` files define selectable entries, independent
of its background index. Half-Life 2 ships 15 configs: 1 through 14, including
9a; 14 starts credits. Episode One ships five, Episode Two seven, Lost Coast
one. Episode One's background file also lists 6–8; those must not become
inherited Half-Life 2 chapters. Episode Two's extra thumbnail assets 0 and 8
do not establish launchable chapters. Its first config contains
`playvideo_exitcommand ep1_recap map ep2_outland_01`; the existing config parser
extracts the start map, without claiming movie playback.

Text is installed localization: `#GameUI_NewGame`, `#GameUI_Chapter`, the
layouts' button `labelText`, and `#<game>_Chapter<key>_Title`. Expansion names
come from `resource/episodic_english.txt`, `resource/ep2_english.txt` and
`resource/lostcoast_english.txt`, loaded with the same decoding/localization
boundary as existing GameUI resources. Lost Coast retains the wider separator dimensions in its override; decorative rules
are sized to the frame interior using the authored left inset, and its single
chapter card is centered. No new English strings or localization
placeholders are needed. The current resource loader uses English, like the
other GameUI screens; alternate UI-language selection is not implemented here.

Identity-pinned retail evidence: `bin/gameui.dll`, SHA256
`62ada6888369bed1a01a2aac50bc81e689a6e7b8df5cbfdc350a8ad690108136`,
original successful exports retrieved from the pass14 index. Private bodies and
identity records remain outside the source checkout. Addresses are evidence
anchors, not adopted names, signatures or a claim of semantic parity:

- `10070840`: enumerates nonempty `cfg/chapter*.cfg` in MOD, or the selected
  expansion's cfg directory in BASE_PATH; constructs game-specific title tokens
  and chapter panels, loads the dialog resource, and lays out the browser.
- `1006f670`: numeric chapter ordering with length/lexical tiebreaking, preserving
  9, 9a, 10 rather than lexical 1, 10, 11, 2.
- `1006e120`: chapter heading through `#GameUI_Chapter`, game-specific title,
  chapter material path and installed chapter-panel resource.
- `1006ff20`: Play and Next/Prev are separate commands; navigation scrolls the
  browser and Play starts the selected entry through its launch flow.
- `1006fa60`: base or expansion-specific unlocked-chapter cvar lookup. Progress
  locking is not reconstructed here. Every discovered launchable entry is exposed.

The public [Valve Source SDK](https://github.com/ValveSoftware/source-sdk-2013)
contains shared VGUI Button, Frame, Label and filesystem interfaces, not this
retail GameUI dialog implementation. SDK `game/server/EnvMessage.cpp`,
`CCredits::RollOutroCredits`, sets the chapter unlock value to 15; that supports
keeping the credits config distinct from an ordinary numbered campaign chapter.
The toolkit supplies the reusable card browser plus dialog button capture, focus, key release activation,
window ordering, frame drawing and placement; none are copied into a new widget.

The existing GameUI/settings reference was searched for chapter-specific findings
and supplied none. Web research located the Valve Developer Community article
[Adding chapters to your mod](https://developer.valvesoftware.com/wiki/Adding_chapters_to_your_mod),
but direct retrieval returned HTTP 403. It is not used as behavioral evidence.
Community guides/screenshots were discovery leads only; resource and private
retail findings above determine the implementation. Native desktop interaction
and pixel-for-pixel comparison remain **not tested**.

## Implementation boundary

`source-assets::vpk::Vfs::list_game_files` enumerates the selected game's own
mounted folders, custom packs and archives, excluding inherited games and
map-embedded files. MOD-scoped VFS priority resolves the discovered config; the selected game's normal
VFS resolves its material. `hl2-ui::gameui::new_game` prepares chapter metadata and interprets installed
resource layouts/localization into `source_ui::card_browser` values. The toolkit
owns the reusable paged card dialog: geometry, drawing, selection, focus, capture
and placement, and returns only an accepted card index. The HL2 adapter converts
that index to its game/map. The console owns its non-modal window and emits `Effect::ChapterMap(GameMap)`. The Bevy host
sets both request game and map; the existing asynchronous campaign loader installs
it as gameplay. Failed loads keep the existing error handling.

New Game opens a three-card browser. Clicking a card selects it; Start New Game
launches it. Left/Right navigate chapters, including across pages; Enter/Space on
a focused card launches it. Prev/Next page by three. Tab/Shift+Tab and all dialog
button press/release behavior use the shared toolkit. Cancel, close and Escape
return to the selected game's menu without loading a map. Escape closes on
release. Chapter labels and textures persist while closed; ellipsized titles are
prepared once, and input/draw do no asset reads or chapter-list allocations.

Resource-driven controls and thumbnails are implemented. Retail scrolling animation,
progress locking, commentary options, intro movies, save overwrite confirmation
and controller/GamepadUI variants are **not implemented** by this change. Chapter
start selection does not establish campaign playthrough parity.
