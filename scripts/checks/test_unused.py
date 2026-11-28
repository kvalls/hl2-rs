import tempfile
import unittest
from pathlib import Path

from architecture import scan, test_mask
from unused import (
    find_orphan_rust_files,
    find_orphan_shaders,
    find_unused_dependencies,
    unused_lint_allowances,
)


def _count(source: str, path: str = "crates/demo/src/lib.rs") -> tuple[int, int]:
    code, _ = scan(source)
    mask = test_mask(Path(path), code)
    return unused_lint_allowances(code, mask)


class TestUnusedLintAllowances(unittest.TestCase):
    def test_counts_single_and_multiline_dead_code_allowances(self):
        src = "\n".join([
            "#[allow(dead_code)]",
            "fn a() {}",
            "#[expect(dead_code, reason = \"keep\")]",
            "fn b() {}",
            "#![allow(dead_code)]",
            "#[cfg_attr(not(test), allow(dead_code))]",
            "fn c() {}",
            "#[cfg_attr(",
            "    not(test),",
            "    allow(dead_code)",
            ")]",
            "fn d() {}",
        ])
        self.assertEqual(_count(src), (5, 0))

    def test_counts_unused_and_unreachable_but_allows_cfg_attr_unused_mut(self):
        src = "\n".join([
            "#[allow(unused)]",
            "#[allow(unused_imports)]",
            "#[allow(unused_variables)]",
            "#[allow(unused_mut)]",
            "#[allow(unreachable_code)]",
            "fn bad() {}",
            "#[cfg_attr(not(any(feature = \"a\", feature = \"b\")), allow(unused_mut))]",
            "fn gated() { let mut x = 1; let _ = x; }",
        ])
        self.assertEqual(_count(src), (0, 5))

    def test_ignores_comments_strings_and_tests(self):
        src = "\n".join([
            "// #[allow(dead_code)] #[allow(unused_imports)]",
            "const DOC: &str = \"#[allow(dead_code)]\";",
            "#[cfg(test)]",
            "mod tests {",
            "    #[allow(dead_code)]",
            "    #[allow(unused)]",
            "    fn helper() {}",
            "}",
        ])
        self.assertEqual(_count(src), (0, 0))
        self.assertEqual(_count("#[allow(dead_code)]\nfn helper() {}\n", "crates/demo/src/foo/tests.rs"), (0, 0))


class TestOrphanRustFiles(unittest.TestCase):
    def test_accepts_mod_path_bin_and_rejects_orphans(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            pkg = repo / "crates" / "demo"
            (pkg / "src" / "sub").mkdir(parents=True)
            (pkg / "src" / "bin").mkdir(parents=True)
            (pkg / "Cargo.toml").write_text('[package]\nname = "demo"\n', encoding="utf-8")
            (pkg / "src" / "lib.rs").write_text(
                'mod sub;\n#[path = "custom_mod.rs"]\nmod custom;\n',
                encoding="utf-8",
            )
            (pkg / "src" / "sub" / "mod.rs").write_text("mod child;\n", encoding="utf-8")
            (pkg / "src" / "sub" / "child.rs").write_text("pub fn ok() {}\n", encoding="utf-8")
            (pkg / "src" / "custom_mod.rs").write_text("pub fn c() {}\n", encoding="utf-8")
            (pkg / "src" / "bin" / "tool.rs").write_text("fn main() {}\n", encoding="utf-8")

            self.assertEqual(find_orphan_rust_files(repo, {"demo": pkg}, scan), [])

            (pkg / "src" / "orphan.rs").write_text("pub fn dead() {}\n", encoding="utf-8")
            (pkg / "src" / "sub" / "nested_orphan.rs").write_text("pub fn dead() {}\n", encoding="utf-8")
            issues = find_orphan_rust_files(repo, {"demo": pkg}, scan)
            self.assertEqual(len(issues), 2)
            self.assertTrue(any("crates/demo/src/orphan.rs:" in msg for msg in issues))
            self.assertTrue(any("crates/demo/src/sub/nested_orphan.rs:" in msg for msg in issues))


class TestOrphanShaders(unittest.TestCase):
    def test_detects_unreferenced_shaders_and_ignores_self_mentions(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            pkg = repo / "crates" / "demo"
            (pkg / "src").mkdir(parents=True)
            (pkg / "assets" / "shaders").mkdir(parents=True)
            (pkg / "src" / "lib.rs").write_text(
                'pub const SHADER: &str = "shaders/main.wgsl";\n',
                encoding="utf-8",
            )
            (pkg / "assets" / "shaders" / "main.wgsl").write_text(
                '#import "shaders/common.wgsl"::util\n',
                encoding="utf-8",
            )
            (pkg / "assets" / "shaders" / "common.wgsl").write_text(
                "// helper\n",
                encoding="utf-8",
            )
            self.assertEqual(find_orphan_shaders(repo, {"demo": pkg}, scan), [])

            (pkg / "assets" / "shaders" / "dead.wgsl").write_text(
                "// dead.wgsl\nfn f() {}\n",
                encoding="utf-8",
            )
            issues = find_orphan_shaders(repo, {"demo": pkg}, scan)
            self.assertEqual(len(issues), 1)
            self.assertIn("crates/demo/assets/shaders/dead.wgsl:", issues[0])

    def test_wesl_shaders_count_module_imports_as_references(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            pkg = repo / "crates" / "demo"
            (pkg / "src").mkdir(parents=True)
            (pkg / "assets" / "shaders").mkdir(parents=True)
            (pkg / "src" / "lib.rs").write_text(
                'pub const SHADER: &str = "shaders/main.wesl";\n',
                encoding="utf-8",
            )
            (pkg / "assets" / "shaders" / "main.wesl").write_text(
                "import package::shaders::common::util;\n",
                encoding="utf-8",
            )
            (pkg / "assets" / "shaders" / "common.wesl").write_text(
                "fn util() {}\n",
                encoding="utf-8",
            )
            self.assertEqual(find_orphan_shaders(repo, {"demo": pkg}, scan), [])

            (pkg / "assets" / "shaders" / "dead.wesl").write_text(
                "// shaders::dead:: is only this file's own name\nfn f() {}\n",
                encoding="utf-8",
            )
            issues = find_orphan_shaders(repo, {"demo": pkg}, scan)
            self.assertEqual(len(issues), 1)
            self.assertIn("crates/demo/assets/shaders/dead.wesl:", issues[0])


class TestUnusedDependencies(unittest.TestCase):
    def test_detects_unused_member_and_workspace_deps_with_crlf(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            pkg = repo / "crates" / "demo"
            (pkg / "src").mkdir(parents=True)
            (pkg / "tests").mkdir(parents=True)
            (repo / "Cargo.toml").write_bytes(
                b'[workspace.dependencies]\r\n'
                b'serde-json = "1"\r\n'
                b'unused-ws = "1"\r\n'
            )
            (pkg / "Cargo.toml").write_bytes(
                b'[package]\r\nname = "demo"\r\n'
                b'[dependencies]\r\n'
                b'serde-json = { workspace = true }\r\n'
                b'unify = "0.1" # feature-unification: enables subfeature\r\n'
                b'stale = "0.2"\r\n'
                b'[dev-dependencies]\r\n'
                b'anyhow = "1"\r\n'
                b'[[bin]]\r\n'
                b'name = "demo-bin"\r\n'
                b'path = "src/lib.rs"\r\n'
            )
            (pkg / "src" / "lib.rs").write_text(
                "pub fn f() -> serde_json::Value { todo!() }\n",
                encoding="utf-8",
            )
            (pkg / "tests" / "it.rs").write_text(
                "#[test]\nfn t() -> anyhow::Result<()> { Ok(()) }\n",
                encoding="utf-8",
            )

            issues = find_unused_dependencies(repo, {"demo": pkg}, scan)
            self.assertEqual(len(issues), 2)
            self.assertTrue(any("crates/demo/Cargo.toml: unused dependency 'stale'" in msg for msg in issues))
            self.assertTrue(any("Cargo.toml: workspace dependency 'unused-ws'" in msg for msg in issues))


if __name__ == "__main__":
    unittest.main()
