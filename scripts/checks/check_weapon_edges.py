"""Owned GPU regression for Advanced weapon coverage at clipped screen edges.

Reuses the established firing/reload fixtures and isolated package runner. A negative
control restores the old full-footprint divisor while leaving every sample unchanged.
"""
import json
import replays as replay
from check_weapon_blur import artifact_count, fixture


def frames(case):
    result = fixture("reload" if case.endswith("reload") else "smg")
    result[0]["actions"][1]["slot"] = 3 if case.startswith("shotgun") else 2
    result[0].update(eye=[-3104, -2018, 128], yaw=-48, pitch=0)
    if case.endswith("reload"):
        for frame in result:
            frame["tick"] = {100: 130, 112: 145}.get(frame["tick"], frame["tick"])
    return result


def run(case, aa, suffix):
    if replay.CONFIG.exists():
        replay.CONFIG.unlink()
    replay.settings(video={"hdr": "none", "hdr_migrated": True, "advanced": {
        "motion_blur": True, "motion_blur_viewmodel": True,
        "antialiasing": aa, "vsync": False}})
    name = f"edge_{case}_{aa}_{suffix}"
    path = replay.OUT / (name + ".input.json")
    path.write_text(json.dumps(frames(case)), encoding="utf-8")
    result = replay.game(name, "--map", "d1_trainstation_02", "--width", "1024",
                         "--height", "768", "--movement-script", str(path),
                         "--capture-live", "--capture-burst", "12")
    sample = result.sample("animation")["gameplay"]
    replay.expect("selected weapon", sample["inventory"]["active"],
                  "weapon_shotgun" if case.startswith("shotgun") else "weapon_smg1")
    expected = "reload" if case.endswith("reload") else "fire"
    replay.holds("authored animation", sample["viewmodel"]["animation"].startswith(expected))
    replay.holds("no GPU errors", "ERROR" not in result.log)
    counts = [artifact_count(p) for p in [result.capture] + [
        replay.OUT / f"{name}-{i}.png" for i in range(1, 13)]]
    return {"lost_coverage": max(x for x, _, _ in counts),
            "valid_edges": max(z for _, _, z in counts)}


def main():
    replay.OUT = replay.REPO / "artifacts/weapon-edges"
    replay.GAME = replay.OUT / "game"
    replay.EXE = replay.GAME / replay.EXE_NAME
    replay.CONFIG = replay.GAME / "hl2-config.json"
    replay.OUT.mkdir(parents=True, exist_ok=True)
    replay.build()
    replay.stage()
    shader = replay.GAME / "bevy-assets/shaders/weapon_blur.wgsl"
    original = shader.read_text(encoding="utf-8")
    hook = "    let alpha = min(coverage/normalization,1.0);"
    probe = '''
    if own.z > 0.99 && valid_taps < f32(count) && coverage > 0.5 {
        let expected = min(coverage/max(valid_taps,1.0),1.0);
        if alpha + 0.001 < expected { return vec4(1.0,0.0,1.0,1.0); }
        return vec4(0.0,1.0,0.0,1.0);
    }
'''
    replay.holds("edge coverage hook", hook in original)
    instrumented = original.replace(hook, hook + probe, 1)
    results = {}
    try:
        shader.write_text(instrumented.replace("let normalization = max(valid_taps,1.0);",
                                               "let normalization = f32(count);", 1))
        rejected = run("smg", "x4", "negative")
        replay.holds("old divisor loses real clipped-weapon coverage", rejected["lost_coverage"] > 100, rejected)
        results["negative_control"] = rejected
        shader.write_text(instrumented)
        for case in ("smg", "smg_reload", "shotgun", "shotgun_reload"):
            for aa in ("none", "x4"):
                result = run(case, aa, "fixed")
                replay.holds("real clipped-edge diagnostic executes", result["valid_edges"] > 100, result)
                replay.expect("coverage lost to offscreen samples", result["lost_coverage"], 0)
                results[f"{case}_{aa}"] = result
    finally:
        shader.write_text(original)
    (replay.OUT / "counts.json").write_text(json.dumps(results, indent=2))
    print("weapon clipped-edge checks passed")


if __name__ == "__main__":
    main()
