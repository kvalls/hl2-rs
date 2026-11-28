#!/usr/bin/env python3
"""Run console commands in a running HL2-RS game through Valve's RCON protocol.

Usage:
    python3 rcon.py "say hello"
    python3 rcon.py --wait 30 ui_dump
    RCON_COMMAND='getpos' python3 rcon.py

Settings (environment or flags): RCON_COMMAND / positional command, RCON_PASSWORD or
RCON_PASSWORD_FILE (defaults to <session>/rcon_password, then <temp>/hl2rs-rcon-password),
RCON_HOST (127.0.0.1), RCON_PORT (defaults to <session>/port, then 27015), RCON_TIMEOUT (10 s),
SESSION (defaults to <temp>/hl2rs-agent-session).
"""

import argparse
import ipaddress
import os
import re
import socket
import struct
import sys
import tempfile
import time
from pathlib import Path
from typing import Callable, Optional

AUTH = 3
AUTH_RESPONSE = 2
EXEC = 2
RESPONSE = 0
AUTH_FAILED_ID = -1


class RconAuthError(RuntimeError):
    """Raised when the game refuses the RCON password."""


def default_session_dir(env: Optional[dict[str, str]] = None) -> Path:
    source = os.environ if env is None else env
    raw = source.get("SESSION", "").strip()
    if raw:
        return Path(raw)
    return Path(tempfile.gettempdir()) / "hl2rs-agent-session"


def live_session_pid(session: Path) -> Optional[int]:
    match = re.fullmatch(r"live-([1-9][0-9]*)", session.name)
    return int(match[1]) if match else None


def read_password(
    password: Optional[str] = None,
    password_file: Optional[Path] = None,
    session_dir: Optional[Path] = None,
    env: Optional[dict[str, str]] = None,
    allow_legacy_fallback: bool = True,
) -> str:
    source = os.environ if env is None else env
    if password is not None:
        return password
    if "RCON_PASSWORD" in source:
        return source["RCON_PASSWORD"]
    if password_file is not None:
        return Path(password_file).read_text(encoding="utf-8").strip()
    env_file = source.get("RCON_PASSWORD_FILE", "").strip()
    if env_file:
        return Path(env_file).read_text(encoding="utf-8").strip()

    session = session_dir or default_session_dir(source)
    candidates = [session / "rcon_password"]
    if allow_legacy_fallback:
        candidates.append(Path(tempfile.gettempdir()) / "hl2rs-rcon-password")
    for candidate in candidates:
        if candidate.is_file():
            return candidate.read_text(encoding="utf-8").strip()
    expected = " or ".join(str(c) for c in candidates)
    raise FileNotFoundError(f"no RCON password file found at {expected}")


def read_port(
    port: Optional[int] = None,
    session_dir: Optional[Path] = None,
    env: Optional[dict[str, str]] = None,
) -> int:
    source = os.environ if env is None else env
    if port is not None:
        return int(port)
    env_port = source.get("RCON_PORT", "").strip()
    if env_port:
        return int(env_port)
    session = session_dir or default_session_dir(source)
    port_file = session / "port"
    if port_file.is_file():
        raw = port_file.read_text(encoding="utf-8").strip()
        if raw:
            return int(raw)
    return 27015


def read_host(
    host: Optional[str] = None,
    env: Optional[dict[str, str]] = None,
) -> str:
    source = os.environ if env is None else env
    if host is not None:
        return host
    return source.get("RCON_HOST", "").strip() or "127.0.0.1"


def send_packet(sock: socket.socket, packet_id: int, kind: int, body: str) -> None:
    data = struct.pack("<ii", packet_id, kind) + body.encode("utf-8") + b"\x00\x00"
    sock.sendall(struct.pack("<i", len(data)) + data)


def read_exact(sock: socket.socket, size: int) -> bytes:
    chunks = []
    remaining = size
    while remaining > 0:
        chunk = sock.recv(remaining)
        if not chunk:
            raise ConnectionError("the game closed the connection")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def receive_packet(sock: socket.socket) -> tuple[int, int, str]:
    (size,) = struct.unpack("<i", read_exact(sock, 4))
    if size < 10:
        raise ConnectionError(f"invalid RCON packet size: {size}")
    data = read_exact(sock, size)
    packet_id, kind = struct.unpack("<ii", data[:8])
    body = data[8:-2].split(b"\x00", 1)[0].decode("utf-8", errors="replace")
    return packet_id, kind, body


def exec_command(
    command: str,
    host: str = "127.0.0.1",
    port: int = 27015,
    password: str = "",
    timeout: float = 10.0,
) -> str:
    address = "127.0.0.1" if host.lower() == "localhost" else host
    try:
        loopback = ipaddress.ip_address(address)
    except ValueError:
        raise ValueError("RCON requires a loopback address") from None
    if not loopback.is_loopback:
        raise ValueError("RCON requires a loopback address")
    with socket.create_connection((str(loopback), port), timeout=timeout) as sock:
        sock.settimeout(timeout)
        send_packet(sock, 1, AUTH, password)
        while True:
            packet_id, kind, _ = receive_packet(sock)
            if kind == AUTH_RESPONSE:
                if packet_id == AUTH_FAILED_ID:
                    raise RconAuthError("the game refused the password")
                break
        send_packet(sock, 2, EXEC, command)
        send_packet(sock, 3, RESPONSE, "")
        output: list[str] = []
        while True:
            packet_id, _, body = receive_packet(sock)
            if packet_id == 3:
                break
            output.append(body)
    return "".join(output)


def game_log_exit_line(
    session_dir: Optional[Path],
    since_wall: Optional[float] = None,
) -> Optional[str]:
    if session_dir is None:
        return None
    game_log = session_dir / "game.log"
    if not game_log.is_file():
        return None
    if since_wall is not None:
        try:
            if game_log.stat().st_mtime < since_wall:
                return None
        except OSError:
            return None
    try:
        text = game_log.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    for line in reversed(text.splitlines()):
        stripped = line.strip()
        if stripped.startswith("AGENT_SESSION_EXIT"):
            return stripped
        if stripped.startswith("AGENT_SESSION_RESTART"):
            return None
    return None


def wait_and_exec(
    command: str,
    host: str = "127.0.0.1",
    port: int = 27015,
    password: str = "",
    timeout: float = 10.0,
    wait_seconds: float = 0.0,
    session_dir: Optional[Path] = None,
    poll_interval: float = 0.25,
    port_getter: Optional[Callable[[], int]] = None,
    password_getter: Optional[Callable[[], str]] = None,
    exit_grace_seconds: float = 0.5,
) -> str:
    start_mono = time.monotonic()
    start_wall = time.time()
    deadline = start_mono + max(0.0, wait_seconds)
    while True:
        try:
            current_port = port_getter() if port_getter is not None else port
            current_pw = password_getter() if password_getter is not None else password
            return exec_command(
                command,
                host=host,
                port=current_port,
                password=current_pw,
                timeout=timeout,
            )
        except RconAuthError:
            raise
        except (OSError, ConnectionError) as error:
            now_mono = time.monotonic()
            if wait_seconds <= 0 or now_mono >= deadline:
                raise error
            if now_mono - start_mono >= exit_grace_seconds:
                exited = game_log_exit_line(session_dir, since_wall=start_wall - 2.0)
                if exited:
                    raise RuntimeError(f"game exited before RCON became ready ({exited})")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise error
            time.sleep(min(poll_interval, remaining))


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Send a command to HL2-RS over Source RCON.")
    parser.add_argument("--session", type=Path, default=None, help="Session directory path.")
    parser.add_argument("--host", default=None, help="Loopback RCON address (default: 127.0.0.1).")
    parser.add_argument("--port", type=int, default=None, help="RCON port (default: session port or 27015).")
    parser.add_argument(
        "--password-file",
        type=Path,
        default=None,
        help="File containing the RCON password.",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=float(os.environ.get("RCON_TIMEOUT", "10")),
        help="Socket timeout in seconds (default: 10).",
    )
    parser.add_argument(
        "--wait",
        type=float,
        default=0.0,
        help="Seconds to retry connection errors while the game starts up (default: 0).",
    )
    parser.add_argument("--greet", choices=("Codex", "Claude", "Gemini"),
                        help="Enable chat mode, then send a connection greeting.")
    parser.add_argument("command", nargs="*", help="Command to execute (or set RCON_COMMAND).")
    return parser.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    command = " ".join(args.command).strip() if args.command else os.environ.get("RCON_COMMAND", "").strip()
    if args.greet and args.command:
        print("rcon: --greet takes the place of a command.", file=sys.stderr)
        return 1
    if not command and not args.greet:
        print("rcon: pass a command argument or set RCON_COMMAND", file=sys.stderr)
        return 1

    session_dir = args.session if args.session is not None else default_session_dir()
    env = {} if args.session is not None or live_session_pid(session_dir) is not None else os.environ
    allow_legacy = args.session is None and env is os.environ and not env.get("SESSION", "").strip()
    try:
        host = read_host(args.host, env=env)
        def send(line: str) -> str:
            return wait_and_exec(
                line,
                host=host,
                port=27015,
                password="",
                timeout=args.timeout,
                wait_seconds=args.wait,
                session_dir=session_dir,
                port_getter=lambda: read_port(args.port, session_dir=session_dir, env=env),
                password_getter=lambda: read_password(
                    password_file=args.password_file,
                    session_dir=session_dir,
                    env=env,
                    allow_legacy_fallback=allow_legacy,
                ),
            )
        if args.greet:
            enabled = "chat_mode = 1" in send("chat_mode 1").splitlines()
            hint = ("Type a message in the console to talk to me." if enabled
                    else "Type say <message> in the console to talk to me.")
            greeting = f"{args.greet} connected. {hint}"
            output = send(f"say {greeting}")
            if f"Console: {greeting}" not in output.splitlines():
                raise RuntimeError("game did not confirm the greeting")
        else:
            output = send(command)
    except (OSError, ValueError, RuntimeError) as error:
        print(f"rcon: {error}", file=sys.stderr)
        return 1

    if output:
        print(output, end="" if output.endswith("\n") else "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
