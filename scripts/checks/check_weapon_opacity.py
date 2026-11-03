"""Packaged SMG opacity regression, including a rejected-coverage negative control.

Runs from an isolated, muted package with no AA and 4x AA. Color diagnostics only
inspect fully covered gather footprints, preserving legitimate silhouette blur.
"""
import json
import replays as replay
from check_weapon_blur import artifact_count, fixture


def diagnostic(shader):
    hooks = {
        "    var valid_taps = 0.0;": "    var all_covered = own.z > 0.999;",
        "        let sample = weapon(uv);": "        all_covered = all_covered && sample.z > 0.999;",
        "    let alpha = min(coverage/normalization,1.0);": """
    if all_covered {
        if alpha < 0.999 { return vec4(1.0,0.0,1.0,1.0); }
        return vec4(0.0,1.0,0.0,1.0);
    }""",
    }
    for hook, addition in hooks.items():
        replay.holds("opacity diagnostic hook exists", hook in shader, hook)
        shader = shader.replace(hook, hook + "\n" + addition)
    return shader


def check(shader, original, aa, control):
    candidate = original
    if control:
        hook = "coverage += weight + retained;"
        replay.holds("negative control hook exists", hook in candidate)
        candidate = candidate.replace(hook, "coverage += weight;")
    shader.write_text(diagnostic(candidate), encoding="utf-8")
    replay.CONFIG.unlink(missing_ok=True)
    replay.settings(video={"hdr": "none", "hdr_migrated": True, "advanced": {
        "motion_blur": True, "motion_blur_viewmodel": True,
        "motion_blur_world": False, "antialiasing": aa, "vsync": False}})
    name = f"smg-opacity-{'control' if control else 'fixed'}-{aa}"
    path = replay.OUT / (name + ".input.json")
    path.write_text(json.dumps(fixture("smg")), encoding="utf-8")
    run = replay.game(name, "--map", "d1_trainstation_02", "--width", "1024", "--height", "768",
                      "--movement-script", str(path), "--capture-live", "--capture-burst", "8")
    state = run.sample("animation")["gameplay"]
    replay.expect("SMG selected", state["inventory"]["active"], "weapon_smg1")
    replay.holds("SMG firing", state["viewmodel"]["animation"].startswith("fire"))
    replay.holds("no rendering errors", "ERROR" not in run.log)
    paths = [run.capture] + [replay.OUT / f"{name}-{i}.png" for i in range(1, 9)]
    counts = [artifact_count(path) for path in paths]
    (replay.OUT / (name + ".counts.json")).write_text(json.dumps(counts), encoding="utf-8")
    replay.holds("opaque positive control present", max(c[2] for c in counts) > 100)
    rejected = max(c[0] for c in counts)
    if control:
        replay.holds("negative control detects lost interior opacity", rejected > 100, counts)
    else:
        replay.expect("translucent fully covered weapon pixels", rejected, 0)
    print(name, counts, flush=True)


def main():
    replay.OUT.mkdir(parents=True, exist_ok=True)
    replay.build()
    replay.stage()
    shader = replay.GAME / "bevy-assets/shaders/weapon_blur.wgsl"
    original = shader.read_text(encoding="utf-8")
    try:
        for aa in ("none", "x4"):
            for control in (True, False):
                check(shader, original, aa, control)
    finally:
        shader.write_text(original, encoding="utf-8")
    print("weapon interior opacity checks passed")


if __name__ == "__main__":
    main()
