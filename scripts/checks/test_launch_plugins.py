"""launch-bevy.sh plugin selection, driven with a stand-in build and executable."""
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import time
import unittest

REPO = Path(__file__).resolve().parents[2]
FAKE_BUILD = """#!/usr/bin/env bash
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
plugins="advanced-options,debug-tools"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --plugins) plugins="$2"; shift 2 ;;
        *) shift ;;
    esac
done
echo "$plugins" >> "$root/builds.log"
mkdir -p "$root/bin/bevy-assets"
printf '#!/usr/bin/env bash\\necho "game $*"\\n' > "$root/bin/hl2-bevy"
chmod +x "$root/bin/hl2-bevy"
printf '{\\n  "profile": "release",\\n  "plugins": "%s"\\n}\\n' "$plugins" > "$root/bin/build-bevy-info.json"
"""


@unittest.skipIf(os.name == "nt", "launch-bevy.sh is the macOS and Linux launcher")
class LaunchPlugins(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        shutil.copy(REPO / "launch-bevy.sh", self.root / "launch-bevy.sh")
        (self.root / "scripts").mkdir()
        build = self.root / "scripts/build-bevy.sh"
        build.write_text(FAKE_BUILD)
        build.chmod(build.stat().st_mode | stat.S_IXUSR)
        (self.root / "bin").mkdir()
        (self.root / "plugins/x").mkdir(parents=True)
        (self.root / "crates").mkdir()

    def tearDown(self):
        self.temporary.cleanup()

    def launch(self, *args, ok=True):
        done = subprocess.run(["bash", str(self.root / "launch-bevy.sh"), *args], capture_output=True,
                              text=True, cwd=self.root,
                              env=dict(os.environ, GIT_DIR=str(self.root / "no-git")))
        self.assertEqual(done.returncode == 0, ok, done.stdout + done.stderr)
        return done.stdout + done.stderr

    def builds(self):
        log = self.root / "builds.log"
        return log.read_text().split() if log.exists() else []

    def test_the_first_build_has_both_plugins(self):
        self.launch()
        self.assertEqual(self.builds(), ["advanced-options,debug-tools"])

    def test_a_selection_builds_once_and_then_runs_the_game(self):
        self.launch("--plugins", "none")
        output = self.launch("--plugins", "none", "--map", "x")
        self.assertEqual(self.builds(), ["none"])
        self.assertIn("game --map x", output)
        self.assertNotIn("--plugins", output.split("game ")[-1])

    def test_changing_the_selection_rebuilds(self):
        self.launch("--plugins=debug-tools")
        self.launch("--plugins", "advanced-options")
        self.assertEqual(self.builds(), ["debug-tools", "advanced-options"])

    def test_the_order_of_names_does_not_matter(self):
        self.launch("--plugins", "advanced-options,debug-tools")
        self.launch("--plugins", "debug-tools,advanced-options")
        self.assertEqual(len(self.builds()), 1)

    def test_without_the_option_the_last_selection_stays(self):
        self.launch("--plugins", "none")
        self.launch()
        self.assertEqual(self.builds(), ["none"])

    def test_an_automatic_rebuild_keeps_the_selection(self):
        self.launch("--plugins", "debug-tools")
        time.sleep(1.1)
        (self.root / "plugins/x/lib.rs").write_text("// changed")
        self.launch()
        self.assertEqual(self.builds(), ["debug-tools", "debug-tools"])

    def test_no_build_refuses_a_selection_the_build_lacks(self):
        self.launch("--plugins", "none")
        self.assertIn("existing build has: none",
                      self.launch("--no-build", "--plugins", "debug-tools", ok=False))
        self.launch("--no-build", "--plugins", "none")
        self.assertEqual(self.builds(), ["none"])

    def test_a_missing_value_is_an_error(self):
        self.launch("--plugins", ok=False)
        self.assertEqual(self.builds(), [])


if __name__ == "__main__":
    unittest.main()
