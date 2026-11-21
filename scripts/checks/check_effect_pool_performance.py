"""Paired packaged effects/NPC experiments; no FPS threshold or automatic optimisation claim.

Pass executables from two isolated package copies under artifacts/, including their assets.
Never point this at a user-launched package. Each launch restores its previous config.
No game is built by this script. Close competing workloads before running it.

Visual checks are separate from timing: prepend an idle tick1, shift every event in
an existing bevy-particles-{fire,smoke,ball}.json fixture by400ticks, and run each
package with --tonemap-scale1.0 --movement-script <derived.json> --capture <frame.png>.
The warm-up lets separately eased bloom converge. Verify forced exposure1,
zero histogram readbacks and equal bloom in the reports before comparing pixels.
Uncontrolled HDR screenshots can differ even between repeated baseline launches.
State digests cover inventory, AI and projectiles; other world transforms are not included.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager, nullcontext
import hashlib
import json
import math
from pathlib import Path
import platform
import statistics
import subprocess
import time

from bench_environment import Monitor, known_interference

REPO = Path(__file__).resolve().parents[2]
FIXTURES = REPO / "test-inputs" / "performance-research"
CASES = ("quiet", "frozen-fire", "animated-fire", "plaza-crowd", "plaza-crowd-retina")


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2), encoding="utf-8")


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def settings(size):
    return {"video": {"width": size[0], "height": size[1], "display_mode": "windowed",
        "fov": 75.0, "hdr": "full", "hdr_migrated": True, "gamma": 2.2,
        "advanced": {"model_detail": "high", "texture_detail": "high",
            "shader_detail": "high", "water_detail": "world", "shadow_detail": "high",
            "color_correction": True, "antialiasing": "none", "filtering": "trilinear",
            "vsync": False, "motion_blur": True, "multicore": True,
            "classic_effects": False, "d3d9ex": True}}}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def size_for(case):
    return (3456, 2168) if case.endswith("-retina") else (1920, 1080)


def validate_report(data, case):
    """Reject incomplete or cheaper workloads before admitting a timing result."""
    render = data["render"]
    sim, present = render["simulation"], render["presentation"]
    game, effects = sim["gameplay"], present["effects"]
    particles = effects["particle_emitters"]
    size = list(size_for(case))
    require(sim["script_finished"] and sim["host_tick"] == 600, "unfinished 600-tick workload")
    require(present["hud"]["viewport"] == size and sim["samples"][-1]["frame"] == size,
            "actual internal frame size differs")
    require(present["gpu_completion"]["errors"] == 0, "GPU completion errors")
    require(not data["texture_errors"] and not effects["sprite_errors"]
            and not effects["model_errors"], "effect assets failed")
    require(present["pose_or_visibility_mismatches"] == effects["pose_mismatches"] == 0,
            "pose or visibility mismatch")
    require(effects["missing_particle_draws"] == particles["capacity_rejections"] == 0,
            "particles dropped or capacity exceeded")
    shots = game["projectiles"]["diagnostics"]
    hunting = 0
    if case == "frozen-fire":
        require(sim["paused"] and effects["sprite_draws"] >= 100,
                "dense frozen sprites missing")
        require(shots["grenade_detonations"] == particles["events"] == 1,
                "expected one frozen grenade explosion")
    elif case == "animated-fire":
        require(not sim["paused"] and effects["pooled_quads"] >= 100,
                "animated effects did not run")
        require(shots["grenade_detonations"] == particles["events"] == 4,
                "expected four actual grenade explosions")
    elif case.startswith("plaza-crowd"):
        spawns = game["npc_spawns"]
        require(len(spawns) == 30 and all(s["entity"] is not None for s in spawns),
                "expected thirty successful NPC spawns")
        actors = [game["ai"]["actors"][str(s["entity"])] for s in spawns]
        hunting = sum("Manhack" in a["class"] and a["npc"]["state"] == "Combat"
                      and a["npc"]["enemy"] == 18446744073709551615 for a in actors)
        require(not sim["paused"] and game["ai"]["enabled"] and hunting == 30,
                "thirty manhacks must hunt the player")
    elif case == "quiet":
        require(not sim["paused"] and not game["npc_spawns"] and particles["events"] == 0,
                "quiet control has spawned work")
    else:
        raise ValueError("unknown performance case")
    frame = present["hud"]["developer_overlay"]["benchmark"]
    require(frame["measured_frames"] > 30 and math.isfinite(frame["mean_frame_ms"])
            and frame["mean_frame_ms"] > 0,
            "insufficient post-warm-up frames")
    return {"host_tick": sim["host_tick"], "scene_time": game["time"],
            "player": sim["player"], "hunting": hunting,
            "grenade_detonations": shots["grenade_detonations"],
            "emitted": particles["emitted"], "events": particles["events"]}


def steady_window(frame):
    """Exact weighted mean of complete windows, with approximate workload alignment."""
    windows = frame["frame_windows"]["windows"]
    end = windows[-1].get("end_unix_seconds") if windows else None
    if end is None:
        return None
    selected = [w for w in windows if w.get("start_unix_seconds") is not None
                and w["start_unix_seconds"] >= end - 6 and w["end_unix_seconds"] <= end - 1
                and w["elapsed_seconds"] >= 0.9]
    if not selected:
        return None
    elapsed, frames = sum(w["elapsed_seconds"] for w in selected), sum(w["frames"] for w in selected)
    return {"mean_frame_ms": elapsed * 1000 / frames, "frames": frames,
            "elapsed_seconds": elapsed, "window_count": len(selected),
            "start_seconds_before_final": end - selected[0]["start_unix_seconds"],
            "end_seconds_before_final": end - selected[-1]["end_unix_seconds"],
            "max_frame_ms": max(w["max_frame_ms"] for w in selected)}


def check_paths(baseline, current, output):
    baseline, current, output = (p.resolve() for p in (baseline, current, output))
    artifacts = (REPO / "artifacts").resolve()
    for exe in (baseline, current):
        require(exe.is_file(), "packaged executable does not exist")
        require(artifacts in exe.parents, "executables must be isolated copies under artifacts/")
    require(baseline.parent != current.parent, "packages must have separate config directories")
    require(output == artifacts or artifacts in output.parents, "output must be under artifacts/")


@contextmanager
def configured(executable, config):
    path = executable.with_name("hl2-config.json")
    require(not path.is_symlink(), "isolated package config must not be a symlink")
    previous = path.read_bytes() if path.exists() else None
    try:
        write_json(path, config)
        yield
    finally:
        if previous is None:
            path.unlink(missing_ok=True)
        else:
            path.write_bytes(previous)


def run_case(executable, version, case, repeat, output, telemetry):
    name = f"{case}-{version}-{repeat}"
    report, log = output / f"{name}.json", output / f"{name}.log"
    require(not report.exists() and not log.exists(), "retain prior attempt; use a fresh output directory")
    size, config = size_for(case), settings(size_for(case))
    write_json(output / f"{name}.config.json", config)
    command = [str(executable), "--background", "--no-setup", "--mute", "--uncapped", "--profile",
               "--map", "d1_trainstation_02", "--width", str(size[0]), "--height", str(size[1]),
               "--frames", "12000", "--movement-script", str(FIXTURES / f"{case}.json"),
               "--report", str(report)]
    monitor = Monitor(executable, 0.5) if telemetry else None
    started = time.monotonic()
    with configured(executable, config), monitor if monitor else nullcontext():
        if monitor:
            require(not known_interference(monitor.baseline), "competing build/game detected")
        with log.open("wb") as stream:
            done = subprocess.run(command, cwd=REPO, stdin=subprocess.DEVNULL,
                                  stdout=stream, stderr=subprocess.STDOUT, timeout=180)
    require(done.returncode == 0, f"{name} failed; inspect retained log")
    data = json.loads(report.read_text(encoding="utf-8"))
    outcome = validate_report(data, case)
    render, sim = data["render"], data["render"]["simulation"]
    present, game = render["presentation"], sim["gameplay"]
    frame = present["hud"]["developer_overlay"]["benchmark"]
    environment = monitor.finish(output / f"{name}.environment.json", frame, size) if monitor else {
        "provider": "skipped", "reason": "--telemetry not requested; missing counters do not prove idleness"}
    state = {k: game[k] for k in ("inventory", "ai", "projectiles")}
    result = {"name": case, "version": version, "round": repeat, "command": command,
        "binary_sha256": sha256(executable), "wall_seconds": time.monotonic() - started,
        "adapter": render["adapter"], "backend": render["backend"], "outcome": outcome,
        "benchmark": frame, "steady_window": steady_window(frame), "environment": environment,
        "effects": present["effects"], "cpu_stages": present["cpu_stages"],
        "actual_frame_size": sim["samples"][-1]["frame"], "candidates": present["render_candidates"],
        "visibility": present["source_visibility"], "asset_warnings": data["asset_warnings"],
        "gameplay_state_sha256": hashlib.sha256(json.dumps(state, sort_keys=True).encode()).hexdigest()}
    print(name, f"{frame['mean_frame_ms']:.3f} ms", flush=True)
    return result


def summarize(rows, rounds):
    result = {}
    for case in CASES:
        groups = {}
        for version in ("baseline", "current"):
            values = [r for r in rows if r["name"] == case and r["version"] == version]
            require(len(values) == rounds, "missing accepted timing trials")
            means = [r["benchmark"]["mean_frame_ms"] for r in values]
            steady = [r["steady_window"]["mean_frame_ms"] for r in values if r["steady_window"]]
            groups[version] = {"trials_ms": means, "median_ms": statistics.median(means),
                "median_fps": 1000 / statistics.median(means), "steady_window_trials_ms": steady,
                "steady_window_median_ms": statistics.median(steady) if steady else None,
                "p95_upper_bounds": [r["benchmark"]["p95_frame_ms_upper_bound"] for r in values],
                "p99_upper_bounds": [r["benchmark"]["p99_frame_ms_upper_bound"] for r in values]}
        pairs = [[next(r for r in rows if r["name"] == case and r["version"] == v
                       and r["round"] == i) for v in ("baseline", "current")]
                 for i in range(1, rounds + 1)]
        require(all(a["outcome"] == b["outcome"] for a, b in pairs), "paired gameplay outcomes differ")
        groups["paired_gameplay_states_identical"] = [a["gameplay_state_sha256"] ==
                                                       b["gameplay_state_sha256"] for a, b in pairs]
        require(all(groups["paired_gameplay_states_identical"]),
                "full gameplay states differ; inspect retained reports before timing claims")
        groups["median_frame_reduction_percent"] = (1 - groups["current"]["median_ms"] /
                                                    groups["baseline"]["median_ms"]) * 100
        result[case] = groups
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True, help="isolated retained executable")
    parser.add_argument("--current", type=Path, required=True, help="isolated candidate executable")
    parser.add_argument("--output", type=Path, required=True, help="fresh directory under artifacts/")
    parser.add_argument("--rounds", type=int, default=3, choices=range(1, 10))
    parser.add_argument("--telemetry", action="store_true", help="optional existing host monitor")
    args = parser.parse_args(argv)
    executables = {"baseline": args.baseline.resolve(), "current": args.current.resolve()}
    output = args.output.resolve()
    check_paths(executables["baseline"], executables["current"], output)
    require(sha256(executables["baseline"]) != sha256(executables["current"]), "identical executables")
    output.mkdir(parents=True, exist_ok=True)
    require(not any((output / name).exists() for name in ("paired-measurements.json", "experiment.json")),
            "use a fresh output directory; retain the previous experiment")
    write_json(output / "experiment.json", {"platform": platform.platform(), "rounds": args.rounds,
        "fixture_sha256": {n: sha256(FIXTURES / f"{n}.json") for n in CASES},
        "caveats": ["Headless profiled throughput is not live FPS or GPU time.",
            "120 render frames cover unequal warm-up simulation intervals.",
            "Steady windows have retained relative endpoints, not exact simulation alignment."]})
    rows = []
    for repeat in range(1, args.rounds + 1):
        cases = CASES if repeat % 2 else tuple(reversed(CASES))
        versions = ("baseline", "current") if repeat % 2 else ("current", "baseline")
        for case in cases:
            for version in versions:
                rows.append(run_case(executables[version], version, case, repeat, output, args.telemetry))
                write_json(output / "paired-measurements.json", rows)
    result = summarize(rows, args.rounds)
    write_json(output / "paired-summary.json", result)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
