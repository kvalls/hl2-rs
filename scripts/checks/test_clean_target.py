"""Target maintenance preserves unrelated caches and supports launcher opt-out."""
import contextlib
import io
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from clean_target import _prune_stale


class CleanTargetTests(unittest.TestCase):
    def setUp(self):
        self.fixture = tempfile.TemporaryDirectory()
        self.addCleanup(self.fixture.cleanup)
        self.root = Path(self.fixture.name)

    def artifact(self, name, timestamp):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"cache fixture")
        os.utime(path, (timestamp, timestamp))
        os.utime(path.parent, (timestamp, timestamp))
        return path

    def clean(self, dry_run=False):
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertTrue(_prune_stale(str(self.root), str(self.root), ["hl2-ui"],
                                        dry_run=dry_run))

    def test_incremental_pruning_preserves_third_party_and_newest_workspace_cache(self):
        for profile in ("debug", "release"):
            old = self.artifact(f"{profile}/incremental/hl2_ui-aaa/cache", 100)
            newest = self.artifact(f"{profile}/incremental/hl2_ui-bbb/cache", 200)
            unrelated = [self.artifact(f"{profile}/incremental/third_party-{h}/cache", t)
                         for h, t in (("aaa", 100), ("bbb", 200))]
            self.clean()
            self.assertFalse(old.exists())
            for path in [newest, *unrelated]:
                self.assertEqual(path.read_bytes(), b"cache fixture")

    def test_dependency_pruning_preserves_third_party_and_profile_retention(self):
        for profile, retained in (("debug", 2), ("release", 2), ("dist", 1)):
            workspace, unrelated = [], []
            for index in range(3):
                suffix = f"{index:016x}.rlib"
                workspace.append(self.artifact(f"{profile}/deps/libhl2_ui-{suffix}", 100 + index))
                unrelated.append(self.artifact(f"{profile}/deps/libthird_party-{suffix}", 100 + index))
            self.clean()
            self.assertEqual([p.exists() for p in workspace], [False] * (3 - retained) + [True] * retained)
            self.assertTrue(all(p.exists() for p in unrelated))

    def test_dry_run_keeps_all_candidate_bytes(self):
        paths = [self.artifact(f"debug/incremental/hl2_ui-{h}/cache", t)
                 for h, t in (("aaa", 100), ("bbb", 200))]
        paths += [self.artifact(f"release/deps/libhl2_ui-{i:016x}.rlib", 100 + i)
                  for i in range(2)]
        self.clean(dry_run=True)
        self.assertTrue(all(p.read_bytes() == b"cache fixture" for p in paths))

    @unittest.skipIf(os.name == "nt" or shutil.which("bash") is None, "POSIX build script")
    def test_build_script_cleans_first_unless_opted_out(self):
        repo = Path(__file__).resolve().parents[2]
        shutil.copy2(repo / "scripts/build-bevy.sh", self.root / "build-bevy.sh")
        (self.root / "scripts").mkdir()
        shutil.move(str(self.root / "build-bevy.sh"), self.root / "scripts/build-bevy.sh")
        shader = self.root / "crates/hl2-bevy/assets/fixture.wgsl"
        shader.parent.mkdir(parents=True)
        shader.write_text("fixture", encoding="utf-8")
        fake_bin = self.root / "fake-bin"
        fake_bin.mkdir()
        for path, body in (("scripts/clean-target.sh", "touch cleaner-ran\n"),
                           ("fake-bin/cargo", "touch cargo-ran\n")):
            script = self.root / path
            script.write_text("#!/usr/bin/env bash\n" + body, encoding="utf-8")
            script.chmod(0o755)
        env = {**os.environ, "PATH": f"{fake_bin}{os.pathsep}{os.environ.get('PATH', '')}"}
        for flags, cleans in (([], True), (["--no-clean"], False), (["--debug"], True)):
            with self.subTest(flags=flags):
                for marker in ("cleaner-ran", "cargo-ran"):
                    (self.root / marker).unlink(missing_ok=True)
                # The fake cargo builds nothing, so packaging fails after the build step.
                subprocess.run(["bash", "scripts/build-bevy.sh", *flags], cwd=self.root, env=env,
                               capture_output=True, text=True, timeout=15)
                self.assertTrue((self.root / "cargo-ran").exists())
                self.assertEqual((self.root / "cleaner-ran").exists(), cleans)

    @unittest.skipIf(os.name == "nt" or shutil.which("bash") is None, "POSIX launcher")
    def test_launcher_default_cleaning_and_opt_out(self):
        repo = Path(__file__).resolve().parents[2]
        shutil.copy2(repo / "launch-bevy.sh", self.root / "launch-bevy.sh")
        for path, body in (
            ("scripts/clean-target.sh", "touch cleaner-ran\n"),
            ("scripts/build-bevy.sh", "printf '%s\\n' \"$@\" > build-args\n"),
            ("bin/hl2-bevy", "printf '%s\\n' \"$@\" > game-args\n"),
        ):
            script = self.root / path
            script.parent.mkdir(parents=True, exist_ok=True)
            script.write_text("#!/usr/bin/env bash\n" + body, encoding="utf-8")
            script.chmod(0o755)
        for flags, cleans in (([], True), (["--no-clean"], False),
                              (["--rebuild"], True), (["--debug"], True),
                              (["--rebuild", "--no-clean"], False),
                              (["--debug", "--no-clean"], False), (["--no-build"], False)):
            with self.subTest(flags=flags):
                (self.root / "build-args").unlink(missing_ok=True)
                (self.root / "cleaner-ran").unlink(missing_ok=True)
                run = subprocess.run(["bash", "launch-bevy.sh", *flags, "--map", "fixture"],
                                     cwd=self.root, capture_output=True, text=True, timeout=15)
                self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
                # The build script cleans; the launcher only passes the opt-out on.
                self.assertFalse((self.root / "cleaner-ran").exists())
                self.assertEqual((self.root / "game-args").read_text().splitlines(), ["--map", "fixture"])
                built = self.root / "build-args"
                self.assertEqual(built.exists(), "--no-build" not in flags)
                if built.exists():
                    args = built.read_text().splitlines()
                    self.assertEqual("--no-clean" not in args, cleans)
                    if "--debug" in flags:
                        self.assertIn("--debug", args)


if __name__ == "__main__":
    unittest.main()
