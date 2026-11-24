import tempfile
import unittest
from pathlib import Path

from architecture import compare, lowered, scan
from unit_tests import active_tests_by_crate, count_active_tests, find_untested_pure_modules


class TestCountActiveTests(unittest.TestCase):
    def test_counts_active_tests_and_skips_ignored_comments_and_strings(self):
        src = "\n".join([
            "// #[test]",
            'const SAMPLE: &str = "#[test]";',
            "#[test]",
            "fn active_one() {}",
            "#[test]",
            "#[should_panic]",
            "pub fn active_two() {}",
            "#[test]",
            "#[ignore]",
            "fn skipped_one() {}",
            '#[ignore = "owned install"]',
            "#[test]",
            "fn skipped_two() {}",
        ])
        code, _ = scan(src)
        self.assertEqual(count_active_tests(code), 2)


class TestFindUntestedPureModules(unittest.TestCase):
    def test_checks_pure_crates_and_exempts_bevy_host_and_apps(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            pure = repo / "crates" / "modkit-core"
            (pure / "src" / "dir_tested").mkdir(parents=True)
            (pure / "src" / "empty_mod").mkdir(parents=True)
            (pure / "src" / "lib.rs").write_text("mod inline; mod dir_tested; mod empty_mod;\n", encoding="utf-8")
            (pure / "src" / "inline.rs").write_text("#[test]\nfn t() {}\n", encoding="utf-8")
            (pure / "src" / "dir_tested" / "mod.rs").write_text("#[cfg(test)]\nmod tests;\n", encoding="utf-8")
            (pure / "src" / "dir_tested" / "tests.rs").write_text("#[test]\nfn ok() {}\n", encoding="utf-8")
            (pure / "src" / "sibling.rs").write_text("pub fn s() {}\n", encoding="utf-8")
            (pure / "src" / "sibling_tests.rs").write_text("#[test]\nfn s_ok() {}\n", encoding="utf-8")
            (pure / "src" / "ignored_only.rs").write_text("#[test]\n#[ignore]\nfn ign() {}\n", encoding="utf-8")
            (pure / "src" / "empty_mod" / "mod.rs").write_text("pub fn e() {}\n", encoding="utf-8")

            bevy = repo / "crates" / "hl2-bevy"
            (bevy / "src").mkdir(parents=True)
            (bevy / "src" / "untested_ok.rs").write_text("pub fn gpu() {}\n", encoding="utf-8")

            packages = {"modkit-core": pure, "hl2-bevy": bevy}
            untested = find_untested_pure_modules(repo, packages, scan)
            self.assertEqual(
                untested,
                [
                    "crates/modkit-core/src/empty_mod/mod.rs",
                    "crates/modkit-core/src/ignored_only.rs",
                ],
            )
            self.assertEqual(active_tests_by_crate(packages, scan), {"modkit-core": 3})


class TestMinTestsAndUntestedRatchets(unittest.TestCase):
    def test_compare_and_lowered_enforce_floors_without_blocking_additions(self):
        baseline = {
            "functions": {},
            "files": {},
            "test_files": {},
            "counts": {},
            "duplicates": [],
            "min_tests": {"modkit-core": 10},
            "untested_modules": ["crates/modkit-core/src/old.rs"],
        }
        grown = {
            "functions": {},
            "files": {},
            "test_files": {},
            "counts": {},
            "duplicates": [],
            "min_tests": {"modkit-core": 12},
            "untested_modules": ["crates/modkit-core/src/old.rs"],
        }
        failures, shrunk = compare(grown, baseline, {})
        self.assertEqual((failures, shrunk), ([], []))
        updated = lowered(baseline, grown)
        self.assertEqual(updated["min_tests"], {"modkit-core": 12})

        dropped = {
            **grown,
            "min_tests": {"modkit-core": 9},
            "untested_modules": [
                "crates/modkit-core/src/old.rs",
                "crates/modkit-core/src/new_untested.rs",
            ],
        }
        failures, _ = compare(dropped, baseline, {})
        self.assertEqual(len(failures), 2)
        self.assertTrue(any("9 active unit tests" in f for f in failures))
        self.assertTrue(any("new_untested.rs" in f for f in failures))

        cleaned = {**grown, "untested_modules": []}
        failures, shrunk = compare(cleaned, baseline, {})
        self.assertEqual(failures, [])
        self.assertEqual(shrunk, ["crates/modkit-core/src/old.rs: now has unit tests"])


if __name__ == "__main__":
    unittest.main()
