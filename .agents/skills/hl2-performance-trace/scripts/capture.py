#!/usr/bin/env python3
"""Cross-platform performance capture & analysis orchestrator for HL2-RS.

Reuses `.agents/skills/hl2-session/scripts/` (`session.py`, `rcon.py`) and combines
`--profile`, `trace-chrome`, `trace-alloc`, `resource_sampler.py`, and macOS Metal System
Trace / `sample` tools into one repeatable workflow. Stdlib-only.
"""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
from typing import Any, Callable, Optional

_SKILL_SCRIPTS = Path(__file__).resolve().parent
_REPO_ROOT = _SKILL_SCRIPTS.parents[3]
_SESSION_SCRIPTS = _REPO_ROOT / ".agents" / "skills" / "hl2-session" / "scripts"
for _path in (_SKILL_SCRIPTS, _SESSION_SCRIPTS):
    if str(_path) not in sys.path:
        sys.path.insert(0, str(_path))

import metal_gpu  # noqa: E402
import resource_sampler  # noqa: E402
import sample_summary  # noqa: E402
import session  # noqa: E402
import trace_summary  # noqa: E402
import trace_waste  # noqa: E402

LOW_DISK_WARN_GIB = 30.0

MODES: dict[str, dict[str, Any]] = {
    "stages": {
        "plugins": "advanced-options,debug-tools",
        "trace_chrome": False,
        "trace_alloc": False,
        "description": "Everyday build with --profile stage timings only.",
    },
    "pacing": {
        "plugins": "advanced-options,debug-tools",
        "trace_chrome": False,
        "trace_alloc": False,
        "description": "Everyday build with --profile for uninflated Metal GPU and surface-pacing captures.",
    },
    "trace": {
        "plugins": "advanced-options,debug-tools,trace-chrome",
        "trace_chrome": True,
        "trace_alloc": False,
        "description": "Chrome trace of Bevy systems/render nodes and HL2-RS game/state spans (~1.8 MB/frame).",
    },
    "trace-alloc": {
        "plugins": "advanced-options,debug-tools,trace-chrome,trace-alloc",
        "trace_chrome": True,
        "trace_alloc": True,
        "description": "Full trace plus counting global allocator (`state_frame`, adds ~0.6 ms/frame).",
    },
}


def parse_plugins(raw: Optional[str]) -> set[str]:
    """Split a comma-separated plugin string into a canonical set."""
    if not isinstance(raw, str):
        return set()
    return {part.strip() for part in raw.split(",") if part.strip() and part.strip() != "none"}


def build_matches_mode(
    info: dict[str, Any],
    mode: str,
    require_debug_tools: bool = True,
) -> tuple[bool, str]:
    """Verify whether `build-bevy-info.json` matches the requested capture `mode`."""
    if mode not in MODES:
        return False, f"unknown mode {mode!r}"
    if not info:
        return False, f"bin/build-bevy-info.json is missing; run `capture.py build --mode {mode}`."
    selected = parse_plugins(info.get("plugins"))
    spec = MODES[mode]
    if require_debug_tools and "debug-tools" not in selected:
        return False, (
            f"installed build ({info.get('plugins')!r}) lacks debug-tools; "
            f"run `capture.py build --mode {mode}`."
        )
    has_trace = "trace-chrome" in selected or "trace-alloc" in selected
    has_alloc = "trace-alloc" in selected
    if has_trace != bool(spec["trace_chrome"]) or has_alloc != bool(spec["trace_alloc"]):
        return False, (
            f"installed build has plugins={info.get('plugins')!r}, which does not match "
            f"mode {mode!r} (expected {spec['plugins']!r}); run `capture.py build --mode {mode}`."
        )
    return True, ""


def build_script_command(
    repo: Path,
    mode: str,
    profile: str = "release",
    no_clean: bool = False,
    is_windows: Optional[bool] = None,
) -> list[str]:
    """Construct the platform build script command for `mode`."""
    if mode not in MODES:
        raise ValueError(f"unknown mode {mode!r}")
    win = (os.name == "nt") if is_windows is None else is_windows
    plugins = str(MODES[mode]["plugins"])
    if win:
        cmd = [
            "powershell",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            str(repo / "scripts" / "build-bevy.ps1"),
        ]
        if profile == "debug":
            cmd.append("-DebugBuild")
        elif profile == "dist":
            cmd.append("-Dist")
        if no_clean:
            cmd.append("-NoClean")
        cmd.extend(["-Plugins", plugins])
        return cmd

    cmd = ["bash", str(repo / "scripts" / "build-bevy.sh")]
    if profile == "debug":
        cmd.append("--debug")
    elif profile == "dist":
        cmd.append("--dist")
    if no_clean:
        cmd.append("--no-clean")
    cmd.extend(["--plugins", plugins])
    return cmd


def default_capture_dir(repo: Path, mode: str, name: Optional[str] = None) -> Path:
    """Return a timestamped capture directory under `artifacts/perf-traces/`."""
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    raw_label = name.strip() if name else mode
    safe_label = "".join(ch if (ch.isalnum() or ch in "-_") else "-" for ch in raw_label).strip("-") or mode
    return repo / "artifacts" / "perf-traces" / f"{stamp}-{safe_label}"


def _free_gib(path: Path) -> Optional[float]:
    target = path
    while not target.exists() and target != target.parent:
        target = target.parent
    try:
        usage = shutil.disk_usage(target)
        return round(usage.free / float(1 << 30), 2)
    except OSError:
        return None


def _git_head(repo: Path) -> str:
    try:
        proc = subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "--short", "HEAD"],
            capture_output=True,
            text=True,
            timeout=3,
            check=False,
        )
        return proc.stdout.strip() or "unknown"
    except (OSError, subprocess.SubprocessError):
        return "unknown"


def _human_size(num_bytes: int) -> str:
    if num_bytes >= (1 << 30):
        return f"{num_bytes / float(1 << 30):.2f} GiB"
    if num_bytes >= (1 << 20):
        return f"{num_bytes / float(1 << 20):.1f} MiB"
    if num_bytes >= (1 << 10):
        return f"{num_bytes / float(1 << 10):.1f} KiB"
    return f"{num_bytes} B"


def _path_size(path: Path) -> int:
    if path.is_file():
        return path.stat().st_size
    if path.is_dir():
        return sum(p.stat().st_size for p in path.rglob("*") if p.is_file())
    return 0


def _delete_path(path: Path) -> int:
    size = _path_size(path)
    if path.is_dir():
        shutil.rmtree(path)
    elif path.exists():
        path.unlink()
    return size


def collect_status(
    repo: Path,
    out_root: Optional[Path] = None,
    include_idle_sample: bool = True,
) -> dict[str, Any]:
    """Inspect packaged build features, disk headroom, running games, OS profilers, and idle baseline."""
    exe = session.executable_path(repo)
    info = session.read_build_info(repo)
    selected = sorted(parse_plugins(info.get("plugins")))
    capture_root = out_root or (repo / "artifacts" / "perf-traces")
    free_gib = _free_gib(capture_root)
    xctrace_available = sys.platform == "darwin" and shutil.which("xcrun") is not None
    if xctrace_available:
        check = subprocess.run(["xcrun", "-f", "xctrace"], capture_output=True, text=True, check=False)
        xctrace_available = check.returncode == 0
    sample_available = sys.platform == "darwin" and shutil.which("sample") is not None
    idle = resource_sampler.collect_once() if include_idle_sample else None

    return {
        "repo": str(repo),
        "git_head": _git_head(repo),
        "executable": str(exe),
        "executable_exists": exe.is_file(),
        "build_info": {
            "profile": info.get("profile"),
            "plugins": info.get("plugins"),
            "selected": selected,
            "is_trace_build": "trace-chrome" in selected or "trace-alloc" in selected,
            "is_alloc_build": "trace-alloc" in selected,
            "debug_tools_enabled": "debug-tools" in selected if selected else None,
        },
        "running_games": session.find_running_games(),
        "capture_root": str(capture_root),
        "free_gib": free_gib,
        "low_disk_warning": free_gib is not None and free_gib < LOW_DISK_WARN_GIB,
        "tools": {
            "xctrace": xctrace_available,
            "sample": sample_available,
        },
        "idle_baseline": idle,
    }


def cmd_status(args: argparse.Namespace) -> int:
    repo = args.repo.resolve() if args.repo else session.default_repo_dir()
    out_root = args.out_root.resolve() if args.out_root else None
    status = collect_status(repo, out_root=out_root, include_idle_sample=not args.no_sample)
    if args.json:
        print(json.dumps(status, indent=2))
        return 0

    b = status["build_info"]
    print(f"Repo: {status['repo']} (HEAD {status['git_head']})")
    print(f"Executable: {status['executable']} ({'ready' if status['executable_exists'] else 'missing'})")
    print(f"Build info: profile={b['profile']} plugins={b['plugins']} (trace_build={b['is_trace_build']})")
    if b["is_trace_build"]:
        print("Reminder: a trace build is currently packaged; run `capture.py restore` after profiling.")
    if status["running_games"]:
        pids = ", ".join(str(g["pid"]) for g in status["running_games"])
        print(f"Running hl2-bevy: PID {pids}")
    else:
        print("Running hl2-bevy: none")
    free_str = f"{status['free_gib']:.2f} GiB free" if status["free_gib"] is not None else "unknown"
    warn = f" (WARNING: < {LOW_DISK_WARN_GIB:.0f} GiB for multi-GB traces)" if status["low_disk_warning"] else ""
    print(f"Capture root: {status['capture_root']} ({free_str}{warn})")
    print(f"OS tools: xctrace={status['tools']['xctrace']} sample={status['tools']['sample']}")
    idle = status.get("idle_baseline")
    if isinstance(idle, dict):
        gpu = idle.get("gpu")
        gpu_text = f"gpu_busy={gpu.get('device_pct')}%" if isinstance(gpu, dict) else "gpu=n/a"
        print(f"Idle baseline: {gpu_text}, free_mem={idle.get('free_mb')} MB, swap={idle.get('swap_used_mb')} MB")
    return 0


def cmd_build(args: argparse.Namespace, force_mode: Optional[str] = None) -> int:
    repo = args.repo.resolve() if args.repo else session.default_repo_dir()
    mode = force_mode or args.mode
    cmd = build_script_command(repo, mode=mode, profile=args.profile, no_clean=args.no_clean)
    if args.dry_run:
        print(" ".join(shlex.quote(tok) for tok in cmd))
        return 0
    proc = subprocess.run(cmd, cwd=str(repo), check=False)
    return int(proc.returncode)


def _write_meta(out_dir: Path, payload: dict[str, Any]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "capture_meta.json").write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def _split_trailing(trailing: list[str]) -> list[str]:
    items = list(trailing)
    if items and items[0] == "--":
        items = items[1:]
    return items


def cmd_launch(args: argparse.Namespace) -> int:
    repo = args.repo.resolve() if args.repo else session.default_repo_dir()
    info = session.read_build_info(repo)
    exe = session.executable_path(repo)
    if not exe.is_file():
        print(f"capture: missing {exe}; run `capture.py build --mode {args.mode}` first.", file=sys.stderr)
        return 1
    ok, reason = build_matches_mode(info, args.mode, require_debug_tools=True)
    if not ok:
        print(f"capture: {reason}", file=sys.stderr)
        return 1
    if session.find_running_games():
        print("capture: hl2-bevy is already running; capture `agent_resume` and close it before launching a trace run.", file=sys.stderr)
        return 1

    out_dir = args.out.resolve() if args.out else default_capture_dir(repo, args.mode, name=args.name)
    out_dir.mkdir(parents=True, exist_ok=True)
    report_path = out_dir / "report.json"
    trace_path = out_dir / "trace.json"
    _write_meta(
        out_dir,
        {
            "kind": "live",
            "mode": args.mode,
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "git_head": _git_head(repo),
            "build_info": {"profile": info.get("profile"), "plugins": info.get("plugins")},
            "idle_baseline": resource_sampler.collect_once(),
            "report_path": str(report_path),
            "trace_path": str(trace_path) if MODES[args.mode]["trace_chrome"] else None,
        },
    )

    env = dict(os.environ)
    if MODES[args.mode]["trace_chrome"]:
        env["TRACE_CHROME"] = str(trace_path)
    else:
        env.pop("TRACE_CHROME", None)

    cmd = [
        sys.executable,
        str(_SESSION_SCRIPTS / "session.py"),
        "launch",
        "--repo",
        str(repo),
        "--port",
        str(args.port),
        "--",
        "--profile",
        "--report",
        str(report_path),
        *_split_trailing(args.trailing),
    ]
    print(f"Capture dir: {out_dir}")
    proc = subprocess.run(cmd, cwd=str(repo), env=env, check=False)
    return int(proc.returncode)


def cmd_headless(args: argparse.Namespace) -> int:
    repo = args.repo.resolve() if args.repo else session.default_repo_dir()
    info = session.read_build_info(repo)
    exe = session.executable_path(repo)
    if not exe.is_file():
        print(f"capture: missing {exe}; run `capture.py build --mode {args.mode}` first.", file=sys.stderr)
        return 1
    if not args.allow_mismatch:
        ok, reason = build_matches_mode(info, args.mode, require_debug_tools=False)
        if not ok:
            print(f"capture: {reason}", file=sys.stderr)
            return 1

    out_dir = args.out.resolve() if args.out else default_capture_dir(repo, args.mode, name=args.name)
    out_dir.mkdir(parents=True, exist_ok=True)
    report_path = out_dir / "report.json"
    trace_path = out_dir / "trace.json"
    run_log = out_dir / "run.log"

    env = dict(os.environ)
    if MODES[args.mode]["trace_chrome"]:
        env["TRACE_CHROME"] = str(trace_path)
    else:
        env.pop("TRACE_CHROME", None)

    cmd = [
        str(exe),
        "--background",
        "--no-setup",
        "--mute",
        "--map",
        args.map,
        "--frames",
        str(args.frames),
        "--profile",
        "--report",
        str(report_path),
        *_split_trailing(args.trailing),
    ]
    meta = {
        "kind": "headless",
        "mode": args.mode,
        "map": args.map,
        "frames": args.frames,
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "git_head": _git_head(repo),
        "build_info": {"profile": info.get("profile"), "plugins": info.get("plugins")},
        "idle_baseline": resource_sampler.collect_once(),
        "report_path": str(report_path),
        "trace_path": str(trace_path) if MODES[args.mode]["trace_chrome"] else None,
    }
    _write_meta(out_dir, meta)

    print(f"Running headless capture into {out_dir}...")
    with run_log.open("w", encoding="utf-8", errors="replace") as log_file:
        proc = subprocess.run(cmd, cwd=str(repo), env=env, stdout=log_file, stderr=subprocess.STDOUT, check=False)
    meta["exit_code"] = int(proc.returncode)
    _write_meta(out_dir, meta)
    if proc.returncode != 0:
        print(f"capture: game exited {proc.returncode}; see {run_log}", file=sys.stderr)
        return int(proc.returncode)

    if args.analyze:
        return analyze_capture_dir(out_dir, delete_raw=args.delete_raw)
    print(f"Headless capture complete: {out_dir}")
    return 0


def _resolve_target_pid(explicit_pid: Optional[int]) -> Optional[int]:
    if explicit_pid is not None:
        return explicit_pid
    games = session.find_running_games()
    if len(games) == 1:
        return int(games[0]["pid"])
    if not games:
        print("capture: no running hl2-bevy process found; pass --pid or launch the game first.", file=sys.stderr)
    else:
        pids = ", ".join(str(g["pid"]) for g in games)
        print(f"capture: multiple hl2-bevy processes running ({pids}); pass --pid explicitly.", file=sys.stderr)
    return None


def export_metal_trace(out_dir: Path, runner: Callable[..., subprocess.CompletedProcess[Any]] = subprocess.run) -> list[str]:
    """Export GPU interval and displayed-surface XML tables from `gpu.trace`; return error messages."""
    trace_bundle = out_dir / "gpu.trace"
    if not trace_bundle.exists():
        return []
    if sys.platform != "darwin" or shutil.which("xcrun") is None:
        return ["gpu.trace present but xcrun xctrace export requires macOS"]

    errors: list[str] = []
    exports = (
        ("metal-gpu-intervals", out_dir / "gpu_intervals.xml"),
        ("displayed-surfaces-interval", out_dir / "displayed_surfaces.xml"),
    )
    for schema, dest in exports:
        if dest.is_file():
            continue
        xpath = f'/trace-toc/run[@number="1"]/data/table[@schema="{schema}"]'
        with dest.open("w", encoding="utf-8") as xml_file:
            proc = runner(
                ["xcrun", "xctrace", "export", "--input", str(trace_bundle), "--xpath", xpath],
                stdout=xml_file,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
            )
        if proc.returncode != 0 or dest.stat().st_size == 0:
            err = (proc.stderr or "").strip() if hasattr(proc, "stderr") else ""
            errors.append(f"xctrace export {schema} failed (exit {proc.returncode}): {err}")
    return errors


def summarize_game_report(report_path: Path) -> dict[str, Any]:
    """Extract frame benchmark and `cpu_stages` tables from HL2-RS `report.json`."""
    raw = json.loads(report_path.read_text(encoding="utf-8", errors="replace"))
    render = raw.get("render") if isinstance(raw.get("render"), dict) else raw
    pres = render.get("presentation") if isinstance(render.get("presentation"), dict) else {}
    hud = pres.get("hud") if isinstance(pres.get("hud"), dict) else {}
    benchmark = hud.get("benchmark") if isinstance(hud.get("benchmark"), dict) else None

    stages_raw = pres.get("cpu_stages") if isinstance(pres.get("cpu_stages"), dict) else {}
    stages = []
    for name, vals in stages_raw.items():
        if not isinstance(vals, dict):
            continue
        stages.append({
            "stage": name,
            "calls": vals.get("calls", 0),
            "samples": vals.get("samples", 0),
            "mean_ms": float(vals.get("mean_ms", 0.0)),
            "p95_ms": float(vals.get("p95_ms", 0.0)),
            "max_ms": float(vals.get("max_ms", 0.0)),
        })
    stages.sort(key=lambda item: -item["p95_ms"])

    return {
        "frames": render.get("frames"),
        "adapter": render.get("adapter"),
        "backend": render.get("backend"),
        "benchmark": benchmark,
        "cpu_stages": stages,
    }


def format_game_report(data: dict[str, Any], top: int = 25) -> str:
    """Format HL2-RS `report.json` frame benchmark and CPU stage timings."""
    lines = [
        f"frames={data.get('frames')} adapter={data.get('adapter')} backend={data.get('backend')}"
    ]
    bench = data.get("benchmark")
    if isinstance(bench, dict) and bench.get("mean_frame_ms") is not None:
        lines.append(
            f"measured_frames={bench.get('measured_frames')} mean={bench.get('mean_frame_ms', 0.0):.2f} ms "
            f"({bench.get('mean_fps', 0.0):.1f} fps) p95<={bench.get('p95_frame_ms_upper_bound')} "
            f"p99<={bench.get('p99_frame_ms_upper_bound')} max={bench.get('max_frame_ms')}"
        )
    stages = data.get("cpu_stages") or []
    if stages:
        lines.append(f"{'stage':28} {'samples':>8} {'mean':>8} {'p95':>8} {'max':>8}")
        for row in stages[:top]:
            lines.append(
                f"{row['stage'][:28]:28} {row['samples']:8d} {row['mean_ms']:8.3f} "
                f"{row['p95_ms']:8.3f} {row['max_ms']:8.2f}"
            )
    else:
        lines.append("cpu_stages: none recorded (check --profile or gameplay warm-up)")
    return "\n".join(lines) + "\n"


def _write_artifact_outputs(out_dir: Path, stem: str, text: str, data: dict[str, Any]) -> None:
    (out_dir / f"{stem}.txt").write_text(text, encoding="utf-8")
    (out_dir / f"{stem}.json").write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def analyze_capture_dir(
    out_dir: Path,
    top: int = 40,
    spikes: int = 10,
    slow: int = 8,
    near: float = 2048.0,
    refresh_hz: float = 120.0,
    delete_raw: bool = False,
    metal_exporter: Callable[..., list[str]] = export_metal_trace,
) -> int:
    """Analyze every captured artifact in `out_dir`, write summaries, and optionally prune raw traces."""
    if not out_dir.is_dir():
        print(f"capture: directory not found: {out_dir}", file=sys.stderr)
        return 1

    sections: list[str] = [f"=== Capture summary: {out_dir} ==="]
    errors: list[str] = []
    trace_ok = False
    gpu_ok = False

    report_path = out_dir / "report.json"
    if report_path.is_file():
        try:
            rep = summarize_game_report(report_path)
            rep_text = format_game_report(rep, top=top)
            _write_artifact_outputs(out_dir, "report_summary", rep_text, rep)
            sections.append("\n## report.json (--profile)\n" + "\n".join(rep_text.splitlines()[:15]))
        except Exception as exc:  # noqa: BLE001
            errors.append(f"report.json: {exc}")

    res_path = out_dir / "resources.jsonl"
    if res_path.is_file():
        try:
            res = resource_sampler.summarize_samples(res_path)
            res_text = resource_sampler.format_summary(res)
            _write_artifact_outputs(out_dir, "resources_summary", res_text, res)
            sections.append("\n## resources.jsonl\n" + res_text.rstrip())
        except Exception as exc:  # noqa: BLE001
            errors.append(f"resources.jsonl: {exc}")

    trace_path = out_dir / "trace.json"
    if trace_path.is_file():
        try:
            tsum = trace_summary.analyze_trace(trace_path, top=top, spikes=spikes, full_paths=True)
            tsum_text = trace_summary.format_report(tsum, top=top)
            _write_artifact_outputs(out_dir, "trace_summary", tsum_text, tsum)

            waste = trace_waste.analyze_waste(trace_path, top=min(top, 25), slow_n=slow, near=near)
            waste_text = trace_waste.format_report(waste, top=min(top, 25))
            _write_artifact_outputs(out_dir, "waste_summary", waste_text, waste)
            trace_ok = True
            sections.append("\n## trace.json (top spans)\n" + "\n".join(tsum_text.splitlines()[:15]))
            sections.append("\n## trace.json (waste & pacing)\n" + "\n".join(waste_text.splitlines()[:18]))
        except Exception as exc:  # noqa: BLE001
            errors.append(f"trace.json: {exc}")

    if (out_dir / "gpu.trace").exists():
        export_errs = metal_exporter(out_dir)
        errors.extend(export_errs)

    gpu_xml = out_dir / "gpu_intervals.xml"
    pacing_xml = out_dir / "displayed_surfaces.xml"
    gpu_errors_before = len(errors)
    if gpu_xml.is_file():
        try:
            gdata = metal_gpu.analyze_gpu_intervals(gpu_xml, top=top)
            gtext = metal_gpu.format_gpu_intervals(gdata, top=top)
            _write_artifact_outputs(out_dir, "gpu_passes", gtext, gdata)
            sections.append("\n## gpu_intervals.xml\n" + "\n".join(gtext.splitlines()[:15]))
        except Exception as exc:  # noqa: BLE001
            errors.append(f"gpu_intervals.xml: {exc}")
    if pacing_xml.is_file():
        try:
            pdata = metal_gpu.analyze_display_pacing(pacing_xml, refresh_hz=refresh_hz)
            ptext = metal_gpu.format_display_pacing(pdata)
            _write_artifact_outputs(out_dir, "display_pacing", ptext, pdata)
            sections.append("\n## displayed_surfaces.xml\n" + ptext.rstrip())
        except Exception as exc:  # noqa: BLE001
            errors.append(f"displayed_surfaces.xml: {exc}")
    if (gpu_xml.is_file() or pacing_xml.is_file()) and len(errors) == gpu_errors_before:
        gpu_ok = True

    sample_path = out_dir / "sample.txt"
    if sample_path.is_file():
        try:
            sdata = sample_summary.analyze_sample(sample_path, top=top)
            stext = sample_summary.format_sample(sdata, top=top)
            _write_artifact_outputs(out_dir, "sample_summary", stext, sdata)
            sections.append("\n## sample.txt\n" + "\n".join(stext.splitlines()[:15]))
        except Exception as exc:  # noqa: BLE001
            errors.append(f"sample.txt: {exc}")

    if errors:
        sections.append("\n## Errors\n" + "\n".join(f"- {err}" for err in errors))

    if delete_raw and not errors:
        freed = 0
        removed: list[str] = []
        if trace_path.is_file() and trace_ok:
            freed += _delete_path(trace_path)
            removed.append("trace.json")
        gpu_trace = out_dir / "gpu.trace"
        if gpu_trace.exists() and gpu_ok:
            freed += _delete_path(gpu_trace)
            removed.append("gpu.trace")
        if removed:
            sections.append(f"\nDeleted raw artifacts ({', '.join(removed)}): reclaimed {_human_size(freed)}")
    elif delete_raw and errors:
        sections.append("\nPreserved raw artifacts because analysis/export reported errors.")

    summary_text = "\n".join(sections) + "\n"
    (out_dir / "summary.txt").write_text(summary_text, encoding="utf-8")
    sys.stdout.write(summary_text)
    return 1 if errors else 0


def cmd_metal_record(args: argparse.Namespace) -> int:
    if sys.platform != "darwin" or shutil.which("xcrun") is None:
        print("capture: Metal System Trace recording requires macOS with Xcode `xcrun xctrace`.", file=sys.stderr)
        return 1
    pid = _resolve_target_pid(args.pid)
    if pid is None:
        return 1

    out_dir = args.out.resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    trace_bundle = out_dir / "gpu.trace"
    cmd = [
        "xcrun",
        "xctrace",
        "record",
        "--template",
        "Metal System Trace",
        "--time-limit",
        f"{args.seconds}s",
        "--output",
        str(trace_bundle),
        "--attach",
        str(pid),
    ]
    log_path = out_dir / "xctrace.log"
    with log_path.open("w", encoding="utf-8", errors="replace") as log_file:
        proc = subprocess.run(cmd, stdout=log_file, stderr=subprocess.STDOUT, check=False)
    if proc.returncode != 0:
        print(f"capture: xctrace record failed (exit {proc.returncode}); see {log_path}", file=sys.stderr)
        return int(proc.returncode)

    if args.export:
        return analyze_capture_dir(out_dir, delete_raw=args.delete_raw)
    print(f"Recorded Metal System Trace: {trace_bundle}")
    return 0


def cmd_sample_record(args: argparse.Namespace) -> int:
    if sys.platform != "darwin" or shutil.which("sample") is None:
        print("capture: macOS `sample` recording requires macOS.", file=sys.stderr)
        return 1
    pid = _resolve_target_pid(args.pid)
    if pid is None:
        return 1

    out_dir = args.out.resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    sample_path = out_dir / "sample.txt"
    proc = subprocess.run(
        ["sample", str(pid), str(args.seconds), str(args.interval_ms), "-file", str(sample_path)],
        check=False,
    )
    if proc.returncode != 0:
        print(f"capture: `sample` failed (exit {proc.returncode})", file=sys.stderr)
        return int(proc.returncode)
    sdata = sample_summary.analyze_sample(sample_path, top=args.top)
    stext = sample_summary.format_sample(sdata, top=args.top)
    _write_artifact_outputs(out_dir, "sample_summary", stext, sdata)
    sys.stdout.write(stext)
    return 0


def cmd_cleanup(args: argparse.Namespace) -> int:
    out_dir = args.dir.resolve()
    if not out_dir.is_dir():
        print(f"capture: directory not found: {out_dir}", file=sys.stderr)
        return 1

    freed = 0
    trace_path = out_dir / "trace.json"
    if trace_path.is_file():
        summaries_ok = (out_dir / "trace_summary.txt").is_file() and (out_dir / "waste_summary.txt").is_file()
        if summaries_ok or args.force:
            freed += _delete_path(trace_path)
            print("Removed trace.json")
        else:
            print("Skipped trace.json (missing trace_summary.txt / waste_summary.txt; pass --force)", file=sys.stderr)

    gpu_trace = out_dir / "gpu.trace"
    if gpu_trace.exists():
        gpu_summaries_ok = (out_dir / "gpu_passes.txt").is_file() or (out_dir / "display_pacing.txt").is_file()
        if gpu_summaries_ok or args.force:
            freed += _delete_path(gpu_trace)
            print("Removed gpu.trace")
        else:
            print("Skipped gpu.trace (missing gpu_passes.txt / display_pacing.txt; pass --force)", file=sys.stderr)

    print(f"Total reclaimed: {_human_size(freed)}")
    return 0


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Orchestrate HL2-RS performance captures and analysis.")
    sub = parser.add_subparsers(dest="subcommand", required=True)

    p_status = sub.add_parser("status", help="Inspect build selection, disk headroom, tools, and idle GPU.")
    p_status.add_argument("--repo", type=Path, default=None, help="HL2-RS repository root.")
    p_status.add_argument("--out-root", type=Path, default=None, help="Capture root directory to check free space on.")
    p_status.add_argument("--no-sample", action="store_true", help="Skip taking a live idle GPU/memory sample.")
    p_status.add_argument("--json", action="store_true", help="Print status as JSON.")

    for sub_name in ("build", "restore"):
        p_b = sub.add_parser(sub_name, help="Build packaged hl2-bevy for a profiling mode (or restore everyday build).")
        p_b.add_argument("--repo", type=Path, default=None, help="HL2-RS repository root.")
        if sub_name == "build":
            p_b.add_argument("--mode", choices=sorted(MODES), required=True, help="Capture mode to build.")
        p_b.add_argument("--profile", choices=["release", "debug", "dist"], default="release", help="Cargo profile.")
        p_b.add_argument("--no-clean", action="store_true", help="Pass --no-clean / -NoClean to build script.")
        p_b.add_argument("--dry-run", action="store_true", help="Print the build command without running it.")

    p_launch = sub.add_parser("launch", help="Launch a live /hl2-session capture with --profile / TRACE_CHROME.")
    p_launch.add_argument("--mode", choices=sorted(MODES), required=True, help="Capture mode.")
    p_launch.add_argument("--repo", type=Path, default=None, help="HL2-RS repository root.")
    p_launch.add_argument("--out", type=Path, default=None, help="Capture output directory.")
    p_launch.add_argument("--name", default=None, help="Short label for auto-created capture directory.")
    p_launch.add_argument("--port", default="auto", help="Loopback RCON port or 'auto'.")
    p_launch.add_argument("trailing", nargs=argparse.REMAINDER, help="Extra game arguments after `--`.")

    p_head = sub.add_parser("headless", help="Run a repeatable headless capture with --profile / TRACE_CHROME.")
    p_head.add_argument("--mode", choices=sorted(MODES), required=True, help="Capture mode.")
    p_head.add_argument("--map", required=True, help="Map name to load directly into gameplay.")
    p_head.add_argument("--frames", type=int, default=400, help="Frames to run before exiting (default: 400).")
    p_head.add_argument("--repo", type=Path, default=None, help="HL2-RS repository root.")
    p_head.add_argument("--out", type=Path, default=None, help="Capture output directory.")
    p_head.add_argument("--name", default=None, help="Short label for auto-created capture directory.")
    p_head.add_argument("--analyze", action="store_true", help="Run `analyze` right after capture completes.")
    p_head.add_argument("--delete-raw", action="store_true", help="With --analyze, delete raw trace on success.")
    p_head.add_argument("--allow-mismatch", action="store_true", help="Skip verifying build plugins match --mode.")
    p_head.add_argument("trailing", nargs=argparse.REMAINDER, help="Extra game arguments after `--`.")

    p_metal = sub.add_parser("metal-record", help="Attach macOS Metal System Trace (`xctrace`) to hl2-bevy.")
    p_metal.add_argument("--out", type=Path, required=True, help="Capture output directory.")
    p_metal.add_argument("--pid", type=int, default=None, help="PID of running hl2-bevy (auto-detected if omitted).")
    p_metal.add_argument("--seconds", type=int, default=45, help="Trace duration limit in seconds (default: 45).")
    p_metal.add_argument("--export", action="store_true", help="Export XML tables and analyze immediately.")
    p_metal.add_argument("--delete-raw", action="store_true", help="With --export, delete gpu.trace if export succeeds.")

    p_sample = sub.add_parser("sample-record", help="Capture macOS `sample` CPU stacks on a running hl2-bevy.")
    p_sample.add_argument("--out", type=Path, required=True, help="Capture output directory.")
    p_sample.add_argument("--pid", type=int, default=None, help="PID of running hl2-bevy (auto-detected if omitted).")
    p_sample.add_argument("--seconds", type=int, default=60, help="Sampling duration in seconds (default: 60).")
    p_sample.add_argument("--interval-ms", type=int, default=1, help="Sampling interval in ms (default: 1).")
    p_sample.add_argument("--top", type=int, default=40, help="Rows in sample summary.")

    p_analyze = sub.add_parser("analyze", help="Analyze all captured files in a directory.")
    p_analyze.add_argument("--dir", type=Path, required=True, help="Capture directory to analyze.")
    p_analyze.add_argument("--top", type=int, default=40, help="Top rows per table (default: 40).")
    p_analyze.add_argument("--spikes", type=int, default=10, help="Slowest frames in trace_summary (default: 10).")
    p_analyze.add_argument("--slow", type=int, default=8, help="Slowest frames in trace_waste (default: 8).")
    p_analyze.add_argument("--near", type=float, default=2048.0, help="Nearby NPC radius in units.")
    p_analyze.add_argument("--refresh-hz", type=float, default=120.0, help="Display refresh rate for pacing.")
    p_analyze.add_argument("--delete-raw", action="store_true", help="Delete trace.json/gpu.trace if all analyzers succeed.")

    p_clean = sub.add_parser("cleanup", help="Remove raw multi-GB traces after summaries exist.")
    p_clean.add_argument("--dir", type=Path, required=True, help="Capture directory to prune.")
    p_clean.add_argument("--force", action="store_true", help="Delete raw traces even without summaries.")

    return parser.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    if args.subcommand == "status":
        return cmd_status(args)
    if args.subcommand == "build":
        return cmd_build(args)
    if args.subcommand == "restore":
        return cmd_build(args, force_mode="stages")
    if args.subcommand == "launch":
        return cmd_launch(args)
    if args.subcommand == "headless":
        return cmd_headless(args)
    if args.subcommand == "metal-record":
        return cmd_metal_record(args)
    if args.subcommand == "sample-record":
        return cmd_sample_record(args)
    if args.subcommand == "analyze":
        return analyze_capture_dir(
            args.dir.resolve(),
            top=args.top,
            spikes=args.spikes,
            slow=args.slow,
            near=args.near,
            refresh_hz=args.refresh_hz,
            delete_raw=args.delete_raw,
        )
    if args.subcommand == "cleanup":
        return cmd_cleanup(args)
    return 2


if __name__ == "__main__":
    sys.exit(main())
