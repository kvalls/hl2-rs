import unittest
from pathlib import Path

from coverage import check_baseline, raise_baseline, summarize_export


class TestCoverageSummary(unittest.TestCase):
    def test_summarizes_pure_crates_and_excludes_tests_external_and_bevy(self):
        repo = Path("/workspace/hl2rs")
        export_json = {
            "data": [{
                "files": [
                    {
                        "filename": "/workspace/hl2rs/crates/modkit-core/src/math.rs",
                        "summary": {"lines": {"covered": 80, "count": 100}},
                    },
                    {
                        "filename": "/workspace/hl2rs/crates/modkit-core/src/math/tests.rs",
                        "summary": {"lines": {"covered": 50, "count": 50}},
                    },
                    {
                        "filename": r"C:\repo\crates\source-ui\src\widgets_tests.rs",
                        "summary": {"lines": {"covered": 10, "count": 10}},
                    },
                    {
                        "filename": r"C:\repo\crates\source-ui\src\layout.rs",
                        "summary": {"lines": {"covered": 30, "count": 60}},
                    },
                    {
                        "filename": "/workspace/hl2rs/plugins/hl2-debug/src/bevy/overlay.rs",
                        "summary": {"lines": {"covered": 1, "count": 90}},
                    },
                    {
                        "filename": "/external/.cargo/registry/src/foo/lib.rs",
                        "summary": {"lines": {"covered": 0, "count": 200}},
                    },
                ]
            }]
        }
        summary, rows = summarize_export(export_json, repo)
        self.assertEqual(
            summary,
            {
                "modkit-core": {"covered": 80, "count": 100, "percent": 80.0},
                "source-ui": {"covered": 30, "count": 60, "percent": 50.0},
            },
        )
        self.assertEqual([r["crate"] for r in rows], ["source-ui", "modkit-core"])
        self.assertEqual(rows[0]["uncovered"], 30)


class TestCoverageBaseline(unittest.TestCase):
    def test_allows_small_variance_and_fails_real_regression(self):
        baseline = {"crates": {"modkit-core": {"percent": 80.0, "covered": 80, "count": 100}}}
        within = {"modkit-core": {"percent": 78.5, "covered": 157, "count": 200}}
        dropped = {"modkit-core": {"percent": 77.9, "covered": 779, "count": 1000}}
        self.assertEqual(check_baseline(within, baseline, tolerance=2.0), [])
        failures = check_baseline(dropped, baseline, tolerance=2.0)
        self.assertEqual(len(failures), 1)
        self.assertIn("dropped more than 2.00%", failures[0])

    def test_raise_baseline_only_moves_upward(self):
        baseline = {
            "crates": {
                "modkit-core": {"percent": 80.0, "covered": 80, "count": 100},
                "source-ui": {"percent": 70.0, "covered": 70, "count": 100},
            }
        }
        current = {
            "modkit-core": {"percent": 84.0, "covered": 84, "count": 100},
            "source-ui": {"percent": 69.0, "covered": 69, "count": 100},
        }
        raised = raise_baseline(baseline, current)["crates"]
        self.assertEqual(raised["modkit-core"]["percent"], 84.0)
        self.assertEqual(raised["source-ui"]["percent"], 70.0)


if __name__ == "__main__":
    unittest.main()
