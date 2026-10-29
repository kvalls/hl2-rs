"""Stale cleanup and Cargo exclude each other for the whole pruning operation."""
import contextlib
import io
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

from clean_target import clean_stale
from target_lock import cargo_locks, local_filesystem


class TargetLockTests(unittest.TestCase):
    def setUp(self):
        self.fixture = tempfile.TemporaryDirectory()
        self.addCleanup(self.fixture.cleanup)
        self.root = Path(self.fixture.name)
        self.target = self.root / "target"
        self.old = self.target / "debug/incremental/hl2_ui-aaa/cache"
        self.new = self.target / "debug/incremental/hl2_ui-bbb/cache"
        for path, stamp in ((self.old, 100), (self.new, 200)):
            path.parent.mkdir(parents=True)
            path.write_bytes(b"preserve while locked")
            os.utime(path, (stamp, stamp))
            os.utime(path.parent, (stamp, stamp))

    def clean(self):
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertTrue(clean_stale(str(self.root), str(self.target), ["hl2-ui"]))
        return output.getvalue()

    def test_unavailable_locking_skips_all_pruning(self):
        with patch.dict("sys.modules", {"fcntl": None}):
            self.assertIn("Skipping stale cleanup", self.clean())
        self.assertTrue(self.old.exists())

    def test_unverified_filesystem_skips_all_pruning(self):
        with patch("target_lock.local_filesystem", return_value=False):
            self.assertIn("Skipping stale cleanup", self.clean())
        self.assertTrue(self.old.exists())

    @unittest.skipUnless(os.name == "posix", "POSIX filesystem probe")
    def test_network_and_unknown_filesystems_are_rejected(self):
        for kind in ("nfs", "nfs4", "smbfs", "cifs", "unknown"):
            with self.subTest(kind=kind), patch("target_lock.sys.platform", "linux"), \
                    patch("target_lock.subprocess.check_output", return_value=kind):
                self.assertFalse(local_filesystem(self.target))
            with patch("target_lock.sys.platform", "darwin"), \
                    patch("target_lock.darwin_filesystem", return_value=kind):
                self.assertFalse(local_filesystem(self.target))

    @unittest.skipUnless(os.name == "posix", "Cargo-compatible POSIX locks")
    def test_busy_debug_or_release_skips_and_releases_partial_locks(self):
        import fcntl

        for profile in ("debug", "release"):
            with self.subTest(profile=profile):
                folder = self.target / profile
                folder.mkdir(exist_ok=True)
                with (folder / ".cargo-lock").open("a+b") as held:
                    fcntl.flock(held, fcntl.LOCK_SH | fcntl.LOCK_NB)
                    self.assertIn("Skipping stale cleanup", self.clean())
                    self.assertTrue(self.old.exists())
                with cargo_locks(self.target) as acquired:
                    self.assertTrue(acquired)

    @unittest.skipUnless(os.name == "posix", "Cargo-compatible POSIX locks")
    def test_locks_cover_pruning_and_release_on_error(self):
        import fcntl

        def prune(*args):
            for profile in ("debug", "release"):
                with (self.target / profile / ".cargo-lock").open("a+b") as contender:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
            raise RuntimeError("pruning fixture failure")

        with patch("clean_target._prune_stale", side_effect=prune):
            with self.assertRaisesRegex(RuntimeError, "pruning fixture failure"):
                self.clean()
        with cargo_locks(self.target) as acquired:
            self.assertTrue(acquired)

    def wait_for(self, condition, process, log):
        deadline = time.monotonic() + 20
        while not condition():
            if process.poll() is not None or time.monotonic() >= deadline:
                self.fail(log.read_text())
            time.sleep(0.01)

    @unittest.skipUnless(os.name == "posix" and shutil.which("cargo"), "POSIX Cargo probe")
    def test_real_cargo_build_and_cleanup_exclude_each_other(self):
        (self.root / "Cargo.toml").write_text(
            '[package]\nname="cache-lock-probe"\nversion="0.0.0"\nedition="2021"\n[workspace]\n')
        (self.root / "src").mkdir()
        (self.root / "src/main.rs").write_text("fn main() {}\n")
        (self.root / "build.rs").write_text('''fn main() {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    std::fs::write(root.join("build-started"), b"ready").unwrap();
    for _ in 0..2000 {
        if root.join("build-release").exists() { return; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("probe release timed out");
}
''')
        log = self.root / "cargo.log"
        with log.open("w") as output:
            with cargo_locks(self.target) as acquired:
                self.assertTrue(acquired)
                process = subprocess.Popen(
                    ["cargo", "build", "--offline", "--target-dir", str(self.target)],
                    cwd=self.root, stdout=output, stderr=output)
                self.addCleanup(lambda: self.stop_probe(process))
                self.wait_for(lambda: "file lock" in log.read_text(), process, log)
                self.assertFalse((self.root / "build-started").exists())
            self.wait_for(lambda: (self.root / "build-started").exists(), process, log)
            self.assertIn("Skipping stale cleanup", self.clean())
            self.assertTrue(self.old.exists())
            self.assertTrue(self.new.exists())
            (self.root / "build-release").touch()
            self.assertEqual(process.wait(timeout=20), 0, log.read_text())
        self.clean()
        self.assertFalse(self.old.exists())
        self.assertTrue(self.new.exists())

    def stop_probe(self, process):
        (self.root / "build-release").touch()
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


if __name__ == "__main__":
    unittest.main()
