"""Owned packaged regression: slow weapon surfaces must not borrow neighboring motion.

Run separately from the main replay suite: python3 scripts/checks/check_weapon_blur.py.
Uses the same isolated package/default settings/muted background runner and existing
viewmodel fixture as replays.py. Generated variations/captures stay under artifacts/.
"""
import json
import replays as replay
from images import pixel_changes, png_rgb


# The softened weapon blur differed by far more; animation timing moves a few hundred pixels.
MAX_ULTRA_WEAPON_DIFFERENCE = 600


def artifact_count(path):
    _, _, rgb = png_rgb(path)
    borrowed = holes = valid = 0
    for r, g, b in zip(rgb[0::3], rgb[1::3], rgb[2::3]):
        borrowed += r > 240 and g < 15 and b > 240
        holes += r < 15 and g > 240 and b > 240
        valid += r < 15 and g > 240 and b < 15
    return borrowed, holes, valid


def diagnostic(shader):
    motion = "    let pixel_motion = length(velocity * data.params.zw);"
    alpha = "    let alpha = min(coverage/normalization,1.0);"
    replay.holds("motion diagnostic hook exists", motion in shader)
    replay.holds("coverage diagnostic hook exists", alpha in shader)
    return shader.replace(motion, motion + '''
    if own.z > 0.99 && length(own.xy * data.params.zw) >= 0.5 &&
        length((velocity / data.params.y - own.xy) * data.params.zw) > 0.01 {
        return vec4(1.0,0.0,1.0,1.0);
    }''').replace(alpha, alpha + '''
    if own.z > 0.99 && alpha == 0.0 { return vec4(0.0,1.0,1.0,1.0); }
    if own.z > 0.99 && alpha > 0.0 { return vec4(0.0,1.0,0.0,1.0); }''')


def fixture(case):
    frames = json.loads((replay.REPO / "test-inputs/bevy-viewmodel-motion-blur.json").read_text())
    frames = [f for f in frames if f["tick"] < 10]
    if case in ("pistol", "reload", "smg"):
        frames[0]["actions"][1]["slot"] = 2 if case == "smg" else 1
        frames += [{"tick": 100, "primary": True}, {"tick": 101, "primary": False},
                   {"tick": 103, "label": "animation"}]
        if case == "reload":
            frames = frames[:-3] + [{"tick": 60, "primary": True}, {"tick": 61, "primary": False},
                                    {"tick": 100, "actions": [{"action": "reload"}]},
                                    {"tick": 112, "label": "animation"}]
    else:
        frames += [{"tick": 100, "actions": [{"action": "slot", "slot": 1}, {"action": "confirm"}]},
                   {"tick": 101}, {"tick": 132, "label": "animation"}]
    return frames


def check(case, aa):
    name = "weapon_surface_" + case + "_" + aa
    replay.settings(video={"hdr": "none", "hdr_migrated": True, "advanced": {
        "motion_blur": True, "motion_blur_viewmodel": True, "antialiasing": aa, "vsync": False}})
    path = replay.OUT / (name + ".input.json")
    path.write_text(json.dumps(fixture(case)), encoding="utf-8")
    run = replay.game(name, "--map", "d1_trainstation_02", "--width", "1024", "--height", "768",
                      "--movement-script", str(path), "--capture-live", "--capture-burst", "8")
    sample = run.sample("animation")["gameplay"]
    replay.expect("selected weapon", sample["inventory"]["active"],
                  "weapon_smg1" if case == "smg" else "weapon_pistol")
    animation = sample["viewmodel"]["animation"]
    if case == "reload":
        replay.holds("reload animation", animation.startswith("reload"), animation)
    elif case == "smg":
        replay.holds("firing animation", animation.startswith("fire"), animation)
    else:
        replay.expect("animation", animation, "fire" if case == "pistol" else "draw")
    replay.expect("posed bounds", run.report["render"]["presentation"]["pose_or_visibility_mismatches"], 0)
    replay.holds("no rendering errors", "Caught rendering error" not in run.log and "ERROR" not in run.log)
    paths = [run.capture] + [replay.OUT / f"{name}-{i}.png" for i in range(1, 9)]
    counts = [artifact_count(path) for path in paths]
    (replay.OUT / (name + ".artifacts.json")).write_text(json.dumps(counts), encoding="utf-8")
    replay.holds("diagnostic reconstruction actually ran", max(z for _, _, z in counts) > 100, counts)
    replay.expect("covered pixels borrowing other surfaces' motion", max(x for x, _, _ in counts), 0)
    replay.expect("covered pixels losing all gathered coverage", max(y for _, y, _ in counts), 0)


def check_ultra_matches_advanced():
    """Ultra softens the world's camera blur only; a still view swings the same weapon blur."""
    captures = {}
    for world in (False, True):
        replay.settings(video={"hdr": "none", "hdr_migrated": True, "advanced": {
            "motion_blur": True, "motion_blur_viewmodel": True, "motion_blur_world": world,
            "antialiasing": "x4", "vsync": False}})
        name = "weapon_swing_" + ("ultra" if world else "advanced")
        run = replay.game(name, "--map", "d1_trainstation_02", "--width", "1024", "--height", "768",
                          "--movement-script", str(replay.REPO / "test-inputs/bevy-viewmodel-motion-blur.json"))
        replay.holds("no render errors", "Caught rendering error" not in run.log)
        captures[world] = run.capture
    changes = pixel_changes(captures[False], captures[True], threshold=16, region=(180, 430, 1000, 768))
    replay.holds("Ultra keeps Advanced's weapon blur", changes["changed"] <= MAX_ULTRA_WEAPON_DIFFERENCE, changes)


def main():
    replay.OUT.mkdir(parents=True, exist_ok=True)
    replay.build()
    replay.stage()
    shader = replay.GAME / "bevy-assets/shaders/weapon_blur.wgsl"
    original = shader.read_text(encoding="utf-8")
    try:
        shader.write_text(diagnostic(original), encoding="utf-8")
        for case in ("pistol", "switch", "reload", "smg"):
            for aa in ("none", "x4"):
                if replay.CONFIG.exists():
                    replay.CONFIG.unlink()
                check(case, aa)
    finally:
        shader.write_text(original, encoding="utf-8")
    if replay.CONFIG.exists():
        replay.CONFIG.unlink()
    check_ultra_matches_advanced()
    print("weapon surface-motion checks passed")


if __name__ == "__main__":
    main()
