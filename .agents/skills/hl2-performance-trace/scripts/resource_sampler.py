#!/usr/bin/env python3
"""Cross-platform external process/system/GPU sampler and JSON-lines summarizer.

Supports:
  resource_sampler.py record --out <path> [--pid <pid>] [--interval 1.0] [--once] [--wait 120]
  resource_sampler.py summary --in <path> [--json-out <path>]

Also accepts OUT / PID / INTERVAL / ONCE=1 env vars when invoked without a subcommand.
Stdlib-only.
"""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
from typing import Any, Optional


def _run_cmd(*cmd: str, timeout: float = 5.0) -> str:
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, check=False)
        return proc.stdout
    except (OSError, subprocess.SubprocessError):
        return ""


def find_game_pid(wait_seconds: float = 120.0, process_name: str = "hl2-bevy") -> Optional[int]:
    """Locate the newest `hl2-bevy` PID, waiting up to `wait_seconds`."""
    deadline = time.time() + max(0.0, wait_seconds)
    while True:
        pids: list[int] = []
        if os.name == "nt":
            ps = shutil.which("powershell") or shutil.which("pwsh")
            if ps:
                out = _run_cmd(
                    ps,
                    "-NoProfile",
                    "-Command",
                    f"@(Get-Process -Name '{process_name}' -ErrorAction SilentlyContinue | Sort-Object StartTime | Select-Object -Last 1 -ExpandProperty Id) -join ''",
                ).strip()
                if out.isdigit():
                    pids = [int(out)]
        else:
            out = _run_cmd("pgrep", "-n", process_name)
            pids = [int(tok) for tok in out.split() if tok.isdigit()]
        if pids:
            return pids[-1]
        if time.time() >= deadline:
            return None
        time.sleep(0.5)


def parse_macos_ps_threads(text: str) -> list[float]:
    """Parse `ps -M -p <pid>` output into sorted descending thread CPU percentages."""
    threads = [
        float(found.group(1))
        for line in text.splitlines()[1:]
        if (found := re.search(r"\s(\d+(?:\.\d+)?)\s+[A-Z]", line))
    ]
    threads.sort(reverse=True)
    return threads[:12]


def parse_ioreg_gpu(text: str) -> Optional[dict[str, Optional[float]]]:
    """Parse macOS `ioreg -r -d 1 -c IOAccelerator` PerformanceStatistics."""
    found = re.search(r'"PerformanceStatistics"\s*=\s*\{([^}]*)\}', text)
    if not found:
        return None
    stats = dict(re.findall(r'"([^"]+)"=(\d+)', found.group(1)))

    def num(key: str, scale: float = 1.0) -> Optional[float]:
        return round(int(stats[key]) / scale, 1) if key in stats else None

    return {
        "device_pct": num("Device Utilization %"),
        "renderer_pct": num("Renderer Utilization %"),
        "tiler_pct": num("Tiler Utilization %"),
        "in_use_mb": num("In use system memory", float(1 << 20)),
        "alloc_mb": num("Alloc system memory", float(1 << 20)),
    }


def parse_macos_memory(swap_text: str, vm_stat_text: str) -> tuple[Optional[float], float]:
    """Parse `sysctl -n vm.swapusage` and `vm_stat` into `(swap_used_mb, free_mb)`."""
    swap_match = re.search(r"used\s*=\s*([0-9.]+)M", swap_text)
    page_match = re.search(r"page size of (\d+)", vm_stat_text)
    page_size = int(page_match.group(1)) if page_match else 16384
    free_pages = sum(
        int(val) for val in re.findall(r"Pages (?:free|speculative):\s+(\d+)", vm_stat_text)
    )
    swap_mb = float(swap_match.group(1)) if swap_match else None
    free_mb = round(free_pages * page_size / float(1 << 20), 1)
    return swap_mb, free_mb


def parse_macos_thermal(text: str) -> Optional[int]:
    """Parse `pmset -g therm` output for `CPU_Speed_Limit` percentage."""
    found = re.search(r"CPU_Speed_Limit\s*=\s*(\d+)", text)
    return int(found.group(1)) if found else None


def _collect_macos_process(pid: int) -> Optional[tuple[float, float, list[float]]]:
    whole = _run_cmd("ps", "-o", "%cpu=,rss=", "-p", str(pid)).split()
    if len(whole) < 2:
        return None
    try:
        cpu = float(whole[0])
        rss = float(whole[1]) / 1024.0
    except ValueError:
        return None
    threads = parse_macos_ps_threads(_run_cmd("ps", "-M", "-p", str(pid)))
    return cpu, rss, threads


def _collect_linux_process(
    pid: int,
    prev_state: dict[str, tuple[float, int]],
) -> Optional[tuple[Optional[float], float, list[float]]]:
    stat_path = Path(f"/proc/{pid}/stat")
    status_path = Path(f"/proc/{pid}/status")
    if not stat_path.is_file() or not status_path.is_file():
        return None
    try:
        stat_text = stat_path.read_text(encoding="utf-8", errors="replace")
        rparen = stat_text.rfind(")")
        if rparen < 0:
            return None
        fields = stat_text[rparen + 2 :].split()
        total_ticks = int(fields[11]) + int(fields[12])
        now = time.monotonic()
        cpu_pct: Optional[float] = None
        if str(pid) in prev_state:
            prev_t, prev_ticks = prev_state[str(pid)]
            dt = now - prev_t
            clk_tck = os.sysconf("SC_CLK_TCK") if hasattr(os, "sysconf") else 100
            if dt > 0.0 and clk_tck > 0:
                cpu_pct = round(100.0 * (total_ticks - prev_ticks) / (clk_tck * dt), 1)
        prev_state[str(pid)] = (now, total_ticks)

        status_text = status_path.read_text(encoding="utf-8", errors="replace")
        rss_match = re.search(r"^VmRSS:\s+(\d+)\s+kB", status_text, re.MULTILINE)
        rss_mb = float(rss_match.group(1)) / 1024.0 if rss_match else 0.0
        return cpu_pct, rss_mb, []
    except (OSError, ValueError, IndexError):
        return None


def _collect_linux_memory() -> tuple[Optional[float], Optional[float]]:
    meminfo = Path("/proc/meminfo")
    if not meminfo.is_file():
        return None, None
    try:
        text = meminfo.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None, None
    vals = {
        m.group(1): int(m.group(2))
        for m in re.finditer(r"^(\w+):\s+(\d+)\s+kB", text, re.MULTILINE)
    }
    swap_total = vals.get("SwapTotal")
    swap_free = vals.get("SwapFree")
    swap_used = (
        round((swap_total - swap_free) / 1024.0, 1)
        if swap_total is not None and swap_free is not None
        else None
    )
    avail = vals.get("MemAvailable", vals.get("MemFree"))
    free_mb = round(avail / 1024.0, 1) if avail is not None else None
    return swap_used, free_mb


def _collect_windows_sample(
    pid: Optional[int],
    prev_state: dict[str, tuple[float, float]],
) -> tuple[Optional[tuple[Optional[float], float, list[float]]], Optional[float]]:
    ps = shutil.which("powershell") or shutil.which("pwsh")
    if not ps:
        return None, None
    proc_part = (
        f"$p = Get-Process -Id {pid} -ErrorAction SilentlyContinue | Select-Object WorkingSet64, CPU;"
        if pid is not None
        else "$p = $null;"
    )
    script = (
        proc_part
        + "$os = Get-CimInstance Win32_OperatingSystem -ErrorAction SilentlyContinue | Select-Object FreePhysicalMemory;"
        + "[pscustomobject]@{ proc = $p; os = $os } | ConvertTo-Json -Depth 3 -Compress"
    )
    raw = _run_cmd(ps, "-NoProfile", "-Command", script, timeout=6.0).strip()
    if not raw:
        return None, None
    try:
        data = json.loads(raw)
    except ValueError:
        return None, None
    os_info = data.get("os") or {}
    free_kb = os_info.get("FreePhysicalMemory")
    free_mb = round(float(free_kb) / 1024.0, 1) if isinstance(free_kb, (int, float)) else None
    proc_info = data.get("proc")
    if pid is not None and not isinstance(proc_info, dict):
        return None, free_mb
    proc_sample: Optional[tuple[Optional[float], float, list[float]]] = None
    if isinstance(proc_info, dict):
        rss_mb = float(proc_info.get("WorkingSet64") or 0) / float(1 << 20)
        cpu_s = proc_info.get("CPU")
        now = time.monotonic()
        cpu_pct: Optional[float] = None
        if isinstance(cpu_s, (int, float)):
            key = str(pid)
            if key in prev_state:
                prev_t, prev_cpu = prev_state[key]
                dt = now - prev_t
                if dt > 0.0:
                    cpu_pct = round(100.0 * (float(cpu_s) - prev_cpu) / dt, 1)
            prev_state[key] = (now, float(cpu_s))
        proc_sample = (cpu_pct, rss_mb, [])
    return proc_sample, free_mb


def collect_once(
    pid: Optional[int] = None,
    prev_state: Optional[dict[str, Any]] = None,
) -> Optional[dict[str, Any]]:
    """Collect a single sample dictionary; returns `None` if `pid` was given and exited."""
    state: dict[str, Any] = {} if prev_state is None else prev_state
    now = time.time()
    cpu_pct: Optional[float] = None
    rss_mb: Optional[float] = None
    threads: list[float] = []
    gpu: Optional[dict[str, Optional[float]]] = None
    swap_used_mb: Optional[float] = None
    free_mb: Optional[float] = None
    thermal: Optional[int] = None

    if sys.platform == "darwin":
        if pid is not None:
            proc = _collect_macos_process(pid)
            if proc is None:
                return None
            cpu_pct, rss_mb, threads = proc
        gpu = parse_ioreg_gpu(_run_cmd("ioreg", "-r", "-d", "1", "-c", "IOAccelerator"))
        swap_used_mb, free_mb = parse_macos_memory(
            _run_cmd("sysctl", "-n", "vm.swapusage"),
            _run_cmd("vm_stat"),
        )
        thermal = parse_macos_thermal(_run_cmd("pmset", "-g", "therm"))
    elif os.name == "nt":
        proc_win, free_mb = _collect_windows_sample(pid, state)
        if pid is not None and proc_win is None:
            return None
        if proc_win is not None:
            cpu_pct, rss_mb, threads = proc_win
    else:
        if pid is not None:
            proc_linux = _collect_linux_process(pid, state)
            if proc_linux is None:
                return None
            cpu_pct, rss_mb, threads = proc_linux
        swap_used_mb, free_mb = _collect_linux_memory()

    return {
        "t": round(now, 3),
        "cpu_pct": round(cpu_pct, 1) if cpu_pct is not None else None,
        "rss_mb": round(rss_mb, 1) if rss_mb is not None else None,
        "threads": threads,
        "gpu": gpu,
        "swap_used_mb": swap_used_mb,
        "free_mb": free_mb,
        "thermal": thermal,
    }


def record_samples(
    out_path: Path,
    pid: Optional[int] = None,
    interval: float = 1.0,
    once: bool = False,
    wait_seconds: float = 120.0,
) -> int:
    """Record resource samples into JSONL `out_path`."""
    target_pid = pid
    if not once and target_pid is None:
        target_pid = find_game_pid(wait_seconds=wait_seconds)
        if target_pid is None:
            print("resource_sampler: no hl2-bevy process found", file=sys.stderr)
            return 1

    out_path.parent.mkdir(parents=True, exist_ok=True)
    state: dict[str, Any] = {}
    with out_path.open("a", encoding="utf-8") as handle:
        if once:
            sample = collect_once(pid=target_pid, prev_state=state)
            if sample is None:
                print("resource_sampler: target process not found", file=sys.stderr)
                return 1
            handle.write(json.dumps(sample) + "\n")
            handle.flush()
            return 0

        print(f"sampling pid {target_pid} every {interval}s into {out_path}", flush=True)
        while True:
            started = time.time()
            sample = collect_once(pid=target_pid, prev_state=state)
            if sample is None:
                break
            handle.write(json.dumps(sample) + "\n")
            handle.flush()
            time.sleep(max(0.0, interval - (time.time() - started)))
        print("process exited", flush=True)
    return 0


def _series_stats(values: list[float]) -> Optional[dict[str, float]]:
    if not values:
        return None
    ordered = sorted(values)
    return {
        "mean": sum(ordered) / len(ordered),
        "p50": ordered[len(ordered) // 2],
        "min": ordered[0],
        "max": ordered[-1],
    }


def summarize_samples(path: Path) -> dict[str, Any]:
    """Summarize a JSON-lines file produced by `record_samples`."""
    rows: list[dict[str, Any]] = []
    with Path(path).open(encoding="utf-8", errors="replace") as handle:
        for raw in handle:
            line = raw.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(obj, dict):
                rows.append(obj)

    def collect_nums(getter: Any) -> list[float]:
        out: list[float] = []
        for row in rows:
            val = getter(row)
            if isinstance(val, (int, float)):
                out.append(float(val))
        return out

    timestamps = collect_nums(lambda r: r.get("t"))
    duration_s = (timestamps[-1] - timestamps[0]) if len(timestamps) >= 2 else 0.0
    swaps = collect_nums(lambda r: r.get("swap_used_mb"))
    thermals = collect_nums(lambda r: r.get("thermal"))

    return {
        "samples": len(rows),
        "duration_s": round(duration_s, 1),
        "cpu_pct": _series_stats(collect_nums(lambda r: r.get("cpu_pct"))),
        "rss_mb": _series_stats(collect_nums(lambda r: r.get("rss_mb"))),
        "top_thread_pct": _series_stats(
            collect_nums(lambda r: (r.get("threads") or [None])[0] if isinstance(r.get("threads"), list) else None)
        ),
        "gpu_device_pct": _series_stats(
            collect_nums(lambda r: (r.get("gpu") or {}).get("device_pct") if isinstance(r.get("gpu"), dict) else None)
        ),
        "gpu_in_use_mb": _series_stats(
            collect_nums(lambda r: (r.get("gpu") or {}).get("in_use_mb") if isinstance(r.get("gpu"), dict) else None)
        ),
        "free_mb": _series_stats(collect_nums(lambda r: r.get("free_mb"))),
        "swap_start_mb": swaps[0] if swaps else None,
        "swap_end_mb": swaps[-1] if swaps else None,
        "swap_delta_mb": round(swaps[-1] - swaps[0], 1) if swaps else None,
        "thermal_min": min(thermals) if thermals else None,
    }


def format_summary(data: dict[str, Any]) -> str:
    """Format resource sampler stats into compact readable lines."""
    lines = [f"samples={data['samples']} duration={data['duration_s']:.1f}s"]
    for label, key, unit in (
        ("cpu", "cpu_pct", "%"),
        ("top_thread", "top_thread_pct", "%"),
        ("rss", "rss_mb", " MB"),
        ("gpu", "gpu_device_pct", "%"),
        ("gpu_mem", "gpu_in_use_mb", " MB"),
        ("free_mem", "free_mb", " MB"),
    ):
        stats = data.get(key)
        if isinstance(stats, dict):
            lines.append(
                f"{label:10} mean={stats['mean']:.1f}{unit} p50={stats['p50']:.1f}{unit} "
                f"min={stats['min']:.1f}{unit} max={stats['max']:.1f}{unit}"
            )
    if data.get("swap_start_mb") is not None:
        lines.append(
            f"swap       start={data['swap_start_mb']:.1f} MB end={data['swap_end_mb']:.1f} MB "
            f"delta={data['swap_delta_mb']:+.1f} MB"
        )
    if data.get("thermal_min") is not None:
        lines.append(f"thermal    min_cpu_limit={data['thermal_min']:.0f}%")
    return "\n".join(lines) + "\n"


def _add_record_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--out", default=os.environ.get("OUT"), help="JSONL output path (or OUT env var).")
    raw_pid = os.environ.get("PID")
    parser.add_argument("--pid", type=int, default=int(raw_pid) if raw_pid else None, help="Target PID.")
    parser.add_argument("--interval", type=float, default=float(os.environ.get("INTERVAL", "1.0")), help="Interval seconds.")
    parser.add_argument("--once", action="store_true", default=os.environ.get("ONCE") == "1", help="Take one baseline sample.")
    parser.add_argument("--wait", type=float, default=120.0, help="Seconds to wait for hl2-bevy when --pid is omitted.")


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    args_list = list(sys.argv[1:] if argv is None else argv)
    # Support bare invocation with `--out` / `OUT` defaulting to `record`.
    if not args_list or args_list[0] not in ("record", "summary", "-h", "--help"):
        parser = argparse.ArgumentParser(description="Sample process/system/GPU metrics or summarize JSONL.")
        _add_record_args(parser)
        ns = parser.parse_args(args_list)
        ns.subcommand = "record"
        return ns

    parser = argparse.ArgumentParser(description="Sample process/system/GPU metrics or summarize JSONL.")
    sub = parser.add_subparsers(dest="subcommand", required=True)
    rec = sub.add_parser("record", help="Record external resource samples into JSONL.")
    _add_record_args(rec)

    summ = sub.add_parser("summary", help="Summarize a recorded JSONL file.")
    summ.add_argument("--in", dest="input_path", required=True, help="Input JSONL path.")
    summ.add_argument("--json-out", default=None, help="Optional JSON summary output path.")
    return parser.parse_args(args_list)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    if args.subcommand == "record":
        if not args.out:
            print("resource_sampler: --out (or OUT env var) is required", file=sys.stderr)
            return 2
        return record_samples(
            Path(args.out),
            pid=args.pid,
            interval=args.interval,
            once=args.once,
            wait_seconds=args.wait,
        )
    if args.subcommand == "summary":
        in_path = Path(args.input_path)
        if not in_path.is_file():
            print(f"resource_sampler: input file not found: {in_path}", file=sys.stderr)
            return 1
        data = summarize_samples(in_path)
        sys.stdout.write(format_summary(data))
        if args.json_out:
            out = Path(args.json_out)
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(json.dumps(data, indent=2), encoding="utf-8")
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
