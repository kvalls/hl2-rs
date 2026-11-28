#!/usr/bin/env python3
"""Analyze exported macOS Instruments Metal System Trace XML tables.

Subcommands:
  metal_gpu.py passes --xml <gpu_intervals.xml> [--process hl2-bevy] [--ref-label auto] [--top 40] [--json-out <path>]
  metal_gpu.py pacing --xml <displayed_surfaces.xml> [--process <name or name (PID)>] [--resolution <substr>] [--refresh-hz 120] [--json-out <path>]

Stdlib-only; runs on any OS once XML tables are exported with `xcrun xctrace export`.
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
import xml.etree.ElementTree as ET


def _resolve(ids: dict[str, ET.Element], el: Optional[ET.Element]) -> Optional[ET.Element]:
    if el is None:
        return None
    ref = el.get("ref")
    return ids.get(ref, el) if ref else el


def _percentile(ordered: list[float], pct: int) -> float:
    idx = min(len(ordered) - 1, len(ordered) * pct // 100)
    return ordered[idx]


def analyze_gpu_intervals(
    xml_path: Path,
    process: str = "hl2-bevy",
    ref_label: str = "auto",
    top: int = 40,
) -> dict[str, Any]:
    """Summarize GPU time per encoder pass/channel and GPU busy union per game frame."""
    ids: dict[str, ET.Element] = {}
    rows: list[dict[str, Any]] = []

    for _event, el in ET.iterparse(Path(xml_path), events=("end",)):
        el_id = el.get("id")
        if el_id:
            ids[el_id] = el
        if el.tag != "row":
            continue
        cells = list(el)
        if len(cells) < 11:
            continue
        start, duration, channel, frame, _lat, depth, label, _state, _conn, _color, proc_el = (
            _resolve(ids, cell) for cell in cells[:11]
        )
        pname = proc_el.get("fmt", "") if proc_el is not None else ""
        if process not in pname:
            continue
        text = ""
        if label is not None:
            first = _resolve(ids, label.find("string"))
            text = first.text if first is not None and first.text else label.get("fmt", "")
        text = re.sub(r"\s+\(.*$", "", text).strip()
        try:
            assert start is not None and start.text is not None
            assert duration is not None and duration.text is not None
            rows.append({
                "start": int(start.text),
                "dur": int(duration.text) / 1e6,
                "channel": channel.get("fmt", "") if channel is not None else "",
                "frame": int(frame.text) if frame is not None and frame.tag != "sentinel" and frame.text else None,
                "depth": int(depth.text) if depth is not None and depth.text else 0,
                "label": text,
            })
        except (AssertionError, TypeError, ValueError, AttributeError):
            continue

    # Include all depths: Instruments' metal-nesting-level counts concurrent channel/process
    # overlap on the GPU timeline rather than parent/child containment.
    effective_ref = ref_label
    if effective_ref == "auto":
        effective_ref = (
            "hud_pass_2d"
            if any(row["label"].startswith("hud_pass_2d") and row["channel"] == "Fragment" for row in rows)
            else "upscaling"
        )
    boundaries = sorted(
        row["start"]
        for row in rows
        if row["label"].startswith(effective_ref) and row["channel"] == "Fragment"
    )
    n_frames = max(len(boundaries), 1)

    groups: dict[tuple[str, str], list[float]] = defaultdict(list)
    for row in rows:
        groups[(row["label"], row["channel"])].append(row["dur"])

    passes: list[dict[str, Any]] = []
    for (label, channel), durs in groups.items():
        ordered = sorted(durs)
        passes.append({
            "label": label,
            "channel": channel,
            "count": len(ordered),
            "per_frame": len(ordered) / n_frames,
            "total": sum(ordered),
            "ms_per_frame": sum(ordered) / n_frames,
            "mean": sum(ordered) / len(ordered),
            "p99": _percentile(ordered, 99),
            "max": ordered[-1],
        })
    passes.sort(key=lambda item: -item["total"])

    per_frame: dict[int, list[tuple[int, int]]] = defaultdict(list)
    for row in rows:
        if boundaries:
            idx = bisect.bisect_left(boundaries, row["start"])
            if idx < len(boundaries):
                per_frame[idx].append((row["start"], row["start"] + int(row["dur"] * 1e6)))

    busy_ms: list[float] = []
    for spans in per_frame.values():
        spans.sort()
        total_ns = 0
        cur_s: Optional[int] = None
        cur_e: Optional[int] = None
        for s_ns, e_ns in spans:
            if cur_e is None or s_ns > cur_e:
                if cur_s is not None and cur_e is not None:
                    total_ns += cur_e - cur_s
                cur_s, cur_e = s_ns, e_ns
            else:
                cur_e = max(cur_e, e_ns)
        if cur_s is not None and cur_e is not None:
            total_ns += cur_e - cur_s
        busy_ms.append(total_ns / 1e6)

    busy_stats: Optional[dict[str, Any]] = None
    if busy_ms:
        ordered_busy = sorted(busy_ms)
        busy_stats = {
            "frames": len(ordered_busy),
            "mean": sum(ordered_busy) / len(ordered_busy),
            "p50": ordered_busy[len(ordered_busy) // 2],
            "p95": _percentile(ordered_busy, 95),
            "p99": _percentile(ordered_busy, 99),
            "max": ordered_busy[-1],
            "over_8_33": sum(1 for val in ordered_busy if val > 8.33),
        }

    return {
        "process": process,
        "ref_label": effective_ref,
        "intervals": len(rows),
        "frames": len(boundaries),
        "passes": passes,
        "busy_stats": busy_stats,
        "busy_ms": busy_ms,
        "top": top,
    }


def format_gpu_intervals(data: dict[str, Any], top: Optional[int] = None) -> str:
    """Format Metal GPU interval table and busy stats."""
    limit = int(data.get("top", 40)) if top is None else top
    lines = [
        f"{data['process']}: {data['intervals']} GPU intervals over {data['frames']} game frames (by {data['ref_label']!r})",
        f"{'ms/frame':>9} {'/frame':>7} {'mean':>7} {'p99':>7} {'max':>7}  {'channel':9} label",
    ]
    for item in data.get("passes", [])[:limit]:
        lines.append(
            f"{item['ms_per_frame']:9.3f} {item['per_frame']:7.1f} {item['mean']:7.3f} "
            f"{item['p99']:7.3f} {item['max']:7.2f}  {item['channel']:9} {item['label'][:90]}"
        )
    busy = data.get("busy_stats")
    if isinstance(busy, dict):
        lines.append(
            f"\nGPU busy per frame: mean {busy['mean']:.2f} ms, p50 {busy['p50']:.2f}, "
            f"p95 {busy['p95']:.2f}, p99 {busy['p99']:.2f}, max {busy['max']:.2f}; "
            f"over 8.33 ms: {busy['over_8_33']}/{busy['frames']} frames"
        )
    return "\n".join(lines) + "\n"


class NoAttributedFrames(ValueError):
    """The export contains no displayed surfaces labeled with the selected process."""


DISPLAY_FRAME = re.compile(r"\((?P<process>[^()]+) \((?P<pid>\d+)\):Frame \d+\b")


def analyze_display_pacing(
    xml_path: Path,
    resolution: Optional[str] = None,
    refresh_hz: float = 120.0,
    process: Optional[str] = None,
) -> dict[str, Any]:
    """Summarize displayed-surface refresh counts and visible stutter from XML export."""
    ids: dict[str, ET.Element] = {}
    rows: list[tuple[int, str]] = []

    for _event, el in ET.iterparse(Path(xml_path), events=("end",)):
        el_id = el.get("id")
        if el_id:
            ids[el_id] = el
        if el.tag != "row":
            continue
        cells = [_resolve(ids, cell) for cell in el]
        if len(cells) < 14:
            continue
        duration_el, label_el, res_el = cells[1], cells[9], cells[13]
        if process:
            if label_el is None:
                continue
            owner = DISPLAY_FRAME.search(label_el.get("fmt") or label_el.text or "")
            if not owner:
                continue
            name, pid = owner.group("process", "pid")
            if process not in (name, f"{name} ({pid})"):
                continue
        try:
            assert duration_el is not None and duration_el.text is not None
            res_text = res_el.get("fmt", "") if res_el is not None else ""
            rows.append((int(duration_el.text), res_text))
        except (AssertionError, TypeError, ValueError):
            continue

    if not rows:
        if process:
            raise NoAttributedFrames(f"no displayed surfaces attributed to {process!r}")
        raise ValueError("no displayed surfaces")

    want = resolution or Counter(res for _dur, res in rows).most_common(1)[0][0]
    interval_ns = 1e9 / refresh_hz
    shown = [dur for dur, res in rows if want in res]
    if not shown:
        raise ValueError(f"no displayed surfaces matching resolution {want!r}")

    counts = Counter(max(1, round(dur / interval_ns)) for dur in shown)
    total = len(shown)
    repeats = sum(cnt * (intervals - 1) for intervals, cnt in counts.items())
    total_ns = sum(shown)
    effective_fps = (total / (total_ns / 1e9)) if total_ns > 0 else 0.0

    return {
        **({"process": process} if process else {}),
        "surface": want,
        "refresh_hz": refresh_hz,
        "total_frames": total,
        "buckets": {str(k): counts[k] for k in sorted(counts)},
        "repeated_refreshes": repeats,
        "effective_fps": round(effective_fps, 2),
    }


def format_display_pacing(data: dict[str, Any]) -> str:
    """Format displayed-surface pacing summary."""
    total = max(int(data["total_frames"]), 1)
    owner = f"{data['process']} " if data.get("process") else ""
    lines = [f"{owner}surface {data['surface']!r}: {data['total_frames']} frames shown at {data['refresh_hz']:.0f} Hz"]
    for key in sorted(data["buckets"], key=lambda k: int(k)):
        cnt = data["buckets"][key]
        lines.append(f"  {int(key)} refresh interval(s): {cnt:6d} frames ({100 * cnt / total:5.1f}%)")
    lines.append(
        f"repeated refreshes (visible stutter): {data['repeated_refreshes']}; "
        f"effective FPS {data['effective_fps']:.1f}"
    )
    return "\n".join(lines) + "\n"


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Analyze exported Metal System Trace XML tables.")
    sub = parser.add_subparsers(dest="subcommand", required=True)

    passes = sub.add_parser("passes", help="Summarize GPU intervals per render pass and frame.")
    passes.add_argument("--xml", default=os.environ.get("XML"), required="XML" not in os.environ, help="Path to gpu intervals XML.")
    passes.add_argument("--process", default=os.environ.get("PROCESS", "hl2-bevy"), help="Process name substring.")
    passes.add_argument("--ref-label", default=os.environ.get("REF_LABEL", "auto"), help="Final pass prefix marking frames ('auto' prefers hud_pass_2d).")
    passes.add_argument("--top", type=int, default=int(os.environ.get("TOP", "40")), help="Number of pass rows.")
    passes.add_argument("--json-out", default=os.environ.get("JSON_OUT"), help="Optional JSON output path.")

    pacing = sub.add_parser("pacing", help="Summarize displayed-surface intervals and stutter.")
    pacing.add_argument("--xml", default=os.environ.get("XML"), required="XML" not in os.environ, help="Path to displayed surfaces XML.")
    pacing.add_argument("--process", default=os.environ.get("PROCESS"), help="Exact process name or name followed by (PID).")
    pacing.add_argument("--resolution", default=os.environ.get("RESOLUTION"), help="Surface resolution substring.")
    pacing.add_argument("--refresh-hz", type=float, default=float(os.environ.get("REFRESH_HZ", "120")), help="Display refresh rate.")
    pacing.add_argument("--json-out", default=os.environ.get("JSON_OUT"), help="Optional JSON output path.")
    return parser.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    xml_path = Path(args.xml)
    if not xml_path.is_file():
        print(f"metal_gpu: XML file not found: {xml_path}", file=sys.stderr)
        return 1
    try:
        if args.subcommand == "passes":
            data = analyze_gpu_intervals(
                xml_path,
                process=args.process,
                ref_label=args.ref_label,
                top=args.top,
            )
            sys.stdout.write(format_gpu_intervals(data, top=args.top))
        else:
            data = analyze_display_pacing(
                xml_path,
                resolution=args.resolution,
                refresh_hz=args.refresh_hz,
                process=args.process,
            )
            sys.stdout.write(format_display_pacing(data))
    except ValueError as err:
        print(f"metal_gpu: {err}", file=sys.stderr)
        return 1

    if args.json_out:
        out_path = Path(args.json_out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(data, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
