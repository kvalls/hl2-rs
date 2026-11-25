#!/usr/bin/env python3
"""Inspect or launch a local HL2-RS RCON session across Windows, macOS, and Linux.

Usage:
    python3 session.py status [--json]
    python3 session.py attach [--pid PID] [--wait SECONDS]
    python3 session.py launch [--port auto|27015] [--build] [-- --map d1_trainstation_01]

Session files are written under <temp>/hl2rs-agent-session/ (or SESSION env var):
rcon_password, port, console_log_path, hl2rs_dir, pid, game.log, and resume.
"""

import argparse
import json
import math
import os
import re
import secrets
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Callable, Optional, TextIO

DEFAULT_PORT = 27015
DEFAULT_PLUGINS = "advanced-options,debug-tools"
_HERE = Path(__file__).resolve().parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

import rcon  # noqa: E402


def default_repo_dir(env: Optional[dict[str, str]] = None) -> Path:
    source = os.environ if env is None else env
    raw = source.get("HL2RS_DIR", "").strip()
    if raw:
        return Path(raw).resolve()
    return Path(__file__).resolve().parents[4]


def executable_path(repo: Path) -> Path:
    name = "hl2-bevy.exe" if os.name == "nt" else "hl2-bevy"
    return repo / "bin" / name


def executable_sha256(exe: Path) -> Optional[str]:
    """The executable's sha256: the build a run measured, which HEAD alone does not show (bin/ may
    hold an experiment or a stale build)."""
    import hashlib

    digest = hashlib.sha256()
    try:
        with exe.open("rb") as stream:
            while chunk := stream.read(65536):
                digest.update(chunk)
        return digest.hexdigest()
    except OSError:
        return None


def console_log_path(repo: Path, env: Optional[dict[str, str]] = None) -> Path:
    source = os.environ if env is None else env
    settings = source.get("HL2RS_SETTINGS", "").strip()
    if settings:
        parent = Path(settings).parent
        if not parent.is_absolute():
            parent = repo / parent
        return parent / "console.log"
    return repo / "bin" / "console.log"


def read_build_info(repo: Path) -> dict:
    info_path = repo / "bin" / "build-bevy-info.json"
    if not info_path.is_file():
        return {}
    try:
        data = json.loads(info_path.read_text(encoding="utf-8-sig"))
    except (OSError, ValueError):
        return {}
    return data if isinstance(data, dict) else {}


def build_info_has_debug_tools(info: dict) -> Optional[bool]:
    plugins = info.get("plugins")
    if not isinstance(plugins, str):
        return None
    selected = {part.strip() for part in plugins.split(",") if part.strip()}
    return "debug-tools" in selected


def is_port_free(port: int, host: str = "127.0.0.1") -> bool:
    if not (1 <= port <= 65535):
        return False
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.settimeout(0.1)
        if probe.connect_ex((host, port)) == 0:
            return False
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        if os.name != "nt":
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            sock.bind((host, port))
            return True
        except OSError:
            return False


def pick_free_port(preferred: int = DEFAULT_PORT, host: str = "127.0.0.1") -> int:
    if is_port_free(preferred, host=host):
        return preferred
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind((host, 0))
        return int(sock.getsockname()[1])


def _parse_host_port(endpoint: str) -> Optional[dict[str, object]]:
    text = endpoint.strip()
    if text.startswith("[") and "]:" in text:
        host, port_text = text[1:].rsplit("]:", 1)
    elif ":" in text:
        host, port_text = text.rsplit(":", 1)
    else:
        return None
    if not port_text.isdigit():
        return None
    return {"address": host, "port": int(port_text)}


def _find_posix_listeners(pid: int) -> list[dict[str, object]]:
    listeners: list[dict[str, object]] = []
    if shutil.which("lsof"):
        proc = subprocess.run(
            ["lsof", "-nP", "-a", "-p", str(pid), "-iTCP", "-sTCP:LISTEN"],
            capture_output=True,
            text=True,
            check=False,
        )
        for line in proc.stdout.splitlines()[1:]:
            match = re.search(r"(\S+)\s+\(LISTEN\)\s*$", line)
            if match:
                parsed = _parse_host_port(match.group(1))
                if parsed:
                    listeners.append(parsed)
        return listeners
    if shutil.which("ss"):
        proc = subprocess.run(
            ["ss", "-ltnp"],
            capture_output=True,
            text=True,
            check=False,
        )
        pid_token = f"pid={pid},"
        for line in proc.stdout.splitlines():
            if pid_token not in line:
                continue
            fields = line.split()
            if len(fields) >= 4:
                parsed = _parse_host_port(fields[3])
                if parsed:
                    listeners.append(parsed)
    return listeners


def _find_running_games_posix() -> list[dict[str, object]]:
    pgrep = shutil.which("pgrep")
    if not pgrep:
        return []
    proc = subprocess.run([pgrep, "-x", "hl2-bevy"], capture_output=True, text=True, check=False)
    if proc.returncode not in (0, 1):
        return []
    games: list[dict[str, object]] = []
    for raw_pid in proc.stdout.splitlines():
        raw_pid = raw_pid.strip()
        if not raw_pid.isdigit():
            continue
        pid = int(raw_pid)
        exe = ""
        proc_exe = Path(f"/proc/{pid}/exe")
        try:
            if proc_exe.exists():
                exe = os.readlink(proc_exe)
        except OSError:
            exe = ""
        if not exe:
            ps_proc = subprocess.run(
                ["ps", "-p", str(pid), "-o", "comm="],
                capture_output=True,
                text=True,
                check=False,
            )
            exe = ps_proc.stdout.strip()
        games.append({
            "pid": pid,
            "exe": exe,
            "listeners": _find_posix_listeners(pid),
        })
    return games


def _find_running_games_windows() -> list[dict[str, object]]:
    script = (
        "$procs = @(Get-Process -Name 'hl2-bevy' -ErrorAction SilentlyContinue | Select-Object Id, Path);"
        "$pids = @($procs | ForEach-Object { $_.Id });"
        "$listeners = @();"
        "if ($pids.Count -gt 0) {"
        "$listeners = @(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue "
        "| Where-Object { $pids -contains $_.OwningProcess } "
        "| Select-Object OwningProcess, LocalAddress, LocalPort)"
        "}"
        "[pscustomobject]@{ processes = $procs; listeners = $listeners } | ConvertTo-Json -Depth 3 -Compress"
    )
    powershell = shutil.which("powershell") or shutil.which("pwsh")
    if not powershell:
        return []
    proc = subprocess.run(
        [powershell, "-NoProfile", "-Command", script],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    raw = proc.stdout.strip()
    if proc.returncode != 0 or not raw:
        return []
    try:
        data = json.loads(raw)
    except ValueError:
        return []
    procs = data.get("processes") or []
    if isinstance(procs, dict):
        procs = [procs]
    listeners = data.get("listeners") or []
    if isinstance(listeners, dict):
        listeners = [listeners]
    by_pid: dict[int, list[dict[str, object]]] = {}
    for entry in listeners:
        if not isinstance(entry, dict):
            continue
        owner = entry.get("OwningProcess")
        port = entry.get("LocalPort")
        if isinstance(owner, int) and isinstance(port, int):
            by_pid.setdefault(owner, []).append({
                "address": str(entry.get("LocalAddress", "127.0.0.1")),
                "port": port,
            })
    games: list[dict[str, object]] = []
    for entry in procs:
        if not isinstance(entry, dict):
            continue
        pid = entry.get("Id")
        if isinstance(pid, int):
            games.append({
                "pid": pid,
                "exe": str(entry.get("Path") or "hl2-bevy.exe"),
                "listeners": by_pid.get(pid, []),
            })
    return games


def find_running_games() -> list[dict[str, object]]:
    """Return active hl2-bevy PIDs, executable names/paths, and TCP listeners (never cmdline args)."""
    if os.name == "nt":
        return _find_running_games_windows()
    return _find_running_games_posix()


def build_launch_command(
    exe: Path,
    port: int,
    password: str,
    keep_args: list[str],
    run_args: list[str],
) -> list[str]:
    return [
        str(exe),
        "-condebug",
        "-usercon",
        "+ip",
        "127.0.0.1",
        "+rcon_password",
        password,
        "+hostport",
        str(port),
        "+con_timestamp",
        "1",
        "+sv_rcon_log",
        "1",
        "+showconsole",
        *keep_args,
        *run_args,
    ]


def build_script_command(
    repo: Path,
    profile: str = "release",
    plugins: str = DEFAULT_PLUGINS,
) -> list[str]:
    if os.name == "nt":
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
        if plugins:
            cmd.extend(["-Plugins", plugins])
        return cmd

    cmd = ["bash", str(repo / "scripts" / "build-bevy.sh")]
    if profile == "debug":
        cmd.append("--debug")
    elif profile == "dist":
        cmd.append("--dist")
    if plugins:
        cmd.extend(["--plugins", plugins])
    return cmd


def run_build_script(
    repo: Path,
    profile: str = "release",
    plugins: str = DEFAULT_PLUGINS,
    log_stream: Optional[TextIO] = None,
) -> int:
    cmd = build_script_command(repo, profile=profile, plugins=plugins)
    proc = subprocess.run(cmd, cwd=str(repo), check=False, stdout=log_stream, stderr=subprocess.STDOUT if log_stream else None)
    return int(proc.returncode)


def write_private_text(path: Path, content: str) -> None:
    if os.name == "nt":
        path.write_text(content, encoding="utf-8")
        return
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    fh: Optional[TextIO] = None
    try:
        os.chmod(path, 0o600)
        fh = os.fdopen(fd, "w", encoding="utf-8")
        fd = -1
        fh.write(content)
    finally:
        if fh is not None:
            fh.close()
        elif fd >= 0:
            os.close(fd)


def _atomic_write_text(path: Path, content: str) -> None:
    tmp = path.with_name(f".{path.name}.tmp")
    tmp.write_text(content, encoding="utf-8")
    tmp.replace(path)


def _read_optional_text(path: Path) -> Optional[str]:
    if not path.is_file():
        return None
    try:
        return path.read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        return None


def collect_status(repo: Path, session_dir: Path, preferred_port: int = DEFAULT_PORT) -> dict[str, object]:
    exe = executable_path(repo)
    build_info = read_build_info(repo)
    has_debug = build_info_has_debug_tools(build_info)
    legacy_pw = Path(tempfile.gettempdir()) / "hl2rs-rcon-password"
    port_text = _read_optional_text(session_dir / "port")
    pid_text = _read_optional_text(session_dir / "pid")
    return {
        "repo": str(repo),
        "executable": str(exe),
        "executable_exists": exe.is_file(),
        "build_info": {
            "profile": build_info.get("profile"),
            "plugins": build_info.get("plugins"),
            "debug_tools_enabled": has_debug,
        },
        "preferred_port": preferred_port,
        "preferred_port_free": is_port_free(preferred_port),
        "running_games": find_running_games(),
        "session_dir": str(session_dir),
        "session": {
            "exists": session_dir.is_dir(),
            "has_password": (session_dir / "rcon_password").is_file(),
            "legacy_password_present": legacy_pw.is_file(),
            "port": int(port_text) if port_text and port_text.isdigit() else None,
            "console_log_path": _read_optional_text(session_dir / "console_log_path"),
            "hl2rs_dir": _read_optional_text(session_dir / "hl2rs_dir"),
            "pid": int(pid_text) if pid_text and pid_text.isdigit() else None,
            "game_log_exists": (session_dir / "game.log").is_file(),
        },
    }


def _emit_line(line: str, out_stream: TextIO, game_log: TextIO) -> None:
    out_stream.write(line)
    out_stream.flush()
    game_log.write(line)
    game_log.flush()


def _read_repackage_request(path: Path) -> tuple[str, str]:
    profile = "release"
    plugins = DEFAULT_PLUGINS
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        if isinstance(data, dict):
            if isinstance(data.get("profile"), str) and data["profile"].strip():
                profile = data["profile"].strip()
            if isinstance(data.get("plugins"), str) and data["plugins"].strip():
                plugins = data["plugins"].strip()
    except (OSError, ValueError):
        pass
    return profile, plugins


def run_supervisor(
    session_dir: Path,
    repo: Path,
    exe: Path,
    port: int,
    password: str,
    keep_args: list[str],
    initial_args: list[str],
    builder: Callable[..., int] = run_build_script,
    env: Optional[dict[str, str]] = None,
    stdout: Optional[TextIO] = None,
) -> int:
    out_stream = sys.stdout if stdout is None else stdout
    current_args = list(initial_args)
    game_log_path = session_dir / "game.log"
    con_log_text = _read_optional_text(session_dir / "console_log_path") or str(console_log_path(repo, env))
    out_stream.write(
        f"Game log: {game_log_path}; console log: {con_log_text}; remote console port {port}\n"
    )
    out_stream.flush()

    while True:
        cmd = build_launch_command(exe, port, password, keep_args, current_args)
        with subprocess.Popen(
            cmd,
            cwd=str(repo),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
            env=env,
        ) as proc:
            _atomic_write_text(session_dir / "pid", f"{proc.pid}\n")
            with game_log_path.open("a", encoding="utf-8", errors="replace") as game_log:
                if proc.stdout is not None:
                    for line in proc.stdout:
                        _emit_line(line, out_stream, game_log)
            returncode = int(proc.wait())
        (session_dir / "pid").unlink(missing_ok=True)

        repackage_file = session_dir / "repackage"
        if repackage_file.exists():
            profile, plugins = _read_repackage_request(repackage_file)
            repackage_file.unlink(missing_ok=True)
            build_rc = int(builder(repo, profile=profile, plugins=plugins))
            if build_rc != 0:
                with game_log_path.open("a", encoding="utf-8", errors="replace") as game_log:
                    _emit_line(f"AGENT_SESSION_EXIT {build_rc}\n", out_stream, game_log)
                return build_rc

        resume_file = session_dir / "resume"
        if resume_file.exists():
            resume_text = resume_file.read_text(encoding="utf-8", errors="replace").strip()
            resume_file.unlink(missing_ok=True)
            current_args = shlex.split(resume_text, posix=True) if resume_text else []
            with game_log_path.open("a", encoding="utf-8", errors="replace") as game_log:
                _emit_line(f"AGENT_SESSION_RESTART {' '.join(current_args)}\n", out_stream, game_log)
            continue

        with game_log_path.open("a", encoding="utf-8", errors="replace") as game_log:
            _emit_line(f"AGENT_SESSION_EXIT {returncode}\n", out_stream, game_log)
        return returncode


def _split_arg_groups(items: list[str]) -> list[str]:
    result: list[str] = []
    for item in items:
        if item.strip():
            result.extend(shlex.split(item, posix=True))
    return result


def cmd_status(args: argparse.Namespace) -> int:
    repo = args.repo.resolve() if args.repo else default_repo_dir()
    session_dir = args.session if args.session else rcon.default_session_dir()
    status = collect_status(repo, session_dir, preferred_port=args.port)
    if args.json:
        print(json.dumps(status, indent=2))
        return 0

    print(f"Repo: {status['repo']}")
    exe_state = "ready" if status["executable_exists"] else "missing"
    print(f"Executable: {status['executable']} ({exe_state})")
    info = status["build_info"]
    if info.get("profile") or info.get("plugins"):
        print(f"Build info: profile={info.get('profile')} plugins={info.get('plugins')}")
    if info.get("debug_tools_enabled") is False:
        print("Warning: debug-tools plugin is not selected; ui_dump/npc_dump/agent_status/agent_resume are disabled.")
    running = status["running_games"]
    if running:
        for game in running:
            ports = ", ".join(f"{l['address']}:{l['port']}" for l in game["listeners"]) or "no TCP listener yet"
            print(f"Running hl2-bevy: PID {game['pid']} ({game['exe']}) listening on {ports}")
    else:
        port_state = "free" if status["preferred_port_free"] else "in use"
        print(f"Running hl2-bevy: none (port {status['preferred_port']} is {port_state})")
    sess = status["session"]
    print(
        f"Session dir: {status['session_dir']} (exists={sess['exists']}, "
        f"has_password={sess['has_password']}, port={sess['port']}, pid={sess['pid']})"
    )
    return 0


def attach_to_game(game: dict, session_dir: Path, rejected: set[tuple[int, str]]) -> Optional[Path]:
    pid = game["pid"]
    published_dir = rcon.default_session_dir({}) / f"live-{pid}"
    ports = {
        listener["port"] for listener in game["listeners"]
        if listener["address"] == "127.0.0.1"
    }
    for candidate in dict.fromkeys((session_dir, published_dir)):
        if _read_optional_text(candidate / "pid") != str(pid):
            continue
        if not _read_optional_text(candidate / "console_log_path"):
            continue
        try:
            port = rcon.read_port(session_dir=candidate, env={})
            if not (candidate / "port").is_file() or port not in ports:
                continue
            password = rcon.read_password(session_dir=candidate, env={}, allow_legacy_fallback=False)
            if not password or (port, password) in rejected:
                continue
        except (OSError, ValueError):
            continue
        try:
            reply = rcon.exec_command("echo HL2RS_ATTACH", port=port, password=password, timeout=2.0)
        except rcon.RconAuthError:
            rejected.add((port, password))
            continue
        except OSError:
            continue
        if reply.strip() == "HL2RS_ATTACH":
            return candidate.resolve()
    return None


def cmd_attach(args: argparse.Namespace) -> int:
    session_dir = args.session if args.session else rcon.default_session_dir()
    if not math.isfinite(args.wait) or args.wait < 0:
        print("attach: --wait must be a finite, non-negative number of seconds.", file=sys.stderr)
        return 2
    deadline = time.monotonic() + args.wait
    pid = args.pid
    rejected: set[tuple[int, str]] = set()
    while True:
        running = find_running_games()
        if pid is None and len(running) > 1:
            pids = ", ".join(str(game["pid"]) for game in running)
            print(f"attach: multiple games are running ({pids}). Select one with --pid.", file=sys.stderr)
            return 1
        games = [game for game in running if pid is None or game["pid"] == pid]
        if not games:
            print("attach: no matching HL2-RS game is running.", file=sys.stderr)
            return 1
        game = games[0]
        pid = game["pid"]
        attached = attach_to_game(game, session_dir, rejected)
        if attached:
            print(attached)
            return 0
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        time.sleep(min(0.5, remaining))
    print(
        f"attach: no authenticated session for PID {pid}. "
        "Run agent_rcon in that game's local developer console, then retry attach.",
        file=sys.stderr,
    )
    return 1


def cmd_launch(args: argparse.Namespace) -> int:
    repo = args.repo.resolve() if args.repo else default_repo_dir()
    session_dir = args.session if args.session else rcon.default_session_dir()

    running = find_running_games()
    if running:
        details = ", ".join(f"PID {g['pid']}" for g in running)
        print(
            f"hl2-bevy is already running ({details}); refusing to launch a duplicate.",
            file=sys.stderr,
        )
        return 1

    exe = executable_path(repo)
    if args.build:
        rc = run_build_script(repo)
        if rc != 0:
            return rc
    elif not exe.is_file():
        hint = r".\scripts\build-bevy.ps1" if os.name == "nt" else "bash scripts/build-bevy.sh"
        print(
            f"Packaged binary missing at {exe}. Build it with `{hint}` or pass --build.",
            file=sys.stderr,
        )
        return 1

    info = read_build_info(repo)
    if build_info_has_debug_tools(info) is False:
        print(
            "Warning: packaged build lacks debug-tools plugin; agent_status/ui_dump/npc_dump/agent_resume will be unavailable.",
            file=sys.stderr,
        )

    port_spec = args.port if args.port is not None else (os.environ.get("RCON_PORT", "").strip() or "auto")
    if port_spec.lower() == "auto":
        if is_port_free(DEFAULT_PORT):
            port = DEFAULT_PORT
        else:
            port = pick_free_port(DEFAULT_PORT)
            print(f"Port {DEFAULT_PORT} is busy; selected free loopback port {port}.")
    else:
        if not port_spec.isdigit():
            print(f"Invalid port: {port_spec}", file=sys.stderr)
            return 2
        port = int(port_spec)
        if not is_port_free(port):
            print(f"Port {port} is already in use; choose another port or pass --port auto.", file=sys.stderr)
            return 1

    session_dir.mkdir(parents=True, exist_ok=True)
    if os.name != "nt":
        os.chmod(session_dir, 0o700)

    password = secrets.token_hex(16)
    write_private_text(session_dir / "rcon_password", f"{password}\n")
    _atomic_write_text(session_dir / "console_log_path", f"{console_log_path(repo)}\n")
    _atomic_write_text(session_dir / "port", f"{port}\n")
    _atomic_write_text(session_dir / "hl2rs_dir", f"{repo}\n")
    for stale in ("resume", "repackage", "watch.cursor", "game.log", "pid"):
        (session_dir / stale).unlink(missing_ok=True)

    keep_env = os.environ.get("KEEP", "")
    extra_env = os.environ.get("EXTRA", "")
    keep_args = _split_arg_groups([keep_env, *args.keep])
    trailing = list(args.trailing)
    if trailing and trailing[0] == "--":
        trailing = trailing[1:]
    initial_args = _split_arg_groups([extra_env, *args.extra]) + trailing

    return run_supervisor(
        session_dir=session_dir,
        repo=repo,
        exe=exe,
        port=port,
        password=password,
        keep_args=keep_args,
        initial_args=initial_args,
    )


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Inspect or launch an HL2-RS RCON agent session.")
    sub = parser.add_subparsers(dest="subcommand", required=True)

    p_status = sub.add_parser("status", help="Inspect running HL2-RS instances, build info, and session files.")
    p_status.add_argument("--repo", type=Path, default=None, help="HL2-RS repository root.")
    p_status.add_argument("--session", type=Path, default=None, help="Session directory path.")
    p_status.add_argument("--port", type=int, default=DEFAULT_PORT, help="Preferred RCON port to check (27015).")
    p_status.add_argument("--json", action="store_true", help="Print status as JSON.")

    p_attach = sub.add_parser("attach", help="Authenticate to a running game and print its session directory.")
    p_attach.add_argument("--session", type=Path, default=None, help="Existing supervisor session directory.")
    p_attach.add_argument("--pid", type=int, default=None, help="Game PID to attach when multiple games are running.")
    p_attach.add_argument("--wait", type=float, default=0.0, help="Seconds to wait for a running game to publish RCON.")

    p_launch = sub.add_parser("launch", help="Launch HL2-RS with RCON and console logging in a restart loop.")
    p_launch.add_argument("--repo", type=Path, default=None, help="HL2-RS repository root.")
    p_launch.add_argument("--session", type=Path, default=None, help="Session directory path.")
    p_launch.add_argument("--port", default=None, help="Loopback RCON port or 'auto' (default: auto).")
    p_launch.add_argument("--build", action="store_true", help="Run the platform build script before launching.")
    p_launch.add_argument("--keep", action="append", default=[], help="Extra game arguments kept across restarts.")
    p_launch.add_argument("--extra", action="append", default=[], help="Extra game arguments for the first launch.")
    p_launch.add_argument("trailing", nargs=argparse.REMAINDER, help="Extra arguments passed to hl2-bevy.")
    return parser.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    if args.subcommand == "status":
        return cmd_status(args)
    if args.subcommand == "attach":
        return cmd_attach(args)
    if args.subcommand == "launch":
        return cmd_launch(args)
    return 2


if __name__ == "__main__":
    sys.exit(main())
