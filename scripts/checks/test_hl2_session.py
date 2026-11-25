"""Cross-platform unit tests for the in-repo /hl2-session skill scripts and docs."""

from contextlib import redirect_stderr, redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import re
import socket
import struct
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

REPO = Path(__file__).resolve().parents[2]
SKILL_DIR = REPO / ".agents" / "skills" / "hl2-session"
SCRIPTS_DIR = SKILL_DIR / "scripts"
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import rcon  # noqa: E402
import restart  # noqa: E402
import session  # noqa: E402
import watch  # noqa: E402


def _recv_exact(conn: socket.socket, size: int) -> bytes:
    chunks = []
    remaining = size
    while remaining > 0:
        chunk = conn.recv(remaining)
        if not chunk:
            raise EOFError("unexpected EOF")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def _send_rcon_packet(conn: socket.socket, packet_id: int, kind: int, body: str) -> None:
    payload = struct.pack("<ii", packet_id, kind) + body.encode("utf-8") + b"\x00\x00"
    conn.sendall(struct.pack("<i", len(payload)) + payload)


class MockRconServer:
    """Minimal Valve Source RCON TCP server for loopback unit tests."""

    def __init__(self, password: str, handler):
        self.password = password
        self.handler = handler
        self.commands: list[str] = []
        self._stop = threading.Event()
        self._sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._sock.bind(("127.0.0.1", 0))
        self._sock.listen(8)
        self._sock.settimeout(0.2)
        self.port = int(self._sock.getsockname()[1])
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()

    def _serve(self) -> None:
        while not self._stop.is_set():
            try:
                conn, _ = self._sock.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            threading.Thread(target=self._handle_client, args=(conn,), daemon=True).start()

    def _handle_client(self, conn: socket.socket) -> None:
        authed = False
        with conn:
            conn.settimeout(2.0)
            while not self._stop.is_set():
                try:
                    (size,) = struct.unpack("<i", _recv_exact(conn, 4))
                    data = _recv_exact(conn, size)
                except (OSError, EOFError):
                    break
                packet_id, kind = struct.unpack("<ii", data[:8])
                body = data[8:-2].split(b"\x00", 1)[0].decode("utf-8", errors="replace")
                if kind == rcon.AUTH:
                    _send_rcon_packet(conn, packet_id, rcon.RESPONSE, "")
                    if body == self.password:
                        authed = True
                        _send_rcon_packet(conn, packet_id, rcon.AUTH_RESPONSE, "")
                    else:
                        authed = False
                        _send_rcon_packet(conn, rcon.AUTH_FAILED_ID, rcon.AUTH_RESPONSE, "")
                elif not authed:
                    break
                elif kind == rcon.EXEC:
                    self.commands.append(body)
                    for part in self.handler(body):
                        _send_rcon_packet(conn, packet_id, rcon.RESPONSE, part)
                else:
                    _send_rcon_packet(conn, packet_id, rcon.RESPONSE, "")

    def close(self) -> None:
        self._stop.set()
        try:
            self._sock.close()
        except OSError:
            pass
        self._thread.join(timeout=2.0)


class TestRcon(unittest.TestCase):
    def test_rcon_refuses_non_loopback_before_opening_a_socket(self):
        with mock.patch.object(rcon.socket, "create_connection") as connect:
            for address in ("0.0.0.0", "192.0.2.1", "::", "2001:db8::1", "game.example"):
                with self.subTest(address=address), self.assertRaisesRegex(ValueError, "loopback"):
                    rcon.exec_command("echo check", host=address)
            connect.assert_not_called()

    def test_exec_success_and_multipacket_response(self):
        server = MockRconServer("pw123", lambda cmd: ["part1 ", "part2\n"] if cmd == "ui_dump" else [])
        try:
            out = rcon.exec_command("ui_dump", host="127.0.0.1", port=server.port, password="pw123")
            self.assertEqual(out, "part1 part2\n")
            self.assertEqual(server.commands, ["ui_dump"])
        finally:
            server.close()

    def test_bad_password_raises_auth_error(self):
        server = MockRconServer("pw123", lambda _cmd: ["ok"])
        try:
            with self.assertRaises(rcon.RconAuthError):
                rcon.exec_command("getpos", host="127.0.0.1", port=server.port, password="wrong")
        finally:
            server.close()

    def test_reads_port_and_password_from_session_dir(self):
        with tempfile.TemporaryDirectory() as tmp:
            sess = Path(tmp) / "sess"
            sess.mkdir()
            (sess / "port").write_text("27099\n", encoding="utf-8")
            (sess / "rcon_password").write_text("secret_pw\n", encoding="utf-8")
            self.assertEqual(rcon.read_port(session_dir=sess, env={}), 27099)
            self.assertEqual(rcon.read_password(session_dir=sess, env={}), "secret_pw")

    def test_wait_and_exec_retries_connection_errors_not_auth_or_exit(self):
        attempts = {"count": 0}

        def flaky_exec(*_args, **_kwargs):
            attempts["count"] += 1
            if attempts["count"] < 3:
                raise ConnectionRefusedError("still starting")
            return "Console: ok\n"

        with mock.patch.object(rcon, "exec_command", side_effect=flaky_exec):
            out = rcon.wait_and_exec(
                "say hi",
                password="pw",
                wait_seconds=2.0,
                poll_interval=0.01,
            )
        self.assertEqual(out, "Console: ok\n")
        self.assertEqual(attempts["count"], 3)

        auth_attempts = {"count": 0}

        def bad_auth(*_args, **_kwargs):
            auth_attempts["count"] += 1
            raise rcon.RconAuthError("bad")

        with mock.patch.object(rcon, "exec_command", side_effect=bad_auth):
            with self.assertRaises(rcon.RconAuthError):
                rcon.wait_and_exec("say hi", password="wrong", wait_seconds=2.0, poll_interval=0.01)
        self.assertEqual(auth_attempts["count"], 1)

        with tempfile.TemporaryDirectory() as tmp:
            sess = Path(tmp)
            (sess / "game.log").write_text("boot\nAGENT_SESSION_EXIT 1\n", encoding="utf-8")
            with mock.patch.object(rcon, "exec_command", side_effect=ConnectionRefusedError("closed")):
                with self.assertRaisesRegex(RuntimeError, "AGENT_SESSION_EXIT 1"):
                    rcon.wait_and_exec(
                        "say hi",
                        password="pw",
                        wait_seconds=2.0,
                        session_dir=sess,
                        exit_grace_seconds=0.0,
                    )

    def test_wait_reloads_port_and_password_and_ignores_stale_log(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            sess = root / "sess"
            sess.mkdir()
            legacy = root / "hl2rs-rcon-password"
            legacy.write_text("stale_pw\n", encoding="utf-8")
            game_log = sess / "game.log"
            game_log.write_text("AGENT_SESSION_EXIT 0\n", encoding="utf-8")
            old_mtime = time.time() - 60.0
            os.utime(game_log, (old_mtime, old_mtime))

            with mock.patch.object(tempfile, "gettempdir", return_value=str(root)):
                with self.assertRaises(FileNotFoundError):
                    rcon.read_password(session_dir=sess, env={}, allow_legacy_fallback=False)
                self.assertEqual(
                    rcon.read_password(session_dir=sess, env={}, allow_legacy_fallback=True),
                    "stale_pw",
                )

            self.assertIsNone(rcon.game_log_exit_line(sess, since_wall=time.time() - 5.0))

            pw_calls = {"n": 0}

            def delayed_password():
                pw_calls["n"] += 1
                if pw_calls["n"] == 2:
                    (sess / "port").write_text("27088\n", encoding="utf-8")
                    (sess / "rcon_password").write_text("fresh_pw\n", encoding="utf-8")
                return rcon.read_password(
                    session_dir=sess, env={}, allow_legacy_fallback=False
                )

            exec_calls = {"n": 0}

            def delayed_exec(_cmd, *, host="127.0.0.1", port=27015, password="", timeout=10.0):
                exec_calls["n"] += 1
                if exec_calls["n"] == 1:
                    raise ConnectionRefusedError("starting")
                return f"{host}:{port}:{password}"

            with mock.patch.object(rcon, "exec_command", side_effect=delayed_exec):
                out = rcon.wait_and_exec(
                    "ui_dump",
                    wait_seconds=2.0,
                    session_dir=sess,
                    poll_interval=0.01,
                    port_getter=lambda: rcon.read_port(session_dir=sess, env={}),
                    password_getter=delayed_password,
                    exit_grace_seconds=0.0,
                )
            self.assertEqual(out, "127.0.0.1:27088:fresh_pw")
            self.assertEqual(pw_calls["n"], 3)
            self.assertEqual(exec_calls["n"], 2)


class TestWatch(unittest.TestCase):
    def test_acquire_lock_prevents_concurrent_watcher(self):
        with tempfile.TemporaryDirectory() as tmp:
            lock_path = Path(tmp) / "watch.lock"
            with watch.acquire_lock(lock_path):
                with self.assertRaises(watch.SessionLockedError):
                    with watch.acquire_lock(lock_path):
                        pass
            with watch.acquire_lock(lock_path):
                pass

    def test_watch_once_cursor_truncation_and_events(self):
        with tempfile.TemporaryDirectory() as tmp:
            sess = Path(tmp)
            con_log = Path(tmp) / "console.log"
            game_log = sess / "game.log"
            con_log.write_text("[ 0.50] Player: historical\r\n", encoding="utf-8")
            game_log.write_text("", encoding="utf-8")
            (sess / "console_log_path").write_text(str(con_log) + "\n", encoding="utf-8")

            notices: list[str] = []
            out = io.StringIO()

            def writer():
                deadline = time.monotonic() + 2.0
                while not (sess / "watch.cursor").exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                # Console lines and partial lines should not wake watch_once.
                with con_log.open("ab") as fh:
                    fh.write(b"[ 1.00] Console: ignored reply\r\n")
                    fh.write(b"[ 1.25] Player: partial")
                    fh.flush()
                time.sleep(0.05)
                with con_log.open("ab") as fh:
                    fh.write(b" message\r\n")
                    fh.flush()

            thread = threading.Thread(target=writer, daemon=True)
            thread.start()
            rc = watch.watch_once(
                sess,
                watch.session_log_paths(sess),
                send=lambda _s, status: notices.append(status) or True,
                poll_interval=0.01,
                out=out,
            )
            thread.join(timeout=2.0)
            self.assertEqual(rc, 0)
            self.assertEqual(notices, ["thinking"])
            self.assertEqual(out.getvalue().strip(), "[ 1.25] Player: partial message")

            # A message arriving between turns must still be picked up via watch.cursor.
            with con_log.open("ab") as fh:
                fh.write(b"[ 2.00] Player: between turns\r\n")
            notices.clear()
            out2 = io.StringIO()
            rc2 = watch.watch_once(
                sess,
                watch.session_log_paths(sess),
                send=lambda _s, status: notices.append(status) or True,
                poll_interval=0.01,
                out=out2,
            )
            self.assertEqual(rc2, 0)
            self.assertEqual(notices, ["thinking"])
            self.assertEqual(out2.getvalue().strip(), "[ 2.00] Player: between turns")

            # Log truncation resets cursor to 0 and surfaces ERROR.
            con_log.write_text("ERROR fatal failure\n", encoding="utf-8")
            notices.clear()
            out3 = io.StringIO()
            rc3 = watch.watch_once(
                sess,
                watch.session_log_paths(sess),
                send=lambda _s, status: notices.append(status) or True,
                poll_interval=0.01,
                out=out3,
            )
            self.assertEqual(rc3, 0)
            self.assertEqual(notices, [])
            self.assertEqual(out3.getvalue().strip(), "ERROR fatal failure")

    def test_stream_receives_immediate_reply_to_greeting(self):
        with tempfile.TemporaryDirectory() as tmp:
            session_dir = Path(tmp)
            log = session_dir / "console.log"
            log.write_text("Player: old session\n", encoding="utf-8")

            def greet_after_ready(marker, **kwargs):
                self.assertEqual(marker, "WATCH_READY")
                self.assertTrue(kwargs["flush"])
                with log.open("a", encoding="utf-8") as stream:
                    stream.write("Console: connected\nPlayer: first\nPlayer: second\nAGENT_SESSION_EXIT 0\n")

            output = io.StringIO()
            with mock.patch("builtins.print", side_effect=greet_after_ready):
                watch.watch_stream(session_dir, [str(log)], send=mock.Mock(),
                                   sleep=lambda _: self.fail("missed immediate chat"), out=output)
            self.assertEqual(output.getvalue().splitlines(),
                             ["Player: first", "Player: second", "AGENT_SESSION_EXIT 0"])

    def test_watch_stream_thinking_reply_timeout_and_exit(self):
        with tempfile.TemporaryDirectory() as tmp:
            sess = Path(tmp)
            con_log = Path(tmp) / "console.log"
            game_log = sess / "game.log"
            con_log.write_text("", encoding="utf-8")
            game_log.write_text("", encoding="utf-8")
            (sess / "console_log_path").write_text(str(con_log) + "\n", encoding="utf-8")

            clock = {"now": 100.0, "step": 0}
            notices: list[str] = []
            out = io.StringIO()

            def fake_sleep(_dt: float) -> None:
                clock["step"] += 1
                s = clock["step"]
                if s == 1:
                    with con_log.open("a", encoding="utf-8") as fh:
                        fh.write("[ 1.0] Player: first\n")
                elif s == 2:
                    with con_log.open("a", encoding="utf-8") as fh:
                        fh.write("[ 2.0] Console: answered\n")
                elif s == 3:
                    clock["now"] += 70.0
                elif s == 4:
                    with con_log.open("a", encoding="utf-8") as fh:
                        fh.write("[ 72.0] Player: slow question\n")
                elif s == 5:
                    clock["now"] += 61.0
                elif s == 6:
                    with game_log.open("a", encoding="utf-8") as fh:
                        fh.write("AGENT_SESSION_EXIT 0\n")

            rc = watch.watch_stream(
                sess,
                watch.session_log_paths(sess),
                timeout=60.0,
                send=lambda _s, status: notices.append(status) or True,
                poll_interval=0.01,
                monotonic=lambda: clock["now"],
                sleep=fake_sleep,
                out=out,
            )
            self.assertEqual(rc, 0)
            self.assertEqual(notices, ["thinking", "thinking", "waiting"])
            lines = out.getvalue().splitlines()
            self.assertEqual(
                lines,
                [
                    "[ 1.0] Player: first",
                    "[ 72.0] Player: slow question",
                    "AGENT_SESSION_EXIT 0",
                ],
            )


class TestSession(unittest.TestCase):
    def test_enable_attach_requests_once_then_authenticates(self):
        game = {"pid": 123, "listeners": []}
        path = Path("live-123")
        with mock.patch.object(session, "find_running_games", return_value=[game]), \
             mock.patch.object(session, "attach_to_game", side_effect=[None, None, path, path]), \
             mock.patch.object(session, "request_agent_rcon", return_value=True) as enable, \
             mock.patch.object(session.time, "sleep"), \
             redirect_stdout(io.StringIO()) as out:
            self.assertEqual(session.main(["attach", "--enable"]), 0)
            self.assertEqual(out.getvalue().strip(), str(path))
            enable.assert_called_once_with(123)

            self.assertEqual(session.main(["attach", "--enable"]), 0)
            self.assertEqual(enable.call_count, 1, "an authenticated session needs no activation")

    def test_enable_attach_does_not_guess_between_multiple_games(self):
        games = [{"pid": 123, "listeners": []}, {"pid": 456, "listeners": []}]
        with mock.patch.object(session, "find_running_games", return_value=games), \
             mock.patch.object(session, "request_agent_rcon") as enable, \
             redirect_stderr(io.StringIO()) as err:
            self.assertEqual(session.main(["attach", "--enable"]), 1)
            self.assertIn("--pid", err.getvalue())
            enable.assert_not_called()

    def test_activation_validates_private_record_and_sends_only_token_to_owned_port(self):
        pid, token = 123, "a" * 48
        with tempfile.TemporaryDirectory() as tmp, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as listener:
            root = Path(tmp)
            published = root / f"live-{pid}"
            published.mkdir(mode=0o700)
            descriptor = published / "bootstrap.json"
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
            valid = {"pid": pid, "port": port, "token": token}
            listener.settimeout(2)
            with mock.patch.object(rcon, "default_session_dir", return_value=root), \
                 mock.patch.object(session, "_owns_bootstrap_port", return_value=False) as owns_port:
                for invalid in ({"pid": pid + 1}, {"port": True}, {"token": token + " extra"}):
                    session.write_private_text(descriptor, json.dumps(valid | invalid))
                    self.assertFalse(session.request_agent_rcon(pid))
                owns_port.assert_not_called()
                session.write_private_text(descriptor, json.dumps(valid))
                if os.name != "nt":
                    descriptor.chmod(0o644)
                    self.assertFalse(session.request_agent_rcon(pid))
                    owns_port.assert_not_called()
                    descriptor.chmod(0o600)
                self.assertFalse(session.request_agent_rcon(pid))
                owns_port.assert_called_once_with(pid, port)
                owns_port.return_value = True
                self.assertTrue(session.request_agent_rcon(pid))
                self.assertEqual(listener.recvfrom(100)[0], token.encode("ascii"))

    def test_attach_authenticates_only_the_selected_local_listener(self):
        server = MockRconServer("test_password", lambda cmd: ["HL2RS_ATTACH\n"] if cmd == "echo HL2RS_ATTACH" else [])
        self.addCleanup(server.close)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            published = root / "live-123"
            published.mkdir()
            (published / "pid").write_text("123\n", encoding="utf-8")
            (published / "port").write_text(str(server.port), encoding="utf-8")
            (published / "console_log_path").write_text(str(root / "console.log"), encoding="utf-8")
            password_file = published / "rcon_password"
            password_file.write_text("test_password\n", encoding="utf-8")
            for name in ("pid", "port", "console_log_path"):
                (root / name).write_bytes((published / name).read_bytes())
            managed_password = root / "rcon_password"
            managed_password.write_text("stale_password\n", encoding="utf-8")
            game = {"pid": 123, "listeners": [{"address": "0.0.0.0", "port": server.port}]}
            args = ["attach", "--pid", "123"]
            with mock.patch.object(session, "find_running_games", return_value=[game]), \
                 mock.patch.object(rcon, "default_session_dir", return_value=root):
                with redirect_stdout(io.StringIO()) as out, redirect_stderr(io.StringIO()) as err:
                    self.assertEqual(session.main(args), 1)
                    self.assertEqual(out.getvalue(), "")
                    self.assertIn("agent_rcon", err.getvalue())
                    self.assertEqual(server.commands, [])

                game["listeners"][0]["address"] = "127.0.0.1"
                with redirect_stdout(io.StringIO()) as out, redirect_stderr(io.StringIO()) as err:
                    self.assertEqual(session.main(args), 0)
                    self.assertEqual(out.getvalue().strip(), str(published.resolve()))
                    self.assertEqual(err.getvalue(), "")
                    self.assertEqual(server.commands, ["echo HL2RS_ATTACH"])

                password_file.write_text("stale_password\n", encoding="utf-8")
                with redirect_stdout(io.StringIO()) as out, redirect_stderr(io.StringIO()) as err:
                    with mock.patch.object(rcon, "exec_command", wraps=rcon.exec_command) as exec_command:
                        self.assertEqual(session.main(args), 1)
                        exec_command.assert_called_once()
                    self.assertEqual(out.getvalue(), "")
                    self.assertIn("agent_rcon", err.getvalue())
                    self.assertNotIn("stale_password", err.getvalue())

                managed_password.write_text("test_password\n", encoding="utf-8")
                with redirect_stdout(io.StringIO()) as out:
                    self.assertEqual(session.main(args), 0)
                    self.assertEqual(out.getvalue().strip(), str(root.resolve()))

    def test_attached_watcher_stops_without_a_supervisor_log(self):
        with tempfile.TemporaryDirectory() as tmp:
            published = Path(tmp) / "live-456"
            published.mkdir()
            (published / "pid").write_text("456\n", encoding="utf-8")
            output = io.StringIO()
            with mock.patch.object(watch, "process_is_alive", return_value=False) as probe:
                self.assertEqual(watch.watch_stream(published, [], out=output), 0)
            probe.assert_called_once_with(456)
            self.assertEqual(output.getvalue().strip(), "AGENT_SESSION_EXIT attached game stopped")

    def test_attached_watcher_retires_only_after_the_game_removes_the_marker(self):
        with tempfile.TemporaryDirectory() as tmp:
            published = Path(tmp) / "live-456"
            published.mkdir()
            marker = published / "pid"
            marker.touch()
            cursor = published / "watch.cursor"
            cursor.touch()

            with mock.patch.object(watch, "watch_once", return_value=0):
                self.assertEqual(watch.main(["--session", str(published), "--once"]), 0)
            self.assertTrue(cursor.is_file())
            self.assertTrue((published / "watch.lock").is_file())

            def exit_during_watch(*args, **kwargs):
                marker.unlink()
                return 0

            with mock.patch.object(watch, "watch_stream", side_effect=exit_during_watch):
                self.assertEqual(watch.main(["--session", str(published)]), 0)
            self.assertFalse(published.exists())

    def test_current_host_process_is_alive(self):
        self.assertTrue(watch.process_is_alive(os.getpid()))

    def test_restart_refuses_an_attached_game_before_building_or_contacting_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            send = mock.Mock()
            build = mock.Mock()
            with redirect_stderr(io.StringIO()) as err:
                self.assertEqual(restart.restart_session(root / "live-123", root, rcon_exec=send, script_builder=build), 1)
            self.assertIn("Leaving it running", err.getvalue())
            send.assert_not_called()
            build.assert_not_called()

    def test_paths_command_and_bom_build_info(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            (repo / "bin").mkdir()
            (repo / "bin" / "build-bevy-info.json").write_bytes(
                b"\xef\xbb\xbf" + json.dumps({"profile": "dist", "plugins": "advanced-options,debug-tools"}).encode("utf-8")
            )
            info = session.read_build_info(repo)
            self.assertEqual(info["profile"], "dist")
            self.assertTrue(session.build_info_has_debug_tools(info))
            self.assertEqual(session.console_log_path(repo, {}), repo / "bin" / "console.log")
            custom = Path(tmp) / "cfg" / "hl2-config.json"
            self.assertEqual(
                session.console_log_path(repo, {"HL2RS_SETTINGS": str(custom)}),
                custom.parent / "console.log",
            )
            cmd = session.build_launch_command(
                session.executable_path(repo),
                27016,
                "pw",
                ["--windowed"],
                ["--map", "d1_trainstation_01"],
            )
            self.assertIn("-usercon", cmd)
            self.assertIn("+showconsole", cmd)
            self.assertEqual(cmd[-9:-6], ["--windowed", "--map", "d1_trainstation_01"])
            overrides = session.build_launch_command(
                session.executable_path(repo), 27016, "pw",
                ["+ip", "0.0.0.0", "+hostport", "27017"], ["+rcon_password", "other"],
            )
            self.assertEqual(overrides[-6:], ["+ip", "127.0.0.1", "+rcon_password", "pw", "+hostport", "27016"])

    def test_port_free_and_pick_free_port(self):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
            sock.bind(("127.0.0.1", 0))
            sock.listen(1)
            busy = int(sock.getsockname()[1])
            self.assertFalse(session.is_port_free(busy))
            picked = session.pick_free_port(preferred=busy)
            self.assertNotEqual(picked, busy)
            self.assertTrue(session.is_port_free(picked))

    def test_write_private_text_permissions(self):
        with tempfile.TemporaryDirectory() as tmp:
            pw_path = Path(tmp) / "rcon_password"
            session.write_private_text(pw_path, "secret\n")
            self.assertEqual(pw_path.read_text(encoding="utf-8"), "secret\n")
            if os.name != "nt":
                self.assertEqual(pw_path.stat().st_mode & 0o777, 0o600)

    def test_executable_sha256_hashes_file_and_handles_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            exe = Path(tmp) / "hl2-bevy"
            self.assertIsNone(session.executable_sha256(exe))
            payload = b"hl2rs-binary-payload" * 4096
            exe.write_bytes(payload)
            self.assertEqual(
                session.executable_sha256(exe),
                hashlib.sha256(payload).hexdigest(),
            )

    def test_supervisor_handles_resume_empty_resume_and_repackage(self):
        fake_script = (
            "import json, os, pathlib, sys\n"
            "sess = pathlib.Path(os.environ['TEST_SESS'])\n"
            "count_file = sess / 'count'\n"
            "run = int(count_file.read_text()) + 1 if count_file.exists() else 1\n"
            "count_file.write_text(str(run))\n"
            "print(f'run={run} args={sys.argv[1:]}')\n"
            "if run == 1:\n"
            "    (sess / 'resume').write_text('--map d1_trainstation_02\\n')\n"
            "    sys.exit(0)\n"
            "if run == 2:\n"
            "    (sess / 'repackage').write_text(json.dumps({'profile': 'dist', 'plugins': 'debug-tools'}))\n"
            "    (sess / 'resume').write_text('')\n"
            "    sys.exit(0)\n"
            "sys.exit(7)\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            sess = root / "sess"
            repo.mkdir()
            (repo / "bin").mkdir()
            sess.mkdir()
            fake_exe = root / "fake_game.py"
            fake_exe.write_text(fake_script, encoding="utf-8")
            (sess / "console_log_path").write_text(str(repo / "bin" / "console.log") + "\n", encoding="utf-8")

            builds: list[tuple[str, str]] = []

            def fake_builder(_repo, profile="release", plugins="advanced-options,debug-tools"):
                builds.append((profile, plugins))
                return 0

            env = dict(os.environ, TEST_SESS=str(sess))
            out = io.StringIO()
            orig_build = session.build_launch_command

            def wrapped_cmd(exe, port, password, keep_args, run_args):
                return [sys.executable, *orig_build(exe, port, password, keep_args, run_args)]

            with mock.patch.object(session, "build_launch_command", side_effect=wrapped_cmd):
                rc = session.run_supervisor(
                    session_dir=sess,
                    repo=repo,
                    exe=fake_exe,
                    port=27015,
                    password="pw",
                    keep_args=["--windowed"],
                    initial_args=["--map", "d1_trainstation_01"],
                    builder=fake_builder,
                    env=env,
                    stdout=out,
                )

            self.assertEqual(rc, 7)
            self.assertEqual(builds, [("dist", "debug-tools")])
            log_text = (sess / "game.log").read_text(encoding="utf-8")
            self.assertIn("AGENT_SESSION_RESTART --map d1_trainstation_02", log_text)
            self.assertIn("AGENT_SESSION_RESTART \n", log_text)
            self.assertIn("AGENT_SESSION_EXIT 7", log_text)
            self.assertIn("'--map', 'd1_trainstation_01'", log_text)
            self.assertIn("'--map', 'd1_trainstation_02'", log_text)
            self.assertFalse((sess / "pid").exists())


class TestRestart(unittest.TestCase):
    def test_parse_resume_output(self):
        self.assertEqual(
            restart.parse_resume_output("noise\nagent_resume --map d1_trainstation_02 --yaw 90.000\n"),
            "--map d1_trainstation_02 --yaw 90.000",
        )
        self.assertEqual(restart.parse_resume_output("agent_resume \n"), "")
        self.assertIsNone(restart.parse_resume_output("Unknown command: agent_resume\n"))

    def test_build_failure_and_missing_resume_abort_without_quit(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            sess = root / "sess"
            (repo / "bin").mkdir(parents=True)
            sess.mkdir()
            (sess / "port").write_text("27015\n", encoding="utf-8")
            (sess / "rcon_password").write_text("pw\n", encoding="utf-8")

            sent: list[str] = []

            def fake_rcon(cmd, **_kwargs):
                sent.append(cmd)
                return "Unknown command: agent_resume\n"

            rc = restart.restart_session(
                sess,
                repo,
                is_windows=False,
                rcon_exec=fake_rcon,
                script_builder=lambda *_a, **_k: 2,
            )
            self.assertEqual(rc, 2)
            self.assertEqual(sent, [])
            self.assertFalse((sess / "resume").exists())

            rc2 = restart.restart_session(
                sess,
                repo,
                no_build=True,
                is_windows=False,
                rcon_exec=fake_rcon,
            )
            self.assertEqual(rc2, 1)
            self.assertEqual(sent, ["agent_resume"])
            self.assertFalse((sess / "resume").exists())

    def test_posix_and_windows_restart_flows(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            sess = root / "sess"
            (repo / "bin").mkdir(parents=True)
            sess.mkdir()
            (repo / "bin" / "build-bevy-info.json").write_text(
                json.dumps({"profile": "release", "plugins": "advanced-options,debug-tools"}),
                encoding="utf-8",
            )
            (sess / "port").write_text("27055\n", encoding="utf-8")
            (sess / "rcon_password").write_text("pw\n", encoding="utf-8")

            sent: list[str] = []

            def fake_rcon(cmd, **_kwargs):
                sent.append(cmd)
                if cmd == "agent_resume":
                    return "agent_resume --map d1_trainstation_02 +showconsole\n"
                if cmd == "quit":
                    raise ConnectionError("game closed")
                return ""

            rc_posix = restart.restart_session(
                sess,
                repo,
                is_windows=False,
                rcon_exec=fake_rcon,
                script_builder=lambda *_a, **_k: 0,
            )
            self.assertEqual(rc_posix, 0)
            self.assertEqual(sent, ["agent_resume", "quit"])
            self.assertEqual(
                (sess / "resume").read_text(encoding="utf-8"),
                "--map d1_trainstation_02 +showconsole\n",
            )
            self.assertFalse((sess / "repackage").exists())

            sent.clear()
            (sess / "resume").unlink()
            cargo_calls: list[tuple[str, str]] = []

            def fake_cargo(_repo, profile="release", plugins="advanced-options,debug-tools"):
                cargo_calls.append((profile, plugins))
                return 0

            rc_win = restart.restart_session(
                sess,
                repo,
                is_windows=True,
                rcon_exec=fake_rcon,
                cargo_builder=fake_cargo,
            )
            self.assertEqual(rc_win, 0)
            self.assertEqual(cargo_calls, [("release", "advanced-options,debug-tools")])
            self.assertEqual(sent, ["agent_resume", "quit"])
            repackage = json.loads((sess / "repackage").read_text(encoding="utf-8"))
            self.assertEqual(repackage, {"profile": "release", "plugins": "advanced-options,debug-tools"})


class TestSkillDocs(unittest.TestCase):
    def test_skill_files_and_referenced_scripts_exist(self):
        canonical = SKILL_DIR / "SKILL.md"
        claude_stub = REPO / ".claude" / "skills" / "hl2-session" / "SKILL.md"
        self.assertTrue(canonical.is_file())
        self.assertTrue(claude_stub.is_file())
        canonical_text = canonical.read_text(encoding="utf-8")
        stub_text = claude_stub.read_text(encoding="utf-8")
        self.assertIn("name: hl2-session", canonical_text)
        self.assertIn("name: hl2-session", stub_text)
        self.assertIn(".agents/skills/hl2-session/SKILL.md", stub_text)

        refs = set(re.findall(r"\.agents/skills/hl2-session/scripts/([A-Za-z0-9_.-]+\.py)", canonical_text))
        self.assertEqual(refs, {"session.py", "rcon.py", "watch.py", "restart.py"})
        for name in refs:
            self.assertTrue((SCRIPTS_DIR / name).is_file(), name)


if __name__ == "__main__":
    unittest.main()
