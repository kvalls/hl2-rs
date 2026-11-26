#!/usr/bin/env python3
"""Summarize a Bevy `trace_chrome` JSON file (one JSON event per line) by span.

Stdlib-only; usable as a library or CLI (`--trace` or `TRACE` env var).
"""

import argparse
import bisect
from collections import defaultdict
import json
import os
from pathlib import Path
import re
import sys
from typing import Any, Optional

SORT_CHOICES = ("total", "max", "calls", "per_frame", "p99")
FIXED_STEP = 'system: name="hl2_bevy::simulation::fixed_step"'
PIPELINE_QUEUE = (
    'system: name="bevy_render::render_resource::pipeline_cache::'
    'PipelineCache::process_pipeline_queue_system"'
)


def _shorten_span(name: str, full_paths: bool) -> str:
    if not full_paths:
        name = re.sub(r"\b(?:[a-z_][a-z0-9_]*::)+", "", name)
    return name[:220]


def _percentile(sorted_vals: list[float], pct: int) -> float:
    idx = min(len(sorted_vals) - 1, len(sorted_vals) * pct // 100)
    return sorted_vals[idx]


def _frame_work(
    frame: tuple[float, float],
    fixed: list[tuple[float, float]],
    pipelines: list[tuple[float, float]],
) -> dict[str, Any]:
    """Fixed invocations and wall-time overlap with the render queue in one frame."""
    start, end = frame
    fixed_here = [(lo, hi) for lo, hi in fixed if start <= lo < end]
    # Render work may start on an earlier frame or run past the end of this one.
    # The union avoids double-counting processing on multiple render threads.
    covered_until = start
    overlap = 0.0
    for lo, hi in pipelines:
        if lo >= end:
            break
        left, right = max(lo, covered_until), min(hi, end)
        if left < right:
            overlap += right - left
            covered_until = right
    return {
        "fixed_steps": len(fixed_here),
        "fixed_step_ms": sum(min(hi, end) - lo for lo, hi in fixed_here) / 1000.0,
        "pipeline_queue_overlap_ms": overlap / 1000.0,
    }



def analyze_trace(
    path: Path,
    frame_span: str = "main app",
    top: int = 50,
    sort_key: str = "total",
    match: str = ".",
    spikes: int = 10,
    full_paths: bool = False,
) -> dict[str, Any]:
    """Stream a Chrome trace JSON file and aggregate per-span and slow-frame stats."""
    if sort_key not in SORT_CHOICES:
        raise ValueError(f"unsupported sort key {sort_key!r}; expected one of {SORT_CHOICES}")

    name_re = re.compile(match)
    durations: dict[str, list[float]] = defaultdict(list)
    starts: dict[str, list[float]] = defaultdict(list)
    stacks: dict[Any, list[tuple[str, float]]] = defaultdict(list)
    frames: list[tuple[float, float]] = []
    thread_names: dict[Any, str] = {}

    with Path(path).open(encoding="utf-8", errors="replace") as handle:
        for raw in handle:
            line = raw.strip().rstrip(",")
            if not line.startswith("{"):
                continue
            try:
                ev = json.loads(line)
            except json.JSONDecodeError:
                continue
            ph = ev.get("ph")
            tid = ev.get("tid")
            if ph == "M" and ev.get("name") == "thread_name":
                args = ev.get("args") or {}
                if isinstance(args, dict):
                    thread_names[tid] = str(args.get("name", ""))
                continue
            if ph == "B":
                stacks[tid].append((str(ev.get("name", "")), float(ev["ts"])))
            elif ph == "E" and stacks[tid]:
                name, ts = stacks[tid].pop()
                end_ts = float(ev["ts"])
                dur_ms = (end_ts - ts) / 1000.0
                if name.rstrip(": ") == frame_span:
                    frames.append((ts, end_ts))
                key = _shorten_span(name, full_paths)
                durations[key].append(dur_ms)
                starts[key].append(ts)
            elif ph == "X":
                name = str(ev.get("name", ""))
                ts = float(ev["ts"])
                dur_us = float(ev.get("dur", 0.0))
                if name.rstrip(": ") == frame_span:
                    frames.append((ts, ts + dur_us))
                key = _shorten_span(name, full_paths)
                durations[key].append(dur_us / 1000.0)
                starts[key].append(ts)

    frames.sort()
    frame_starts = [start for start, _ in frames]

    def system_intervals(name: str) -> list[tuple[float, float]]:
        key = _shorten_span(name, full_paths)
        return sorted(
            (start, start + ms * 1000.0)
            for start, ms in zip(starts.get(key, []), durations.get(key, []))
        )

    fixed = system_intervals(FIXED_STEP)
    pipelines = system_intervals(PIPELINE_QUEUE)
    first_frames = [
        {"frame": idx, "ms": (frame[1] - frame[0]) / 1000.0, **_frame_work(frame, fixed, pipelines)}
        for idx, frame in enumerate(frames[:5])
    ]
    n_frames = max(len(frames), 1)

    rows: list[dict[str, Any]] = []
    for key, values in durations.items():
        if not name_re.search(key):
            continue
        ordered = sorted(values)
        total = sum(ordered)
        rows.append({
            "span": key,
            "calls": len(ordered),
            "per_frame": len(ordered) / n_frames,
            "total": total,
            "mean": total / len(ordered),
            "p99": _percentile(ordered, 99),
            "max": ordered[-1],
        })
    rows.sort(key=lambda row: -row[sort_key])

    frame_stats: Optional[dict[str, float]] = None
    if frames:
        fd = sorted((end - start) / 1000.0 for start, end in frames)
        frame_stats = {
            "mean": sum(fd) / len(fd),
            "p50": fd[len(fd) // 2],
            "p95": _percentile(fd, 95),
            "p99": _percentile(fd, 99),
            "max": fd[-1],
        }

    slow_frames: list[dict[str, Any]] = []
    if spikes > 0 and frames:
        short_frame = _shorten_span(frame_span, full_paths)
        by_frame: dict[int, list[tuple[float, str]]] = defaultdict(list)
        for key, values in durations.items():
            if key == short_frame:
                continue
            for ts, dur in zip(starts[key], values):
                if dur < 1.0:
                    continue
                idx = bisect.bisect_right(frame_starts, ts) - 1
                if idx >= 0 and ts < frames[idx][1]:
                    by_frame[idx].append((dur, key))
        slow_indices = sorted(range(len(frames)), key=lambda idx: frames[idx][0] - frames[idx][1])[:spikes]
        for idx in slow_indices:
            start_ts, end_ts = frames[idx]
            top_spans = sorted(by_frame.get(idx, []), reverse=True)[:8]
            slow_frames.append({
                "frame": idx,
                "ms": (end_ts - start_ts) / 1000.0,
                "spans": [{"ms": dur, "span": span} for dur, span in top_spans],
                **_frame_work((start_ts, end_ts), fixed, pipelines),
            })

    return {
        "frame_span": frame_span,
        "frames": len(frames),
        "spans": len(durations),
        "threads": len(thread_names),
        "frame_ms": frame_stats,
        "rows": rows,
        "slow_frames": slow_frames,
        "first_frames": first_frames,
        "top": top,
    }


def format_report(data: dict[str, Any], top: Optional[int] = None) -> str:
    """Format an analyzed trace dictionary into a readable table report."""
    limit = int(data.get("top", 50)) if top is None else top
    lines = [
        f"frames ({data['frame_span']!r}): {data['frames']}; spans: {data['spans']}; threads: {data['threads']}"
    ]
    fstats = data.get("frame_ms")
    if isinstance(fstats, dict) and fstats:
        lines.append(
            f"frame ms: mean {fstats['mean']:.2f} p50 {fstats['p50']:.2f} "
            f"p95 {fstats['p95']:.2f} p99 {fstats['p99']:.2f} max {fstats['max']:.2f}"
        )
    lines.append(f"{'calls':>9} {'/frame':>8} {'total ms':>10} {'mean':>8} {'p99':>8} {'max':>8}  span")
    for row in data.get("rows", [])[:limit]:
        lines.append(
            f"{row['calls']:9d} {row['per_frame']:8.1f} {row['total']:10.1f} "
            f"{row['mean']:8.3f} {row['p99']:8.3f} {row['max']:8.2f}  {row['span']}"
        )
    first = data.get("first_frames") or []
    if first:
        lines.append("\nfirst complete frames (pipeline queue overlap is concurrent wall time):")
        for entry in first:
            lines.append(
                f"  frame {entry['frame']}: {entry['ms']:.2f} ms, "
                f"{entry['fixed_steps']} fixed steps ({entry['fixed_step_ms']:.2f} ms), "
                f"{entry['pipeline_queue_overlap_ms']:.2f} ms pipeline queue overlap"
            )
    slow = data.get("slow_frames") or []
    if slow:
        lines.append("\nslowest frames (spans >= 1 ms that began in them):")
        for entry in slow:
            line = f"  frame {entry['frame']}: {entry['ms']:.2f} ms"
            if "fixed_steps" in entry:
                line += (
                    f", {entry['fixed_steps']} fixed steps ({entry['fixed_step_ms']:.2f} ms)"
                    f", {entry['pipeline_queue_overlap_ms']:.2f} ms pipeline queue overlap"
                )
            lines.append(line)
            for item in entry.get("spans", []):
                lines.append(f"      {item['ms']:8.2f}  {item['span']}")
    return "\n".join(lines) + "\n"


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Summarize a Bevy Chrome trace JSON file by span.")
    parser.add_argument("--trace", default=os.environ.get("TRACE"), help="Path to trace JSON (or TRACE env var).")
    parser.add_argument("--frame-span", default=os.environ.get("FRAME_SPAN", "main app"), help="Frame span name.")
    parser.add_argument("--top", type=int, default=int(os.environ.get("TOP", "50")), help="Number of rows to print.")
    parser.add_argument(
        "--sort",
        choices=SORT_CHOICES,
        default=os.environ.get("SORT", "total"),
        help="Column to sort by.",
    )
    parser.add_argument("--match", default=os.environ.get("MATCH", "."), help="Regex filter for span names.")
    parser.add_argument("--spikes", type=int, default=int(os.environ.get("SPIKES", "10")), help="Slowest frames to show.")
    parser.add_argument(
        "--full-paths",
        action="store_true",
        default=os.environ.get("FULL_PATHS") == "1",
        help="Keep full Rust module paths in span names.",
    )
    parser.add_argument("--json-out", default=os.environ.get("JSON_OUT"), help="Optional path to write JSON report.")
    return parser.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    if not args.trace:
        print("trace_summary: --trace (or TRACE env var) is required", file=sys.stderr)
        return 2
    trace_path = Path(args.trace)
    if not trace_path.is_file():
        print(f"trace_summary: trace file not found: {trace_path}", file=sys.stderr)
        return 1
    data = analyze_trace(
        trace_path,
        frame_span=args.frame_span,
        top=args.top,
        sort_key=args.sort,
        match=args.match,
        spikes=args.spikes,
        full_paths=args.full_paths,
    )
    sys.stdout.write(format_report(data, top=args.top))
    if args.json_out:
        out_path = Path(args.json_out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(data, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
