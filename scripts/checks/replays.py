"""Packaged replays and captures, with assertions on each run's report.

Cross-platform (Python 3.9+, no packages): builds the packaged game once
(scripts/build-bevy.sh, or scripts/build-bevy.ps1 on Windows), copies it to
artifacts/replays/game, runs every check as its own bounded launch of that copy with its own
`--report`, and checks what the report records. Prints one ok/FAIL line per check; details in
artifacts/replays/<check>.log, reports and captures next to it.

Groups:
  ui          menus, Options, console, campaign transitions, captions, saved settings: the
              labelled UI states, loaded maps and saved settings
  gameplay    every other test-inputs fixture on its map, three layers deep: checks every
              fixture must pass (labels at their ticks, capture, entity poses, I/O budget, asset
              and sound loading), specific assertions for movement, doors, weapons, effects,
              particles and attention, and each labelled sample's outcome compared with
              test-inputs/expected/<fixture>.json; plus `coverage`, which fails when a fixture
              is not run by any check

Every check starts from default settings (the copy's own hl2-config.json; bin/hl2-config.json
is never touched; a check that needs particular settings writes them itself).

Categories:
  shared      every platform, in the background (`--background`: no window, nothing takes
              focus, `--capture` reads the game frame)
  windowed    every platform, needs a real window and takes focus (opt in: WINDOWED=1)
  macos       macOS only, windowed (opt in: WINDOWED=1)
  windows     Windows only, windowed (opt in: WINDOWED=1)

Environment (no arguments, so the command line never changes):
  ONLY="console options"   run these checks only
  GROUP=ui | GROUP=gameplay   run one group
  JOBS=4                   run up to 4 gameplay replays at once (default 1), never more than
                           fit in physical memory (GAME_MEMORY_GB per game, default 6, plus
                           4 GB left to the system), so the machine never swaps
  WINDOWED=1               also run the windowed checks of this platform
  NO_BUILD=1               use bin/ as it is
  UPDATE_EXPECTED=1        record the gameplay outcomes as expected (after a reviewed change)
"""
from __future__ import annotations

import json
import os
import platform
import re
import shutil
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path
from images import pixel_changes, png_rgb
from typing import Callable, Optional

REPO = Path(__file__).resolve().parents[2]
OUT = REPO / "artifacts" / "replays"
SYSTEM = platform.system()  # Darwin, Windows, Linux
EXE_NAME = "hl2-bevy.exe" if SYSTEM == "Windows" else "hl2-bevy"
# The checks run a copy of the packaged game, so its settings (saved next to the executable) are
# the checks' own: bin/hl2-config.json and a game running from bin/ are never touched.
GAME = OUT / "game"
EXE = GAME / EXE_NAME
CONFIG = GAME / "hl2-config.json"


# ---- running the game ---------------------------------------------------------------------

@dataclass
class Run:
    name: str
    exit: int
    report: dict
    log: str
    capture: Optional[Path]

    def samples(self) -> dict:
        return {s["label"]: s for s in self.report["render"]["simulation"]["samples"] if s.get("label")}

    def sample(self, label: str) -> dict:
        found = self.samples().get(label)
        if found is None:
            raise Failure(f"{self.name}: no sample labelled {label!r}")
        return found

    def console(self) -> dict:
        return self.report["render"]["presentation"]["console"]

    def loaded(self) -> list[str]:
        return [line[len("Loaded "):] for line in self.console()["output"] if line.startswith("Loaded ")]

    def campaign(self) -> dict:
        return self.report["render"]["presentation"]["campaign"]


class Failure(Exception):
    pass


def game(name: str, *args: str, timeout: int = 240, background: bool = True, capture: bool = True) -> Run:
    """One bounded launch; the report and capture are written to artifacts/replays/<name>.*"""
    report = OUT / f"{name}.json"
    png = OUT / f"{name}.png" if capture else None
    for stale in (report, png):
        if stale and stale.exists():
            stale.unlink()
    # --mute silences the run without touching the saved Audio settings.
    command = [str(EXE), "--no-setup", "--mute", *args, "--report", str(report)]
    command.insert(2, "--background" if background else "--no-focus")
    if png:
        command += ["--capture", str(png)]
    env = dict(os.environ, RUST_LOG=os.environ.get("RUST_LOG", "info,hl2_bevy=debug"))
    try:
        done = subprocess.run(command, cwd=REPO, env=env, capture_output=True, text=True,
                              errors="replace", timeout=timeout)
        status, log = done.returncode, done.stdout + done.stderr
    except subprocess.TimeoutExpired as expired:
        status, log = -1, f"timed out after {timeout} s\n{expired.stdout or ''}{expired.stderr or ''}"
    log = re.sub(r"\x1b\[[0-9;]*m", "", log if isinstance(log, str) else log.decode(errors="replace"))
    (OUT / f"{name}.game.log").write_text(log, encoding="utf-8")
    if status != 0:
        raise Failure(f"{name}: exit {status} (see {name}.game.log)")
    if not report.exists():
        raise Failure(f"{name}: no report")
    run = Run(name, status, json.loads(report.read_text(encoding="utf-8")), log, png)
    if png and not (png.exists() and png.stat().st_size > 0):
        raise Failure(f"{name}: no capture")
    if "--movement-script" in args and not run.report["render"]["simulation"].get("script_finished"):
        raise Failure(f"{name}: the replay did not finish")
    # Checks never make a sound: every sink stays at zero volume under --mute.
    audio = run.report.get("render", {}).get("presentation", {}).get("audio")
    if audio is not None and not (audio.get("muted") and audio.get("peak_output") == 0):
        raise Failure(f"{name}: audio was not silent (muted {audio.get('muted')}, "
                      f"peak output {audio.get('peak_output')})")
    return run


def settings(**sections: dict) -> None:
    """Default settings with the given sections merged in (written before a launch)."""
    current = json.loads(CONFIG.read_text(encoding="utf-8")) if CONFIG.exists() else {}
    for key, values in sections.items():
        current.setdefault(key, {}).update(values)
    CONFIG.write_text(json.dumps(current, indent=2), encoding="utf-8")


def saved() -> dict:
    return json.loads(CONFIG.read_text(encoding="utf-8")) if CONFIG.exists() else {}


def expect(what: str, actual, expected) -> None:
    print(f"{what}: {actual!r}")
    if actual != expected:
        raise Failure(f"{what}: expected {expected!r}, got {actual!r}")


def holds(what: str, condition: bool, detail="") -> None:
    print(f"{'ok' if condition else 'FAILED'}: {what}{f' ({detail})' if detail != '' else ''}")
    if not condition:
        raise Failure(what)


MENU = ("--map", "background01", "--main-menu", "--width", "1024", "--height", "768", "--frames", "400")


# ---- shared checks (background) ------------------------------------------------------------

def check_submenus():
    """Episode One, back, Half-Life 2, back, Extras > Lost Coast: each game's background loads."""
    run = game("submenus", "--map", "background01", "--main-menu", "--frames", "2400",
               "--movement-script", "test-inputs/bevy-menu-submenus.json")
    expect("loaded", run.loaded(), ["hl2/background01", "episodic/ep1_background01",
                                    "hl2/background01", "lostcoast/background01"])
    expect("mode", run.console()["mode"], "MainMenu")


def check_return_menu():
    """Episode Two menu, `map d1_trainstation_02`, Return to main menu: back over hl2/background01."""
    run = game("return_menu", "--map", "background01", "--main-menu", "--frames", "3200",
               "--movement-script", "test-inputs/bevy-menu-return.json")
    expect("loaded", run.loaded(), ["hl2/background01", "ep2/ep2_background01",
                                    "hl2/d1_trainstation_02", "hl2/background01"])
    expect("mode", run.console()["mode"], "NewGame")
    browser = run.sample("chapters-after-return")["console"]
    expect("chapter game after return", browser["chapters"]["game"], "hl2")
    expect("no stale confirmation", browser["confirm"], None)
    expect("saved menu background", saved()["gameplay"].get("background_map"), "background01")


def check_chapter_menu():
    """Leaving d2_coast_01 shows hl2/background04 (chapter 7), d3_c17_10a hl2/background06."""
    run = game("chapter_menu", "--map", "background01", "--main-menu", "--frames", "4400",
               "--movement-script", "test-inputs/bevy-menu-chapter-return.json", timeout=300)
    expect("loaded", run.loaded(), ["hl2/background01", "hl2/d2_coast_01", "hl2/background04",
                                    "hl2/d3_c17_10a", "hl2/background06"])


def check_chapter_selection():
    """Installed chapter browser and explicit game/map loading in every expansion."""
    for game_name, count, key, map_name in [
        ("hl2", 15, "2", "d1_trainstation_05"),
        ("episodic", 5, "2", "ep1_citadel_03"),
        ("ep2", 7, "2", "ep2_outland_02"),
        ("lostcoast", 1, "1", "d2_lostcoast"),
    ]:
        args = (*MENU[:-2], "--frames", "1800", "--movement-script")
        preview = game(f"chapters_{game_name}_preview", *args,
                       f"test-inputs/bevy-chapters-{game_name}-preview.json")
        cancelled = preview.sample("cancelled")["console"]
        expect(f"{game_name}: Cancel returns to menu", cancelled["mode"], "MainMenu")
        expect(f"{game_name}: Cancel closes browser", cancelled["chapters"], None)
        holds(f"{game_name}: Cancel retains background", cancelled["background_level"])
        browser = preview.sample("browser-settled")["console"]
        expect(f"{game_name}: browser mode", browser["mode"], "NewGame")
        chapters = browser["chapters"]
        expect(f"{game_name}: chapter count", len(chapters["entries"]), count)
        expect(f"{game_name}: thumbnail count", chapters["thumbnails"], count)
        expect(f"{game_name}: chapter game", chapters["game"], game_name)
        holds(f"{game_name}: localized names", all("_chapter" not in e["title"].lower()
              for e in chapters["entries"]), chapters["entries"])
        holds(f"{game_name}: opening does not load gameplay", browser["background_level"])
        run = game(f"chapters_{game_name}", *args,
                   f"test-inputs/bevy-chapters-{game_name}.json", timeout=300)
        selected = run.sample("selected-chapter")["console"]["chapters"]
        expect(f"{game_name}: selected key", selected["entries"][selected["selected"]]["key"], key)
        pointer = run.sample("pointer-selected")["console"]["chapters"]
        expect(f"{game_name}: pointer-selected key", pointer["entries"][pointer["selected"]]["key"], key)
        if game_name == "hl2":
            fractional = run.sample("chapter9a")["console"]["chapters"]
            expect("HL2 fractional chapter", fractional["entries"][fractional["selected"]]["key"], "9a")
        expect(f"{game_name}: completed chapter load", run.campaign()["loaded_map"], map_name)
        expect(f"{game_name}: game identity", run.campaign()["loaded_game"], game_name)
        expect(f"{game_name}: gameplay", run.console()["mode"], "Gameplay")
        holds(f"{game_name}: not a background", not run.console()["background_level"])
        expect(f"{game_name}: no loader failures", run.campaign()["failures"], [])


def check_menu_options():
    """Options is not modal: the menu beside it works, the menu under it gets nothing."""
    run = game("menu_options", *MENU[:-2], "--frames", "800",
               "--movement-script", "test-inputs/bevy-menu-options-nonmodal.json")
    beside = run.sample("menu-beside-clicked")["console"]
    expect("a click beside Options focuses the menu", beside["mode"], "MainMenu")
    expect("Options stays open", beside.get("windows"), ["Options"])
    expect("the Episode One row ran", beside.get("pending_map"), "ep1_background01")
    expect("dragging the caption focuses Options", run.sample("options-dragged")["console"]["mode"], "Options")
    under = run.sample("menu-under-clicked")["console"]
    expect("NEW GAME under the dialog does not start a game", under.get("pending_map"), None)
    expect("Options still open and focused", (under["mode"], under.get("windows")), ("Options", ["Options"]))
    expect("loaded", run.loaded(), ["hl2/background01", "episodic/ep1_background01"])
    # An open drop-down list takes every click: one on the menu only closes it. (Default
    # settings again: the drag above saved the dialog's new place.)
    if CONFIG.exists():
        CONFIG.unlink()
    run = game("menu_options-list", *MENU[:-2], "--frames", "400",
               "--movement-script", "test-inputs/bevy-menu-options-list.json")
    expect("the Resolution list opened",
           (run.sample("list-opened")["console"].get("options") or {}).get("list_open"), True)
    after = run.sample("menu-clicked-with-list")["console"]
    expect("a click on the menu closed the list",
           (after.get("options") or {}).get("list_open"), False)
    expect("and ran nothing", after.get("pending_map"), None)
    expect("Options kept the focus", (after["mode"], after.get("windows")), ("Options", ["Options"]))


EIGHT_K = "7680x4320,3840x2160,2560x1440,1920x1080,1280x720"


def check_video_8k():
    """Options > Video on a faked 8K monitor (--fake-monitor): the Resolution list (16:9 by
    default) starts at the panel's 7680 x 4320 and is shown open in the capture."""
    if CONFIG.exists():
        CONFIG.unlink()
    run = game("video_8k", *MENU, "--fake-monitor", EIGHT_K,
               "--movement-script", "test-inputs/bevy-menu-video-8k.json")
    options = run.sample("resolution-open")["console"].get("options") or {}
    expect("Video tab", options.get("tab"), "Video")
    holds("the Resolution list is open", options.get("list_open") is True)
    expect("16:9 choices of an 8K panel", options.get("resolutions"),
           ["7680 x 4320", "5120 x 2880", "3840 x 2160", "3200 x 1800", "2560 x 1440",
            "1920 x 1080", "1600 x 900", "1366 x 768", "1280 x 720"])


def check_video_portrait():
    """A rotated 1080 x 1920 monitor (--fake-monitor) running at 1080 x 1920: the menu and
    Options work on a portrait frame; by keyboard, Video > Aspect Ratio opens with Portrait 9:16
    selected and the Resolution choices are the 9:16 modes up to the panel."""
    if CONFIG.exists():
        CONFIG.unlink()
    settings(video={"display_mode": "windowed", "width": 1080, "height": 1920})
    run = game("video_portrait", "--map", "background01", "--main-menu", "--width", "1080",
               "--height", "1920", "--frames", "400", "--fake-monitor", "1080x1920",
               "--movement-script", "test-inputs/bevy-menu-video-portrait.json")
    options = run.sample("aspect-open")["console"].get("options") or {}
    expect("Video tab", options.get("tab"), "Video")
    expect("Aspect Ratio focused", options.get("focus"), "AspectRatio")
    holds("its list is open", options.get("list_open") is True)
    expect("9:16 choices of the panel", options.get("resolutions"),
           ["1080 x 1920", "900 x 1600", "768 x 1366", "720 x 1280"])
    expect("rendered in portrait", run.sample("settle").get("frame"), [1080, 1920])
    x, y, w, h = options.get("frame") or [0, 0, 1e9, 1e9]
    holds("the Options dialog fits the portrait screen", x >= 0 and y >= 0 and x + w <= 1080 and y + h <= 1920,
          options.get("frame"))
    expect("the capture follows", run.report["render"]["capture_size"], [1080, 1920])


def check_video_keep():
    """Options > Video: a new aspect ratio (Normal 4:3 by keyboard on a faked 1920 x 1080
    monitor, so 1440 x 1080) applies at once and asks to keep it. The timeout and Escape put
    back 1280 x 720; OK keeps the change past the timeout."""
    def run(name):
        if CONFIG.exists():
            CONFIG.unlink()
        result = game(f"video_keep_{name}", *MENU[:-2], "--frames", "2000", "--fake-monitor", "1920x1080",
                      "--movement-script", f"test-inputs/bevy-menu-video-keep-{name}.json")
        asked = result.sample("asked")
        expect(f"{name}: Normal picked", (result.sample("picked-normal")["console"].get("options") or {}).get("changed"), True)
        expect(f"{name}: the keep box asks", (asked["console"].get("confirm") or {}).get("kind"), "keep_video")
        expect(f"{name}: the change is shown at once", asked.get("frame"), [1440, 1080])
        return result
    def saved_size():
        video = saved().get("video", {})
        return [video.get("width"), video.get("height")]
    timeout = run("timeout")
    expect("timeout: the box closed", timeout.sample("reverted")["console"].get("confirm"), None)
    expect("timeout: reverted", timeout.sample("reverted").get("frame"), [1280, 720])
    expect("timeout: saved reverted", saved_size(), [1280, 720])
    ok = run("ok")
    expect("ok: the box closed", ok.sample("kept")["console"].get("confirm"), None)
    expect("ok: kept past the timeout", ok.sample("still-kept").get("frame"), [1440, 1080])
    expect("ok: saved", saved_size(), [1440, 1080])
    escape = run("escape")
    expect("escape: the box closed", escape.sample("escaped")["console"].get("confirm"), None)
    expect("escape: reverted", escape.sample("reverted").get("frame"), [1280, 720])
    expect("escape: saved reverted", saved_size(), [1280, 720])


def check_narrow_layout():
    """Menu rows and the developer console on screens narrower than the UI design, and on
    landscape as the reference: rows keep a readable size and fit the width, the console is
    bounded to the design area and stays unchanged on landscape (report `console.menu`,
    `console.console_frame`)."""
    sizes = {"portrait": (1080, 1920), "square": (1080, 1080), "landscape": (1920, 1080),
             "ultrawide": (3440, 1440), "super_ultrawide": (5120, 1440)}
    for name, (w, h) in sizes.items():
        if CONFIG.exists():
            CONFIG.unlink()
        run = game(f"narrow_{name}", "--map", "background01", "--main-menu", "--width", str(w),
                   "--height", str(h), "--frames", "200",
                   "--movement-script", "test-inputs/bevy-console-open.json")
        console = run.sample("console")["console"]
        menu, frame = console["menu"], console["console_frame"]
        design = min(h, w / 1.25)
        holds(f"{name}: menu rows fit the width", menu["x"] + menu["row_wide"] <= w, menu)
        # A menu background keeps the vertical view of 4:3 up to 16:9 (cropping the sides below
        # 4:3), and cuts the top and bottom of the 16:9 frame on ultrawide screens.
        expected_bg_fov = {"ultrawide": 46.37, "super_ultrawide": 32.11}.get(name, 59.84)
        holds(f"{name}: menu background vertical FOV is {expected_bg_fov} degrees",
              abs(run.sample("console")["camera"][0] - expected_bg_fov) < 0.05, run.sample("console")["camera"])
        if name in ("landscape", "ultrawide", "super_ultrawide"):
            p = lambda units: units * h // 480
            holds(f"{name}: retail console origin", (round(frame[0]), round(frame[1])) == (w // 2 - p(16), p(64)), frame)
            holds(f"{name}: half the screen wide", abs(frame[2] - w // 2) <= 3, frame)
            holds(f"{name}: down to 64 units above the bottom", abs(frame[1] + frame[3] - (h - p(64))) <= 3, frame)
        else:
            holds(f"{name}: menu text is not shrunk with the width", menu["font_tall"] >= 16, menu)
            holds(f"{name}: the console spans the width", frame[0] > 0 and frame[0] + frame[2] > w * 0.9
                  and frame[0] + frame[2] < w, frame)
            holds(f"{name}: the console stays in the design area", frame[1] + frame[3] <= design, frame)
    portrait_menu = game_report("narrow_portrait")["menu"]
    holds("portrait: rows are larger than at the design height", portrait_menu["item_tall"] > 38, portrait_menu)
    # Wide screens: the menu grows (relative to the height) and stays inside the screen.
    retail = game_report("narrow_landscape")["menu"]
    for name, (w, h) in (("ultrawide", sizes["ultrawide"]), ("super_ultrawide", sizes["super_ultrawide"])):
        menu = game_report(f"narrow_{name}")["menu"]
        holds(f"{name}: rows are larger relative to the height than on 16:9",
              menu["item_tall"] / h > 1.15 * retail["item_tall"] / 1080, (menu, retail))
        holds(f"{name}: the stack stays inside the screen", 0 < menu["y"] and menu["y"] + 7 * menu["item_tall"] <= h, menu)


def game_report(name: str) -> dict:
    """The console state of the last sample of a finished run `name`."""
    report = json.loads((OUT / f"{name}.json").read_text(encoding="utf-8"))
    return [s for s in report["render"]["simulation"]["samples"] if s.get("label")][-1]["console"]


def check_fov_settings():
    """The Video > Advanced vertical and viewmodel field of view settings reach the cameras
    (report sample `camera`: vertical degrees of the world and viewmodel cameras): automatic by
    default (59.84 degrees on 5:4..=16:9 frames, aspect-tuned outside that range to avoid
    vertical fish-eye on portrait and peripheral stretching on ultrawide), an explicit vertical
    value replaces it, the viewmodel value moves the viewmodel camera, and aspect-adaptive
    framing in `tuning.json` adjusts the viewmodel outside 5:4..=16:9."""
    def camera(name, size, **video):
        if CONFIG.exists():
            CONFIG.unlink()
        settings(video=video)
        run = game(name, "--map", "d1_trainstation_02", "--width", str(size[0]), "--height", str(size[1]),
                   "--movement-script", "test-inputs/bevy-hud.json")
        return run.sample("hud-final")["camera"]
    near = lambda got, want: all(abs(a - b) < 0.05 for a, b in zip(got, want))
    default = camera("fov_default", (1920, 1080))
    holds("default: 59.84 degrees vertical, 41.83 for the viewmodel", near(default, [59.84, 41.83]), default)
    custom = camera("fov_custom", (1920, 1080), vertical_fov=90.0, viewmodel_fov=70.0)
    holds("custom: 90 degrees vertical, 55.41 for the viewmodel", near(custom, [90.0, 55.41]), custom)
    portrait = camera("fov_portrait", (1080, 1920))
    # The portrait profiles change only the viewmodel offset (fov_offset 0), so the viewmodel
    # keeps its 54 degree 4:3 horizontal view: 84.34 degrees vertical at 9:16.
    holds("portrait: taller vertical view without extreme fish-eye (89.16), viewmodel keeps its 4:3 width (84.34)",
          near(portrait, [89.16, 84.34]), portrait)
    ultrawide = camera("fov_ultrawide", (5120, 1440))
    # Likewise ultrawide (fov_offset 0): wider than 4:3 the viewmodel keeps its 41.83 vertical view.
    holds("super-ultrawide: world trims to 50.22 vertical (116.19 horizontal), viewmodel keeps its 41.83 vertical view",
          near(ultrawide, [50.22, 41.83]), ultrawide)


def check_resolution_stress():
    """Resolution changes never stall or crash (background, faked 8K monitor): Options Apply of
    7680 x 4320, the smallest and largest sizes the console accepts, and four changes on
    consecutive ticks (full screen and windowed mixed), where the last one wins. Each labelled
    sample reports the size of the frame image being rendered."""
    if CONFIG.exists():
        CONFIG.unlink()
    run = game("resolution_stress", *MENU[:-2], "--frames", "1200", "--fake-monitor", EIGHT_K,
               "--movement-script", "test-inputs/bevy-menu-resolution-stress.json")
    frame = lambda label: run.sample(label).get("frame")
    expect("the 8K row was picked from the list",
           (run.sample("picked-8k")["console"].get("options") or {}).get("changed"), True)
    expect("Options Apply rendered 7680 x 4320", frame("after-options-apply"), [7680, 4320])
    expect("OK closed Options", run.sample("options-closed")["console"]["mode"], "MainMenu")
    expect("the smallest console size", frame("smallest"), [320, 240])
    expect("consecutive changes: the last one wins", frame("rapid"), [1280, 720])
    expect("the largest console size", frame("largest"), [8192, 8192])
    expect("back to 1024 x 768", frame("final"), [1024, 768])
    expect("the capture follows", run.report["render"]["capture_size"], [1024, 768])
    video = saved()["video"]
    expect("saved", (video["width"], video["height"], video["display_mode"]), (1024, 768, "windowed"))
    ticks = [run.sample(l)["host_tick"] for l in ("smallest", "rapid", "largest", "final", "settle")]
    holds("the game kept running", ticks == sorted(ticks) and len(set(ticks)) == len(ticks), ticks)


def check_chapter_quit():
    """Loading a gameplay map saves its chapter's background, so a mid-game quit relaunches over it."""
    settings(gameplay={"background_game": "hl2", "background_map": "background01"})
    game("chapter_quit", "--map", "d2_coast_01", "--frames", "600", timeout=180)
    gameplay = saved()["gameplay"]
    expect("saved menu background", f"{gameplay.get('background_game')}/{gameplay.get('background_map')}",
           "hl2/background04")


KEYBOARD_FOCUS = {
    # replay suffix: (label, Options tab, focused control)
    "keyboard-check": ("focus-reverse-mouse", "Mouse", "ReverseMouse"),
    "keyboard-slider": ("focus-sensitivity-slider", "Mouse", "Slider"),
    "keyboard-entry": ("focus-sensitivity-entry", "Mouse", "SensitivityLabel"),
    "keyboard-radio": ("focus-skill-radio", "Gameplay", "Skill2Radio"),
    "keyboard-combo": ("focus-resolution", "Video", "Resolution"),
    "keyboard-button": ("focus-advanced", "Video", "AdvancedButton"),
    "keyboard-fov": ("focus-fov", "Video", "FovSlider"),  # in Video > Advanced: 7 Shift+Tab from the end (the two HL2-RS sliders come after it)
}
OPTIONS_STATES = {
    # replay suffix: (label, what the report must show there)
    "": ("keyboard-advanced", {"mode": "Options", "sub": "KeyboardAdvanced"}),
    "video-advanced": ("video-advanced", {"mode": "Options", "sub": "VideoAdvanced"}),
    "drag": ("options-dragged", {"mode": "Options", "sub": None}),
    "console": ("type-map", {"mode": "Console"}),
    "console-resize": ("console-moved", {"mode": "Console"}),
    "console-wrap": ("long-word", {"mode": "Console"}),
}


def check_options():
    """Every Options capture replay: the labelled state (dialog, sub-dialog, keyboard focus)."""
    for script in sorted((REPO / "test-inputs").glob("bevy-options-capture*.json")):
        suffix = script.stem[len("bevy-options-capture"):].lstrip("-")
        if suffix not in KEYBOARD_FOCUS and suffix not in OPTIONS_STATES and suffix != "keyboard-credits":
            raise Failure(f"{script.name}: no expectation in replays.py")
        if CONFIG.exists():
            CONFIG.unlink()
        run = game(f"options-{suffix or 'keyboard-advanced'}", *MENU,
                   "--movement-script", str(script.relative_to(REPO)))
        if suffix == "keyboard-credits":
            check_credits_keys(run)
        elif suffix in KEYBOARD_FOCUS:
            label, tab, focus = KEYBOARD_FOCUS[suffix]
            options = run.sample(label)["console"].get("options") or {}
            expect(f"{suffix} tab", options.get("tab"), tab)
            expect(f"{suffix} focus", options.get("focus"), focus)
            holds(f"{suffix} focus drawn", options.get("focus_visible") is True)
            if suffix == "keyboard-entry":
                holds("the focused entry edits", options.get("editing") is True)
        else:
            label, wanted = OPTIONS_STATES[suffix]
            console = run.sample(label)["console"]
            expect(f"{suffix or 'keyboard-advanced'} mode", console["mode"], wanted["mode"])
            if "sub" in wanted:
                expect(f"{suffix or 'keyboard-advanced'} sub-dialog", (console.get("options") or {}).get("sub"),
                       wanted["sub"])


def check_credits_keys(run: Run) -> None:
    """Audio > 3rd party credits by keyboard: Close holds the focus, Enter closes on release."""
    options = lambda label: run.sample(label)["console"].get("options") or {}
    expect("credits opened by Space", options("credits-open").get("sub"), "AudioCredits")
    expect("Close focused", options("focus-close").get("focus"), "cancel")
    holds("and drawn after Tab", options("focus-close").get("focus_visible") is True)
    expect("Enter down keeps the credits open", options("enter-down").get("sub"), "AudioCredits")
    expect("held too", options("enter-held").get("sub"), "AudioCredits")
    expect("its release closes them", options("enter-released").get("sub"), None)
    expect("focus back on the credits button", options("enter-released").get("focus"), "ThirdPartySoundCredits")
    expect("Options stays open", run.console()["mode"], "Options")


def check_placement():
    """A dragged Options dialog is saved, and reopens there in a second launch."""
    game("placement-drag", *MENU, "--movement-script", "test-inputs/bevy-options-capture-drag.json")
    dialogs = saved().get("dialogs", {})
    holds("the Options placement is saved", bool(dialogs.get("options")), dialogs)
    steps = json.loads((REPO / "test-inputs/bevy-options-capture.json").read_text(encoding="utf-8"))
    reopen = OUT / "placement-reopen-input.json"
    reopen.write_text(json.dumps([s for s in steps if s["tick"] <= 25] + [steps[-1]]), encoding="utf-8")
    run = game("placement-reopen", *MENU, "--movement-script", str(reopen))
    expect("reopened", run.console()["mode"], "Options")
    expect("placement kept", saved().get("dialogs", {}).get("options"), dialogs.get("options"))


def check_captions():
    """ep1_background02 (Kleiner's broadcast): captions shown when on, none when off."""
    counts = {}
    for mode in ("closed_captions", "none"):
        if CONFIG.exists():
            CONFIG.unlink()
        settings(gameplay={"background_game": "episodic", "background_map": "ep1_background02"},
                 audio={"captions": mode})
        run = game(f"captions-{mode}", "--frames", "420", "--width", "1024", "--height", "768", timeout=180)
        captions = run.report["render"]["presentation"]["hud"]["captions"]
        counts[mode] = captions.get("processed", 0)
        print(f"captions {mode}: processed {counts[mode]}, dictionaries {captions.get('dictionaries')}")
    holds("captions on processes speech", counts["closed_captions"] > 0, counts)
    holds("captions off processes none", counts["none"] == 0, counts)


def check_commentary():
    """Developer commentary: balloon nodes, skin switching, and teleportation via secondary attack."""
    if CONFIG.exists():
        CONFIG.unlink()
    settings(gameplay={"commentary": True}, audio={"captions": "subtitles"})
    run = game("commentary", "--map", "d2_lostcoast", "--width", "1280", "--height", "720",
               "--movement-script", "test-inputs/bevy-lostcoast-commentary.json", timeout=240)
    s = run.samples()
    c1 = s["hovered-node-1"]["gameplay"]["commentary"]
    holds("hovered node 0", c1["hovered"] == 0, c1["hovered"])
    holds("node 0 inactive", c1["active"] is None, c1["active"])
    holds("node 0 not yet listened", not c1["nodes"][0]["listened"], c1["nodes"][0]["listened"])

    c2 = s["started-node-1"]["gameplay"]["commentary"]
    holds("active node 0", c2["active"] == 0, c2["active"])
    holds("node 0 active flag", c2["nodes"][0]["active"], c2["nodes"][0]["active"])
    holds("node 0 listened flag", c2["nodes"][0]["listened"], c2["nodes"][0]["listened"])

    c3 = s["stopped-node-1"]["gameplay"]["commentary"]
    holds("stopped active none", c3["active"] is None, c3["active"])
    holds("node 0 inactive", not c3["nodes"][0]["active"], c3["nodes"][0]["active"])
    holds("node 0 marked listened", c3["nodes"][0]["listened"], c3["nodes"][0]["listened"])

    c4 = s["teleported-node-1"]["gameplay"]["commentary"]
    feet1 = s["teleported-node-1"]["player"]["feet"]
    node0_origin = c4["nodes"][0]["origin"]
    holds("teleported to node 0", abs(feet1[0] - node0_origin[0]) < 100.0 and abs(feet1[1] - node0_origin[1]) < 100.0,
          (feet1, node0_origin))

    c5 = s["teleported-node-2"]["gameplay"]["commentary"]
    feet2 = s["teleported-node-2"]["player"]["feet"]
    node1_origin = c5["nodes"][1]["origin"]
    holds("teleported to node 1", abs(feet2[0] - node1_origin[0]) < 100.0 and abs(feet2[1] - node1_origin[1]) < 100.0,
          (feet2, node1_origin))
    holds("node 1 hovered", c5["hovered"] == 1, c5["hovered"])


def check_video():
    """Modal full-screen video playback, skip handling, and fallback execution on missing video."""
    if CONFIG.exists():
        CONFIG.unlink()
    run = game("video", "--map", "background01", "--main-menu", "--width", "1280", "--height", "720",
               "--movement-script", "test-inputs/bevy-video.json", timeout=120)
    s = run.samples()
    v1 = s["video-active"]["console"].get("video")
    holds("video active during playback", v1 and v1.get("active") is True, v1)
    holds("video filename is valve", v1 and "valve" in v1.get("filename", ""), v1)
    holds("video duration > 0", v1 and v1.get("duration", 0) > 0, v1)
    holds("video exit command recorded", v1 and v1.get("exit_command") == "echo test_exit_command_fired", v1)

    v2 = s["video-skipped"]["console"].get("video")
    holds("video inactive after space skip", not v2 or not v2.get("active"), v2)
    out2 = s["video-skipped"]["console"]["output"]
    holds("exit command executed after skip", any("test_exit_command_fired" in line for line in out2), out2)

    v3 = s["video-fallback"]["console"].get("video")
    holds("video inactive after fallback", not v3 or not v3.get("active"), v3)
    out3 = s["video-fallback"]["console"]["output"]
    holds("fallback warning logged", any("Unable to play video: missing_test_vid" in line for line in out3), out3)
    holds("fallback exit command executed", any("test_missing_exit_fired" in line for line in out3), out3)


def check_background_reveal():
    """The install frame already uses the authored background camera, before tick one."""
    if CONFIG.exists():
        CONFIG.unlink()
    settings(video={"fov": 90.0, "hdr": "none", "hdr_migrated": True,
                    "advanced": {"motion_blur": False, "antialiasing": "x4"}})
    run = game("background_reveal", "--map", "background01", "--main-menu",
               "--width", "1024", "--height", "768", "--frames", "30")
    installed = run.capture.with_name("background_reveal-installed.png")
    holds("actual install-frame capture exists", installed.exists())
    changes = pixel_changes(installed, run.capture, threshold=35)
    # Animated leaves/sky can change; a whole-camera jump changes most of the frame.
    holds("no spawn-view flash before the menu camera", changes["changed"] < 1024 * 768 * 0.05, changes)


def check_background_fov_independence():
    """Background projection ignores saved gameplay FOV and vertical-FOV overrides."""
    for width, height in [(1024, 768), (1280, 720)]:
        results = []
        for fov, vertical in [(75.0, 0.0), (110.0, 90.0)]:
            if CONFIG.exists():
                CONFIG.unlink()
            settings(video={"fov": fov, "vertical_fov": vertical, "hdr": "none",
                            "hdr_migrated": True, "advanced": {"motion_blur": False}})
            name = f"background_fov_{width}_{int(fov)}"
            results.append(game(name, "--map", "background01", "--main-menu",
                                "--width", str(width), "--height", str(height), "--frames", "30"))
        changes = pixel_changes(results[0].capture, results[1].capture, threshold=35)
        holds("gameplay settings do not reframe the background",
              changes["changed"] < width * height * 0.02, changes)


def check_bg_compare():
    """Each game's chapter 1 background at 1024x768, FOV 75 (compare with the retail images)."""
    for game_, map_ in (("hl2", "background01"), ("episodic", "ep1_background01"),
                        ("ep2", "ep2_background01"), ("lostcoast", "background01")):
        if CONFIG.exists():
            CONFIG.unlink()
        settings(gameplay={"background_game": game_, "background_map": map_}, video={"fov": 75.0})
        run = game(f"bg_compare-{game_}", "--frames", "300", "--width", "1024", "--height", "768", timeout=180)
        campaign = run.campaign()
        expect(game_, f"{campaign['loaded_game']}/{campaign['loaded_map']}", f"{game_}/{map_}")


def check_save_config():
    """Console variables persist: volume, sensitivity, fov, mat_hdr_level."""
    game("save_config", "--map", "d1_trainstation_02", "--frames", "120",
         "--movement-script", "test-inputs/bevy-save-config.json", capture=False)
    config = saved()
    expect("volume", config["audio"]["volume"], 0.55)
    expect("sensitivity", config["mouse"]["sensitivity"], 4.2)
    expect("fov", config["video"]["fov"], 95.0)
    expect("hdr", config["video"]["hdr"], "full")


def check_console():
    """Console: cheats, setpos/ent_fire while paused, Unicode editing, weapon selection vs Escape,
    the pause menu and the console over it (test-inputs/bevy-console.json)."""
    run = game("console", "--map", "d1_trainstation_02", "--width", "1280", "--height", "720",
               "--movement-script", "test-inputs/bevy-console.json")
    s = run.samples()
    out = lambda label: s[label]["console"]["output"]
    mode = lambda label: s[label]["console"]["mode"]
    g = lambda label: s[label]["gameplay"]
    holds("impulse 101 refused without sv_cheats",
          not g("cheat-denied")["inventory"]["suit"]
          and any("Can't use impulse" in x for x in out("cheat-denied")))
    holds("sv_cheats 1 then impulse 101 grants the suit and weapons",
          g("cheat-granted")["inventory"]["suit"] and len(g("cheat-granted")["inventory"]["owned"]) >= 6)
    holds("a quoted semicolon stays in the argument", "value;kept" in out("cheat-granted"))
    holds("the console freezes the scene", g("setpos-paused")["time"] == g("cheat-denied")["time"] == 0)
    expect("setpos moves the feet", s["setpos-paused"]["player"]["feet"], [-3260, -2024, 64])
    holds("ent_fire is queued while paused",
          any("Queued station_entrance.Close" in x for x in out("setpos-paused")))
    expect("Unicode input", s["unicode-input"]["console"]["input"], "é\U0001f980")
    expect("Unicode editing keeps whole characters", s["unicode-edit"]["console"]["input"], "é")
    # The console opened mid-game brings up the pause menu; closing it leaves that menu up
    # and Escape then resumes (docs/gameui.md section 18).
    holds("closing the console returns to the pause menu",
          mode("resume-transition") == "Pause" and s["resume-transition"]["paused"])
    holds("Escape on the pause menu resumes the game",
          mode("resumed-from-pause") == "Gameplay" and not s["resumed-from-pause"]["paused"])
    holds("the weapon selection opens without switching",
          g("selection-open")["pending"] == "weapon_smg1"
          and g("selection-open")["inventory"]["active"] == "weapon_pistol")
    holds("Escape cancels the selection before pausing",
          g("selection-canceled")["pending"] is None and mode("selection-canceled") == "Gameplay")
    holds("Escape opens the pause menu", mode("pause-open") == "Pause" and s["pause-open"]["paused"])
    holds("a click outside the menu stays paused",
          mode("outside-click") == "Pause" and g("outside-click")["time"] == g("pause-open")["time"])
    holds("the console opens over the pause menu and returns to it",
          mode("console-from-pause") == "Console" and mode("return-to-pause") == "Pause")
    holds("the scene stays frozen meanwhile",
          g("console-paused")["time"] == g("return-to-pause")["time"] == g("pause-open")["time"])
    holds("unknown commands are rejected",
          "Unknown or unimplemented command: unsupported_command" in out("console-paused"))
    audio = run.report["render"]["presentation"]["audio"]
    # Sounds pause with the UI and resume after it. (The replay means to end paused, every sound
    # paused; with the `menu-resume` step below not resuming here, its last Escape closes the
    # pause menu instead, so the end state is not compared.)
    holds("sounds play", audio["started_sinks"] > 0 and audio["failed_requests"] == 0,
          (audio["started_sinks"], audio["failed_requests"]))
    holds("playing sounds pause with the UI and resume after it",
          audio["pause_changes"] >= 2 and audio["resume_changes"] >= 2,
          (audio["pause_changes"], audio["resume_changes"]))
    expect("viewport", run.report["render"]["presentation"]["hud"]["viewport"], [1280, 720])
    # The replay's `menu-resume` step presses Enter with no pause-menu row selected; here Escape
    # clears the menu selection, so Enter does nothing and its resume assertions do not apply.
    large = game("console-1080p", "--map", "d1_trainstation_02", "--width", "1920", "--height", "1080",
                 "--movement-script", "test-inputs/bevy-console.json")
    expect("1080p viewport", large.report["render"]["presentation"]["hud"]["viewport"], [1920, 1080])
    expect("1080p capture", png_size(large.capture), [1920, 1080])
    expect("1080p console ends the same", large.console()["mode"], run.console()["mode"])


def check_console_map():
    """`map` from the console: two direct loads, a missing map reported without losing the scene."""
    run = game("console_map", "--map", "d1_trainstation_02", "--width", "1280", "--height", "720",
               "--movement-script", "test-inputs/bevy-console-map.json")
    s = run.samples()
    expect("loaded", run.loaded()[-2:], ["hl2/d1_trainstation_01", "hl2/d1_trainstation_02"])
    holds("a direct map starts without the old inventory",
          not s["direct-map-arrival"]["gameplay"]["inventory"]["owned"]
          and s["direct-map-arrival"]["console"]["mode"] == "Gameplay")
    failures = run.campaign()["failures"]
    holds("the missing map is one explicit error",
          len(failures) == 1 and "hl2_rs_missing_test_map" in failures[0], failures)
    holds("the failed load keeps the player and scene",
          s["missing-map-request"]["player"] == s["missing-map-recovered"]["player"]
          and s["missing-map-request"]["gameplay"]["time"] == s["missing-map-recovered"]["gameplay"]["time"])
    holds("console state survives a load",
          s["second-direct-arrival"]["console"]["sv_cheats"]
          and "history-survives" in s["second-direct-arrival"]["console"]["output"])
    holds("nothing is loading at the end", run.campaign()["loading"] is None)


def check_campaign():
    """Level transitions d1_trainstation_01 > 02 > 03 through their triggers, inventory kept."""
    run = game("campaign", "--map", "d1_trainstation_01", "--width", "1280", "--height", "720",
               "--movement-script", "test-inputs/bevy-campaign.json")
    s = run.samples()
    expect("loaded", run.loaded()[-3:], ["hl2/d1_trainstation_01", "hl2/d1_trainstation_02",
                                         "hl2/d1_trainstation_03"])
    campaign = run.campaign()
    holds("ends on station 03 without failures",
          campaign["active_map"] == "d1_trainstation_03" and not campaign["failures"]
          and campaign["loading"] is None, campaign)
    holds("the inventory crosses the transitions",
          s["station03-final"]["gameplay"]["inventory"]["suit"]
          and "weapon_pistol" in s["station03-final"]["gameplay"]["inventory"]["owned"])
    holds("the clock restarts on the new map",
          s["station02-fire-after-reset"]["gameplay"]["time"] < s["station01-exit-trigger"]["gameplay"]["time"])


def check_dev_overlay():
    """F1 developer overlay with live statistics, also while paused."""
    run = game("dev_overlay", "--map", "d1_trainstation_02", "--width", "1280", "--height", "720",
               "--movement-script", "test-inputs/bevy-dev-overlay.json")
    overlay = run.report["render"]["presentation"]["hud"]["developer_overlay"]
    holds("visible with five lines", overlay["visible"] and len(overlay["lines"]) == 5)
    holds("frame statistics", overlay["fps"] > 0 and overlay["frame_ms"] > 0)
    holds("renderer statistics", any("triangles" in x and "textures" in x for x in overlay["lines"]))


def check_hud():
    """Weapon HUD: loadout, slot selection panels and the secondary ammo row."""
    run = game("hud", "--map", "d1_trainstation_02", "--width", "1280", "--height", "720",
               "--movement-script", "test-inputs/bevy-hud.json")
    s = run.samples()
    holds("unarmed at the start", not s["unarmed"]["gameplay"]["inventory"]["owned"])
    holds("the loadout arrives", s["hud-final"]["gameplay"]["inventory"]["suit"])
    holds("HUD quads drawn", run.report["render"]["presentation"]["hud"]["quads"] > 0)


def check_power_hud():
    """Legacy auxiliary power and Episode Two's separate flashlight battery."""
    def run(name: str, map_name: str, fixture: str) -> dict:
        result = game(name, "--map", map_name, "--width", "1920", "--height", "1080",
                      "--movement-script", f"test-inputs/{fixture}.json")
        holds("capture exists", result.capture is not None and result.capture.exists())
        holds("script completed", result.report["render"]["simulation"]["script_finished"])
        return result.report["render"]["presentation"]
    on = run("power_hud_on", "d1_trainstation_02", "bevy-flashlight-hud-on")
    power = on["hud"]["power"]
    holds("legacy device and charge", power["device"] and 90 < power["suit_power"] < 100, power)
    holds("auxiliary panel visible", power["color"][3] > 0.8, power)
    expect("one-device layout", power["rect"], [36, 877, 229, 81])
    off = run("power_hud_recharged", "d1_trainstation_02", "bevy-flashlight-hud-recharged")
    power = off["hud"]["power"]
    holds("recharged and faded", not power["device"] and power["suit_power"] == 100
          and power["color"][3] == 0, power)
    ep2 = run("power_hud_ep2", "ep2_background01", "bevy-flashlight-hud-on")
    power = ep2["hud"]["power"]
    state = ep2["flashlight"]["state"]
    holds("Episode Two battery", power["battery_initialized"] and not power["device"]
          and power["suit_power"] == 100 and 90 < state["battery"] < 100 and state["on"], power)


def check_flashlight_shadows():
    """High has posed and cutout depth shadows; Low/Medium/off have no depth pass."""
    def run(view: str, detail: str, suffix: str = "") -> Run:
        settings(video={"width": 1920, "height": 1080, "hdr": "none", "hdr_migrated": True,
                        "advanced": {"shadow_detail": detail}})
        fixture = ("bevy-flashlight-hud-recharged" if view == "off"
                   else f"bevy-flashlight-depth-{view}")
        result = game(f"flashlight_depth_{view}_{detail}{suffix}", "--map", "d1_trainstation_02",
                      "--width", "1920", "--height", "1080",
                      "--movement-script", f"test-inputs/{fixture}.json")
        holds("script completed", result.report["render"]["simulation"]["script_finished"])
        holds("capture exists", result.capture is not None and result.capture.exists())
        expect("fixed capture dimensions", png_size(result.capture), [1920, 1080])
        presentation = result.report["render"]["presentation"]
        expect("depth map only for High/on", presentation["flashlight"]["depth_shadows"],
               detail == "high" and view != "off")
        expect("current posed bounds", presentation["pose_or_visibility_mismatches"], 0)
        expect("depth resolution", presentation["flashlight"]["depth_resolution"], 1024)
        return result
    for view in ("actor", "fence"):
        medium, high = run(view, "medium"), run(view, "high")
        changes = pixel_changes(medium.capture, high.capture, threshold=8)
        holds(f"{view}: actual depth occlusion", changes["darker"] > 1000, changes)
    run("actor", "low")
    off = run("off", "high")
    expect("off light removed", off.report["render"]["presentation"]["flashlight"]["light"], None)
    # Negative control: only the private packaged copy's depth shader treats cutouts as solid.
    # The colour shader and installation stay untouched; always restore before another check.
    depth = GAME / "bevy-assets/shaders/source_depth.wgsl"
    original = depth.read_text(encoding="utf-8")
    try:
        depth.write_text("#import bevy_pbr::prepass_io::VertexOutput\n@fragment\nfn fragment(mesh: VertexOutput) {}\n", encoding="utf-8")
        solid = run("fence", "high", "_solid_control")
    finally:
        depth.write_text(original, encoding="utf-8")
    normal = OUT / "flashlight_depth_fence_high.png"
    changes = pixel_changes(solid.capture, normal, threshold=8)
    holds("alpha cutout holes transmit the flashlight", changes["brighter"] > 1000, changes)


def check_motion_blur_options():
    if CONFIG.exists():
        CONFIG.unlink()
    preview = json.loads((REPO / "test-inputs/bevy-options-motion-blur.json").read_text())
    fixture = OUT / "motion_blur_selected.input.json"
    fixture.write_text(json.dumps([f for f in preview if f["tick"] <= 150]))
    game("motion_blur_selected", *MENU, "--movement-script", str(fixture))
    if CONFIG.exists():
        CONFIG.unlink()
    result = game("motion_blur_options", *MENU, "--movement-script", "test-inputs/bevy-options-motion-blur.json")
    expect("Video tab reached", result.sample("video-tab")["console"]["options"]["tab"], "Video")
    expect("Advanced dialog opened", result.sample("video-advanced")["console"]["options"]["sub"], "VideoAdvanced")
    selected = result.sample("advanced-selected")["console"]["options"]
    holds("Advanced changes the draft", selected["changed"] and not selected["list_open"], selected)
    a = saved()["video"]["advanced"]
    expect("camera blur saved", a["motion_blur"], True)
    expect("Advanced weapon blur saved", a["motion_blur_viewmodel"], True)
    expect("Advanced leaves Ultra world blur disabled", a["motion_blur_world"], False)


def check_viewmodel_motion_blur():
    """A still world view has animated weapon vectors, with MSAA and the option lifecycle."""
    script = json.loads((REPO / "test-inputs/bevy-viewmodel-motion-blur.json").read_text())
    shader = GAME / "bevy-assets/shaders/weapon_blur.wgsl"
    original = shader.read_text(encoding="utf-8")
    hook = "if alpha == 0.0 { return select(world,base,own.z < 0.5); }"
    holds("animation reconstruction hook", hook in original)

    def run(name, aa="none", enabled=True, state="swing", advanced=True):
        settings(video={"hdr": "none", "hdr_migrated": True, "advanced": {
            "motion_blur": enabled, "motion_blur_viewmodel": advanced, "antialiasing": aa, "vsync": False}})
        frames = json.loads(json.dumps(script))
        if state == "still":
            frames[-1]["tick"] = 170
        elif state == "paused":
            frames[-1]["paused"] = True
        elif state == "switch":
            frames[-1]["actions"] = [{"action": "slot", "slot": 1}, {"action": "confirm"}]
        fixture = OUT / f"{name}.input.json"
        fixture.write_text(json.dumps(frames))
        result = game(name, "--map", "d1_trainstation_02", "--width", "1024", "--height", "768",
                      "--movement-script", str(fixture), "--capture-burst", "4")
        sample = result.sample("swing")
        expect("fixed player view", result.report["render"]["simulation"]["eye"], [-4324.0, -224.0, 64.0])
        expect("posed geometry", result.report["render"]["presentation"]["pose_or_visibility_mismatches"], 0)
        holds("no rendering errors", "Caught rendering error" not in result.log)
        if state == "swing":
            holds("crowbar attack animation", sample["gameplay"]["viewmodel"]["animation"].startswith("miss"),
                  sample["gameplay"]["viewmodel"])
        return result

    def moving_pixels(result, burst=True):
        images = [result.capture] + ([OUT / f"{result.name}-{i}.png" for i in range(1, 5)] if burst else [])
        counts = []
        for path in images:
            width, height, rgb = png_rgb(path)
            locations = [(i // 3 % width, i // 3 // width) for i in range(0, len(rgb), 3)
                         if rgb[i] > 240 and rgb[i+1] < 15 and rgb[i+2] > 240]
            # Diagnostic magenta belongs to the lower weapon region, never the still world/HUD.
            holds("vectors restricted to weapon", all(y > height * 0.4 for _, y in locations))
            counts.append(len(locations))
        return max(counts)

    normal = run("viewmodel_blur_swing")
    try:
        shader.write_text(original.replace("if pixel_motion < 1.0 || data.params.x == 0.0", "if true"), encoding="utf-8")
        control = run("viewmodel_blur_sharp")
        changes = pixel_changes(control.capture, normal.capture, threshold=2, region=(180, 430, 1000, 768))
        holds("reconstruction changes the moving weapon", changes["changed"] > 100, changes)
        world = pixel_changes(control.capture, normal.capture, threshold=2, region=(0, 0, 1024, 300))
        expect("still world unchanged", world["changed"], 0)
        # Debug reconstruction only after the shader's one-pixel motion threshold. This
        # proves skinned motion reaches the GPU, independently of similar-looking poses.
        shader.write_text(original.replace(hook, hook + "\n    return vec4(1.0, 0.0, 1.0, 1.0);"), encoding="utf-8")
        for aa in ("none", "x4"):
            result = run("viewmodel_vectors_" + aa, aa=aa)
            holds("animated pixels", moving_pixels(result) > 100)
        shader.write_text(original.replace(hook, hook + "\n    if own.z < 0.01 && alpha > 0.01 { return vec4(1.0,0.0,1.0,1.0); }\n    return base;"), encoding="utf-8")
        holds("blur extends outside the current silhouette", moving_pixels(run("viewmodel_silhouette_trail")) > 100)
        shader.write_text(original.replace(hook, hook + "\n    return vec4(1.0,0.0,1.0,1.0);"), encoding="utf-8")
        for state in ("still", "paused", "switch"):
            expect(state + " has no stale motion", moving_pixels(run("viewmodel_vectors_" + state, state=state), burst=state != "switch"), 0)
        expect("disabled has no motion pass", moving_pixels(run("viewmodel_vectors_off", enabled=False)), 0)
        expect("retail Enabled keeps the weapon sharp", moving_pixels(run("viewmodel_vectors_retail", advanced=False)), 0)
    finally:
        shader.write_text(original, encoding="utf-8")


def check_flashlight_viewmodel():
    """The same beam lights the weapon, including rotated views and portrait framing."""
    def run(weapon: str, name: str, on: bool = True, portrait: bool = False):
        width, height = (1080, 1920) if portrait else (1920, 1080)
        settings(video={"width": width, "height": height, "hdr": "none", "hdr_migrated": True,
                        "advanced": {"shadow_detail": "medium"}})
        script = json.loads((REPO / f"test-inputs/bevy-flashlight-viewmodel-{weapon}.json").read_text())
        if not on:
            script[0]["ui"][1]["command"] = "sv_cheats 1; impulse 101"
        fixture = OUT / f"{name}.input.json"
        fixture.write_text(json.dumps(script))
        result = game(name, "--map", "d1_trainstation_02", "--width", str(width), "--height", str(height),
                      "--movement-script", str(fixture))
        expect("fixed dimensions", png_size(result.capture), [width, height])
        holds("completed", result.report["render"]["simulation"]["script_finished"])
        inventory = result.sample(f"flashlight-viewmodel-{weapon}")["gameplay"]["inventory"]
        expect("selected weapon", inventory["active"], "weapon_pistol" if weapon == "pistol" else "weapon_smg1")
        expect("no stale posed bounds", result.report["render"]["presentation"]["pose_or_visibility_mismatches"], 0)
        return result
    shader = GAME / "bevy-assets/shaders/source.wgsl"
    original = shader.read_text(encoding="utf-8")
    marker = "if material.viewmodel_flashlight.limits.z < 0.5"
    holds("control hook exists", marker in original)
    for weapon, portrait in (("pistol", False), ("smg", False), ("pistol", True)):
        label = f"flashlight_vm_{weapon}_{'portrait' if portrait else 'landscape'}"
        normal = run(weapon, label, portrait=portrait)
        try:
            # Suppress only the weapon's beam contribution: world/HUD/baked lighting stay live.
            shader.write_text(original.replace(marker, "if true"), encoding="utf-8")
            control = run(weapon, label + "_control", portrait=portrait)
        finally:
            shader.write_text(original, encoding="utf-8")
        width, height = png_size(normal.capture)
        region = (int(width * 0.35), int(height * 0.4), int(width * 0.85), int(height * 0.87))
        changes = pixel_changes(control.capture, normal.capture, threshold=8, region=region)
        holds("beam illuminates weapon pixels above the HUD", changes["brighter"] > 500
              and changes["darker"] < 50, changes)
        world = pixel_changes(control.capture, normal.capture, threshold=2,
                              region=(0, 0, width, int(height * 0.4)))
        holds("weapon control preserves the world", world["changed"] < 10, world)
    off = run("pistol", "flashlight_vm_off", on=False)
    try:
        shader.write_text(original.replace(marker, "if true"), encoding="utf-8")
        control = run("pistol", "flashlight_vm_off_control", on=False)
    finally:
        shader.write_text(original, encoding="utf-8")
    changes = pixel_changes(control.capture, off.capture, threshold=2, region=(672, 432, 1632, 939))
    holds("off has no weapon light", changes["changed"] < 10, changes)


def check_flashlight_availability():
    result = game("flashlight_availability", "--map", "d1_canals_01",
                  "--movement-script", "test-inputs/bevy-flashlight-availability.json")
    for label, suit, on, disabled in (
        ("no-suit", False, False, False), ("suit-picked-up", True, False, False),
        ("suit-light-on", True, True, False), ("disabled-off", True, False, True),
        ("disabled-blocked", True, False, True), ("enabled-still-off", True, False, False),
        ("reenabled-on", True, True, False),
    ):
        sample = result.sample(label)["gameplay"]
        expect(label + " suit", sample["inventory"]["suit"], suit)
        expect(label + " light", sample["flashlight"]["on"], on)
        expect(label + " permission", sample["flashlight"]["disabled"], disabled)
    state = result.sample("reenabled-on")["gameplay"]["flashlight"]
    holds("legacy suit power drains", state["battery"] is None
          and 90 < state["suit_power"]["power"] < 100, state)

    for label in ("dead-off", "dead-blocked"):
        sample = result.sample(label)
        holds(label + " player died", sample["player"]["dead"])
        expect(label + " no beam", sample["gameplay"]["flashlight"]["on"], False)
    expect("permission before direct load", result.sample("disabled-before-direct")["gameplay"]["flashlight"]["disabled"], True)
    fresh = result.sample("direct-load-fresh")["gameplay"]["flashlight"]
    holds("direct load resets player flashlight", not fresh["on"] and not fresh["disabled"]
          and fresh["suit_power"]["power"] == 100, fresh)


def check_options_status():
    """Platform-only visibility, cyan development status and unchanged orange retail hints."""
    for case, tab in (("mouse", "Mouse"), ("audio", "Audio"), ("advanced", "Video")):
        if CONFIG.exists():
            CONFIG.unlink()
        run = game("options_status_" + case, *MENU,
                   "--movement-script", f"test-inputs/bevy-options-status-{case}.json")
        options = run.sample("status-final")["console"]["options"]
        expect(case + " tab", options["tab"], tab)
        holds(case + " development hint", bool(options["tooltip"]["development"]), options["tooltip"])
        if case == "mouse":
            holds("retail and development hints coexist", bool(options["tooltip"]["retail"]))
            retail = run.sample("retail-only")["console"]["options"]["tooltip"]
            holds("finished control keeps only retail hint", bool(retail["retail"]) and retail["development"] is None, retail)
            for label in ("pressed-dismissed", "popup-occluded"):
                expect(label, run.sample(label)["console"]["options"]["tooltip"], None)
            holds("completed mouse controls unmarked", not {"ReverseMouse", "MouseFilter", "Slider"}
                  .intersection(options["development_controls"]))
            if SYSTEM == "Darwin":
                holds("unavailable macOS controls hidden", not {"MouseRaw", "MouseAccelerationCheckbox",
                      "MouseAccelerationSlider", "MouseAccelerationLabel"}.intersection(options["controls"]))
        elif case == "audio":
            expect("development-only cyan hint", options["tooltip"]["retail"], "")
            holds("working volume/captions unmarked", not {"SFXSlider", "MusicSlider", "CloseCaptionCheck"}
                  .intersection(options["development_controls"]))
        else:
            expect("Advanced subdialog", options["sub"], "VideoAdvanced")
            holds("completed rendering controls unmarked", not {"ModelDetail", "TextureDetail", "ShadowDetail",
                  "ColorCorrection", "MotionBlur", "AntialiasingMode", "FilteringMode", "VSync"}
                  .intersection(options["development_controls"]))
            expect("finished model has no tooltip", run.sample("completed-model")["console"]["options"]["tooltip"], None)
            if SYSTEM != "Windows":
                holds("Aero control and label hidden", not {"D3D9Ex", "D3D9ExLabel"}.intersection(options["controls"]))
        # The status fill is deliberately light blue, visibly different from the orange retail fill.
        _, _, rgb = png_rgb(run.capture)
        pixels = sum(1 for r, g, b in zip(rgb[::3], rgb[1::3], rgb[2::3]) if b > 180 and g > 170 and r < 150)
        holds(case + " cyan hint pixels", pixels > 1000, pixels)


def check_lamp_emission():
    """The installed tube mask emits independently of its baked static-prop lighting."""
    settings(video={"width": 1920, "height": 1080, "hdr": "none", "hdr_migrated": True})
    args = ("--map", "d1_canals_01", "--width", "1920", "--height", "1080",
            "--movement-script", "test-inputs/bevy-canals-lamp.json")
    normal = game("canals_lamp", *args)
    holds("completed", normal.report["render"]["simulation"]["script_finished"])
    shader = GAME / "bevy-assets/shaders/source.wgsl"
    original = shader.read_text(encoding="utf-8")
    marker = "srgb_to_linear(material_albedo * material.tint.rgb), base_alpha"
    holds("emissive albedo control exists", marker in original)
    try:
        shader.write_text(original.replace(marker, "srgb_to_linear(base.rgb), base_alpha"),
                          encoding="utf-8")
        control = game("canals_lamp_baked_control", *args)
    finally:
        shader.write_text(original, encoding="utf-8")
    expect("fixed dimensions", png_size(normal.capture), [1920, 1080])
    changes = pixel_changes(control.capture, normal.capture, threshold=8, region=(630, 500, 1290, 580))
    holds("tubes emit without baked-light dimming", changes["brighter"] > 10000
          and changes["darker"] < 10, changes)
    wall = pixel_changes(control.capture, normal.capture, threshold=2, region=(0, 0, 1920, 490))
    holds("unmasked wall remains unchanged", wall["changed"] < 10, wall)


def check_hud_aspect():
    """Options > Video > HUD Aspect Ratio: the HUD panels sit inside a centered frame of the chosen
    aspect (report `hud.frame` = [x, width] and `hud.panels` = health, suit, ammo, secondary ammo
    rectangles in screen pixels), Fit Display spans the screen, and 16:9 is unchanged."""
    def hud(name: str, aspect: str, width: int, height: int) -> dict:
        if CONFIG.exists():
            CONFIG.unlink()
        settings(video={"hud_aspect": aspect})
        run = game(f"hud_aspect_{name}", "--map", "d1_trainstation_02", "--width", str(width),
                   "--height", str(height), "--movement-script", "test-inputs/bevy-hud.json")
        return run.report["render"]["presentation"]["hud"]
    h = 1440
    fit = hud("fit_ultrawide", "fit", 3440, h)
    expect("fit frame", fit["frame"], [0, 3440])
    holds("fit: health at the left edge, ammo at the right",
          fit["panels"][0][0] < 100 and fit["panels"][2][0] + fit["panels"][2][2] > 3440 - 100, fit["panels"])
    for aspect, ratio in (("normal", 4 / 3), ("wide16x9", 16 / 9), ("wide16x10", 16 / 10)):
        report = hud(f"{aspect}_ultrawide", aspect, 3440, h)
        x, width = report["frame"]
        holds(f"{aspect}: centered {ratio:.3f} frame", abs(width - h * ratio) < 1 and abs(x * 2 + width - 3440) < 1,
              report["frame"])
        health, _, ammo, _ = report["panels"]
        holds(f"{aspect}: panels inside the frame", health[0] >= x and ammo[0] + ammo[2] <= x + width + 1,
              report["panels"])
        # Same layout as a 16:9 style screen of the frame's size, moved right by the frame's x.
        holds(f"{aspect}: health keeps its distance from the frame edge",
              abs((health[0] - x) - fit["panels"][0][0]) < 1, (health, x))
        holds(f"{aspect}: ammo keeps its distance from the frame edge",
              abs((x + width) - (ammo[0] + ammo[2]) - (3440 - (fit["panels"][2][0] + fit["panels"][2][2]))) < 1, ammo)
    # A screen no wider than the chosen aspect is unchanged, bit for bit.
    plain = hud("fit_16x9", "fit", 1920, 1080)
    centered = hud("wide16x9_16x9", "wide16x9", 1920, 1080)
    expect("16:9 frame", centered["frame"], plain["frame"])
    expect("16:9 panels", centered["panels"], plain["panels"])
    expect("16:9 quads", centered["quads"], plain["quads"])
    CONFIG.unlink()


# ---- gameplay replays (background) ---------------------------------------------------------
#
# Every other movement-script fixture in test-inputs/ runs on its map with the generic checks
# below, plus the specific assertions of the fixtures listed in GAMEPLAY_CHECKS. A fixture that
# is in neither the UI checks, GAMEPLAY_MAPS nor SKIPPED fails the `coverage` check, so a new
# fixture is never silently left out.

STATION_01 = ("bevy-activity-idle", "bevy-attention", "bevy-audio-scenes", "bevy-face-security-1805",
              "bevy-face-security-2100", "bevy-face-security-2130", "bevy-face-security-2245",
              "bevy-gestures-security-1805", "bevy-gestures-security-2245", "bevy-gestures-timeline",
              "bevy-monitors-kleiner-scene")
STATION_02 = ("bevy-barrier-hum", "bevy-bugbait", "bevy-bugbait-wall", "bevy-carry-bottle-shove",
              "bevy-crate-drop-rest", "bevy-crate-on-box-stack", "bevy-crossbow", "bevy-crowbar-surfaces",
              "bevy-door-inputs", "bevy-door-swing", "bevy-effect-ball", "bevy-effect-decal",
              "bevy-effect-decal-door", "bevy-effect-explosion", "bevy-effect-grenade",
              "bevy-entities-weapons", "bevy-footsteps", "bevy-frag", "bevy-frag-empty-switch",
              "bevy-frag-high", "bevy-frag-lob", "bevy-frag-native", "bevy-frag-roll",
              "bevy-frag-use-pickup", "bevy-frag-wall", "bevy-monitors-breen-screen",
              "bevy-monitors-breen-slate", "bevy-movement", "bevy-pallet-gibs-fade",
              "bevy-pallet-kills-cop-physdamagescale1", "bevy-pallet-launched-into-wall",
              "bevy-pallet-on-npc-hull", "bevy-pallet-tips-off-npc-hull", "bevy-pallet-tips-off-plaza-cop",
              "bevy-particles-ball", "bevy-particles-fire", "bevy-particles-smoke",
              "bevy-physcannon", "bevy-physcannon-npc-impact", "bevy-physcannon-pallet",
              "bevy-physcannon-props", "bevy-pistol-breaks-bottle-skin4",
              "bevy-pistol-breaks-teeter-seat-chunks", "bevy-pistol-level", "bevy-player-on-box",
              "bevy-player-on-npc-hull", "bevy-player-push", "bevy-player-walk-into-boxes",
              "bevy-playground-constraints", "bevy-rpg", "bevy-soundscapes",
              "bevy-studio-decal-bugbait-citizen", "bevy-studio-decal-bugbait-cop",
              "bevy-studio-decal-cop", "bevy-tictactoe-use-spins-block", "bevy-trace-bleed-cop-wall",
              "bevy-use-carry")
STATION_04 = ("bevy-player-death", "bevy-player-hurt")  # station 02 keeps the player invulnerable
GAMEPLAY_MAPS = {**{f: "d1_trainstation_01" for f in STATION_01},
                 **{f: "d1_trainstation_02" for f in STATION_02},
                 **{f: "d1_trainstation_04" for f in STATION_04}}
# Fixtures used by the UI checks above (by file name stem).
UI_FIXTURES = tuple(f"bevy-chapters-{game}{suffix}" for game in ("hl2", "episodic", "ep2", "lostcoast")
                    for suffix in ("", "-preview")) + ("bevy-menu-submenus", "bevy-menu-return", "bevy-menu-chapter-return",
               "bevy-menu-options-nonmodal", "bevy-menu-options-list",
               "bevy-menu-resolution-change", "bevy-options-keyboard-apply-resolution", "bevy-save-config",
               "bevy-console", "bevy-console-map", "bevy-campaign", "bevy-dev-overlay", "bevy-hud",
               "bevy-menu-video-8k", "bevy-menu-resolution-stress",
               "bevy-menu-resolution-stress-windowed", "bevy-menu-video-portrait",
               "bevy-menu-video-keep-timeout", "bevy-menu-video-keep-ok", "bevy-menu-video-keep-escape",
               "bevy-console-open", "bevy-flashlight-hud-on", "bevy-flashlight-hud-recharged",
               "bevy-flashlight-depth-actor", "bevy-flashlight-depth-fence",
               "bevy-options-motion-blur", "bevy-viewmodel-motion-blur", "bevy-flashlight-viewmodel-pistol", "bevy-flashlight-viewmodel-smg", "bevy-canals-lamp", "bevy-flashlight-availability",
               "bevy-options-status-mouse", "bevy-options-status-audio", "bevy-options-status-advanced",
               "bevy-lostcoast-commentary", "bevy-video")
SKIPPED: dict[str, str] = {}  # fixture: why it is not run
# Sounds the installed maps reference but the installation does not have.
MISSING_SOUNDS = {"d1_trainstation_01": {"Intro.zoomout", "ambience/particle_suck1.wav",
                                          "ambience/steamburst1.wav"}}
# Sound scripts that installed Half-Life 2 models name in their events but only another game
# defines: the female citizen and Alyx `shoot_shotgun` AE_CL_PLAYSOUND names an Episode One
# script. Every map preloads it through the F4 citizen spawn pool (one template per head).
MISSING_MODEL_SOUNDS = {"Weapon_Alyx_Shotgun.Cock"}


# Sound errors that main shows too, by fixture: (wave prefix, reason).
KNOWN_SOUND_GAPS = {
    f: ("physics/body/body_medium_impact_soft", "a body impact variant is not preloaded (main too, not a regression)")
    for f in ("bevy-pallet-on-npc-hull", "bevy-pallet-tips-off-npc-hull", "bevy-pallet-tips-off-plaza-cop",
              "bevy-pallet-kills-cop-physdamagescale1", "bevy-physcannon-npc-impact")
}
# Collision models the loader does not support yet (it reports them and uses the render hull).
KNOWN_COLLISION_WARNINGS = {
    "d1_trainstation_01": {
        "models/props_c17/SuitCase_Passenger_Physics.mdl: unsupported PHY root rotation; dynamic prop uses its render hull",
        "models/props_junk/wood_crate001a_damaged.mdl: invalid KeyValues entry"},
    "d1_trainstation_02": {
        "models/props_c17/doll01.mdl: unsupported PHY root rotation; dynamic prop uses its render hull"},
}


def png_size(path: Path) -> Optional[list[int]]:
    header = path.read_bytes()[:24]
    if len(header) == 24 and header[:8] == b"\x89PNG\r\n\x1a\n" and header[12:16] == b"IHDR":
        return [int.from_bytes(header[16:20], "big"), int.from_bytes(header[20:24], "big")]
    return None


def near(a, b, tolerance=0.001) -> bool:
    if isinstance(a, (list, tuple)) and isinstance(b, (list, tuple)):
        return len(a) == len(b) and all(near(x, y, tolerance) for x, y in zip(a, b))
    return isinstance(a, (int, float)) and isinstance(b, (int, float)) and abs(a - b) <= tolerance


def replay(fixture: str) -> Run:
    """Run one gameplay fixture on its map and apply the checks every fixture must pass."""
    script = REPO / "test-inputs" / f"{fixture}.json"
    run = game(fixture, "--map", GAMEPLAY_MAPS[fixture], "--width", "1280", "--height", "720",
               "--movement-script", str(script.relative_to(REPO)), timeout=300)
    check_report(run, json.loads(script.read_text(encoding="utf-8")), GAMEPLAY_MAPS[fixture], fixture)
    return run


def check_report(run: Run, steps: list, map_: str, fixture: str = "") -> None:
    report, render = run.report, run.report["render"]
    simulation, presentation = render["simulation"], render["presentation"]
    authored = {step["label"]: step["tick"] for step in steps if step.get("label")}
    sampled = {s["label"]: s["host_tick"] for s in simulation["samples"] if s.get("label")}
    holds("every label sampled at its tick", sampled == authored,
          {k: (authored.get(k), sampled.get(k)) for k in set(authored) | set(sampled)
           if authored.get(k) != sampled.get(k)})
    holds("capture written at the reported size", render["capture_completed"]
          and report["capture_write_error"] is None and run.capture is not None
          and png_size(run.capture) == render["capture_size"], render["capture_size"])
    holds("drawn entities agree with the simulation", presentation["pose_or_visibility_mismatches"] == 0
          and presentation["effects"]["pose_mismatches"] == 0,
          (presentation["pose_or_visibility_mismatches"], presentation["effects"]["pose_mismatches"]))
    io = simulation["gameplay"]["io"]
    holds("entity I/O within its budget", io["budget_exhaustions"] == 0 and io["malformed_outputs"] == 0)
    fixture_errors = [k for k in io["unsupported"] if k.startswith("fixture ")]
    holds("every fixture action found its entity", not fixture_errors, fixture_errors)
    models = report["models"]
    warnings = set(models["collision_warnings"]) - KNOWN_COLLISION_WARNINGS.get(map_, set())
    holds("models and collision load", not models["errors"] and not warnings
          and models["collision_models_missing"] == 0, models["errors"] or sorted(warnings))
    holds("no texture budget failure", not any("budget" in e.lower() or "exceeds" in e.lower()
                                               for e in map(str, report["texture_errors"])))
    effects = presentation["effects"]
    holds("effect sprites and models load", not effects["sprite_errors"] and not effects["model_errors"],
          effects["sprite_errors"] or effects["model_errors"])
    audio = set(presentation["audio"]["errors"]) - MISSING_SOUNDS.get(map_, set()) - MISSING_MODEL_SOUNDS
    if fixture in KNOWN_SOUND_GAPS:
        prefix, why = KNOWN_SOUND_GAPS[fixture]
        known = {k for k in audio if k.startswith(prefix)}
        if known:
            print(f"known gap: {sorted(known)}: {why}")
        audio -= known
    holds("sounds play", not audio, {k: presentation["audio"]["errors"][k] for k in audio})


def samples_of(run: Run) -> dict:
    return run.samples()


def check_movement(run: Run) -> None:
    """Bench jumps: air duck keeps the trajectory, pause freezes the player, held jump re-arms."""
    s = run.samples()
    sim = run.report["render"]["simulation"]
    p = lambda label: s[label]["player"]
    eye = lambda label: p(label)["feet"][2] + p(label)["eye_height"]
    holds("walk mode throughout", all(x["fly"] is False for x in sim["samples"]))
    ground = p("bench-ground")
    holds("settles on the bench top", ground["grounded"] and near(ground["feet"][2], 36.17742157), ground["feet"])
    ordinary, crouch = p("ordinary-first"), p("crouch-first")
    holds("both first jumps match before the air duck",
          all(near(ordinary[k], crouch[k]) for k in ("feet", "velocity", "eye_height"))
          and not ordinary["grounded"] and not crouch["grounded"]
          and not ordinary["crouched"] and not crouch["crouched"])
    holds("the first jump rises", ordinary["velocity"][2] > 0 and ordinary["feet"][2] > ground["feet"][2])
    for phase in ("rise", "apex"):
        a, b = p("ordinary-" + phase), p("crouch-" + phase)
        holds(f"air duck {phase}: same trajectory, feet raised 36 once",
              near(a["velocity"], b["velocity"]) and near(a["feet"][:2], b["feet"][:2])
              and near(b["feet"][2] - a["feet"][2], 36.) and near(eye("ordinary-" + phase), eye("crouch-" + phase))
              and not a["crouched"] and b["crouched"] and not a["grounded"] and not b["grounded"],
              b["feet"][2] - a["feet"][2])
    unduck, descend = p("air-unduck"), p("ordinary-descend")
    holds("unducking in the air restores the ordinary descent", not unduck["crouched"]
          and all(near(unduck[k], descend[k]) for k in ("feet", "velocity", "eye_height")))
    for label in ("ordinary-landed", "crouch-landed", "bench-final"):
        holds(f"{label} on the same bench", p(label)["grounded"] and near(p(label)["feet"][2], ground["feet"][2])
              and near(p(label)["velocity"][2], 0.))
    holds("pause freezes the whole player state",
          p("pause-moving") == p("paused-before") == p("paused-held") and not s["pause-moving"]["paused"]
          and s["paused-before"]["paused"] and s["paused-held"]["paused"])
    holds("resume advances one fixed tick", p("resumed-held")["ticks"] == p("paused-held")["ticks"] + 1
          and not s["resumed-held"]["paused"])
    landed, released, rearmed = p("held-landed"), p("released-on-ground"), p("rearmed-first")
    holds("a held jump lands without jumping again", landed["grounded"]
          and near(landed["feet"][2], ground["feet"][2]) and near(landed["velocity"][2], 0.))
    holds("releasing keeps the ground", released["grounded"] and near(released["feet"], landed["feet"]))
    holds("the next press jumps like the first", not rearmed["grounded"]
          and rearmed["ticks"] == released["ticks"] + 1 and near(rearmed["velocity"], ordinary["velocity"])
          and near(rearmed["feet"], ordinary["feet"]))
    holds("and keeps rising", p("rearmed-rise")["feet"][2] > rearmed["feet"][2])
    holds("the final state holds, paused", sim["paused"] and s["done"]["paused"] and sim["player"] == p("done"))
    holds("the camera is at the player's eye", near(run.report["render"]["camera_source"], sim["eye"])
          and near(sim["eye"][2], eye("done")))
    textures = run.report["textures"]
    holds("decoded textures within their budget",
          0 < textures["decoded_base_bytes"] <= textures["decoded_base_budget"])


def check_entities_weapons(run: Run) -> None:
    """Station doors block and open, E uses both leaves, pistol / SMG grenade / AR2 ball ammo."""
    s = run.samples()
    g = lambda label: s[label]["gameplay"]
    door = lambda label: s[label]["ray_entity"] in (350, 351)
    holds("closed doors stop walking", s["closed-blocks"]["player"]["feet"][0] < -3199)
    holds("a closed door blocks the weapon ray", door("closed-blocks"))
    holds("the open doorway can be crossed", s["open-passed"]["player"]["feet"][0] > -3199)
    expect("opening completes both leaves", g("open-passed")["io"]["door_completions"], 2)
    holds("the reclosed door blocks the ray", door("reclosed"))
    expect("E opens both linked leaves", g("use-opened")["io"]["door_completions"], 6)
    holds("a selection leaves the active weapon in place", g("pistol-pending")["pending"] == "weapon_pistol"
          and g("pistol-pending")["inventory"]["active"] == "weapon_crowbar")
    holds("the pistol fires once and spends one round", g("pistol-fired")["inventory"]["shots"] == 1
          and g("pistol-fired")["inventory"]["owned"]["weapon_pistol"] == 17)
    holds("the SMG launches a grenade and spends secondary ammo",
          g("smg-secondary")["projectiles"]["diagnostics"]["grenades_spawned"] == 1
          and g("smg-secondary")["inventory"]["reserve_ammo"]["SMG1_Grenade"] == 2)
    holds("the AR2 charge does not launch at once",
          g("ar2-charge")["projectiles"]["diagnostics"]["balls_spawned"] == 0
          and g("ar2-charge")["inventory"]["reserve_ammo"]["AR2AltFire"] == 3)
    holds("the delayed AR2 launch spends one secondary round",
          g("ar2-launched")["projectiles"]["diagnostics"]["balls_spawned"] == 1
          and g("ar2-launched")["inventory"]["reserve_ammo"]["AR2AltFire"] == 2)
    holds("entity meshes drawn", run.report["render"]["presentation"]["owned_meshes"] > 0)


def rotate(q, p):
    x, y, z, w = q
    px, py, pz = p
    t = (2 * (y * pz - z * py), 2 * (z * px - x * pz), 2 * (x * py - y * px))
    return (px + w * t[0] + y * t[2] - z * t[1], py + w * t[1] + z * t[0] - x * t[2],
            pz + w * t[2] + x * t[1] - y * t[0])


def doors_swing_away(run: Run) -> None:
    s = run.samples()
    for label, side, count in (("inside-open", -1, 2), ("outside-open", 1, 6)):
        expect(f"{label} completions", s[label]["gameplay"]["io"]["door_completions"], count)
        leaves = [e for e in s[label]["gameplay"]["entities"] if e["entity"] in (350, 351)]
        holds(f"{label}: both leaves swing away from the opener", len(leaves) == 2
              and all(rotate(e["rotation"], [0, 47, 0])[0] * side < -46 for e in leaves))
    expect("closed completions", s["closed"]["gameplay"]["io"]["door_completions"], 4)
    holds("the closed door blocks the ray", s["closed"]["ray_entity"] in (350, 351))


def check_door_swing(run: Run) -> None:
    """Linked station doors open away from the player from both sides and close again."""
    doors_swing_away(run)


def check_door_inputs(run: Run) -> None:
    """OpenAwayFrom with a named opener, from both sides."""
    doors_swing_away(run)
    unsupported = run.report["render"]["simulation"]["gameplay"]["io"]["unsupported"]
    holds("OpenAwayFrom is implemented", not any("openawayfrom" in k.lower() for k in unsupported))


def frozen_final(run: Run) -> None:
    sim = run.report["render"]["simulation"]
    holds("ends paused on the last sample", sim["paused"]
          and sim["gameplay"]["time"] == sim["samples"][-1]["gameplay"]["time"])


def check_effect(name: str) -> Callable[[Run], None]:
    def check(run: Run) -> None:
        frozen_final(run)
        g, p = run.report["render"]["simulation"]["gameplay"], run.report["render"]["presentation"]["effects"]
        if name == "grenade":
            holds("the live grenade is drawn", p["grenade_draws"] == p["observed_grenades"] == 1)
            holds("one grenade, one 15 ms step out", len(g["projectiles"]["active"]) == 1
                  and g["projectiles"]["active"][0]["position"][0] == -3245, g["projectiles"]["active"])
            expect("SMG grenades left", g["inventory"]["reserve_ammo"]["SMG1_Grenade"], 2)
        elif name == "ball":
            # sprite_draws counts drawn quads, which also include particles (2 to 10 here).
            holds("the paused ball draws its body and halo", p["sprite_draws"] >= 2 and p["pooled_quads"] == 10,
                  (p["sprite_draws"], p["pooled_quads"]))
            holds("the AR2 launches after the charge and spends one", g["projectiles"]["diagnostics"]["balls_spawned"] == 1
                  and g["inventory"]["reserve_ammo"]["AR2AltFire"] == 2)
        elif name == "explosion":
            holds("the grenade detonated", g["projectiles"]["diagnostics"]["grenade_detonations"] == 1
                  and not g["projectiles"]["active"])
            # At least the two clouds: sprite_draws also counts the explosion's particles.
            holds("both explosion clouds drawn", p["sprite_draws"] >= 2 and p["effect_frames"] > 0
                  and g["projectiles"]["effects"][0]["kind"] == "GrenadeExplosion", p["sprite_draws"])
        else:  # decal, decal-door
            impacts = g["impacts"]
            holds("one clipped, drawn impact mark", impacts["created"] == impacts["active"] == p["decal_draws"]
                  == p["observed_decals"] == 1 and impacts["unclippable"] == 0)
            holds("one pistol round spent", g["inventory"]["shots"] == 1
                  and g["inventory"]["owned"]["weapon_pistol"] == 17)
            # The installation has no base textures for five unused sand shot decals.
            holds("only the sand shot decals are missing", len(impacts["errors"]) == 5
                  and all(k.startswith("decals/sand/") for k in impacts["errors"]), impacts["errors"])
            if name == "decal-door":
                expect("the marked door opens both leaves", g["io"]["door_completions"], 2)
    check.__doc__ = f"Effects fixture {name}: frozen final frame and its effect state."
    return check


def check_particles(name: str) -> Callable[[Run], None]:
    def check(run: Run) -> None:
        frozen_final(run)
        sim, p = run.report["render"]["simulation"], run.report["render"]["presentation"]["effects"]
        e = p["particle_emitters"]
        holds("every particle drawn", p["missing_particle_draws"] == 0 and e["capacity_rejections"] == 0)
        holds("quads presented", p["sprite_draws"] > 0)
        projectiles = sim["gameplay"]["projectiles"]
        if name == "ball":
            holds("the ball expired", not projectiles["active"]
                  and any(x["kind"] == "BallExplosion" for x in projectiles["effects"]))
            holds("both timed rings and sparks", e["rings_emitted"] == e["rings_live"] == 2
                  and e["live"]["Spark"] > 0 and e["live"]["Glow"] > 0)
            ids = [x["id"] for x in projectiles["effects"]]
            holds("unique effect ids", len(ids) == len(set(ids)))
            return
        holds("one grenade detonation", projectiles["diagnostics"]["grenade_detonations"] == 1 and e["events"] == 1)
        emitted = e["emitted"]
        holds("smoke, ember, fire and debris counts", emitted["Smoke"] == 44 and emitted["Ember"] == 16
              and 40 <= emitted["Fire"] <= 48 and 16 <= emitted["Debris"] <= 32, emitted)
        if name == "fire":
            holds("fire still burning", e["live"].get("Fire", 0) > 0, e["live"])
        else:
            holds("fire out, smoke and embers live", e["live"].get("Fire", 0) == 0
                  and e["live"].get("Smoke", 0) > 0 and e["live"].get("Ember", 0) == 16, e["live"])
    check.__doc__ = f"Particles fixture {name}: emitter counts and lifetimes."
    return check


def check_attention(run: Run) -> None:
    """Barney's LOOKAT interests in security_02: pause, UI freeze, overlap, cancel and tail."""
    s = run.samples()
    targets = lambda label: s[label]["gameplay"]["look_targets"].get("798", [])
    scene = lambda label: next(x for x in s[label]["gameplay"]["choreography"] if x["entity"] == 662)
    holds("security_02 runs", scene("scene-paused")["active"] or scene("scene-held")["active"],
          scene("scene-held"))
    holds("only the tested scene gives Barney interests",
          all(x["scene"] == 662 for v in s.values() for x in v["gameplay"]["look_targets"].get("798", [])))
    holds("a scene pause holds its clock", scene("scene-paused")["elapsed"] == scene("scene-held")["elapsed"]
          and scene("scene-held")["pause"])
    holds("the interests keep refreshing meanwhile", targets("scene-held")[0]["start"] > targets("scene-paused")[0]["start"]
          and targets("scene-held")[0]["importance"] == targets("scene-paused")[0]["importance"])
    overlap = targets("player-monitor-overlap")
    holds("an overlap selects the player after the monitor", [x["target"] for x in overlap] == [{"Entity": 183}, "Player"])
    holds("not the largest head importance", overlap[-1]["importance"] < overlap[0]["importance"])
    holds("the UI freezes scene and interest clocks", s["ui-paused"]["gameplay"]["time"] == s["ui-held"]["gameplay"]["time"]
          and targets("ui-paused") == targets("ui-held"))
    holds("resuming advances the interest clock", targets("ui-resumed")[-1]["start"] > targets("ui-held")[-1]["start"])
    holds("self interest supersedes the monitor", targets("self-interest")[-1]["target"] == {"Entity": 798})
    holds("cancel stops the scene, keeps the tail", not scene("cancel-retains-tail")["active"]
          and targets("cancel-retains-tail") and targets("cancel-retains-tail") == targets("cancel-request"))
    holds("the tail expires within 0.1 s", not targets("cancel-expired") and not targets("attention-final"))


# ---- expected outcomes ----
# Each labelled sample's gameplay outcome (I/O counters, inventory, scenes, projectiles, gibs,
# impacts, the player's feet and state, console mode, the forward ray) is compared with
# test-inputs/expected/<fixture>.json. Any change is reported: a regression to fix, or an
# intended change to record with UPDATE_EXPECTED=1 (review the diff of the expected files).
EXPECTED = REPO / "test-inputs" / "expected"
# Fields that differ between two runs of the same build, by fixture (left out of the comparison).
UNSTABLE = {
    "bevy-pallet-gibs-fade": {"hits"},
    "bevy-pallet-launched-into-wall": {"hits", "ray"},
    "bevy-pallet-tips-off-plaza-cop": {"ray"},
}
FEET_TOLERANCE = 1.0  # units
TIME_TOLERANCE = 1e-3  # seconds


def outcome(sample: dict) -> dict:
    g, player, inventory = sample["gameplay"], sample["player"], sample["gameplay"]["inventory"]
    diagnostics = dict(g["projectiles"]["diagnostics"])
    diagnostics.pop("occluded_blast_targets", None)  # varies between runs of the same build
    counters = lambda d: {k: v for k, v in d.items() if v}  # zero counters left out
    return {
        "io": counters({k: v for k, v in g["io"].items() if k != "unsupported"}),
        "owned": inventory["owned"], "reserve": inventory["reserve_ammo"], "active": inventory["active"],
        "pending": g["pending"], "shots": inventory["shots"], "bullets": inventory["bullets"],
        "hits": inventory["hits"], "kills": inventory["kills"], "health": inventory["health"],
        "armor": inventory["armor"], "suit": inventory.get("suit"),
        "scenes": sorted(c["targetname"] for c in g["choreography"] if c["active"]),
        "gesture_layers": len(g["gesture_layers"]),
        "look_targets": {k: len(v) for k, v in g["look_targets"].items()},
        "projectiles": counters(diagnostics), "projectiles_active": len(g["projectiles"]["active"]),
        "gibs": counters({k: g["gibs"][k] for k in ("spawned", "removed")}),
        "impacts": counters({k: g["impacts"][k] for k in ("created", "active", "studio_created", "unclippable")}),
        "carrying": g["carrying"], "feet": [round(x, 1) for x in player["feet"]],
        "grounded": player["grounded"], "crouched": player["crouched"], "player_ticks": player["ticks"],
        "time": round(g["time"], 4), "paused": sample["paused"], "mode": sample["console"]["mode"],
        "ray": sample["ray_entity"],
    }


def outcome_differences(want: dict, got: dict, unstable: set) -> list[str]:
    differences = []
    for label in sorted(want.keys() | got.keys()):
        if label not in got or label not in want:
            differences.append(f"{label}: {'missing' if label not in got else 'new label'}")
            continue
        for key in sorted(want[label].keys() | got[label].keys()):
            a, b = want[label].get(key), got[label].get(key)
            if key in unstable or a == b:
                continue
            if key == "feet" and near(a, b, FEET_TOLERANCE) or key == "time" and near(a, b, TIME_TOLERANCE):
                continue
            differences.append(f"{label}.{key}: expected {a!r}, got {b!r}")
    return differences


def expected_text(outcomes: dict) -> str:
    """One line per label, so a diff shows which sample changed."""
    line = lambda label: f"{json.dumps(label)}: {json.dumps(outcomes[label], sort_keys=True, separators=(',', ':'))}"
    return "{\n" + ",\n".join(line(label) for label in outcomes) + "\n}\n"


def check_outcomes(fixture: str, run: Run) -> None:
    got = {label: outcome(sample) for label, sample in run.samples().items()}
    path = EXPECTED / f"{fixture}.json"
    if os.environ.get("UPDATE_EXPECTED") == "1":
        EXPECTED.mkdir(exist_ok=True)
        path.write_text(expected_text(got), encoding="utf-8")
        print(f"recorded {path.relative_to(REPO)}")
        return
    holds("expected outcomes recorded", path.exists(), "record them with UPDATE_EXPECTED=1")
    differences = outcome_differences(json.loads(path.read_text(encoding="utf-8")), got,
                                      UNSTABLE.get(fixture, set()))
    for line in differences:
        print(f"  {line}")
    holds("outcomes as expected", not differences, f"{len(differences)} differences: {differences[:3]}")


GAMEPLAY_CHECKS: dict[str, Callable[[Run], None]] = {
    "bevy-movement": check_movement,
    "bevy-entities-weapons": check_entities_weapons,
    "bevy-door-swing": check_door_swing,
    "bevy-door-inputs": check_door_inputs,
    "bevy-attention": check_attention,
    **{f"bevy-effect-{n}": check_effect(n) for n in ("grenade", "ball", "decal", "explosion", "decal-door")},
    **{f"bevy-particles-{n}": check_particles(n) for n in ("fire", "smoke", "ball")},
}


def gameplay_check(fixture: str) -> Callable[[], None]:
    def run() -> None:
        result = replay(fixture)
        failure = None
        if fixture in GAMEPLAY_CHECKS:
            try:
                GAMEPLAY_CHECKS[fixture](result)
            except Failure as error:
                failure = error  # still compare the outcomes, then report the first failure
        check_outcomes(fixture, result)
        if failure:
            raise failure
    run.__doc__ = (GAMEPLAY_CHECKS[fixture].__doc__ if fixture in GAMEPLAY_CHECKS
                   else f"{fixture}: the generic replay checks")
    return run


def check_coverage():
    """Every test-inputs fixture is run by a check (or skipped with a reason)."""
    fixtures = {p.stem for p in (REPO / "test-inputs").glob("*.json")}
    covered = set(UI_FIXTURES) | set(GAMEPLAY_MAPS) | set(SKIPPED)
    covered |= {f for f in fixtures if f.startswith("bevy-options-capture")}
    holds("every fixture has a check", fixtures <= covered, sorted(fixtures - covered))
    holds("every listed fixture exists", covered <= fixtures, sorted(covered - fixtures))


# ---- windowed checks -----------------------------------------------------------------------

def check_window():
    """The presented window, a console resolution change and an Options (keyboard) Apply."""
    settings(video={"display_mode": "windowed", "width": 1024, "height": 768})
    run = game("window-console-resolution", "--map", "background01", "--main-menu", "--windowed",
               "--width", "1024", "--height", "768", "--frames", "600", "--capture-window",
               "--movement-script", "test-inputs/bevy-menu-resolution-change.json", background=False)
    holds("the frame was resized", "frame resize done" in run.log)
    expect("Options opened after the resize", run.console()["mode"], "Options")
    if CONFIG.exists():
        CONFIG.unlink()
    settings(video={"display_mode": "windowed", "width": 1024, "height": 768})
    run = game("window-options-apply", "--map", "background01", "--main-menu", "--windowed",
               "--width", "1024", "--height", "768", "--frames", "600", "--capture-window",
               "--movement-script", "test-inputs/bevy-options-keyboard-apply-resolution.json",
               background=False)
    options = run.sample("focus-apply")["console"].get("options") or {}
    expect("Shift+Tab reaches Apply", options.get("focus"), "apply")
    video = saved()["video"]
    holds("Space on Apply changed the resolution", (video["width"], video["height"]) != (1024, 768), video)
    holds("the window took it", "[video] apply" in run.log)
    # Windowed resolution changes on a real window never stall: the smallest and largest sizes
    # the console accepts and four changes on consecutive ticks (the last one wins).
    if CONFIG.exists():
        CONFIG.unlink()
    settings(video={"display_mode": "windowed", "width": 1024, "height": 768})
    run = game("window-resolution-stress", "--map", "background01", "--main-menu", "--windowed",
               "--width", "1024", "--height", "768", "--frames", "1200", "--capture-window",
               "--movement-script", "test-inputs/bevy-menu-resolution-stress-windowed.json",
               background=False)
    for label, size in (("hd", [1920, 1080]), ("smallest", [320, 240]), ("rapid", [1280, 720]),
                        ("largest", [8192, 8192]), ("final", [1024, 768])):
        expect(f"windowed frame at {label}", run.sample(label).get("frame"), size)
    ticks = [run.sample(l)["host_tick"] for l in ("hd", "smallest", "rapid", "largest", "final", "settle")]
    holds("the game kept running", ticks == sorted(ticks) and len(set(ticks)) == len(ticks), ticks)


def check_window_place():
    """macOS: a windowed window is sized and centred in the screen's visible frame."""
    for width, height in ((2560, 1600), (1024, 768)):
        settings(video={"display_mode": "windowed"})
        run = game(f"window_place-{width}x{height}", "--main-menu", "--windowed", "--width", str(width),
                   "--height", str(height), "--frames", "240", background=False, capture=False)
        placed = re.search(r"desktop area DesktopArea \{ visible: Rect \{ min: Vec2\(([-\d.]+), ([-\d.]+)\), "
                           r"max: Vec2\(([-\d.]+), ([-\d.]+)\) \}, title_bar: ([-\d.]+) \} at \[(-?\d+), (-?\d+)\]",
                           run.log)
        holds(f"{width}x{height} placed in the visible frame", placed is not None)
        top = float(placed.group(7)) / 2  # points on a 2x display; the log is in physical pixels
        holds(f"{width}x{height} below the menu bar", top >= float(placed.group(2)) - 1, top)


def check_no_border():
    """Windows: "Windowed (No Border)" is a borderless window covering the monitor."""
    settings(video={"display_mode": "no_border"})
    run = game("no_border", "--map", "background01", "--main-menu", "--borderless", "--frames", "240",
               "--capture-window", background=False)
    holds("the window is borderless full screen", "BorderlessFullscreen" in run.log)


GIB = 1 << 30
# Memory left to the operating system and the user's own applications while replays run.
MEMORY_RESERVE = 4 * GIB


def physical_memory() -> Optional[int]:
    """Installed memory in bytes, or None when the platform does not say."""
    try:
        return os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")  # macOS, Linux
    except (AttributeError, ValueError, OSError):
        pass
    if platform.system() == "Windows":
        import ctypes

        class MemoryStatus(ctypes.Structure):
            _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong),
                        ("total", ctypes.c_ulonglong), ("available", ctypes.c_ulonglong),
                        ("total_page", ctypes.c_ulonglong), ("available_page", ctypes.c_ulonglong),
                        ("total_virtual", ctypes.c_ulonglong), ("available_virtual", ctypes.c_ulonglong),
                        ("available_extended", ctypes.c_ulonglong)]
        status = MemoryStatus(length=ctypes.sizeof(MemoryStatus))
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
            return status.total
    return None


def parallel_jobs(requested: int, memory: Optional[int], per_game: int) -> int:
    """Gameplay replays to run at once: `requested`, but no more games than fit in `memory`
    beside MEMORY_RESERVE (a packaged game takes about 5-6 GB on a station map); unknown memory
    keeps `requested`. More games than memory makes the system swap: every check, and
    everything else on the machine, slows down instead of the suite finishing sooner."""
    if memory is None or per_game <= 0:
        return max(1, requested)
    return max(1, min(requested, (memory - MEMORY_RESERVE) // per_game))


def check_parallel_jobs() -> None:
    """The JOBS cap keeps parallel games within physical memory."""
    expect("16 GB fits two 6 GB games", parallel_jobs(4, 16 * GIB, 6 * GIB), 2)
    expect("64 GB keeps the requested four", parallel_jobs(4, 64 * GIB, 6 * GIB), 4)
    expect("too little memory still runs one", parallel_jobs(4, 4 * GIB, 6 * GIB), 1)
    expect("unknown memory keeps the request", parallel_jobs(3, None, 6 * GIB), 3)
    holds("this machine reports its memory", (physical_memory() or 0) > 0, physical_memory())


@dataclass
class Check:
    name: str
    run: Callable[[], None]
    category: str  # shared | windowed | macos | windows
    group: str = "ui"  # ui | gameplay (GROUP=gameplay runs one group)


CHECKS = [
    Check("submenus", check_submenus, "shared"),
    Check("return_menu", check_return_menu, "shared"),
    Check("chapter_menu", check_chapter_menu, "shared"),
    Check("chapter_selection", check_chapter_selection, "shared"),
    Check("chapter_quit", check_chapter_quit, "shared"),
    Check("menu_options", check_menu_options, "shared"),
    Check("options", check_options, "shared"),
    Check("video_8k", check_video_8k, "shared"),
    Check("video_portrait", check_video_portrait, "shared"),
    Check("video_keep", check_video_keep, "shared"),
    Check("narrow_layout", check_narrow_layout, "shared"),
    Check("fov_settings", check_fov_settings, "shared"),
    Check("resolution_stress", check_resolution_stress, "shared"),
    Check("placement", check_placement, "shared"),
    Check("captions", check_captions, "shared"),
    Check("bg_compare", check_bg_compare, "shared"),
    Check("background_reveal", check_background_reveal, "shared"),
    Check("background_fov", check_background_fov_independence, "shared"),
    Check("save_config", check_save_config, "shared"),
    Check("console", check_console, "shared"),
    Check("console_map", check_console_map, "shared"),
    Check("campaign", check_campaign, "shared"),
    Check("dev_overlay", check_dev_overlay, "shared"),
    Check("hud", check_hud, "shared"),
    Check("hud_aspect", check_hud_aspect, "shared"),
    Check("power_hud", check_power_hud, "shared"),
    Check("flashlight_shadows", check_flashlight_shadows, "shared"),
    Check("flashlight_availability", check_flashlight_availability, "shared"),
    Check("options_status", check_options_status, "shared"),
    Check("lamp_emission", check_lamp_emission, "shared"),
    Check("motion_blur_options", check_motion_blur_options, "shared"),
    Check("viewmodel_motion_blur", check_viewmodel_motion_blur, "shared"),
    Check("flashlight_viewmodel", check_flashlight_viewmodel, "shared"),
    Check("commentary", check_commentary, "shared"),
    Check("video", check_video, "shared"),
    Check("window", check_window, "windowed"),
    Check("window_place", check_window_place, "macos"),
    Check("no_border", check_no_border, "windows"),
    Check("coverage", check_coverage, "shared", "gameplay"),
    Check("parallel_jobs", check_parallel_jobs, "shared", "gameplay"),
    *(Check(f[len("bevy-"):], gameplay_check(f), "shared", "gameplay") for f in sorted(GAMEPLAY_MAPS)),
]


def selected() -> list[Check]:
    only = os.environ.get("ONLY", "").split()
    group = os.environ.get("GROUP", "")
    windowed = os.environ.get("WINDOWED") == "1"
    platform_category = {"Darwin": "macos", "Windows": "windows"}.get(SYSTEM)
    chosen = []
    for check in CHECKS:
        if only and check.name not in only:
            continue
        if group and check.group != group:
            continue
        if check.category in ("macos", "windows") and check.category != platform_category:
            continue
        if check.category != "shared" and not windowed and not only:
            continue
        chosen.append(check)
    return chosen


class ThreadOutput:
    """sys.stdout while checks run: each thread's prints go to its own check log."""

    def __init__(self, console):
        self.console, self.local = console, threading.local()

    def write(self, text):
        return getattr(self.local, "handle", self.console).write(text)

    def flush(self):
        getattr(self.local, "handle", self.console).flush()


def run_check(check: Check, output: ThreadOutput) -> tuple[bool, str, float]:
    begin = time.time()
    with (OUT / f"{check.name}.log").open("w", encoding="utf-8") as handle:
        output.local.handle = handle
        try:
            check.run()
            ok, why = True, ""
        except Failure as failure:
            ok, why = False, str(failure)
            print(f"FAIL: {failure}")
        except Exception as error:  # a broken report or check is a failure too
            ok, why = False, f"{type(error).__name__}: {error}"
            print(f"FAIL: {why}")
        finally:
            del output.local.handle
    seconds = time.time() - begin
    print(f"  {'ok  ' if ok else 'FAIL'}  {check.name:38} {check.category:8} {seconds:5.0f} s  {why}".rstrip())
    return ok, why, seconds


def build() -> None:
    if os.environ.get("NO_BUILD") == "1":
        return
    if SYSTEM == "Windows":
        command = ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                   str(REPO / "scripts" / "build-bevy.ps1")]
    else:
        command = ["bash", str(REPO / "scripts" / "build-bevy.sh")]
    done = subprocess.run(command, cwd=REPO, capture_output=True, text=True, errors="replace")
    (OUT / "build.log").write_text(done.stdout + done.stderr, encoding="utf-8")
    if done.returncode != 0:
        sys.exit(f"build failed: {OUT / 'build.log'}")


def stage() -> None:
    """Copy the packaged game (executable and its bevy-assets) to artifacts/replays/game."""
    if GAME.exists():
        shutil.rmtree(GAME)
    GAME.mkdir(parents=True)
    shutil.copy2(REPO / "bin" / EXE_NAME, EXE)
    shutil.copytree(REPO / "bin" / "bevy-assets", GAME / "bevy-assets")


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    build()
    stage()
    checks, started = selected(), time.time()
    output = ThreadOutput(sys.stdout)
    sys.stdout = output
    results = []
    try:
        # UI checks write settings, so they run one at a time, each from default settings.
        for check in (c for c in checks if c.group == "ui"):
            if CONFIG.exists():
                CONFIG.unlink()
            results.append(run_check(check, output))
        # Gameplay replays only read the default settings; JOBS of them run at once.
        if CONFIG.exists():
            CONFIG.unlink()
        gameplay = [c for c in checks if c.group == "gameplay"]
        requested = max(1, int(os.environ.get("JOBS", "1")))
        jobs = parallel_jobs(requested, physical_memory(),
                             int(float(os.environ.get("GAME_MEMORY_GB", "6")) * GIB))
        if jobs < requested:
            print(f"JOBS={requested} capped to {jobs}: each game takes up to "
                  f"{os.environ.get('GAME_MEMORY_GB', '6')} GB and this machine has "
                  f"{physical_memory() / GIB:.0f} GB")
        with ThreadPoolExecutor(jobs) as pool:
            results += pool.map(lambda c: run_check(c, output), gameplay)
    finally:
        sys.stdout = output.console
    failed = sum(1 for ok, _, _ in results if not ok)
    print(f"=== {len(checks)} checks, {failed} failed, {time.time() - started:.0f} s; logs in {OUT} ===")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
