#!/usr/bin/env python3
"""Waste and culprit report for HL2-RS game/state spans inside a `trace-chrome` JSON trace.

Stdlib-only; usable as a library or CLI (`--trace` or `TRACE` env var).
"""

import argparse
import bisect
from collections import Counter, defaultdict
import json
import os
from pathlib import Path
import re
import sys
from typing import Any, Optional

OURS = ("npc", "npc_ai", "npc_route", "actor_pose", "player_move", "weapons", "viewmodel_pose")
STATES = ("state_player", "state_viewmodel", "state_npc", "state_frame")
_FIELD_RE = re.compile(r'(\w+)=("[^"]*"|\S+)')


def split_span(name: str) -> tuple[str, dict[str, Any]]:
    """Split a span name like `npc: id=12 ...` into base name and key/value fields."""
    base, _, rest = name.partition(":")
    return base.strip(), {k: v.strip('"') for k, v in _FIELD_RE.findall(rest)}


def _percentile(sorted_vals: list[float], pct: int) -> float:
    idx = min(len(sorted_vals) - 1, len(sorted_vals) * pct // 100)
    return sorted_vals[idx]


def analyze_waste(
    path: Path,
    top: int = 25,
    slow_n: int = 8,
    near: float = 2048.0,
) -> dict[str, Any]:
    """Analyze HL2-RS game spans, duplicate work, allocations, and slow frames."""
    frames: list[tuple[float, float]] = []
    spans: list[tuple[float, float, str, str, dict[str, Any]]] = []
    stacks: dict[Any, list[tuple[str, float]]] = defaultdict(list)

    with Path(path).open(encoding="utf-8", errors="replace") as handle:
        for raw in handle:
            line = raw.strip().rstrip(",")
            if not line.startswith("{"):
                continue
            try:
                ev = json.loads(line)
            except json.JSONDecodeError:
                continue
            ph, tid = ev.get("ph"), ev.get("tid")
            if ph == "B":
                stacks[tid].append((str(ev.get("name", "")), float(ev["ts"])))
            elif ph == "E" and stacks[tid]:
                name, ts = stacks[tid].pop()
                if name.rstrip(": ") == "main app":
                    frames.append((ts, float(ev["ts"])))
                    continue
                base, fields = split_span(name)
                if base in OURS or base in STATES:
                    caller, caller_ts = next(
                        ((n, t) for n, t in reversed(stacks[tid]) if n.startswith("system:")),
                        ("", 0.0),
                    )
                    fields["_caller"] = re.sub(r'^system:\s*name="|"$', "", caller)
                    fields["_caller_run"] = caller_ts
                    spans.append((ts, (float(ev["ts"]) - ts) / 1000.0, base, name, fields))

    frames.sort()
    starts = [start for start, _ in frames]
    per_frame: dict[int, list[tuple[float, float, str, str, dict[str, Any]]]] = defaultdict(list)
    for span in spans:
        idx = bisect.bisect_right(starts, span[0]) - 1
        if idx >= 0:
            per_frame[idx].append(span)
    n_frames = max(len(frames), 1)

    report: dict[str, Any] = {
        "frames": len(frames),
        "hl2rs_spans": len(spans),
        "top": top,
        "near": near,
    }

    # 0. Frame pacing (start-to-start wall time) and per-frame allocations (`state_frame`).
    intervals = sorted((b[0] - a[0]) / 1000.0 for a, b in zip(frames, frames[1:]))
    if intervals:
        over = [sum(1 for x in intervals if x > limit) for limit in (8.33, 16.67, 33.3)]
        report["pacing"] = {
            "intervals": len(intervals),
            "mean": sum(intervals) / len(intervals),
            "p50": intervals[len(intervals) // 2],
            "p95": _percentile(intervals, 95),
            "p99": _percentile(intervals, 99),
            "max": intervals[-1],
            "over_8_33": over[0],
            "over_16_67": over[1],
            "over_33_3": over[2],
        }
    allocs = sorted(int(s[4].get("allocations", 0)) for s in spans if s[2] == "state_frame")
    alloc_bytes = sorted(int(s[4].get("bytes", 0)) for s in spans if s[2] == "state_frame")
    if allocs:
        report["allocations"] = {
            "mean": sum(allocs) / len(allocs),
            "p50": allocs[len(allocs) // 2],
            "p99": allocs[min(len(allocs) - 1, len(allocs) * 99 // 100)],
            "max": allocs[-1],
            "bytes_mean": sum(alloc_bytes) / len(alloc_bytes),
            "bytes_max": alloc_bytes[-1],
        }

    # 1. Repeated identical work within one frame.
    repeats: dict[str, list[Any]] = defaultdict(lambda: [0, 0, 0.0])
    examples: dict[str, tuple[str, int]] = {}
    callers: dict[str, Counter[str]] = defaultdict(Counter)
    per_tick: Counter[str] = Counter()
    for items in per_frame.values():
        seen: dict[str, list[tuple[float, str, Any]]] = defaultdict(list)
        for _ts, dur, base, full, fields in items:
            if base in OURS and len(fields) > 2:
                seen[full].append((dur, str(fields.get("_caller", "")), fields.get("_caller_run")))
        hit: set[str] = set()
        for full, calls in seen.items():
            if len(calls) <= 1:
                continue
            systems = {c for _, c, _ in calls}
            runs = {r for _, _, r in calls}
            # One call per run of the same system (separate fixed ticks in one frame) is not a duplicate.
            if len(systems) == 1 and len(runs) == len(calls):
                per_tick[split_span(full)[0]] += len(calls) - 1
                continue
            base = split_span(full)[0]
            durs = [d for d, _, _ in calls]
            repeats[base][1] += len(durs) - 1
            repeats[base][2] += sum(sorted(durs)[:-1])
            hit.add(base)
            examples.setdefault(base, (full, len(durs)))
            combo = " + ".join(sorted((c.rsplit("::", 1)[-1] or "?") for _, c, _ in calls))
            callers[base][combo] += 1
        for base in hit:
            repeats[base][0] += 1
    report["per_tick_repeats"] = dict(per_tick)
    repeat_rows = sorted(repeats.items(), key=lambda kv: -kv[1][2])
    report["repeats"] = {
        base: {
            "frames": vals[0],
            "extra_calls": vals[1],
            "extra_ms": vals[2],
            "example": list(examples[base]),
            "callers": callers[base].most_common(4),
        }
        for base, vals in repeat_rows
    }

    # 2. NPC cost by class, schedule, and task (`state_npc` carries state forward).
    slow = sorted(range(len(frames)), key=lambda i: frames[i][0] - frames[i][1])[:slow_n]
    slow_set = set(slow)
    npcs_at: dict[int, list[dict[str, Any]]] = {}
    by: dict[str, dict[str, list[float]]] = {
        "class": defaultdict(list),
        "schedule": defaultdict(list),
        "task": defaultdict(list),
    }
    info: dict[Any, dict[str, Any]] = {}
    for idx in sorted(per_frame):
        items = per_frame[idx]
        for s in items:
            if s[2] == "state_npc":
                info[s[4].get("id")] = s[4]
        if idx in slow_set:
            npcs_at[idx] = list(info.values())
        for _ts, dur, base, _full, fields in items:
            if base == "npc":
                st = info.get(fields.get("id"), {})
                for key in by:
                    by[key][str(st.get(key, "?"))].append(dur)
    report["npc_cost"] = {}
    for key, groups in by.items():
        grouped = sorted(groups.items(), key=lambda kv: -sum(kv[1]))
        report["npc_cost"][key] = {
            name: {
                "calls": len(durs),
                "ms_per_frame": sum(durs) / n_frames,
                "mean": sum(durs) / len(durs),
                "max": max(durs),
            }
            for name, durs in grouped
        }

    # 3. HL2-RS spans per frame.
    totals: dict[str, list[float]] = defaultdict(list)
    for span in spans:
        if span[2] in OURS:
            totals[span[2]].append(span[1])
    report["spans"] = {}
    for base, durs in sorted(totals.items(), key=lambda kv: -sum(kv[1])):
        ordered = sorted(durs)
        report["spans"][base] = {
            "per_frame": len(ordered) / n_frames,
            "ms_per_frame": sum(ordered) / n_frames,
            "p99": _percentile(ordered, 99),
            "max": ordered[-1],
        }

    # 4. Slowest frames with state and nearby NPCs.
    report["slow_frames"] = []
    for idx in slow:
        start_ts, end_ts = frames[idx]
        items = per_frame.get(idx, [])
        states = [s[3] for s in items if s[2] in STATES and s[2] != "state_npc"]
        work = sorted((s for s in items if s[2] in OURS), key=lambda s: -s[1])[:6]
        nearby_summary: list[tuple[str, int]] = []
        nearby_count = 0
        for npc in npcs_at.get(idx, []):
            try:
                dist = float(npc.get("distance", "1e9"))
            except (TypeError, ValueError):
                dist = 1e9
            if dist <= near:
                nearby_count += 1
                nearby_summary.append((f"{npc.get('class')}/{npc.get('schedule')}", 1))
        counts = Counter()
        for label, count in nearby_summary:
            counts[label] += count
        report["slow_frames"].append({
            "frame": idx,
            "ms": (end_ts - start_ts) / 1000.0,
            "states": states,
            "nearby_npcs": {"count": nearby_count, "summary": counts.most_common(6)},
            "work": [{"ms": s[1], "span": s[3]} for s in work],
        })

    return report


def format_report(report: dict[str, Any], top: Optional[int] = None) -> str:
    """Format a waste/culprit report as plain text."""
    limit = int(report.get("top", 25)) if top is None else top
    lines = [f"frames: {report['frames']}; HL2RS spans: {report['hl2rs_spans']}"]

    pacing = report.get("pacing")
    if isinstance(pacing, dict):
        lines.append(
            f"\n0. Frame pacing: mean {pacing['mean']:.2f} ms, p50 {pacing['p50']:.2f}, "
            f"p95 {pacing['p95']:.2f}, p99 {pacing['p99']:.2f}, max {pacing['max']:.2f}"
        )
        lines.append(
            f"   over 8.33 ms (missed 120 Hz): {pacing['over_8_33']}/{pacing['intervals']}; "
            f"over 16.67 ms: {pacing['over_16_67']}; over 33.3 ms: {pacing['over_33_3']}"
        )

    alloc = report.get("allocations")
    if isinstance(alloc, dict):
        lines.append(
            f"   heap allocations per frame: mean {alloc['mean']:.0f}, p50 {alloc['p50']}, "
            f"p99 {alloc['p99']}, max {alloc['max']}; bytes per frame: "
            f"mean {alloc['bytes_mean'] / 1024:.0f} KB, max {alloc['bytes_max'] / 1048576:.1f} MB"
        )

    lines.append("\n1. Repeated identical work in one frame")
    lines.append(f"{'span':16} {'frames':>7} {'extra calls':>12} {'extra ms':>10}  example")
    repeats = report.get("repeats") or {}
    for base, entry in list(repeats.items())[:limit]:
        ex, count = entry["example"]
        lines.append(
            f"{base:16} {entry['frames']:7d} {entry['extra_calls']:12d} {entry['extra_ms']:10.1f}  "
            f"{ex[:90]} x{count}"
        )
        for combo, num in entry.get("callers", []):
            lines.append(f"{'':16}   {num:7d} groups by: {combo[:110]}")
    per_tick = report.get("per_tick_repeats") or {}
    if per_tick:
        ordered = sorted(per_tick.items(), key=lambda kv: -kv[1])
        lines.append(
            "   not duplicates, one per run of the same system (several fixed ticks in a frame): "
            + ", ".join(f"{base} {num}" for base, num in ordered)
        )

    npc_cost = report.get("npc_cost") or {}
    for key in ("class", "schedule", "task"):
        groups = npc_cost.get(key) or {}
        lines.append(f"\n2. NPC tick cost by {key}")
        lines.append(f"{key:32} {'calls':>8} {'ms/frame':>9} {'mean':>7} {'max':>7}")
        for name, row in list(groups.items())[:limit]:
            lines.append(
                f"{name[:32]:32} {row['calls']:8d} {row['ms_per_frame']:9.3f} "
                f"{row['mean']:7.3f} {row['max']:7.2f}"
            )

    lines.append("\n3. Player, viewmodel and NPC spans")
    lines.append(f"{'span':16} {'/frame':>7} {'ms/frame':>9} {'p99':>7} {'max':>7}")
    for base, row in (report.get("spans") or {}).items():
        lines.append(
            f"{base:16} {row['per_frame']:7.1f} {row['ms_per_frame']:9.3f} "
            f"{row['p99']:7.3f} {row['max']:7.2f}"
        )

    lines.append("\n4. Slowest frames and what was happening")
    for frame in report.get("slow_frames") or []:
        lines.append(f"  frame {frame['frame']}: {frame['ms']:.2f} ms")
        for st in frame.get("states", []):
            lines.append(f"      {st[:150]}")
        nearby = frame.get("nearby_npcs") or {}
        if nearby.get("count"):
            summary = ", ".join(f"{k} x{v}" for k, v in nearby.get("summary", []))
            lines.append(f"      nearby NPCs: {nearby['count']}  {summary}")
        for item in frame.get("work", []):
            lines.append(f"      {item['ms']:7.2f} ms  {item['span'][:110]}")

    return "\n".join(lines) + "\n"


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Analyze HL2-RS game-span waste and slow frames.")
    parser.add_argument("--trace", default=os.environ.get("TRACE"), help="Path to trace JSON (or TRACE env var).")
    parser.add_argument("--top", type=int, default=int(os.environ.get("TOP", "25")), help="Rows per section.")
    parser.add_argument("--slow", type=int, default=int(os.environ.get("SLOW", "8")), help="Slowest frames to list.")
    parser.add_argument(
        "--near",
        type=float,
        default=float(os.environ.get("NEAR", "2048")),
        help="Distance threshold for nearby NPC summary.",
    )
    parser.add_argument("--json-out", default=os.environ.get("JSON_OUT"), help="Optional path to write JSON report.")
    return parser.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    if not args.trace:
        print("trace_waste: --trace (or TRACE env var) is required", file=sys.stderr)
        return 2
    trace_path = Path(args.trace)
    if not trace_path.is_file():
        print(f"trace_waste: trace file not found: {trace_path}", file=sys.stderr)
        return 1
    report = analyze_waste(trace_path, top=args.top, slow_n=args.slow, near=args.near)
    sys.stdout.write(format_report(report, top=args.top))
    if args.json_out:
        out_path = Path(args.json_out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
