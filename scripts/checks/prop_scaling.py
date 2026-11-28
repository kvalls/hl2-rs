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
WAKE = END + 2
WAKE_END, RESETTLE_START, RESETTLE_END = WAKE + 146, WAKE + 420, WAKE + 620
WAKE_HEIGHT, WAKE_SPEED = 36., 240.
MAX_ACTIONS_PER_TICK = 64  # movement-script action limit
STAGES = ("fixed_simulation", "sim_physics", "sim_scene", "shadows", "render_assets",
          "render_graph", "frame_total")
WAKE_STAGES = ("sim_physics", "sim_query_refresh", "fixed_simulation", "frame_total")
WINDOWS = {"spawning": "placed", "resting": "finish", "awake": "awake-end",
           "resettled": "resettled-end"}


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


def prop_records(gameplay, kind, expected):
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
    return spawns, bodies


def body_counts(gameplay, kind, expected):
    spawns, bodies = prop_records(gameplay, kind, expected)
    spawn_z = {s["entity"]: s["origin"][2] for s in spawns}
    return {"awake": sum(not e["body"][1] for e in bodies),
            "fell": sum(e["origin"][2] < spawn_z[e["entity"]] - 1. for e in bodies)}


def wake_reference(report, kind, count):
    samples = report["render"]["simulation"]["samples"]
    game = next(s["gameplay"] for s in samples if s["label"] == "finish")
    spawns, bodies = prop_records(game, kind, count)
    positions = {body["entity"]: body["origin"] for body in bodies}
    # Fixture actions use numeric entity IDs. Verify these IDs and their model
    # placements against the measured run before trusting the wake results.
    return {s["entity"]: {"class": s["class"], "prop": s["prop"],
                         "origin": [*s["origin"][:2], positions[s["entity"]][2] + WAKE_HEIGHT]}
            for s in spawns}


def wake_fixture(kind, count, reference):
    steps = fixture(kind, count)
    steps[0].update(label="spawn-start", profile_window="start")
    steps[-3]["profile_window"] = "end"
    steps[-2]["profile_window"] = "start"
    steps[-1]["profile_window"] = "end"
    actions = [{"action": "prop_pose", "entity": entity, "origin": prop["origin"],
                "velocity": [0., 0., WAKE_SPEED]} for entity, prop in sorted(reference.items())]
    steps.extend([
        {"tick": WAKE - 1, "actions": actions[:MAX_ACTIONS_PER_TICK]},
        {"tick": WAKE, "actions": actions[MAX_ACTIONS_PER_TICK:],
         "label": "awake-start", "profile_window": "start"},
        {"tick": WAKE + 68, "label": "landing"},
        {"tick": WAKE_END, "label": "awake-end", "profile_window": "end"},
        {"tick": RESETTLE_START, "label": "resettled-start", "profile_window": "start"},
        {"tick": RESETTLE_END, "label": "resettled-end", "profile_window": "end"},
    ])
    return steps


def spawn_ms(spawns, key):
    if not spawns:
        return None
    if not all(key in spawn for spawn in spawns):
        return None
    values = [spawn[key] for spawn in spawns]
    return {"median": statistics.median(values), "max": max(values), "total": sum(values)}


def wake_result(samples, kind, count, reference):
    awake = samples["awake-start"]["gameplay"]
    spawns, bodies = prop_records(awake, kind, count)
    require({s["entity"] for s in spawns} == reference.keys(), "wake reference IDs changed")
    for spawn in spawns:
        target = reference[spawn["entity"]]
        require(spawn["class"] == target["class"] and spawn["prop"] == target["prop"]
                and all(abs(a - b) < 0.05 for a, b in zip(spawn["origin"][:2], target["origin"][:2])),
                f"wake reference changed for entity {spawn['entity']}")
    require(all(not body["body"][1] and body["origin"][2] >= reference[body["entity"]]["origin"][2] - 1.
                for body in bodies), "not all props woke at the requested height")

    checkpoints = {}
    for label in ("finish", "awake-start", "landing", "awake-end", "resettled-end"):
        _, bodies = prop_records(samples[label]["gameplay"], kind, count)
        contacts = (sum(body["body_contact"] for body in bodies)
                    if all("body_contact" in body for body in bodies) else None)
        checkpoints[label] = {"awake": sum(not body["body"][1] for body in bodies),
                              "contacting": contacts}
    if count:
        require(checkpoints["resettled-end"]["awake"] < count, "no woken props returned to sleep")
        landing = [checkpoints[label]["contacting"] for label in ("landing", "awake-end")]
        if all(contacts is not None for contacts in landing):
            require(any(landing), "no contacts recorded during landing")

    phase_stages = {}
    for phase, label in WINDOWS.items():
        stages = samples[label]["cpu_stages"]
        require(stages["sim_physics"]["samples"] > 30, f"insufficient {phase} physics samples")
        phase_stages[phase] = {stage: stages.get(stage) for stage in WAKE_STAGES}
    return {"checkpoints": checkpoints, "phase_stages": phase_stages,
            "load_ms": spawn_ms(spawns, "load_ms"), "install_ms": spawn_ms(spawns, "install_ms")}


def result(report, kind, count, reference=None):
    render = report["render"]
    sim, present = render["simulation"], render["presentation"]
    end = END if reference is None else RESETTLE_END
    require(sim["script_finished"] and sim["host_tick"] == end, "unfinished movement script")
    require(present["hud"]["viewport"] == [1920, 1080], "unexpected resolution")
    require(present["gpu_completion"]["errors"] == 0, "GPU completion error")
    samples = {s["label"]: s for s in sim["samples"]}
    checkpoints = {label: body_counts(samples[label]["gameplay"], kind, count)
                   for label in ("settled", "finish")}
    frame = present["hud"]["developer_overlay"]["benchmark"]
    require(frame["measured_frames"] > 120, "insufficient measured frames")
    settled = steady_window(frame)
    require(settled is not None, "insufficient settled samples")
    row = {"kind": kind, "count": count, "body_checkpoints": checkpoints,
            "settled_mean_ms": settled["mean_frame_ms"],
            "p95_ms": frame["p95_frame_ms_upper_bound"],
            "p99_ms": frame["p99_frame_ms_upper_bound"],
            "mean_ms": {name: present["cpu_stages"][name]["mean_ms"] for name in STAGES},
            "native_convex_shapes": sim["native_convex_shapes"],
            "shadow_casters": present["shadows"]["casters"]}
    if reference is not None:
        row["wake"] = wake_result(samples, kind, count, reference)
        row["settled_mean_ms"] = row["wake"]["phase_stages"]["resting"]["frame_total"]["mean_ms"]
    return row


def run_case(out, kind, count, package=REPO, reference=None):
    out.mkdir(parents=True, exist_ok=False)
    config, script = (out / name for name in ("settings.json", "movement.json"))
    write_json(config, settings((1920, 1080)))
    write_json(script, fixture(kind, count) if reference is None else wake_fixture(kind, count, reference))
    item = "O44" if reference is None else "O45"
    subprocess.run([sys.executable, str(CAPTURE), "headless", "--mode", "stages",
                    "--map", "d1_trainstation_02", "--frames", "12000", "--item", item,
                    "--repo", str(package), "--out", str(out), "--", "--uncapped",
                    "--movement-script", str(script)],
                   env={**os.environ, "HL2RS_SETTINGS": str(config)}, check=True)
    row = result(json.loads((out / "report.json").read_text()), kind, count, reference)
    write_json(out / "summary.json", row)
    print(json.dumps(row), flush=True)
    return row


def summarize_wake(runs):
    def median(values):
        return statistics.median(values) if all(value is not None for value in values) else None

    def bounds(values):
        return [min(values), max(values)] if all(value is not None for value in values) else None

    def phase_mean(run, phase, stage):
        stats = run["phase_stages"][phase][stage]
        return stats["mean_ms"] if stats is not None else None

    return {
        "checkpoint_ranges": {
            label: {field: bounds([r["checkpoints"][label][field] for r in runs])
                    for field in runs[0]["checkpoints"][label]}
            for label in runs[0]["checkpoints"]},
        "phase_mean_ms": {
            phase: {stage: median([phase_mean(r, phase, stage) for r in runs]) for stage in WAKE_STAGES}
            for phase in WINDOWS},
        "spawn_total_ms": {metric: median([r[metric]["total"] if r[metric] is not None else None
                                           for r in runs])
                           for metric in ("load_ms", "install_ms")},
    }


def summarize(rows):
    summary = []
    for kind, count in CASES:
        runs = [r for r in rows if (r["kind"], r["count"]) == (kind, count)]
        if not runs:
            continue
        row = {"kind": kind, "count": count, "runs": len(runs),
                        "mean_ms": {name: statistics.median(r["mean_ms"][name] for r in runs)
                                    for name in STAGES},
                        "settled_mean_ms": statistics.median(r["settled_mean_ms"] for r in runs),
                        "p95_ms": statistics.median(r["p95_ms"] for r in runs),
                        "p99_ms": statistics.median(r["p99_ms"] for r in runs)}
        if all("wake" in r for r in runs):
            row["wake"] = summarize_wake([r["wake"] for r in runs])
        summary.append(row)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--case", choices=[f"{kind}-{count}" for kind, count in CASES])
    parser.add_argument("--package", type=Path, default=REPO)
    parser.add_argument("--wake-reference", type=Path,
                        help="O44 capture root with round-1 reports for the same placements")
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
            reference = None
            if args.wake_reference:
                path = args.wake_reference / "round-1" / label / "report.json"
                reference = wake_reference(json.loads(path.read_text(encoding="utf-8")), kind, count)
            rows.append(run_case(root / f"round-{repeat + 1}" / label, kind, count,
                                 args.package, reference))
    write_json(root / "summary.json", summarize(rows))


if __name__ == "__main__":
    main()
