"""Owned Advanced world/projectile GPU probes; isolated muted package, no user settings.

Run after building: NO_BUILD=1 python3 scripts/checks/check_world_blur.py.
Generated scripts/captures and shader probes stay in artifacts/advanced-world-blur.
"""
import json
import math
import replays as replay
from images import png_rgb


def configure():
    replay.OUT = replay.REPO / "artifacts/advanced-world-blur/runs"
    replay.GAME = replay.OUT / "game"
    replay.EXE = replay.GAME / replay.EXE_NAME
    replay.CONFIG = replay.GAME / "hl2-config.json"
    replay.OUT.mkdir(parents=True, exist_ok=True)
    replay.build()
    replay.stage()


def fixture(case):
    if case in ("camera", "lamp"):
        if case == "lamp":
            return [{"tick": 1, "fly": True, "eye": [607.53, -8173.45, -25.545], "yaw": -60}] + [
                {"tick": tick, "yaw": (tick-100)*2, **({"label": "flight"} if tick == 100 else {})}
                for tick in range(70, 101)]
        return [{"tick": 1, "fly": True, "eye": [-3104, -2018, 128], "yaw": -48}] + [
            {"tick": tick, "yaw": -48 + (tick-70)*2, **({"label": "flight"} if tick == 100 else {})}
            for tick in range(70, 101)]
    source = {"grenade": "frag", "rocket": "rpg", "bugbait": "bugbait"}[case]
    frames = json.loads((replay.REPO / f"test-inputs/bevy-{source}.json").read_text())
    end = {"grenade": 250, "rocket": 145, "bugbait": 170}[case]
    frames = [f for f in frames if f["tick"] <= end]
    frames.append({"tick": end+1, "label": "flight"})
    return frames


def run(case, aa="none", mode="advanced", suffix="normal", paused=False, size=(1024, 768)):
    if replay.CONFIG.exists():
        replay.CONFIG.unlink()
    replay.settings(video={"width": size[0], "height": size[1], "hdr": "none", "hdr_migrated": True, "advanced": {
        "motion_blur": mode != "off", "motion_blur_viewmodel": mode == "advanced", "motion_blur_world": mode == "advanced",
        "antialiasing": aa, "vsync": False}})
    name = f"world_{case}_{aa}_{mode}_{suffix}"
    path = replay.OUT / (name + ".input.json")
    frames = fixture(case)
    if paused:
        frames[-1]["paused"] = True
    path.write_text(json.dumps(frames), encoding="utf-8")
    result = replay.game(name, "--map", "d1_canals_01" if case == "lamp" else "d1_trainstation_02", "--width", str(size[0]), "--height", str(size[1]),
                         "--movement-script", str(path), "--capture-live", "--capture-burst", "8")
    replay.holds("no rendering errors", "ERROR" not in result.log and "Caught rendering error" not in result.log)
    replay.expect("current poses", result.report["render"]["presentation"]["pose_or_visibility_mismatches"], 0)
    replay.expect("actual render dimensions", result.report["render"]["presentation"]["hud"]["viewport"],
                  [float(size[0]), float(size[1])])
    if case not in ("camera", "lamp"):
        replay.holds("projectile still flying", len(result.sample("flight")["gameplay"]["projectiles"]["active"]) > 0)
    return result


def diagnostic(shader):
    hook = "    let object_pixels = length(dominant*data.params.zw);"
    replay.holds("world GPU probe hook", hook in shader)
    return shader.replace(hook, hook + '''
    if length(relative*data.params.zw) > 1.0 { return vec4(1.0,0.0,1.0,1.0); }
    if camera_pixels > 3.0 { return vec4(0.0,1.0,0.0,1.0); }''')


def projectile_region(result):
    simulation = result.report["render"]["simulation"]
    sample = result.sample("flight")
    active = sample.get("gameplay", {}).get("projectiles", {}).get("active", [])
    if not active:
        return None
    forward = simulation["camera_forward"]
    yaw, pitch = math.atan2(forward[1], forward[0]), -math.asin(forward[2])
    right = [math.sin(yaw), -math.cos(yaw), 0]
    up = [math.sin(pitch)*math.cos(yaw), math.sin(pitch)*math.sin(yaw), math.cos(pitch)]
    delta = [p-e for p, e in zip(active[0]["position"], simulation["eye"])]
    dot = lambda v: sum(a*b for a, b in zip(v, delta))
    focal = 384/math.tan(math.radians(sample["camera"][0])/2)
    replay.holds("projectile in front of view", dot(forward) > 0)
    return 512+focal*dot(right)/dot(forward), 384-focal*dot(up)/dot(forward)


def colors(result):
    counts = []
    region = projectile_region(result)
    for path in [result.capture] + [replay.OUT / f"{result.name}-{i}.png" for i in range(1, 9)]:
        width, _, rgb = png_rgb(path)
        # Object trails are gathered at quarter size: one diagnostic pixel upscales to about 77%
        # magenta over the scene. The off, retail and paused runs below must still count zero.
        objects = sum(r > 150 and g < 110 and b > 150 and r-g > 60 and b-g > 60 and (region is None or
                      abs(i % width-region[0]) < 96 and abs(i // width-region[1]) < 96)
                      for i, (r, g, b) in enumerate(zip(rgb[::3], rgb[1::3], rgb[2::3])))
        counts.append({"object": objects,
                       "camera": sum(r < 15 and g > 240 and b < 15 for r, g, b in zip(rgb[::3], rgb[1::3], rgb[2::3]))})
    (replay.OUT / (result.name + ".counts.json")).write_text(json.dumps(counts), encoding="utf-8")
    return {kind: max(c[kind] for c in counts) for kind in ("object", "camera")}


def main():
    configure()
    shader = replay.GAME / "bevy-assets/shaders/weapon_blur.wgsl"
    original = shader.read_text(encoding="utf-8")
    try:
        for case in ("camera", "lamp", "grenade", "rocket", "bugbait"):
            run(case)
        run("camera", aa="x4", suffix="partial_tiles", size=(1031, 773))
        shader.write_text(diagnostic(original), encoding="utf-8")
        for aa in ("none", "x4"):
            for case in ("camera", "lamp", "grenade", "rocket", "bugbait"):
                count = colors(run(case, aa, suffix="vectors"))
                kind = "camera" if case in ("camera", "lamp") else "object"
                replay.holds(case + " reaches GPU reconstruction", count[kind] > 0, count)
                if kind == "camera":
                    replay.holds("sky is not object motion", count["object"] < 2000, count)
        world = "    let reconstructed = foreground/f32(count)+backdrop*(1.0-alpha);"
        shader.write_text(original.replace(world, world +
            "\n    if !moving && alpha > 0.005 { return vec4(1.0,0.0,1.0,1.0); }\n    return base;"), encoding="utf-8")
        for case in ("grenade", "rocket", "bugbait"):
            count = colors(run(case, suffix="silhouette"))
            replay.holds(case + " expands its silhouette", count["object"] > 0, count)
        shader.write_text(diagnostic(original), encoding="utf-8")
        for mode in ("off", "retail"):
            replay.expect(mode + " omits world reconstruction", colors(run("camera", mode=mode, suffix="vectors")),
                          {"object": 0, "camera": 0})
        replay.expect("pause omits world reconstruction",
                      colors(run("camera", suffix="paused", paused=True)), {"object": 0, "camera": 0})
    finally:
        shader.write_text(original, encoding="utf-8")
    print("world motion probes passed")


if __name__ == "__main__":
    main()
