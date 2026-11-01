# Non-retail features

HL2-RS adds the following controls and developer tools beyond retail Half-Life 2.
These are deliberate extensions, not claims of retail fidelity. This guide describes
current behavior; implementation history is in [GameUI notes](gameui.md).

They are optional source-built plugins, both on by default, and a build can leave
either out ([plugins](plugins.md)). `hl2-advanced-options` owns motion blur tiers,
aspect choices, the keep-or-revert dialog, the FOV sliders and portrait and ultrawide
weapon framing. `hl2-debug` owns the debug modes below and the F1, F2, F3 and F4 keys.
Saved values for an omitted plugin are kept and apply again when it returns.

## Advanced and Ultra motion blur

Open **Options > Video > Advanced > Motion Blur**. The four choices are:

| Choice | Behavior |
| --- | --- |
| Disabled | Motion blur off. |
| Enabled | Retail-style camera motion blur. |
| Advanced | Retail-style camera blur plus reconstructed weapon/viewmodel animation blur. |
| Ultra | Weapon animation blur plus denser reconstructed camera/world motion and opaque or masked moving-model motion, including projectiles. |

Advanced and Ultra are HL2-RS extensions. Ultra does not reconstruct transparent
particle motion, and its GPU performance tuning remains experimental. Cost depends
on resolution, antialiasing and hardware; no sustained FPS guarantee is established.
See [performance evidence](performance.md) and [validation](validation.md).
The selected tier is saved with the video settings.

## Additional aspect ratios

Open **Options > Video > Aspect Ratio**. Alongside the retail aspect choices,
HL2-RS offers:

- **Fit Display**: follows the current window or display shape dynamically, bounded
  between 9:16 and 32:9. The Resolution selector is disabled in this mode and shows
  the active content size. Internal rendering detail and presentation size remain
  distinct; a larger display can upscale the rendered frame.
- **Ultrawide 21:9** and **Super-ultrawide 32:9**.
- **Square 1:1**.
- **Portrait 3:4**, **Portrait 10:16** and **Portrait 9:16**.

Fixed aspect choices offer matching resolution lists. Fit Display fills the window
within its supported bounds; shapes beyond them retain bars. These choices are
separate from **HUD Aspect Ratio**, which controls HUD placement rather than the
3D rendering frame. Aspect settings are saved with the video settings.

## Keep or revert video changes

Applying a different **aspect-ratio choice**, including entering or leaving Fit
Display, opens a keep/revert confirmation after applying the new mode. Its countdown
is **15 seconds**. **OK** keeps the mode; **Cancel**, the close button, Escape or
expiry restores the previous resolution, display mode and Fit Display setting.
Cancel is initially focused, and unrelated settings are preserved on revert.

The current trigger compares aspect-ratio choices: changing only the resolution
within the same aspect ratio, or only the display mode, does not open this dialog.
It is therefore not a confirmation for every resolution change. The dialog shares
`source-ui::MessageBox` layout with the quit and return-to-menu confirmations.

## Vertical FOV slider

Open **Options > Video > Advanced > Vertical FOV**. **Auto** derives the gameplay
camera's vertical field of view from the normal FOV and aspect-dependent framing.
An explicit value overrides that vertical angle, currently **40–120 degrees** on the
slider. The `vertical_fov` console command accepts 40–100.
Weapon and scripted zoom still scale the view. Main-menu background projection
ignores this gameplay setting. The value is saved with the video settings.

## Viewmodel FOV slider

Open **Options > Video > Advanced > Viewmodel FOV**. This changes the weapon camera's
base field of view: currently **30–110 degrees** on the slider, with a default of
**54 degrees**. The `viewmodel_fov` console command accepts 40–80.
The angle is horizontal at the 4:3 reference aspect, not a vertical FOV. A larger
value makes the weapon appear smaller and exposes more of the model. Zoom and
per-weapon/aspect framing adjustments still apply. The value is saved with the
video settings.

## Viewmodel debug mode

Load a gameplay map, open the developer console and enter `viewmodel_debug_mode 1`.
This runtime tool shows the active weapon, aspect profile, FOV offset and separate
X/Y/Z offsets. It cannot start on a menu background or while a map is loading.

| Control | Effect |
| --- | --- |
| `[` / `]` | Decrease / increase the FOV offset. |
| `,` / `.` | Decrease / increase X, the forward offset. |
| Left / Right | Increase / decrease Y, the leftward offset. |
| Up / Down | Increase / decrease Z, the upward offset. |
| Shift | Use a 1.0 step instead of the default 0.1. |
| Alt / Option | Use a 0.01 step. |
| `0` or `viewmodel_tweak_reset` | Zero the current weapon/profile's FOV and position offsets. |
| `viewmodel_debug_mode 0` | End the session and discard its temporary edits. |

Adjustment keys repeat while held. Normal movement, aiming, firing, reloading and
weapon selection remain available. Pause and Options suspend adjustment without
ending the session, allowing aspect changes between comparisons. Edits are retained
by weapon and aspect profile within that session; a map load or return to the main
menu ends it. F5 and F6 are not bound to this tool.

Each adjustment removes the previous tuning snapshot and appends the latest complete
tuning JSON at the end of the console log, after any intervening ordinary output. Zero FOV offsets and entirely zero offset arrays are omitted, including in
untouched profiles. Zero components remain in nonzero arrays, for example
`"standard": {"offset": [0.0, 0.0, 0.2]}`. The tool does not save its edits automatically: copy that JSON into
`plugins/hl2-advanced-options/assets/ui/tuning.json` to retain authored changes. Without
that plugin only a `standard` profile exists, the snapshot lists just the session's edits,
and there is no framing data file to copy them into.

## Menu background debug mode

`background_force_default_blur 0|1` is a saved option that works independently of
debug mode. **0** (the default) selects widescreen blur artwork on wide aspect
ratios, with the normal existing fallback when that artwork is unavailable. **1**
always selects the normal blur texture, even on wide frames. The choice applies to
menu and loading backgrounds and survives restarting the game. Both variants fill
the frame by center-cropping against their authored aspect (normal 4:3, wide 16:9),
so normal artwork loses top/bottom portions on wide frames without stretching.
With the override enabled, the background camera matches this 4:3 crop; disabling
it restores the original aspect-based FOV. Toggling resets temporary debug FOV
tuning; arrows and reset then use the selected artwork baseline. Query it by entering
`background_force_default_blur` without an argument.


From the main menu, enter `background_debug_mode 1` in the developer console.
It focuses the menu so its debug keys work immediately. The overlay reports the
background state, map, pre-blurred texture, overlay opacity and camera FOV. This is
a tool for comparing the live background with the installed pre-blurred artwork,
not an Advanced/Ultra motion-blur setting.

| Control | Effect |
| --- | --- |
| Up / Down | Increase / decrease the background camera's vertical FOV. |
| `0` | Restore the authored background FOV. |
| `-` | Toggle the saved `background_force_default_blur` option; its 0/1 state and this binding appear in the overlay. |
| `,` / `.` | Decrease / increase overlay opacity by one percentage point. |
| Left / Right | Toggle opacity to 0% / 100%; repeat to restore the base opacity. |
| `1`, `2`, `3`, `4` | Switch to the initial background for Half-Life 2, Episode One, Episode Two or Lost Coast, using installed resources. |
| `[` / `]` | Previous / next unique chapter background across installed games, wrapping at the ends. |
| Shift / Alt or Option | FOV step 1.0 / 0.01, instead of the default 0.1. |
| `background_debug_mode 0` | Disable the mode and clear its temporary FOV override. |

FOV and incremental opacity keys repeat while held. Opening the console suspends
the debug keys. `background_blur_opacity 0..100` sets opacity while debug mode is
active; `background_map_fov [degrees]` queries or sets the background FOV override
(the explicit range is 1–170 degrees).

The mode is limited to the main-menu background/loading context. Starting gameplay
disables it, and background debug and viewmodel debug modes exclude each other.
During background loading or failure, the loading presentation can force the blur
fully opaque; the opacity comparison becomes meaningful once the background is
ready. The FOV/opacity tools are runtime diagnostics, not saved gameplay camera settings.
The standard-blur override is the saved exception: turning debug mode off does not
reset it. Holding `-` does not repeat the toggle, and the key is inactive while the
console is open.
