"""Scheduling invariants of the Station 02 locality workloads."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from sector_locality import SECTORS, STOPS, TEAMS, fixture, main, population


class SectorLocalityTests(unittest.TestCase):
    def test_spawn_counts_and_unique_positions(self):
        for case, counts in (
            ("control", (0, 0, 0)), ("crowds", (16, 32, 32)),
            ("fights", (16, 32, 32)), ("supplies", (20, 32, 32)),
        ):
            roster = population(case)
            self.assertEqual(
                tuple(sum(e[0] == sector for e in roster.values()) for sector in SECTORS),
                counts,
            )
            self.assertEqual(len({e[3] for e in roster.values()}), len(roster))

    def test_control_and_workload_cases_share_the_same_route_and_timing(self):
        for case in TEAMS:
            steps = fixture(case)
            self.assertEqual([s["tick"] for s in steps], sorted({s["tick"] for s in steps}))
            neutral = [s.copy() for s in steps if not any(
                a["command"].startswith("spawn_at ") for a in s.get("ui", [])
            )]
            if case == "fights":
                start = next(s for s in neutral if s.get("ui") == [{"action": "command", "command": "notarget"}])
                rivalries = start.pop("actions")
                self.assertEqual(len(rivalries), 32)
                self.assertTrue(all(a["target"].startswith("locality_plaza_") and
                                    a["input"] == "SetRelationship" and len(a["parameter"].split()) == 48
                                    for a in rivalries))
            self.assertEqual(neutral, fixture("control"))
            self.assertEqual([s["label"] for s in steps if s.get("profile_window") == "end"],
                             [f"{name}-end" for name, *_ in STOPS])

    def test_strict_mode_reports_distant_activity_after_writing_results(self):
        for thinking, awake in ((0, 0), (1, 0), (0, 1)):
            with self.subTest(thinking=thinking, awake=awake), tempfile.TemporaryDirectory() as out:
                row = {"case": "crowds", "spawned": 80, "phases": {
                    "before-trashcan": {"far_thinking": thinking, "far_awake": awake},
                }}
                argv = ["sector_locality", "--out", out, "--rounds", "1", "--case", "crowds", "--require-local"]
                with patch("sector_locality.run", return_value=row), patch("sys.argv", argv), \
                        contextlib.redirect_stdout(io.StringIO()):
                    if thinking or awake:
                        with self.assertRaisesRegex(SystemExit, "Distant benchmark entities"):
                            main()
                    else:
                        main()
                self.assertEqual(json.loads((Path(out) / "summary.json").read_text()), [row])
