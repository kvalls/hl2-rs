"""Authored workload controls; no installed game or GPU required on any platform."""
import copy
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import check_effect_pool_performance as bench


def report(case):
    size = list(bench.size_for(case))
    frozen, animated = case == "frozen-fire", case == "animated-fire"
    crowd = case.startswith("plaza-crowd")
    events = 1 if frozen else 4 if animated else 0
    spawns = [{"entity": i} for i in range(30)] if crowd else []
    actors = {str(i): {"class": {"Manhack": {}}, "npc": {
        "state": "Combat", "enemy": 18446744073709551615}} for i in range(30)} if crowd else {}
    effects = {"sprite_errors": [], "model_errors": [], "pose_mismatches": 0,
        "missing_particle_draws": 0, "sprite_draws": 142 if frozen else 62,
        "pooled_quads": 142 if frozen else 110,
        "particle_emitters": {"capacity_rejections": 0, "events": events, "emitted": {}}}
    return {"texture_errors": [], "render": {"simulation": {"script_finished": True,
        "host_tick": 600, "samples": [{"frame": size}], "paused": frozen, "player": {},
        "gameplay": {"time": 1.275 if frozen else 9, "npc_spawns": spawns,
            "projectiles": {"diagnostics": {"grenade_detonations": events}},
            "ai": {"enabled": True, "actors": actors}}}, "presentation": {
        "gpu_completion": {"errors": 0}, "pose_or_visibility_mismatches": 0,
        "effects": effects, "hud": {"viewport": size, "developer_overlay": {
            "benchmark": {"measured_frames": 1000, "mean_frame_ms": 8}}}}}}


class WorkloadAdmission(unittest.TestCase):
    def test_all_five_authored_workloads_are_admitted(self):
        for case in bench.CASES:
            with self.subTest(case=case):
                result = bench.validate_report(report(case), case)
                self.assertEqual(result["host_tick"], 600)
                self.assertEqual(result["hunting"], 30 if case.startswith("plaza-crowd") else 0)

    def test_dropped_work_never_becomes_a_faster_accepted_result(self):
        original = report("frozen-fire")
        for field, value in (("sprite_draws", 0), ("missing_particle_draws", 1),
                             ("pose_mismatches", 1), ("sprite_errors", ["missing material"])):
            with self.subTest(field=field):
                data = copy.deepcopy(original)
                data["render"]["presentation"]["effects"][field] = value
                with self.assertRaises(ValueError):
                    bench.validate_report(data, "frozen-fire")
        data = copy.deepcopy(original)
        data["render"]["presentation"]["effects"]["particle_emitters"]["capacity_rejections"] = 1
        with self.assertRaises(ValueError):
            bench.validate_report(data, "frozen-fire")

    def test_retina_must_be_actual_internal_resolution_not_only_a_cli_request(self):
        data = report("plaza-crowd-retina")
        data["render"]["simulation"]["samples"][-1]["frame"] = [1920, 1080]
        with self.assertRaises(ValueError):
            bench.validate_report(data, "plaza-crowd-retina")

    def test_nonfinite_timing_is_rejected(self):
        for value in (float("nan"), float("inf"), -float("inf")):
            with self.subTest(value=value):
                data = report("quiet")
                data["render"]["presentation"]["hud"]["developer_overlay"]["benchmark"]["mean_frame_ms"] = value
                with self.assertRaises(ValueError):
                    bench.validate_report(data, "quiet")

    def test_incomplete_or_idle_crowd_is_rejected(self):
        for change in ("unfinished", "failed spawn", "idle", "wrong class"):
            with self.subTest(change=change):
                data = report("plaza-crowd")
                sim = data["render"]["simulation"]
                if change == "unfinished":
                    sim["script_finished"] = False
                elif change == "failed spawn":
                    sim["gameplay"]["npc_spawns"][0]["entity"] = None
                elif change == "idle":
                    sim["gameplay"]["ai"]["actors"]["0"]["npc"]["state"] = "Idle"
                else:
                    sim["gameplay"]["ai"]["actors"]["0"]["class"] = {"Scanner": {}}
                with self.assertRaises(ValueError):
                    bench.validate_report(data, "plaza-crowd")

    def test_contained_windows_are_weighted_without_partial_window_interpolation(self):
        windows = [{"start_unix_seconds": i, "end_unix_seconds": i + 1,
                    "elapsed_seconds": 1, "frames": 100 if i % 2 else 50,
                    "max_frame_ms": 20} for i in range(9)]
        result = bench.steady_window({"frame_windows": {"windows": windows}})
        self.assertEqual((result["frames"], result["elapsed_seconds"]), (400, 5))
        self.assertEqual(result["mean_frame_ms"], 12.5)
        self.assertEqual(result["start_seconds_before_final"], 6)
        self.assertEqual(result["end_seconds_before_final"], 1)
        self.assertIsNone(bench.steady_window({"frame_windows": {"windows": []}}))

    def test_package_configuration_is_restored_after_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            exe = Path(folder) / "game.exe"
            path = exe.with_name("hl2-config.json")
            original = b'{"user":"retained"}\r\n'
            path.write_bytes(original)
            with self.assertRaises(RuntimeError):
                with bench.configured(exe, {"video": {}}):
                    raise RuntimeError("child failed")
            self.assertEqual(path.read_bytes(), original)
            path.unlink()
            with bench.configured(exe, {}):
                self.assertTrue(path.exists())
            self.assertFalse(path.exists())

    def test_user_bin_and_shared_package_directories_are_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for name in ("bin", "artifacts/baseline", "artifacts/current"):
                (root / name).mkdir(parents=True)
                (root / name / "game.exe").touch()
            baseline, current = (root / "artifacts" / name / "game.exe" for name in ("baseline", "current"))
            with patch.object(bench, "REPO", root):
                bench.check_paths(baseline, current, root / "artifacts/output")
                for a, b in ((root / "bin/game.exe", current), (baseline, baseline)):
                    with self.assertRaises(ValueError):
                        bench.check_paths(a, b, root / "artifacts/output")

    def test_symlinked_package_config_cannot_modify_another_file(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            outside = root / "user-config.json"
            outside.write_bytes(b"retained user settings")
            exe = root / "isolated" / "game.exe"
            exe.parent.mkdir()
            try:
                exe.with_name("hl2-config.json").symlink_to(outside)
            except (OSError, NotImplementedError) as error:
                self.skipTest(f"symlink creation unavailable: {type(error).__name__}")
            with self.assertRaises(ValueError):
                with bench.configured(exe, {}):
                    self.fail("symlink guard did not reject")
            self.assertEqual(outside.read_bytes(), b"retained user settings")

    def test_matching_reduced_outcomes_cannot_hide_changed_full_gameplay(self):
        rows = []
        for case in bench.CASES:
            data = report(case)
            frame = {"mean_frame_ms": 8, "p95_frame_ms_upper_bound": 10,
                     "p99_frame_ms_upper_bound": 12}
            for version in ("baseline", "current"):
                rows.append({"name": case, "version": version, "round": 1,
                    "outcome": bench.validate_report(data, case), "benchmark": frame,
                    "steady_window": None, "gameplay_state_sha256": "same"})
        self.assertTrue(bench.summarize(rows, 1)["quiet"]["paired_gameplay_states_identical"][0])
        rows[1]["gameplay_state_sha256"] = "changed-inventory-or-ai-or-projectile-state"
        with self.assertRaisesRegex(ValueError, "full gameplay states differ"):
            bench.summarize(rows, 1)

    def test_failed_first_attempt_manifest_is_retained_without_launching(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            executables = []
            for version in ("baseline", "current"):
                exe = root / "artifacts" / version / "game.exe"
                exe.parent.mkdir(parents=True)
                exe.write_bytes(version.encode())
                executables.append(str(exe))
            output = root / "artifacts" / "run"
            output.mkdir()
            manifest = output / "experiment.json"
            original = b'{"retained":"failed first attempt"}\n'
            manifest.write_bytes(original)
            with patch.object(bench, "REPO", root), patch.object(bench.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "fresh output directory"):
                    bench.main(["--baseline", executables[0], "--current", executables[1],
                                "--output", str(output), "--rounds", "1"])
                run.assert_not_called()
            self.assertEqual(manifest.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
