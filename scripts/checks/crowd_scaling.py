"""O32 packaged plaza sweep. Run from any directory with a built release package."""
import argparse
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import time

from check_effect_pool_performance import REPO, require, settings, write_json

COUNTS = (0, 10, 30, 60, 100)
END = 1200
CAPTURE = REPO / ".agents/skills/hl2-performance-trace/scripts/capture.py"


def fixture(count):
    # A coprime stride distributes every prefix around the plaza.
    steps = [{"tick": 1, "fly": True, "ui": [{"action": "command", "command": "sv_cheats 1"}]}]
    for index in range(100):
        cell = index * 37 % 100
        x = -2456 + cell % 10 * 64
        y = -2556 + cell // 10 * 64
        step = {"tick": 3 + 2 * index, "eye": [x, y, 128], "pitch": -89, "yaw": 0}
        if index < count:
            step["ui"] = [{"action": "command", "command": f"ent_create npc_manhack targetname scale_{index}"}]
        steps.append(step)
    steps.extend([{"tick": 205, "fly": True, "eye": [-2020, -2680, 158], "yaw": 90,
                   "pitch": -14, "label": "ready"}, {"tick": END, "label": "end"}])
    return steps


def result(report, count):
    render = report["render"]
    sim, present = render["simulation"], render["presentation"]
    game = sim["gameplay"]
    ids = {s["entity"] for s in game["npc_spawns"]}
    require(len(game["npc_spawns"]) == len(ids) == count and None not in ids,
            f"expected {count} successful placements")
    require(sim["script_finished"] and sim["host_tick"] == END, "unfinished sweep")
    require(present["hud"]["viewport"] == [1920, 1080], "wrong resolution")
    require(present["gpu_completion"]["errors"] == 0, "GPU completion error")
    frame = present["hud"]["developer_overlay"]["benchmark"]
    require(frame["measured_frames"] > 30, "insufficient measured frames")
    require(present["pose_or_visibility_mismatches"] == 0, "stale actor bounds")
    require(present["background_profile_interval_ms"] == 0, "background pacing active")
    stages = present["cpu_stages"]
    return {"spawned": count, "ai_thinks": game["ai"]["diagnostics"]["thinks"],
            "frames": frame["measured_frames"], "p95_bound_ms": frame["p95_frame_ms_upper_bound"],
            "p99_bound_ms": frame["p99_frame_ms_upper_bound"],
            "mean_ms": {k: stages[k]["mean_ms"] for k in ("fixed_simulation", "sim_ai",
                        "sim_hitboxes", "animation", "render_assets", "render_graph", "frame_total")}}


def sight_share(sample):
    """Count inclusive fixed-step and sight stacks once across wrappers and inlining."""
    graph = sample.split("Call graph:", 1)[1].split("Total number in stack", 1)[0]
    fixed_depth = sight_depth = None
    fixed = sight = 0
    for line in graph.splitlines():
        node = re.match(r"^([ +!:|]*)([0-9]+) (.*)$", line)
        if not node:
            continue
        depth, count, symbol = len(node[1]), int(node[2]), node[3]
        if fixed_depth is not None and depth <= fixed_depth:
            fixed_depth = None
        if sight_depth is not None and depth <= sight_depth:
            sight_depth = None
        if fixed_depth is None and "fixed_step" in symbol:
            fixed_depth = depth
            fixed += count
        if fixed_depth is not None and sight_depth is None and "PhysicsSight" in symbol:
            if "visible" in symbol or "first_solid" in symbol:
                sight_depth = depth
                sight += count
    require(fixed > 0, "no fixed-step CPU samples")
    return {"fixed_samples": fixed, "sight_samples": sight}


def record_mac(capture, dest):
    """Attach only to the packaged game started by this capture process."""
    pid = None
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline and capture.poll() is None:
        children = subprocess.run(["pgrep", "-P", str(capture.pid), "-x", "hl2-bevy"],
                                  capture_output=True, text=True).stdout.split()
        if len(children) == 1:
            pid = children[0]
            break
        time.sleep(0.1)
    require(pid is not None, "packaged capture child unavailable")
    log = dest / "run.log"
    while time.monotonic() < deadline and capture.poll() is None:
        if log.exists() and "profiling: gameplay started" in log.read_text(errors="replace"):
            break
        time.sleep(0.1)
    else:
        raise RuntimeError("gameplay did not start before the sampling deadline")
    time.sleep(4)  # At normal game speed 205 fixed ticks take 3.075 seconds.
    with subprocess.Popen(["sample", pid, "4", "2", "-file", str(dest / "sample.txt")]) as sample:
        subprocess.run([sys.executable, str(CAPTURE), "metal-record", "--out", str(dest),
                        "--pid", pid, "--seconds", "4", "--export", "--delete-raw"], check=True)
        require(sample.wait() == 0, "stack sampler failed")
    return trace_result(dest)


def trace_result(dest):
    # Multiple cameras upscale. The single HUD pass delimits an actual game frame.
    summary = dest / "gpu_by_hud_frame.json"
    subprocess.run([sys.executable, str(CAPTURE.with_name("metal_gpu.py")), "passes",
                    "--xml", str(dest / "gpu_intervals.xml"), "--ref-label", "hud_pass_2d",
                    "--json-out", str(summary)], check=True, stdout=subprocess.DEVNULL)
    gpu = json.loads(summary.read_text())
    require(gpu["busy_stats"] and gpu["busy_stats"]["frames"] > 0, "no Metal frames")
    return {"gpu_busy_ms": gpu["busy_stats"],
            **sight_share((dest / "sample.txt").read_text(errors="replace"))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--counts", type=int, choices=COUNTS, nargs="+", default=COUNTS)
    parser.add_argument("--mac-trace", action="store_true",
                        help="separate instrumented run for Metal busy time and sight-ray stacks")
    args = parser.parse_args()
    if args.mac_trace and sys.platform != "darwin":
        parser.error("--mac-trace requires macOS with Xcode")
    root = args.out.resolve()
    if len(set(args.counts)) != len(args.counts):
        parser.error("each population needs its own output directory; specify each count once")
    for count in args.counts:
        dest = root / str(count)
        dest.mkdir(parents=True, exist_ok=False)
        config, script = dest / "settings.json", dest / "movement.json"
        write_json(config, settings((1920, 1080)))
        write_json(script, fixture(count))
        env = {**os.environ, "HL2RS_SETTINGS": str(config)}
        if args.mac_trace:
            env["RUST_LOG"] = "warn,hl2_bevy::performance::status=info"
        cmd = [sys.executable, str(CAPTURE), "headless", "--mode", "stages",
               "--map", "d1_trainstation_02", "--frames", "12000", "--item", "O32",
               "--out", str(dest), "--", "--uncapped", "--movement-script", str(script)]
        with subprocess.Popen(cmd, env=env) as capture:
            traced = record_mac(capture, dest) if args.mac_trace else {}
            require(capture.wait() == 0, f"packaged capture failed (see {dest / 'run.log'})")
        row = result(json.loads((dest / "report.json").read_text()), count)
        row.update(traced)
        write_json(dest / "summary.json", row)
        print(json.dumps(row), flush=True)


if __name__ == "__main__":
    main()
