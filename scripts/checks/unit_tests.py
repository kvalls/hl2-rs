"""Structural unit-test coverage guardrails (stdlib only)."""

import re
from pathlib import Path

from unused import _explicit_mod_targets

PURE_CRATES = frozenset({
    "modkit-core",
    "source-assets",
    "source-ui",
    "hl2-simulation",
    "hl2-ui",
    "hl2-advanced-options",
    "hl2-debug",
})

TEST_ATTR = re.compile(r"^(?:core::prelude::v1::)?test$")
IGNORE_ATTR = re.compile(r"^ignore(?:\s*=.*)?$")
FN_TOKEN = re.compile(r"\bfn\b")


def count_active_tests(code: list[str]) -> int:
    """Count `#[test]` functions whose attribute stack has no `#[ignore]`."""
    active = 0
    saw_test = False
    saw_ignore = False
    i = 0
    while i < len(code):
        line = code[i]
        pos = 0
        while pos < len(line):
            stripped = line[pos:].lstrip()
            if not stripped:
                break
            pos = len(line) - len(stripped)
            if line.startswith("#![", pos):
                # Skip inner module/crate attributes without disturbing function attribute stacks.
                depth = 1
                j = i
                k = pos + 3
                while j < len(code) and depth > 0:
                    seg = code[j]
                    while k < len(seg) and depth > 0:
                        if seg[k] == "[":
                            depth += 1
                        elif seg[k] == "]":
                            depth -= 1
                        k += 1
                    if depth == 0:
                        i = j
                        line = code[i]
                        pos = k
                        break
                    j += 1
                    k = 0
                else:
                    return active
                continue
            if line.startswith("#[", pos):
                depth = 1
                parts = []
                j = i
                k = pos + 2
                attr_start = k
                while j < len(code) and depth > 0:
                    seg = code[j]
                    seg_start = attr_start if j == i else 0
                    while k < len(seg) and depth > 0:
                        if seg[k] == "[":
                            depth += 1
                        elif seg[k] == "]":
                            depth -= 1
                        k += 1
                    if depth == 0:
                        parts.append(seg[seg_start : k - 1])
                        i = j
                        line = code[i]
                        pos = k
                        break
                    parts.append(seg[seg_start:])
                    j += 1
                    k = 0
                else:
                    return active
                inner = " ".join(p.strip() for p in parts if p.strip()).strip()
                if TEST_ATTR.fullmatch(inner):
                    saw_test = True
                elif IGNORE_ATTR.match(inner):
                    saw_ignore = True
                continue
            # Non-attribute code before the next attribute.
            nxt_attr = line.find("#[", pos)
            nxt_inner = line.find("#![", pos)
            next_candidates = [idx for idx in (nxt_attr, nxt_inner) if idx != -1]
            chunk_end = min(next_candidates) if next_candidates else len(line)
            chunk = line[pos:chunk_end]
            if FN_TOKEN.search(chunk):
                if saw_test and not saw_ignore:
                    active += 1
                saw_test = False
                saw_ignore = False
            elif any(ch in chunk for ch in "{};"):
                saw_test = False
                saw_ignore = False
            pos = chunk_end
        i += 1
    return active


def active_tests_by_crate(packages: dict[str, Path], scan_fn) -> dict[str, int]:
    """Return active `#[test]` counts per crate across `src/` and `tests/`."""
    totals: dict[str, int] = {}
    for crate in sorted(packages):
        pkg_dir = packages[crate]
        total = 0
        for sub in ("src", "tests"):
            sub_dir = pkg_dir / sub
            if not sub_dir.is_dir():
                continue
            for path in sorted(sub_dir.rglob("*.rs")):
                code, _ = scan_fn(path.read_text(encoding="utf-8", errors="replace"))
                total += count_active_tests(code)
        if total:
            totals[crate] = total
    return totals


def _module_files(src_dir: Path, name: str, scan_fn) -> list[Path]:
    """Return all `.rs` files belonging to top-level module `name` (including sibling/#[path] tests)."""
    files: dict[Path, Path] = {}
    for direct in (src_dir / f"{name}.rs", src_dir / f"{name}_tests.rs"):
        if direct.is_file():
            files[direct.resolve()] = direct
    sub_dir = src_dir / name
    if sub_dir.is_dir():
        for path in sorted(sub_dir.rglob("*.rs")):
            files[path.resolve()] = path
    queue = list(files.values())
    seen = set(files)
    while queue:
        path = queue.pop(0)
        _, plain = scan_fn(path.read_text(encoding="utf-8", errors="replace"))
        for target in sorted(_explicit_mod_targets(path, plain)):
            if target not in seen:
                seen.add(target)
                files[target] = target
                queue.append(target)
    return [files[k] for k in sorted(files)]


def find_untested_pure_modules(repo: Path, packages: dict[str, Path], scan_fn) -> list[str]:
    """Return canonical paths of top-level modules in PURE_CRATES with zero active `#[test]`s."""
    untested = []
    for crate in sorted(PURE_CRATES & set(packages)):
        src_dir = packages[crate] / "src"
        if not src_dir.is_dir():
            continue
        modules: dict[str, Path] = {}
        for entry in sorted(src_dir.iterdir()):
            if entry.is_file() and entry.suffix == ".rs":
                stem = entry.stem
                if stem in {"lib", "main", "tests", "bevy"} or stem.endswith("_tests"):
                    continue
                modules.setdefault(stem, entry)
            elif entry.is_dir():
                name = entry.name
                if name in {"bevy", "tests", "bin"}:
                    continue
                mod_rs = entry / "mod.rs"
                if mod_rs.is_file():
                    modules.setdefault(name, mod_rs)
        for name in sorted(modules):
            canonical = modules[name]
            total = 0
            for path in _module_files(src_dir, name, scan_fn):
                code, _ = scan_fn(path.read_text(encoding="utf-8", errors="replace"))
                total += count_active_tests(code)
            if total == 0:
                untested.append(canonical.relative_to(repo).as_posix())
    return sorted(untested)
