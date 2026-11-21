#!/usr/bin/env python3
"""Rebuild (if needed) and restart a running HL2-RS RCON session back at its current state.

On POSIX, runs the packaging build script while the game stays running, captures `agent_resume`,
writes `<session>/resume`, and sends `quit` so `session.py` relaunches into the new build.

On Windows, `bin/hl2-bevy.exe` cannot be overwritten while running, so `restart.py` compiles
with `cargo build` while the game keeps running, writes `<session>/repackage` + `<session>/resume`,
sends `quit`, and lets `session.py` finish packaging with `scripts/build-bevy.ps1` before relaunching.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Callable, Optional

_HERE = Path(__file__).resolve().parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

import rcon  # noqa: E402
import session  # noqa: E402


def parse_resume_output(output: str) -> Optional[str]:
    for line in reversed(output.splitlines()):
        if line == "agent_resume":
            return ""
        if line.startswith("agent_resume "):
            return line[len("agent_resume "):].strip()
    return None


def cargo_build_command(
    repo: Path,
    profile: str = "release",
    plugins: str = session.DEFAULT_PLUGINS,
) -> list[str]:
    cmd = [
        "cargo",
        "build",
        "--locked",
        "-p",
        "hl2-app",
        "--bin",
        "hl2-bevy",
        "--no-default-features",
    ]
    if plugins and plugins != "none":
        cmd.extend(["--features", plugins])
    if profile == "dist":
        cmd.extend(["--profile", "dist"])
    elif profile != "debug":
        cmd.append("--release")
    return cmd


def run_cargo_build(
    repo: Path,
    profile: str = "release",
    plugins: str = session.DEFAULT_PLUGINS,
) -> int:
    cmd = cargo_build_command(repo, profile=profile, plugins=plugins)
    proc = subprocess.run(cmd, cwd=str(repo), check=False)
    return int(proc.returncode)


def _atomic_write(path: Path, content: str) -> None:
    tmp = path.with_name(f".{path.name}.tmp")
    tmp.write_text(content, encoding="utf-8")
    tmp.replace(path)


def restart_session(
    session_dir: Path,
    repo: Path,
    no_build: bool = False,
    profile: Optional[str] = None,
    plugins: Optional[str] = None,
    is_windows: Optional[bool] = None,
    rcon_exec: Callable[..., str] = rcon.exec_command,
    script_builder: Callable[..., int] = session.run_build_script,
    cargo_builder: Callable[..., int] = run_cargo_build,
) -> int:
    win = (os.name == "nt") if is_windows is None else is_windows
    info = session.read_build_info(repo)
    chosen_profile = profile or (info.get("profile") if isinstance(info.get("profile"), str) else None) or "release"
    chosen_plugins = plugins or (info.get("plugins") if isinstance(info.get("plugins"), str) else None) or session.DEFAULT_PLUGINS

    host = rcon.read_host()
    port = rcon.read_port(session_dir=session_dir)
    password = rcon.read_password(session_dir=session_dir)

    write_repackage = False
    if not no_build:
        if win:
            rc = int(cargo_builder(repo, profile=chosen_profile, plugins=chosen_plugins))
            if rc != 0:
                print("restart: cargo build failed; leaving the running game untouched.", file=sys.stderr)
                return rc
            write_repackage = True
        else:
            rc = int(script_builder(repo, profile=chosen_profile, plugins=chosen_plugins))
            if rc != 0:
                print("restart: build failed; leaving the running game untouched.", file=sys.stderr)
                return rc

    output = rcon_exec("agent_resume", host=host, port=port, password=password, timeout=10.0)
    resume_args = parse_resume_output(output)
    if resume_args is None:
        print(
            "restart: agent_resume did not return resume arguments "
            "(ensure the build includes the debug-tools plugin); leaving the running game untouched.",
            file=sys.stderr,
        )
        return 1

    if write_repackage:
        _atomic_write(
            session_dir / "repackage",
            json.dumps({"profile": chosen_profile, "plugins": chosen_plugins}) + "\n",
        )
    _atomic_write(session_dir / "resume", f"{resume_args}\n" if resume_args else "")
    print(f"Resume arguments: {resume_args or '(main menu)'}")

    try:
        rcon_exec("quit", host=host, port=port, password=password, timeout=5.0)
    except (OSError, ConnectionError):
        pass
    return 0


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build and restart an HL2-RS RCON session.")
    parser.add_argument("--session", type=Path, default=None, help="Session directory path.")
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Restart into the existing packaged binary without rebuilding (or set BUILD=0).",
    )
    parser.add_argument("--profile", choices=["debug", "release", "dist"], default=None, help="Build profile override.")
    parser.add_argument("--plugins", default=None, help="Comma-separated plugins override or 'none'.")
    return parser.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    session_dir = args.session if args.session is not None else rcon.default_session_dir()
    dir_file = session_dir / "hl2rs_dir"
    if dir_file.is_file():
        repo = Path(dir_file.read_text(encoding="utf-8").strip()).resolve()
    else:
        repo = session.default_repo_dir()
    no_build = args.no_build or os.environ.get("BUILD") == "0"
    try:
        return restart_session(
            session_dir=session_dir,
            repo=repo,
            no_build=no_build,
            profile=args.profile,
            plugins=args.plugins,
        )
    except (OSError, ValueError, RuntimeError) as error:
        print(f"restart: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
