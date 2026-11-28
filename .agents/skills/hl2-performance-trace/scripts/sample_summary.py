#!/usr/bin/env python3
"""Summarize a macOS `sample` call-tree text file by inclusive sample count or direct callers.

Stdlib-only; usable on any platform when analyzing saved sample outputs.
"""

import argparse
from collections import defaultdict
import json
import os
from pathlib import Path
import re
import sys
from typing import Any, Optional

_LINE_RE = re.compile(
    r"^(?P<indent>[\s+!:|]*)(?P<count>\d+) (?P<sym>.+?)(?:  \(in (?P<lib>[^)]+)\).*)?$"
)
_THREAD_RE = re.compile(r"^\s+\d+ Thread_")


def analyze_sample(
    path: Path,
    thread_filter: str = "",
    match: str = ".",
    callers_pattern: Optional[str] = None,
    top: int = 40,
) -> dict[str, Any]:
    """Parse the `Call graph:` tree in a macOS `sample` output file."""
    match_re = re.compile(match)
    callers_re = re.compile(callers_pattern) if callers_pattern else None

    inclusive: dict[str, int] = defaultdict(int)
    callers: dict[str, int] = defaultdict(int)
    in_tree = False
    active = False
    stack: list[tuple[int, str]] = []

    with Path(path).open(encoding="utf-8", errors="replace") as handle:
        for raw in handle:
            line = raw.rstrip("\n")
            if line.startswith("Call graph:"):
                in_tree = True
                continue
            if line.startswith("Total number in stack") or line.startswith("Sort by top of stack"):
                in_tree = False
            if not in_tree:
                continue
            if _THREAD_RE.match(line):
                active = thread_filter in line
                stack = []
                continue
            if not active:
                continue
            found = _LINE_RE.match(line)
            if not found:
                continue
            depth = len(found.group("indent"))
            count = int(found.group("count"))
            sym = re.sub(r"\s+\+\s+\d+.*$", "", found.group("sym").strip())
            while stack and stack[-1][0] >= depth:
                stack.pop()
            parent = stack[-1][1] if stack else None
            stack.append((depth, sym))
            if not any(existing == sym for _, existing in stack[:-1]):
                inclusive[sym] += count
            if callers_re and callers_re.search(sym) and parent:
                callers[parent] += count

    source = callers if callers_re else inclusive
    rows = []
    for sym, count in sorted(source.items(), key=lambda kv: -kv[1]):
        if not callers_re and not match_re.search(sym):
            continue
        rows.append({"count": count, "symbol": sym[:170]})
        if top > 0 and len(rows) >= top:
            break

    return {
        "mode": "callers" if callers_re else "inclusive",
        "thread_filter": thread_filter,
        "match": match,
        "callers_pattern": callers_pattern,
        "rows": rows,
        "top": top,
    }


def format_sample(data: dict[str, Any], top: Optional[int] = None) -> str:
    """Format analyzed sample rows into a plain-text summary."""
    limit = int(data.get("top", 40)) if top is None else top
    rows = data.get("rows", [])
    if limit > 0:
        rows = rows[:limit]
    lines = [f"{row['count']:8d}  {row['symbol']}" for row in rows]
    return ("\n".join(lines) + "\n") if lines else "No matching sample frames.\n"


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Rank functions or callers from macOS `sample` output.")
    parser.add_argument("--file", default=os.environ.get("SAMPLE_FILE"), help="Path to sample output (or SAMPLE_FILE).")
    parser.add_argument("--thread", default=os.environ.get("THREAD", ""), help="Substring filter for thread headers.")
    parser.add_argument("--match", default=os.environ.get("MATCH", "."), help="Regex filter for symbol names.")
    parser.add_argument("--callers", default=os.environ.get("CALLERS"), help="Regex of symbol to list direct callers.")
    parser.add_argument("--top", type=int, default=int(os.environ.get("TOP", "40")), help="Number of rows.")
    parser.add_argument("--json-out", default=os.environ.get("JSON_OUT"), help="Optional path to write JSON output.")
    return parser.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    if not args.file:
        print("sample_summary: --file (or SAMPLE_FILE env var) is required", file=sys.stderr)
        return 2
    sample_path = Path(args.file)
    if not sample_path.is_file():
        print(f"sample_summary: sample file not found: {sample_path}", file=sys.stderr)
        return 1
    data = analyze_sample(
        sample_path,
        thread_filter=args.thread,
        match=args.match,
        callers_pattern=args.callers,
        top=args.top,
    )
    sys.stdout.write(format_sample(data, top=args.top))
    if args.json_out:
        out_path = Path(args.json_out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(data, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
