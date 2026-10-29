"""Owned GPU check: accepted cutout fragments must have opaque output alpha.

Uses an isolated muted package; diagnostic shaders and captures stay in artifacts/.
The negative control removes only the opaque-alpha assignment in the private copy.
"""
import json
import replays as replay
from images import png_rgb


PROBE = '''
#ifdef MAY_DISCARD
    if base.a < 0.999 { return vec4(1.0,0.0,1.0,1.0); }
    return vec4(0.0,1.0,0.0,1.0);
#endif
'''


def counts(path):
    _, _, rgb = png_rgb(path)
    colors = list(zip(rgb[::3], rgb[1::3], rgb[2::3]))
    return {
        "partial": sum(r > 240 and g < 15 and b > 240 for r, g, b in colors),
        "opaque": sum(r < 15 and g > 240 and b < 15 for r, g, b in colors),
    }


def run(name, aa, world=False, fence=False):
    if replay.CONFIG.exists():
        replay.CONFIG.unlink()
    replay.settings(video={"hdr": "none", "hdr_migrated": True, "advanced": {
        "motion_blur": world, "motion_blur_viewmodel": world, "motion_blur_world": world,
        "antialiasing": aa, "vsync": False}})
    fixture = replay.OUT / "cutouts.input.json"
    fixture.write_text(json.dumps([
        {"tick": 1, "fly": True,
         "eye": [-4304, -200, 48] if fence else [-3104, -2018, 128],
         "yaw": 0 if fence else -48, "pitch": -8 if fence else 0},
        {"tick": 120, "paused": True, "label": "cutouts"},
    ]), encoding="utf-8")
    result = replay.game(name, "--map", "d1_trainstation_02", "--width", "1024",
                         "--height", "768", "--movement-script", str(fixture))
    replay.holds("no GPU validation errors", "ERROR" not in result.log)
    return counts(result.capture)


def main():
    replay.OUT = replay.REPO / "artifacts/cutout-alpha"
    replay.GAME = replay.OUT / "game"
    replay.EXE = replay.GAME / replay.EXE_NAME
    replay.CONFIG = replay.GAME / "hl2-config.json"
    replay.OUT.mkdir(parents=True, exist_ok=True)
    replay.build()
    replay.stage()
    shader = replay.GAME / "bevy-assets/shaders/source.wgsl"
    original = shader.read_text(encoding="utf-8")
    hook = "    let baked = select("
    replay.holds("cutout probe hook", hook in original)
    instrumented = original.replace(hook, PROBE + hook, 1)
    try:
        shader.write_text(instrumented.replace("    base.a = 1.0;", "", 1), encoding="utf-8")
        rejected = run("negative_control", "x4")
        replay.holds("negative control detects partial cutout alpha", rejected["partial"] > 100, rejected)
        shader.write_text(instrumented, encoding="utf-8")
        results = {}
        for aa, world in [("none", False), ("x4", False), ("x4", True)]:
            name = f"cutout_{aa}_{'ultra' if world else 'off'}"
            result = run(name, aa, world)
            replay.holds("cutout GPU diagnostic executes", result["opaque"] > 100, result)
            replay.expect("accepted cutout fragments with partial alpha", result["partial"], 0)
            results[name] = result
        result = run("cutout_fence_x4", "x4", fence=True)
        replay.holds("fence cutout diagnostic executes", result["opaque"] > 100, result)
        replay.expect("fence survivors with partial alpha", result["partial"], 0)
        results["cutout_fence_x4"] = result
        (replay.OUT / "counts.json").write_text(json.dumps({"negative_control": rejected, **results}, indent=2))
    finally:
        shader.write_text(original, encoding="utf-8")
    run("normal_trees_x4", "x4")
    run("normal_fence_x4", "x4", fence=True)
    print("cutout alpha GPU checks passed")


if __name__ == "__main__":
    main()
