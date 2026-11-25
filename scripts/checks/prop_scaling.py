"""Packaged plaza benchmark for loose supplies and miscellaneous physics props."""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys

from check_effect_pool_performance import REPO, require, settings, steady_window, write_json
from crowd_scaling import CAPTURE

COUNTS = (30, 60, 100)
MIX = (
    "ent_create item_battery",
    "ent_create item_healthkit",
    "ent_create item_healthvial",
    "ent_create item_ammo_ar2",
    "ent_create item_ammo_smg1",
    "ent_create weapon_ar2",
    "prop_physics_create props_junk/popcan01a",
    "prop_physics_create props_junk/watermelon01",
    "prop_physics_create props_junk/wood_crate001a",
)
CASES = [("empty", 0)] + [(mix, count) for count in COUNTS for mix in ("mixed", "battery")]
READY, END = 490, 780
STAGES = ("fixed_simulation", "sim_physics", "sim_scene", "shadows", "render_assets",
          "render_graph", "frame_total")


def fixture(kind, count):
    """Hold the placement camera path fixed, including in the zero-spawn control."""
    steps = [{"tick": 1, "fly": True, "ui": [
        {"action": "command", "command": "sv_cheats 1; notarget"}]}]
    for index in range(100):
        cell = index * 37 % 100
        x, y = -2456 + cell % 10 * 64, -2556 + cell // 10 * 64
        frame = {"tick": 3 + 3 * index, "eye": [x, y, 128], "pitch": -89, "yaw": 0}
        if index < count:
            command = MIX[index % len(MIX)] if kind == "mixed" else MIX[0]
            frame["ui"] = [{"action": "command", "command": command}]
        steps.append(frame)
    steps.extend([
        {"tick": 305, "eye": [-2020, -2680, 158], "yaw": 90, "pitch": -14, "label": "placed"},
        {"tick": READY, "label": "settled"},
        {"tick": END, "label": "finish"},
    ])
    return steps


def body_counts(gameplay, kind, expected):
    spawns = [spawn for spawn in gameplay["npc_spawns"] if "prop" in spawn]
    ids = {s.get("entity") for s in spawns}
    require(len(spawns) == len(ids) == expected and None not in ids, "incomplete/failed prop spawns")
    require(all(s["draws"] > 0 for s in spawns), "a spawned prop is missing its draw")
    expected_classes = Counter()
    for index in range(expected):
        command = MIX[index % len(MIX)] if kind == "mixed" else MIX[0]
        action, model_or_class = command.split()
        expected_classes[model_or_class if action == "ent_create" else "prop_physics"] += 1
    require(Counter(s["class"] for s in spawns) == expected_classes, "wrong prop mix")
    bodies = [e for e in gameplay["entities"] if e["entity"] in ids]
    require(len(bodies) == expected, "prop diagnostics omitted an entity")
    require(all(e["body"] and e["body"][0] > 0 and e["body"][2] and not e["killed"]
                for e in bodies), "prop is missing its movable body or was collected")
    spawn_z = {s["entity"]: s["origin"][2] for s in spawns}
    return {"awake": sum(not e["body"][1] for e in bodies),
            "fell": sum(e["origin"][2] < spawn_z[e["entity"]] - 1. for e in bodies)}


def result(report, kind, count):
    render = report["render"]
    sim, present = render["simulation"], render["presentation"]
    require(sim["script_finished"] and sim["host_tick"] == END, "unfinished movement script")
    require(present["hud"]["viewport"] == [1920, 1080], "unexpected resolution")
    require(present["gpu_completion"]["errors"] == 0, "GPU completion error")
    samples = {s["label"]: s for s in sim["samples"]}
    checkpoints = {label: body_counts(samples[label]["gameplay"], kind, count)
                   for label in ("settled", "finish")}
    frame = present["hud"]["developer_overlay"]["benchmark"]
    require(frame["measured_frames"] > 120, "insufficient measured frames")
    settled = steady_window(frame)
    require(settled is not None, "insufficient settled samples")
    return {"kind": kind, "count": count, "body_checkpoints": checkpoints,
            "settled_mean_ms": settled["mean_frame_ms"],
            "p95_ms": frame["p95_frame_ms_upper_bound"],
            "p99_ms": frame["p99_frame_ms_upper_bound"],
            "mean_ms": {name: present["cpu_stages"][name]["mean_ms"] for name in STAGES},
            "native_convex_shapes": sim["native_convex_shapes"],
            "shadow_casters": present["shadows"]["casters"]}


def run_case(out, kind, count):
    out.mkdir(parents=True, exist_ok=False)
    config, script = (out / name for name in ("settings.json", "movement.json"))
    write_json(config, settings((1920, 1080)))
    write_json(script, fixture(kind, count))
    subprocess.run([sys.executable, str(CAPTURE), "headless", "--mode", "stages",
                    "--map", "d1_trainstation_02", "--frames", "12000", "--item", "O44",
                    "--out", str(out), "--", "--uncapped", "--movement-script", str(script)],
                   env={**os.environ, "HL2RS_SETTINGS": str(config)}, check=True)
    row = result(json.loads((out / "report.json").read_text()), kind, count)
    write_json(out / "summary.json", row)
    print(json.dumps(row), flush=True)
    return row


def summarize(rows):
    summary = []
    for kind, count in CASES:
        runs = [r for r in rows if (r["kind"], r["count"]) == (kind, count)]
        if not runs:
            continue
        summary.append({"kind": kind, "count": count, "runs": len(runs),
                        "mean_ms": {name: statistics.median(r["mean_ms"][name] for r in runs)
                                    for name in STAGES},
                        "settled_mean_ms": statistics.median(r["settled_mean_ms"] for r in runs),
                        "p95_ms": statistics.median(r["p95_ms"] for r in runs),
                        "p99_ms": statistics.median(r["p99_ms"] for r in runs)})
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--case", choices=[f"{kind}-{count}" for kind, count in CASES])
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("rounds must be positive")
    root = args.out.resolve()
    rows = []
    for repeat in range(args.rounds):
        order = CASES if repeat % 2 == 0 else CASES[::-1]
        for kind, count in order:
            label = f"{kind}-{count}"
            if args.case and label != args.case:
                continue
            rows.append(run_case(root / f"round-{repeat + 1}" / label, kind, count))
    write_json(root / "summary.json", summarize(rows))


if __name__ == "__main__":
    main()
