"""Station 02 workload-locality benchmark with paired timing windows."""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys

from check_effect_pool_performance import REPO, require, settings, write_json
from crowd_scaling import CAPTURE

MAP = "d1_trainstation_02"
SECTORS = ("backyard", "plaza", "playground")
STOPS = (
    ("before-trashcan", None, (-3728, -545, 145), 270, 0),
    ("backyard", "backyard", (-4300, -47, 41), 90, -18),
    ("plaza-hall", "plaza", (-3390, -2043, 132), 0, 0),
    ("plaza", "plaza", (-2020, -2680, 158), 90, -14),
    ("playground-corridor", "playground", (-3740, -3455, 69), 270, 0),
    ("playground", "playground", (-4600, -3650, 115), 207, -18),
)
APPROACHES = {
    "plaza": [STOPS[2][2], (-3050, -2043, 130), (-2640, -2370, 124), STOPS[3][2]],
    "playground": [STOPS[4][2], (-3710, -3610, 82), (-3875, -3690, 86), STOPS[5][2]],
}

TEAMS = {
    "crowds": ((("npc_citizen_rebel", 16, "weapon_smg1"),),
               (("npc_combine_s", 32, "weapon_ar2"),),
               (("npc_zombie", 32, None),)),
    "fights": ((("npc_citizen_rebel", 10, "weapon_smg1"), ("npc_manhack", 6, None)),
               (("npc_combine_s", 16, "weapon_ar2"), ("npc_citizen_rebel", 16, "weapon_smg1")),
               (("npc_zombie", 16, None), ("npc_metropolice", 16, "weapon_pistol"))),
    "supplies": ((("item_healthkit", 20, None),), (("weapon_smg1", 32, None),),
                 (("item_battery", 32, None),)),
    "control": ((), (), ()),
}
STAGES = ("fixed_simulation", "sim_ai", "sim_hitboxes", "sim_physics", "animation",
          "source_pvs", "shadows", "render_assets", "render_graph", "frame_total")


def grid(start, step, count):
    x, y, z = start
    dx, dy = step
    columns, rows = count
    return [(x + c * dx, y + r * dy, z) for c in range(columns) for r in range(rows)]


def population(case):
    cells = (
        grid((-4376, -20, -57), (48, 48), (5, 6)),
        grid((-2456, -2556, 12), (64, 64), (8, 4)),
        grid((-5260, -3980, 9), (45, 50), (8, 4)),
    )
    result = {}
    for sector, teams, positions in zip(SECTORS, TEAMS[case], cells):
        roster = [(label, weapon) for label, n, weapon in teams for _ in range(n)]
        if case == "fights" and sector == "plaza":
            # Alternate columns so intervening friendly soldiers don't block sight.
            positions = [p for parity in (0, 1) for i, p in enumerate(positions) if i // 4 % 2 == parity]
        for index, ((label, weapon), xyz) in enumerate(zip(roster, positions, strict=False)):
            name = f"locality_{sector}_{index:02d}"
            result[name] = (sector, label, weapon, xyz)
        require(len(roster) <= len(positions), f"{sector} grid too small")
    return result


def command(text):
    return [{"action": "command", "command": text}]


def plaza_rivalries(roster):
    # Station 02 is precriminal. Its citizens and Combine are neutral to one another.
    # Give only the named benchmark squads explicit relationships.
    combine = [name for name, (sector, label, _, _) in roster.items()
               if sector == "plaza" and label == "npc_combine_s"]
    rebels = [name for name, (sector, label, _, _) in roster.items()
              if sector == "plaza" and label == "npc_citizen_rebel"]
    return [dict(action="send", target=name, input="SetRelationship",
                 parameter=" ".join(f"{enemy} D_HT 0" for enemy in targets))
            for subjects, targets in ((combine, rebels), (rebels, combine)) for name in subjects]


def approach(name, yaw, pitch, tick):
    steps = []
    delay = 54 if name == "plaza" else 0
    if delay:
        steps.append(dict(tick=tick, actions=[
            {"action": "send", "target": "station_entrance", "input": "Open"}]))
    tick += delay
    points = APPROACHES[name]
    for start, end in zip(points, points[1:]):
        for index in range(8):
            eye = [a + (b-a) * (index+1)/8 for a, b in zip(start, end)]
            steps.append(dict(tick=tick, eye=eye, yaw=yaw, pitch=pitch))
            tick += 4
    return steps, tick


def fixture(case):
    roster = population(case)
    steps = [{"tick": 1, "fly": True, "ui": command("sv_cheats 1; notarget")}]
    tick = 3
    for sector, eye, yaw, pitch in (
        ("backyard", STOPS[1][2], 90, -18), ("plaza", STOPS[3][2], 90, -14),
        ("playground", STOPS[5][2], 207, -18),
    ):
        steps.append(dict(tick=tick, eye=eye, yaw=yaw, pitch=pitch))
        tick += 2
        for name, (group, label, weapon, xyz) in roster.items():
            if group != sector:
                continue
            text = f"spawn_at {label} pos {' '.join(map(str, xyz))} targetname {name}"
            if weapon:
                text += f" additionalequipment {weapon}"
            steps.append(dict(tick=tick, ui=command(text)))
            tick += 3
        # Use the same warmup and move times for the control and all populations.
        tick += (32 - sum(entry[0] == sector for entry in roster.values())) * 3 + 40
    steps.append(dict(tick=tick, label="seeded"))
    tick += 2
    for name, _, eye, yaw, pitch in STOPS:
        if name in APPROACHES:
            motion, tick = approach(name, yaw, pitch, tick)
            steps.extend(motion)
        view = dict(tick=tick, eye=eye, yaw=yaw, pitch=pitch)
        if name == "before-trashcan":
            view["ui"] = command("notarget")
            if case == "fights":
                view["actions"] = plaza_rivalries(roster)
        steps.append(view)
        steps.append(dict(tick=tick + 80, label=f"{name}-start", profile_window="start"))
        steps.append(dict(tick=tick + 300, label=f"{name}-end", profile_window="end"))
        tick += 302
    return steps


def check_spawns(gameplay, roster):
    found = {}
    for spawn in gameplay["npc_spawns"]:
        request = spawn.get("request") or {}
        name = request.get("name") or spawn.get("name")
        if name not in roster:
            continue
        require(name not in found and spawn.get("entity") is not None, f"{name} spawn failed or repeated")
        sector, label, weapon, xyz = roster[name]
        observed = spawn.get("placement", {}).get("origin") if request else spawn.get("origin")
        require(observed and all(abs(a-b) < .02 for a, b in zip(observed, xyz)), f"{name} misplaced")
        expected = "npc_citizen" if label == "npc_citizen_rebel" else label
        require((request.get("classname") or spawn.get("class")) == expected, f"{name} wrong class")
        if request:
            require(request.get("equipment", "") == (weapon or ""), f"{name} wrong equipment")
            if label == "npc_citizen_rebel":
                require(request["variant"] == label, f"{name} is missing its rebel variant")
        else:
            require(spawn.get("draws", 0) > 0 and (spawn.get("mass") or 0) > 0, f"{name} missing body/draw")
        found[name] = (str(spawn["entity"]), sector, bool(request))
    require(found.keys() == roster.keys(), f"missing spawns: {sorted(roster.keys() - found.keys())}")
    require(len({entry[0] for entry in found.values()}) == len(found), "duplicate entity ids")
    return found


def activity(before, after, spawned):
    ai = after["ai"]["actors"]
    props = {str(prop["entity"]): prop for prop in after["entities"]}
    npc_sectors = {entity: sector for entity, sector, npc in spawned.values() if npc}
    sectors = {s: dict(alive=0, thinking=0, awake=0, engaging=0) for s in SECTORS}
    for entity, sector, npc in spawned.values():
        dst = sectors[sector]
        if npc and (actor := ai.get(entity)):
            dst["alive"] += 1
            dst["thinking"] += actor["last_think"] > before["time"]
            enemy = actor["npc"].get("enemy")
            dst["engaging"] += npc_sectors.get(str(enemy)) == sector
        if not npc and (prop := props.get(entity)):
            body = prop["body"]
            dst["alive"] += not prop["killed"]
            dst["awake"] += bool(body and body[2] and not body[1])
    return sectors


def summarize(report, case):
    render = report["render"]
    sim, present = render["simulation"], render["presentation"]
    require(sim["script_finished"] and not sim["dead"], "route did not finish with a live player")
    require(present["hud"]["viewport"] == [1920, 1080], "wrong viewport")
    require(present["gpu_completion"]["errors"] == 0, "GPU completion error")
    require(not sim["extension_rejected"], "debug command extension rejected")
    samples = {sample["label"]: sample for sample in sim["samples"]}
    seeded = samples["seeded"]["gameplay"]
    require(seeded["ai"]["ignore_player"], "spawned with notarget off")
    spawned = check_spawns(seeded, population(case))
    phases = {}
    for name, near, *_ in STOPS:
        before = samples[f"{name}-start"]["gameplay"]
        sample = samples[f"{name}-end"]
        after = sample["gameplay"]
        require(not after["ai"]["ignore_player"], "notarget still on during measurements")
        stages = sample["cpu_stages"]
        require(all(stages[k]["samples"] > 30 for k in STAGES), f"{name} lacks profile samples")
        phase = activity(before, after, spawned)
        phases[name] = {
            "near": near, "sectors": phase,
            "far_thinking": sum(v["thinking"] for k, v in phase.items() if k != near),
            "far_awake": sum(v["awake"] for k, v in phase.items() if k != near),
            "ai_thinks": after["ai"]["diagnostics"]["thinks"] - before["ai"]["diagnostics"]["thinks"],
            "shots": after["ai"]["diagnostics"]["shots_fired"] - before["ai"]["diagnostics"]["shots_fired"],
            "mean_ms": {stage: stages[stage]["mean_ms"] for stage in STAGES},
            "frame_p95_ms": stages["frame_total"]["p95_ms"],
            "frame_p99_ms": stages["frame_total"]["p99_ms"],
        }
    return {"case": case, "spawned": len(spawned), "phases": phases}


def run(out, case):
    out.mkdir(parents=True, exist_ok=False)
    config, script = out / "settings.json", out / "movement.json"
    write_json(config, settings((1920, 1080)))
    write_json(script, fixture(case))
    subprocess.run([sys.executable, str(CAPTURE), "headless", "--mode", "stages",
                    "--map", MAP, "--frames", "18000", "--item", "O47", "--out", str(out),
                    "--", "--uncapped", "--movement-script", str(script)],
                   env={**os.environ, "HL2RS_SETTINGS": str(config)}, check=True)
    row = summarize(json.loads((out / "report.json").read_text()), case)
    write_json(out / "summary.json", row)
    if case == "fights":
        for sector in SECTORS:
            require(any(p["sectors"][sector]["engaging"] for p in row["phases"].values()),
                    f"no combat between benchmark NPCs in {sector}")
    return row


def compare(rows):
    control = [r for r in rows if r["case"] == "control"]
    if not control:
        return
    for row in rows:
        for stop, phase in row["phases"].items():
            phase["vs_control_ms"] = {
                k: value - statistics.median(r["phases"][stop]["mean_ms"][k] for r in control)
                for k, value in phase["mean_ms"].items()
            }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--case", choices=TEAMS)
    parser.add_argument("--require-local", action="store_true",
                        help="fail if any distant test spawn is thinking or awake")
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("rounds must be positive")
    rows = []
    for round_no in range(args.rounds):
        order = [args.case] if args.case else list(TEAMS)
        if round_no % 2:
            order.reverse()
        for case in order:
            row = run(args.out.resolve() / f"round-{round_no+1}" / case, case)
            rows.append(row)
            print(case, row["spawned"], [
                (name, p["far_thinking"], p["far_awake"]) for name, p in row["phases"].items()
            ], flush=True)
    compare(rows)
    write_json(args.out.resolve() / "summary.json", rows)
    if args.require_local and any(
        p["far_thinking"] or p["far_awake"] for row in rows for p in row["phases"].values()
    ):
        sys.exit("Distant benchmark entities stayed active. See summary.json.")


if __name__ == "__main__":
    main()
