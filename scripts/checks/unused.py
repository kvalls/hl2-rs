"""Unused-code, orphan-file, orphan-shader, and unused-dependency guardrails (stdlib only)."""

import re
from pathlib import Path

UNUSED_LINTS = frozenset({
    "unused",
    "unused_imports",
    "unused_variables",
    "unused_mut",
    "unused_assignments",
    "unused_macros",
    "unreachable_code",
})

MOD_DECL = re.compile(r"\bmod\s+(?:r#)?([A-Za-z_][A-Za-z0-9_]*)\s*;")
PATH_ATTR = re.compile(r'#\[\s*path\s*=\s*"([^"]+)"\s*\]')
INCLUDE_MACRO = re.compile(r'\binclude!\s*\(\s*"([^"]+)"\s*\)')
ATTR_LINT_LIST = re.compile(r"\b(allow|expect)\s*\(")
TARGET_PATH_KEY = re.compile(r'^path\s*=\s*"([^"]+)"')
MANIFEST_TABLES = frozenset({"lib", "bin", "test", "bench", "example"})
DEP_TABLE = re.compile(r"(?:^|\.)(?:dev-|build-)?dependencies$")
DOTTED_DEP_TABLE = re.compile(r"(?:^|\.)(?:dev-|build-)?dependencies\.([A-Za-z0-9_-]+)$")


def _extract_attr_blocks(code: list[str], mask: list[bool]) -> list[str]:
    """Return full inner text of every `#[...]` / `#![...]` attribute starting outside tests."""
    blocks = []
    i = 0
    while i < len(code):
        if mask[i]:
            i += 1
            continue
        line = code[i]
        pos = 0
        while pos < len(line):
            idx_outer = line.find("#[", pos)
            idx_inner = line.find("#![", pos)
            candidates = [idx for idx in (idx_outer, idx_inner) if idx != -1]
            if not candidates:
                break
            start = min(candidates)
            prefix_len = 3 if start == idx_inner else 2
            depth = 1
            parts = [line[start + prefix_len :]]
            j = i
            closed = False
            while j < len(code):
                seg = parts[-1] if j == i else code[j]
                limit = len(seg)
                idx = 0
                while idx < limit:
                    ch = seg[idx]
                    if ch == "[":
                        depth += 1
                    elif ch == "]":
                        depth -= 1
                        if depth == 0:
                            if j == i:
                                parts[-1] = seg[:idx]
                                pos = start + prefix_len + idx + 1
                            else:
                                parts.append(seg[:idx])
                                pos = idx + 1
                            i = j
                            line = code[i]
                            closed = True
                            break
                    idx += 1
                if closed:
                    break
                if j > i:
                    parts.append(code[j])
                j += 1
            blocks.append(" ".join(p.strip() for p in parts if p.strip()))
            if not closed:
                i = len(code)
                break
        i += 1
    return blocks


def _balanced_parens(text: str, open_idx: int) -> tuple[str, int]:
    """Return (contents inside `(...)`, index after matching `)`) starting at `open_idx`."""
    depth = 1
    idx = open_idx + 1
    while idx < len(text) and depth > 0:
        if text[idx] == "(":
            depth += 1
        elif text[idx] == ")":
            depth -= 1
        idx += 1
    end = idx - 1 if depth == 0 else len(text)
    return text[open_idx + 1 : end], idx


def unused_lint_allowances(code: list[str], mask: list[bool]) -> tuple[int, int]:
    """Count production `(dead_code allowances, unused* allowances)` across single/multiline attrs.

    Feature-gated `cfg_attr(..., allow(unused_mut))` is excluded because optional-feature builds
    legitimately need conditional mutability.
    """
    dead_code = 0
    unused = 0
    for attr in _extract_attr_blocks(code, mask):
        is_cfg_attr = bool(re.match(r"^\s*cfg_attr\s*\(", attr))
        pos = 0
        while pos < len(attr):
            match = ATTR_LINT_LIST.search(attr, pos)
            if not match:
                break
            open_paren = match.end() - 1
            inner, next_pos = _balanced_parens(attr, open_paren)
            for item in inner.split(","):
                token = item.strip()
                if token == "dead_code":
                    dead_code += 1
                elif token in UNUSED_LINTS:
                    if not (token == "unused_mut" and is_cfg_attr):
                        unused += 1
            pos = next_pos
    return dead_code, unused


def _manifest_entry_points(manifest: Path, pkg_dir: Path) -> set[Path]:
    """Return explicit `[lib]` / `[[bin]]` / `[[bench]]` / `[[test]]` / `[[example]]` paths."""
    targets = set()
    table = None
    for raw in manifest.read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw.split("#")[0].strip()
        header = re.match(r"^\[+([^\[\]]+)\]+$", line)
        if header:
            table = header.group(1).strip()
            continue
        if table in MANIFEST_TABLES:
            path_match = TARGET_PATH_KEY.match(line)
            if path_match:
                targets.add((pkg_dir / path_match.group(1)).resolve())
    return targets


def _explicit_mod_targets(file_path: Path, plain: list[str]) -> set[Path]:
    """Return `.rs` targets referenced via `#[path = "..."]` or `include!("...")` in one file."""
    targets = set()
    bases = (file_path.parent, file_path.with_suffix(""))
    for line in plain:
        for pattern in (PATH_ATTR, INCLUDE_MACRO):
            for match in pattern.finditer(line):
                rel = match.group(1)
                for base in bases:
                    candidate = (base / rel).resolve()
                    if candidate.is_file():
                        targets.add(candidate)
    return targets


def _declared_mods(code: list[str]) -> set[str]:
    """Return all external `mod <name>;` names declared in comment/string-stripped lines."""
    mods = set()
    for line in code:
        for match in MOD_DECL.finditer(line):
            mods.add(match.group(1))
    return mods


def _is_standard_entry_point(path: Path, src_dir: Path) -> bool:
    if path in (src_dir / "lib.rs", src_dir / "main.rs"):
        return True
    try:
        rel_parts = path.relative_to(src_dir).parts
    except ValueError:
        return False
    return (
        len(rel_parts) == 2 and rel_parts[0] == "bin" and path.suffix == ".rs"
    ) or (
        len(rel_parts) == 3 and rel_parts[0] == "bin" and rel_parts[2] == "main.rs"
    )


def find_orphan_rust_files(repo: Path, packages: dict[str, Path], scan_fn) -> list[str]:
    """Fail when any `.rs` file under `src/` is not wired via `mod`, `#[path]`, or manifest target."""
    problems = []
    for crate in sorted(packages):
        pkg_dir = packages[crate]
        src_dir = pkg_dir / "src"
        if not src_dir.is_dir():
            continue
        entry_points = _manifest_entry_points(pkg_dir / "Cargo.toml", pkg_dir)
        rs_files = sorted(src_dir.rglob("*.rs"))
        mods_by_file: dict[Path, set[str]] = {}
        explicit_targets: set[Path] = set()
        for path in rs_files:
            code, plain = scan_fn(path.read_text(encoding="utf-8", errors="replace"))
            mods_by_file[path.resolve()] = _declared_mods(code)
            explicit_targets |= _explicit_mod_targets(path, plain)
        for path in rs_files:
            resolved = path.resolve()
            if resolved in entry_points or resolved in explicit_targets:
                continue
            if _is_standard_entry_point(path, src_dir):
                continue
            if path.name == "mod.rs":
                mod_name = path.parent.name
                parent_dir = path.parent.parent
            else:
                mod_name = path.stem
                parent_dir = path.parent
            if parent_dir == src_dir:
                parents = (src_dir / "lib.rs", src_dir / "main.rs")
            else:
                parents = (parent_dir.with_suffix(".rs"), parent_dir / "mod.rs")
            if any(mod_name in mods_by_file.get(p.resolve(), set()) for p in parents):
                continue
            rel = path.relative_to(repo).as_posix()
            problems.append(
                f"{rel}: unreferenced Rust source file (not declared by mod or #[path]); "
                f"wire it into a module or delete it"
            )
    return problems


def _strip_c_comments(text: str) -> str:
    """Remove `//` and `/* ... */` comments (for `.wgsl` shader checks)."""
    out = []
    i, n = 0, len(text)
    in_block = 0
    while i < n:
        if in_block:
            if text.startswith("*/", i):
                in_block -= 1
                i += 2
            elif text.startswith("/*", i):
                in_block += 1
                i += 2
            else:
                i += 1
        elif text.startswith("//", i):
            nl = text.find("\n", i)
            i = n if nl == -1 else nl
        elif text.startswith("/*", i):
            in_block = 1
            i += 2
        else:
            out.append(text[i])
            i += 1
    return "".join(out)


def find_orphan_shaders(repo: Path, packages: dict[str, Path], scan_fn) -> list[str]:
    """Fail when any `.wgsl` shader under `assets/` is unreferenced by `.rs` or other `.wgsl` files."""
    shaders: list[Path] = []
    rust_plain_chunks: list[str] = []
    for crate in sorted(packages):
        pkg_dir = packages[crate]
        assets_dir = pkg_dir / "assets"
        if assets_dir.is_dir():
            shaders.extend(sorted(assets_dir.rglob("*.wgsl")))
        for rs_path in sorted(pkg_dir.rglob("*.rs")):
            _, plain = scan_fn(rs_path.read_text(encoding="utf-8", errors="replace"))
            rust_plain_chunks.append("\n".join(plain))
    if not shaders:
        return []
    rust_corpus = "\n".join(rust_plain_chunks)
    shader_texts = {
        s.resolve(): _strip_c_comments(s.read_text(encoding="utf-8", errors="replace"))
        for s in shaders
    }
    problems = []
    for shader in sorted(shaders):
        name = shader.name
        resolved = shader.resolve()
        in_rust = name in rust_corpus
        in_other_wgsl = any(
            other_path != resolved and name in other_text
            for other_path, other_text in shader_texts.items()
        )
        if not (in_rust or in_other_wgsl):
            rel = shader.relative_to(repo).as_posix()
            problems.append(f"{rel}: unreferenced shader file; load it or delete it")
    return problems


def parse_manifest_deps(manifest: Path, include_workspace: bool = False) -> dict[str, bool]:
    """Return `{dep_name: is_feature_unification}` for dependencies in a `Cargo.toml`."""
    deps: dict[str, bool] = {}
    table = None
    in_deps = False
    for raw in manifest.read_text(encoding="utf-8", errors="replace").splitlines():
        code_part, _, comment_part = raw.partition("#")
        line = code_part.strip()
        is_unification = "feature-unification" in comment_part.lower()
        header = re.match(r"^\[+([^\[\]]+)\]+$", line)
        if header:
            table = header.group(1).strip()
            if line.startswith("[["):
                in_deps = False
            elif include_workspace:
                in_deps = table == "workspace.dependencies"
            else:
                dotted = DOTTED_DEP_TABLE.search(table)
                if dotted:
                    deps[dotted.group(1)] = deps.get(dotted.group(1), False) or is_unification
                    in_deps = False
                else:
                    in_deps = bool(DEP_TABLE.search(table))
            continue
        if in_deps and line:
            match = re.match(r"^([A-Za-z0-9_-]+)\s*(?:\.|=)", line)
            if match:
                name = match.group(1)
                deps[name] = deps.get(name, False) or is_unification
    return deps


def find_unused_dependencies(repo: Path, packages: dict[str, Path], scan_fn) -> list[str]:
    """Fail when a crate or workspace Cargo.toml declares a dependency not used in source."""
    problems = []
    all_member_deps: set[str] = set()
    for crate in sorted(packages):
        pkg_dir = packages[crate]
        manifest = pkg_dir / "Cargo.toml"
        deps = parse_manifest_deps(manifest)
        all_member_deps.update(deps)
        code_chunks = []
        for sub in ("src", "tests", "benches", "examples"):
            sub_dir = pkg_dir / sub
            if sub_dir.is_dir():
                for rs_path in sorted(sub_dir.rglob("*.rs")):
                    code, _ = scan_fn(rs_path.read_text(encoding="utf-8", errors="replace"))
                    code_chunks.append("\n".join(code))
        build_rs = pkg_dir / "build.rs"
        if build_rs.is_file():
            code, _ = scan_fn(build_rs.read_text(encoding="utf-8", errors="replace"))
            code_chunks.append("\n".join(code))
        combined = "\n".join(code_chunks)
        rel_manifest = manifest.relative_to(repo).as_posix()
        for dep, is_unification in sorted(deps.items()):
            if is_unification:
                continue
            ident = dep.replace("-", "_")
            if not re.search(rf"\b{re.escape(ident)}\b", combined):
                problems.append(
                    f"{rel_manifest}: unused dependency '{dep}' (not referenced in crate source); "
                    f"remove it or mark '# feature-unification: <reason>'"
                )
    root_manifest = repo / "Cargo.toml"
    if root_manifest.is_file():
        ws_deps = parse_manifest_deps(root_manifest, include_workspace=True)
        for dep, is_unification in sorted(ws_deps.items()):
            if not is_unification and dep not in all_member_deps:
                problems.append(
                    f"Cargo.toml: workspace dependency '{dep}' is not used by any member crate; remove it"
                )
    return problems
