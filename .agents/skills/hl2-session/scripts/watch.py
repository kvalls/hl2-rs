#!/usr/bin/env python3
"""Forward player chat and critical game events from a running HL2-RS RCON session.

Streaming mode (default, for Monitor tools): sends `agent_status thinking` as soon as a
`Player:` line arrives, prints the line, clears the timer on `Console:`, sends
`agent_status waiting` after `--timeout` seconds (default 60) without a reply, and exits
on `AGENT_SESSION_EXIT`.

One-shot mode (`--once` or `WATCH_ONCE=1`): tracks byte offsets in `<session>/watch.cursor`,
sends `agent_status thinking` on the next `Player:` line, prints the event, and exits 0.
"""

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import re
import sys
import time
from typing import Callable, Iterator, Optional, TextIO

_HERE = Path(__file__).resolve().parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

import rcon  # noqa: E402

STAMP = re.compile(r"^\[\s*\d+(?:\.\d+)?\]\s+")
TIMEOUT_STATUS = "waiting"


class SessionLockedError(RuntimeError):
    """Raised when another watcher already holds the session lock."""


@contextmanager
def acquire_lock(lock_path: Path) -> Iterator[None]:
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    fh = lock_path.open("a+b")
    locked = False
    try:
        if os.name == "nt":
            import msvcrt

            fh.seek(0, os.SEEK_END)
            if fh.tell() == 0:
                fh.write(b"\x00")
                fh.flush()
            fh.seek(0)
            try:
                msvcrt.locking(fh.fileno(), msvcrt.LK_NBLCK, 1)
                locked = True
            except OSError as error:
                raise SessionLockedError("The session already has a watcher") from error
        else:
            import fcntl

            try:
                fcntl.flock(fh.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                locked = True
            except BlockingIOError as error:
                raise SessionLockedError("The session already has a watcher") from error
        yield
    finally:
        if locked:
            try:
                if os.name == "nt":
                    import msvcrt

                    fh.seek(0)
                    msvcrt.locking(fh.fileno(), msvcrt.LK_UNLCK, 1)
                else:
                    import fcntl

                    fcntl.flock(fh.fileno(), fcntl.LOCK_UN)
            except OSError:
                pass
        fh.close()


def send_notice(session: Path, status: str) -> bool:
    try:
        port = rcon.read_port(session_dir=session)
        password = rcon.read_password(session_dir=session)
        output = rcon.exec_command(
            f"agent_status {status}",
            host=rcon.read_host(),
            port=port,
            password=password,
            timeout=3.0,
        )
    except (OSError, ValueError, RuntimeError):
        print("AGENT_STATUS_FAILED: the game did not confirm the notice", flush=True)
        return False
    if output.strip():
        print("AGENT_STATUS_FAILED: the game did not confirm the notice", flush=True)
        return False
    return True


def session_log_paths(session: Path) -> list[str]:
    paths: list[str] = []
    con_file = session / "console_log_path"
    if con_file.is_file():
        con_path = con_file.read_text(encoding="utf-8", errors="replace").strip()
        if con_path:
            paths.append(con_path)
    paths.append(str(session / "game.log"))
    return paths


def classify_line(line: str) -> Optional[str]:
    text = STAMP.sub("", line.rstrip("\r\n"))
    if text.startswith("Player: "):
        return "player"
    if text.startswith("Console: "):
        return "console"
    if text.startswith("AGENT_SESSION_EXIT"):
        return "exit"
    if text.startswith("AGENT_SESSION_RESTART"):
        return "restart"
    if text.startswith("AGENT_SESSION_") or "panicked" in text or "ERROR" in text:
        return "alert"
    return None


def init_eof_cursors(paths: list[str]) -> dict[str, int]:
    cursors: dict[str, int] = {}
    for p in paths:
        path = Path(p)
        cursors[p] = path.stat().st_size if path.exists() else 0
    return cursors


def load_cursors(cursor_file: Path, paths: list[str]) -> dict[str, int]:
    if cursor_file.exists():
        try:
            saved = json.loads(cursor_file.read_text(encoding="utf-8"))
            if isinstance(saved, dict):
                result: dict[str, int] = {}
                for p in paths:
                    if p in saved:
                        result[p] = int(saved[p])
                    else:
                        path = Path(p)
                        result[p] = path.stat().st_size if path.exists() else 0
                return result
        except (OSError, ValueError):
            pass
    return init_eof_cursors(paths)


def save_cursors(cursor_file: Path, cursors: dict[str, int]) -> None:
    tmp = cursor_file.with_name(f".{cursor_file.name}.tmp")
    tmp.write_text(json.dumps(cursors), encoding="utf-8")
    tmp.replace(cursor_file)


def read_complete_lines(paths: list[str], cursors: dict[str, int]) -> list[str]:
    lines: list[str] = []
    for p in paths:
        path = Path(p)
        if not path.exists():
            continue
        try:
            size = path.stat().st_size
        except OSError:
            continue
        pos = cursors.get(p, 0)
        if size < pos:
            pos = 0
        if size == pos:
            continue
        try:
            with path.open("rb") as fh:
                fh.seek(pos)
                while True:
                    raw = fh.readline()
                    if not raw or not raw.endswith(b"\n"):
                        break
                    pos = fh.tell()
                    cursors[p] = pos
                    lines.append(raw.decode("utf-8", errors="replace").rstrip("\r\n"))
        except OSError:
            continue
    return lines


def watch_once(
    session: Path,
    paths: list[str],
    send: Callable[[Path, str], bool] = send_notice,
    poll_interval: float = 0.15,
    out: Optional[TextIO] = None,
) -> int:
    stream = sys.stdout if out is None else out
    cursor_file = session / "watch.cursor"
    cursors = load_cursors(cursor_file, paths)
    save_cursors(cursor_file, cursors)
    while True:
        updated = False
        for p in paths:
            path = Path(p)
            if not path.exists():
                continue
            try:
                size = path.stat().st_size
            except OSError:
                continue
            pos = cursors.get(p, 0)
            if size < pos:
                pos = 0
                cursors[p] = 0
                updated = True
            if size == pos:
                continue
            try:
                with path.open("rb") as fh:
                    fh.seek(pos)
                    while True:
                        raw = fh.readline()
                        if not raw or not raw.endswith(b"\n"):
                            break
                        pos = fh.tell()
                        cursors[p] = pos
                        updated = True
                        line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
                        kind = classify_line(line)
                        if kind == "player":
                            save_cursors(cursor_file, cursors)
                            send(session, "thinking")
                            stream.write(line + "\n")
                            stream.flush()
                            return 0
                        if kind in ("exit", "restart", "alert"):
                            save_cursors(cursor_file, cursors)
                            stream.write(line + "\n")
                            stream.flush()
                            return 0
            except OSError:
                continue
        if updated:
            save_cursors(cursor_file, cursors)
        time.sleep(poll_interval)


def watch_stream(
    session: Path,
    paths: list[str],
    timeout: float = 60.0,
    send: Callable[[Path, str], bool] = send_notice,
    poll_interval: float = 0.15,
    monotonic: Callable[[], float] = time.monotonic,
    sleep: Callable[[float], None] = time.sleep,
    out: Optional[TextIO] = None,
) -> int:
    stream = sys.stdout if out is None else out
    cursors = init_eof_cursors(paths)
    deadline: Optional[float] = None
    while True:
        for line in read_complete_lines(paths, cursors):
            kind = classify_line(line)
            if kind == "player":
                send(session, "thinking")
                stream.write(line + "\n")
                stream.flush()
                deadline = monotonic() + timeout
            elif kind == "console":
                deadline = None
            elif kind in ("exit", "restart", "alert"):
                stream.write(line + "\n")
                stream.flush()
                if kind in ("exit", "restart"):
                    deadline = None
                if kind == "exit":
                    return 0
        if deadline is not None and monotonic() >= deadline:
            send(session, TIMEOUT_STATUS)
            deadline = None
        sleep(poll_interval)


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Watch HL2-RS session logs for player chat and alerts.")
    parser.add_argument(
        "session_pos",
        nargs="?",
        type=Path,
        default=None,
        help="Optional session directory (defaults to --session, SESSION env, or <temp>/hl2rs-agent-session).",
    )
    parser.add_argument("--session", type=Path, default=None, help="Session directory path.")
    parser.add_argument(
        "--once",
        action="store_true",
        help="Exit after the next event using persistent byte-offset cursors.",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=60.0,
        help="Seconds before sending `agent_status waiting` in streaming mode (default: 60).",
    )
    parser.add_argument(
        "--poll",
        type=float,
        default=0.15,
        help="Log polling interval in seconds (default: 0.15).",
    )
    return parser.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    session = args.session or args.session_pos or rcon.default_session_dir()
    if not session.is_dir():
        print(f"watch: session directory does not exist: {session}", file=sys.stderr)
        return 1
    once = args.once or os.environ.get("WATCH_ONCE") == "1"
    paths = session_log_paths(session)
    try:
        with acquire_lock(session / "watch.lock"):
            if once:
                return watch_once(session, paths, poll_interval=args.poll)
            return watch_stream(
                session,
                paths,
                timeout=args.timeout,
                poll_interval=args.poll,
            )
    except SessionLockedError as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
