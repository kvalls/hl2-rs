"""Opt-in owned GPU stress guard: native-resolution 4x MSAA, idle/turn/swing/door.

NO_BUILD=1 python3 scripts/checks/check_blur_performance.py
Defaults reproduce the reported 3456x2168 regression. BLUR_WIDTH/BLUR_HEIGHT,
BLUR_AA, BLUR_SCENES and BLUR_RUNS override the stress workload (three alternating pairs by default); not part of headless CI.
BLUR_PRIME=1 records two declared priming runs separately before the measured pairs.
Runs only an isolated muted package and records frame tails as well as means.
"""
import json
import math
import os
import statistics
import shutil
import subprocess
import replays as replay
from bench_environment import Monitor, known_interference, pair_environment
from blur_environment_report import write as write_resource_report


def fixture(scene):
    if scene == "door":
        # Standing at the station entrance while its doors swing fills the view with object motion.
        frames = [{"tick": 1, "fly": True, "eye": [-3225, -2048, 128], "yaw": 0, "actions": [
            {"action": "send", "target": "station_entrance", "input": "Unlock"},
            {"action": "send", "target": "station_entrance", "input": "Close"}]}]
        frames += [{"tick": tick, "actions": [{"action": "send", "target": "station_entrance",
                                               "input": "Open" if i % 2 == 0 else "Close"}]}
                   for i, tick in enumerate(range(100, 1100, 110))]
        frames.append({"tick": 1200, "label": "performance"})
        return frames
    frames = [{"tick": 1, "fly": True, "eye": [-3104, -2018, 128], "yaw": 12,
               "actions": [{"action": "loadout"}, {"action": "slot", "slot": 0}]},
              {"tick": 2, "actions": [{"action": "confirm"}]}]
    if scene == "turn":
        frames += [{"tick": t, "yaw": 12+90*math.sin((t-100)*0.075)} for t in range(200, 1200)]
    elif scene == "swing":
        for tick in range(100, 1150, 40):
            frames += [{"tick": tick, "primary": True}, {"tick": tick+1, "primary": False}]
    frames.append({"tick": 1200, "label": "performance"})
    return frames


def measure(scene, advanced, repeat):
    width, height = int(os.environ.get("BLUR_WIDTH", "3456")), int(os.environ.get("BLUR_HEIGHT", "2168"))
    aa = os.environ.get("BLUR_AA", "x4")
    if replay.CONFIG.exists():
        replay.CONFIG.unlink()
    replay.settings(video={"width": width, "height": height, "hdr": "none", "hdr_migrated": True,
                           "advanced": {"motion_blur": True, "motion_blur_viewmodel": advanced,
                                        "motion_blur_world": advanced and os.environ.get("BLUR_TIER", "ultra") == "ultra",
                                        "antialiasing": aa, "vsync": False}})
    name = f"{scene}-{'advanced' if advanced else 'retail'}-{repeat}"
    path = replay.OUT / (scene + ".input.json")
    path.write_text(json.dumps(fixture(scene)), encoding="utf-8")
    with Monitor(replay.EXE, float(os.environ.get("BLUR_TELEMETRY_INTERVAL", "1"))) as monitor:
        blocked = known_interference(monitor.baseline)
        (replay.OUT/(name+".preflight.json")).write_text(json.dumps({
            "hardware": monitor.metadata, "baseline": monitor.baseline,
            "baseline_history": monitor.baseline_history, "known_interference": blocked,
        }, indent=2), encoding="utf-8")
        replay.holds("no known competing workload before launch", not blocked, blocked)
        run = replay.game(name, "--map", "d1_trainstation_02", "--width", str(width), "--height", str(height),
                          "--movement-script", str(path), "--frames", "36000", "--profile", capture=False)
    replay.holds("no render errors", "ERROR" not in run.log and "Caught rendering error" not in run.log)
    hud = run.report["render"]["presentation"]["hud"]
    replay.expect("actual internal frame size", hud["viewport"], [float(width), float(height)])
    result = hud["developer_overlay"]["benchmark"]
    metrics = {k: v for k, v in result.items() if k != "frame_windows"}
    replay.holds("measured beyond warm-up", result["measured_frames"] >= 100, metrics)
    replay.holds("profiled frame tails", result.get("p95_frame_ms_upper_bound") is not None, metrics)
    windows = result.get("frame_windows", {})
    replay.holds("aligned frame windows present", bool(windows.get("windows")))
    if not windows.get("dropped_windows"):
        replay.expect("profile window frame count", sum(w["frames"] for w in windows["windows"]), result["measured_frames"])
    monitor.metadata.update(adapter=run.report["render"].get("adapter"), backend=run.report["render"].get("backend"))
    result["blur_tier"] = (os.environ.get("BLUR_TIER", "ultra") if advanced else "enabled")
    result["environment"] = monitor.finish(replay.OUT/(name+".environment.json"), result, (width, height))
    replay.expect("uncapped background profiling", run.report["render"]["presentation"].get("background_profile_interval_ms"), 0.0)
    completion = run.report["render"]["presentation"].get("gpu_completion", {})
    replay.holds("bounded GPU completion pacing", completion.get("limit") == 2 and
                 completion.get("max_pending_fences", 3) <= 2 and completion.get("errors") == 0, completion)
    replay.holds("GPU completions observed", completion.get("completed_frames_observed", 0) >= 100)
    requested_mode = os.environ.get("HL2RS_PROFILE_DIRECT_DRAW", os.environ.get("HL2RS_DIRECT_DRAW"))
    if requested_mode in ("0", "1"):
        replay.expect("camera indirect drawing disabled", completion.get("camera_indirect_drawing_disabled"), requested_mode == "1")
    else:
        replay.holds("camera draw policy reported", isinstance(completion.get("camera_indirect_drawing_disabled"), bool))
    result["gpu_completion"] = completion
    result["adapter"] = run.report["render"].get("adapter")
    result["backend"] = run.report["render"].get("backend")
    summary = {k: v for k, v in result.items() if k not in ("environment", "frame_windows")}
    summary["resources"] = result["environment"]["active_mean"]
    summary["quality_flags"] = result["environment"]["quality_flags"]
    print(name, json.dumps(summary), flush=True)
    return result


def requested_scenes(value):
    scenes = tuple(value.split())
    if not scenes or len(set(scenes)) != len(scenes) or any(s not in ("still", "turn", "swing", "door") for s in scenes):
        raise ValueError("BLUR_SCENES must select unique still/turn/swing/door scenes")
    return scenes


def main():
    label = os.environ.get("BLUR_LABEL", "guard")
    if not label or label in (".", "..") or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_" for c in label):
        raise ValueError("BLUR_LABEL must be a simple directory label")
    replay.OUT = replay.REPO / "artifacts/blur-performance-regression" / label
    replay.GAME = replay.OUT / "game"
    replay.EXE = replay.GAME / replay.EXE_NAME
    replay.CONFIG = replay.GAME / "hl2-config.json"
    replay.OUT.mkdir(parents=True, exist_ok=True)
    replay.build()
    replay.stage()
    metadata = replay.REPO / "bin/build-bevy-info.json"
    if metadata.exists():
        shutil.copy2(metadata, replay.OUT / "build-info.json")
    (replay.OUT / "source-state.json").write_text(json.dumps({
        "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=replay.REPO, text=True).strip(),
        "uncommitted_tracked_changes": bool(subprocess.check_output(["git", "diff", "HEAD", "--name-only"], cwd=replay.REPO)),
    }), encoding="utf-8")
    results, failures = {}, []
    scenes = requested_scenes(os.environ.get("BLUR_SCENES", "still turn swing"))
    if os.environ.get("BLUR_PRIME") == "1":
        # Declared before collection: preserve cold/priming runs separately, then measure.
        priming = [measure(scenes[0], mode, "prime") for mode in (False, True)]
        (replay.OUT / "priming.json").write_text(json.dumps(priming, indent=2), encoding="utf-8")
    for scene in scenes:
        modes = {False: [], True: []}
        for repeat in range(int(os.environ.get("BLUR_RUNS", "3"))):
            for advanced in ((False, True) if repeat % 2 == 0 else (True, False)):
                modes[advanced].append(measure(scene, advanced, repeat))
        median = lambda mode, key: statistics.median(r[key] for r in modes[mode])
        pairs = [pair_environment(a["environment"], b["environment"])
                 for a, b in zip(modes[False], modes[True])]
        guards = {"mean": median(True, "mean_frame_ms") <= median(False, "mean_frame_ms")*1.2+0.25,
                  "p95": median(True, "p95_frame_ms_upper_bound") <= median(False, "p95_frame_ms_upper_bound")*1.25+1}
        iteration_stats = {name: {key: {
            "median": statistics.median(values), "minimum": min(values), "maximum": max(values)}
            for key in ("mean_frame_ms", "p95_frame_ms_upper_bound", "p99_frame_ms_upper_bound")
            if (values := [r[key] for r in modes[mode]])}
            for name, mode in (("retail", False), ("advanced", True))}
        ratios = [b["mean_frame_ms"]/a["mean_frame_ms"] for a, b in zip(modes[False], modes[True])]
        results[scene] = {"retail": modes[False], "advanced": modes[True], "environment_pairs": pairs,
                          "frame_time_ratio_advanced_to_retail": median(True, "mean_frame_ms")/median(False, "mean_frame_ms"),
                          "performance_guards": guards, "iteration_statistics": iteration_stats,
                          "paired_frame_time_ratios": ratios, "median_paired_frame_time_ratio": statistics.median(ratios)}
        if not all(guards.values()) or not all(p["comparable"] for p in pairs):
            failures.append(scene)
        (replay.OUT / "summary.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
        write_resource_report(replay.OUT / "resource-report.md", results)
        print(scene, "guards", guards, "environment", pairs, flush=True)
    if failures:
        raise replay.Failure("native-resolution blur not qualified: " + ", ".join(failures))
    print("blur performance guard passed for selected scenes:", ", ".join(scenes))


if __name__ == "__main__":
    main()
