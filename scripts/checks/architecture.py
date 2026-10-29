#!/usr/bin/env python3
"""Architecture guardrails (run by scripts/checks/validate.sh and CI; standard library only).

Fails when a change:
- makes a function longer than FUNCTION_LINES or a file's production code longer than FILE_LINES
  (test code: TEST_LINES), or grows one that already is (listed in architecture-baseline.json);
- adds a dependency between workspace crates that LAYERS does not allow, uses an engine crate
  outside the crate that owns it (ONLY_IN), reads files in a crate that must not (NO_FILE_IO), or
  names HL2 code in the toolkit;
- repeats DUPLICATE_WINDOW significant lines of production code that already exist elsewhere;
- raises a counted smell of a crate (mutable statics, `too_many_arguments` and `dead_code`
  allowances, glob imports in production code);
- makes AGENTS.md longer than AGENTS_BYTES (it holds rules, one line each).

Existing offenders are in architecture-baseline.json and may only shrink. After shrinking one,
run `UPDATE_BASELINE=1 python3 scripts/checks/architecture.py`: it only lowers or removes entries.
`ACCEPT_GROWTH=1` records the current state as it is, growth included: only after merging main
(for upstream code) or with the owner's approval, and say so in MODLOG.
"""
import hashlib
import json
import os
import re
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BASELINE = Path(__file__).with_name("architecture-baseline.json")

FUNCTION_LINES = 100
FILE_LINES = 1000
TEST_LINES = 2000
DUPLICATE_WINDOW = 8
AGENTS_BYTES = 20_000

# Workspace crate -> the workspace crates it may depend on (any dependency table).
LAYERS = {
    "modkit-core": set(),
    "source-assets": {"modkit-core"},
    "source-ui": {"source-assets"},
    "hl2-simulation": {"modkit-core", "source-assets"},
    "hl2-ui": {"modkit-core", "source-assets", "source-ui", "hl2-simulation"},
    "hl2-bevy": {"modkit-core", "source-assets", "source-ui", "hl2-simulation", "hl2-ui"},
}
# External crate -> the only workspace crates that may depend on it directly.
ONLY_IN = {
    "bevy": {"hl2-bevy"},
    "rfd": {"hl2-bevy"},
    "rapier3d": {"hl2-simulation"},
    "symphonia": {"hl2-simulation"},
    "hound": {"hl2-simulation"},
}
# Crates that get their bytes from source-assets or the host, never from the file system.
NO_FILE_IO = {"modkit-core", "hl2-simulation", "source-ui"}
FILE_IO = re.compile(r"\bstd::fs\b|\bfs::(read|write|create_dir|remove|rename|copy|File)|\bFile::(open|create)\b")
# The toolkit knows nothing of HL2: no HL2 crate paths or HL2RS-only tokens in its code.
TOOLKIT = "source-ui"
HL2_NAMES = re.compile(r"\bhl2_(ui|simulation|bevy)\b|HL2RS_")

FN = re.compile(r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:(?:const|async|unsafe|extern\s+"\w*")\s+)*fn\s+(\w+)')
GLOB = re.compile(r"::\*")
STATIC = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?static\s+\w+\s*:\s*([^=]+)")
MUTABLE = re.compile(r"Mutex|RwLock|Atomic|Cell<")


def scan(text):
    """Each line's code without comments, string and char contents (for braces and patterns),
    and without comments only (for duplicates)."""
    code_lines, plain_lines = [], []
    in_block = 0
    in_string = None  # the closing quote: '"' (escapes apply), or a raw string's '"#..' (none)
    raw_closing = False
    for raw in text.replace("\r\n", "\n").splitlines():
        code, plain = [], []
        i = 0
        while i < len(raw):
            c = raw[i]
            if in_block:
                if raw.startswith("*/", i):
                    in_block -= 1
                    i += 2
                elif raw.startswith("/*", i):
                    in_block += 1
                    i += 2
                else:
                    i += 1
                continue
            if in_string:
                if not raw_closing:
                    if c == "\\":
                        plain.append(raw[i:i + 2])
                        i += 2
                        continue
                    if c == '"':
                        in_string = None
                        code.append('"')
                    plain.append(c)
                    i += 1
                    continue
                if raw.startswith(in_string, i):
                    plain.append(in_string)
                    code.append(in_string)
                    i += len(in_string)
                    in_string = None
                    continue
                plain.append(c)
                i += 1
                continue
            if raw.startswith("//", i):
                break
            if raw.startswith("/*", i):
                in_block = 1
                i += 2
                continue
            raw_string = re.match(r'b?r(#*)"', raw[i:])
            if raw_string and (i == 0 or not (raw[i - 1].isalnum() or raw[i - 1] == "_")):
                start = raw_string.group(0)
                in_string, raw_closing = '"' + raw_string.group(1), True
                code.append(start)
                plain.append(start)
                i += len(start)
                continue
            if c == '"':
                in_string, raw_closing = '"', False
                code.append('"')
                plain.append('"')
                i += 1
                continue
            char = re.match(r"'(\\[^']{1,10}|[^'\\])'", raw[i:])
            if char:
                code.append("' '")
                plain.append(char.group(0))
                i += len(char.group(0))
                continue
            code.append(c)
            plain.append(c)
            i += 1
        code_lines.append("".join(code))
        plain_lines.append("".join(plain))
    return code_lines, plain_lines


def test_mask(path, code):
    """Which lines are test code: `#[cfg(test)]` items (a declaration, or a block to its closing
    brace), or all of a `tests.rs` file."""
    if path.name == "tests.rs" or "/tests/" in path.as_posix():
        return [True] * len(code)
    mask = [False] * len(code)
    i = 0
    while i < len(code):
        stripped = code[i].strip()
        if stripped.startswith("#[cfg(test)]"):
            j, depth, opened = i, 0, False
            while j < len(code):
                line = code[j] if j > i else stripped[len("#[cfg(test)]"):]
                depth += line.count("{") - line.count("}")
                opened = opened or "{" in line
                mask[j] = True
                if (opened and depth <= 0) or (not opened and line.rstrip().endswith(";")):
                    break
                j += 1
            i = j + 1
        else:
            i += 1
    return mask


def functions(code, mask):
    """(name, first line, length) of each outermost production function with a body."""
    found, depth, current, pending = [], 0, None, None
    for i, line in enumerate(code):
        if mask[i]:
            continue
        if current is None:
            match = FN.match(line)
            if match:
                pending = (match.group(1), i, depth)
            if pending:
                if "{" in line:
                    current, pending = pending, None
                elif line.rstrip().endswith(";"):
                    pending = None
        depth += line.count("{") - line.count("}")
        if current and depth <= current[2] and i > current[1]:
            found.append((current[0], current[1] + 1, i - current[1] + 1))
            current = None
    return found


def dependencies(manifest):
    """Dependency names of every table of a Cargo.toml ([dependencies], dev, build, target)."""
    names, table = set(), None
    for line in manifest.read_text(encoding="utf-8").splitlines():
        line = line.split("#")[0].strip()
        header = re.match(r"^\[(.+)\]$", line)
        if header:
            table = header.group(1)
            dotted = re.search(r"(?:^|\.)(?:dev-|build-)?dependencies\.([A-Za-z0-9_-]+)$", table)
            if dotted:
                names.add(dotted.group(1))
            continue
        if table and re.search(r"(^|\.)(dev-|build-)?dependencies$", table):
            name = re.match(r'^([A-Za-z0-9_-]+)\s*(\.|=)', line)
            if name:
                names.add(name.group(1))
    return names


def measure():
    state = {"functions": defaultdict(list), "files": {}, "test_files": {},
             "counts": defaultdict(lambda: defaultdict(int))}
    problems, windows = [], defaultdict(list)
    crates = sorted(p.name for p in (REPO / "crates").iterdir() if (p / "Cargo.toml").is_file())
    for crate in crates:
        deps = dependencies(REPO / "crates" / crate / "Cargo.toml")
        allowed = LAYERS.get(crate)
        if allowed is None:
            problems.append(f"crate {crate} is not in LAYERS: decide its layer with the owner and add it")
            allowed = set()
        for dep in sorted(deps & set(crates)):
            if dep not in allowed:
                problems.append(f"crate {crate} depends on {dep}, which its layer does not allow "
                                f"(allowed: {', '.join(sorted(allowed)) or 'none'}); move the shared "
                                f"code down a layer instead")
        for dep in sorted(deps):
            owners = ONLY_IN.get(dep)
            if owners and crate not in owners:
                problems.append(f"crate {crate} depends on {dep}, which only {', '.join(sorted(owners))} "
                                f"may use directly")
        for path in sorted((REPO / "crates" / crate / "src").rglob("*.rs")):
            rel = path.relative_to(REPO).as_posix()
            code, plain = scan(path.read_text(encoding="utf-8", errors="replace"))
            mask = test_mask(path, code)
            production = sum(1 for m in mask if not m)
            tests = len(mask) - production
            if production > FILE_LINES:
                state["files"][rel] = production
            if tests > TEST_LINES:
                state["test_files"][rel] = tests
            for name, line, length in functions(code, mask):
                if length > FUNCTION_LINES:
                    state["functions"][f"{rel}::{name}"].append(length)
            counts = state["counts"][crate]
            significant = []
            for i, line in enumerate(code):
                if mask[i]:
                    continue
                if crate in NO_FILE_IO and FILE_IO.search(line):
                    problems.append(f"{rel}:{i + 1}: {crate} must not use the file system "
                                    f"(read through source-assets' Vfs, or let the host do it)")
                if crate == TOOLKIT and HL2_NAMES.search(plain[i]):
                    problems.append(f"{rel}:{i + 1}: {TOOLKIT} names HL2 code; the app passes "
                                    f"what is its own in through a trait or a value")
                static = STATIC.match(line)
                if static and MUTABLE.search(static.group(1)):
                    counts["mutable statics"] += 1
                if "clippy::too_many_arguments" in line:
                    counts["too_many_arguments allowances"] += 1
                if re.search(r"#\[(allow|expect)\([^)]*\bdead_code\b", line):
                    counts["dead_code allowances"] += 1
                counts["glob imports"] += len(GLOB.findall(line))
                text = re.sub(r"\s+", " ", plain[i]).strip()
                if len(text) > 3 and not re.fullmatch(r"[{}()\[\];,.]*", text) \
                        and not text.startswith(("use ", "#[", "#![", "mod ", "pub mod ")):
                    significant.append((i + 1, text))
            for k in range(len(significant) - DUPLICATE_WINDOW + 1):
                block = "\n".join(t for _, t in significant[k:k + DUPLICATE_WINDOW])
                windows[block].append(f"{rel}:{significant[k][0]}")
    duplicates = {}
    for block, places in windows.items():
        if len(places) > 1:
            duplicates[hashlib.sha1(block.encode()).hexdigest()[:12]] = places
    agents = REPO / "AGENTS.md"
    if agents.exists() and agents.stat().st_size > AGENTS_BYTES:
        problems.append(f"AGENTS.md is {agents.stat().st_size} bytes (limit {AGENTS_BYTES}): it holds "
                        f"rules, one line each; move details to docs/ and point to them")
    state["functions"] = {k: sorted(v, reverse=True) for k, v in state["functions"].items()}
    state["counts"] = {c: dict(v) for c, v in state["counts"].items()}
    state["duplicates"] = sorted(duplicates)
    return state, problems, duplicates


def compare(state, baseline, duplicates):
    failures, shrunk = [], []
    fix = {
        "functions": f"split it into smaller functions (limit {FUNCTION_LINES} lines)",
        "files": f"move a concern into its own module (limit {FILE_LINES} production lines)",
        "test_files": f"move tests into a tests.rs next to the code or split them by concern "
                      f"(limit {TEST_LINES} lines)",
    }
    for section in ("functions", "files", "test_files"):
        old, new = baseline.get(section, {}), state[section]
        for key, value in sorted(new.items()):
            if key not in old:
                failures.append(f"{key}: {value} lines, over the limit: {fix[section]}")
            elif section == "functions":
                if any(v > (old[key] + [0] * len(value))[i] for i, v in enumerate(value)) or len(value) > len(old[key]):
                    failures.append(f"{key}: {value} lines, was {old[key]}: it may not grow; {fix[section]}")
                elif value != old[key]:
                    shrunk.append(f"{key}: {old[key]} -> {value}")
            elif value > old[key]:
                failures.append(f"{key}: {value} lines, was {old[key]}: it may not grow; {fix[section]}")
            elif value < old[key]:
                shrunk.append(f"{key}: {old[key]} -> {value}")
        for key in sorted(set(old) - set(new)):
            shrunk.append(f"{key}: now within the limit")
    for crate, counts in sorted(state["counts"].items()):
        for name, value in sorted(counts.items()):
            was = baseline.get("counts", {}).get(crate, {}).get(name, 0)
            if value > was:
                failures.append(f"{crate}: {value} {name}, was {was}: remove one instead of adding one")
            elif value < was:
                shrunk.append(f"{crate} {name}: {was} -> {value}")
    for crate, counts in sorted(baseline.get("counts", {}).items()):
        for name, was in counts.items():
            if was and not state["counts"].get(crate, {}).get(name):
                shrunk.append(f"{crate} {name}: {was} -> 0")
    old_dups = set(baseline.get("duplicates", []))
    for key in sorted(set(state["duplicates"]) - old_dups):
        places = duplicates[key]
        failures.append(f"duplicated code ({DUPLICATE_WINDOW}+ lines) at {', '.join(places[:4])}: "
                        f"reuse one copy (a shared function, or a table) instead of repeating it")
    gone = old_dups - set(state["duplicates"])
    if gone:
        shrunk.append(f"{len(gone)} duplicated blocks gone")
    return failures, shrunk


def lowered(baseline, state):
    """The baseline with every entry lowered to what it is now (never raised, nothing added)."""
    out = {}
    for section in ("functions", "files", "test_files"):
        old, new = baseline.get(section, {}), state[section]
        if section == "functions":
            out[section] = {k: [min(a, b) for a, b in zip(old[k], new[k])] for k in old if k in new}
        else:
            out[section] = {k: min(old[k], new[k]) for k in old if k in new}
    out["counts"] = {}
    for crate, counts in baseline.get("counts", {}).items():
        now = state["counts"].get(crate, {})
        kept = {name: min(was, now.get(name, 0)) for name, was in counts.items() if now.get(name, 0)}
        if kept:
            out["counts"][crate] = kept
    out["duplicates"] = sorted(set(baseline.get("duplicates", [])) & set(state["duplicates"]))
    return out


def write(data):
    BASELINE.write_text(json.dumps(data, indent=1, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    state, problems, duplicates = measure()
    if os.environ.get("ACCEPT_GROWTH") == "1":
        write(state)
        print(f"architecture: recorded the current state in {BASELINE.name} (growth accepted)")
        return 1 if problems else 0
    baseline = json.loads(BASELINE.read_text(encoding="utf-8")) if BASELINE.exists() else {}
    failures, shrunk = compare(state, baseline, duplicates)
    if os.environ.get("UPDATE_BASELINE") == "1":
        write(lowered(baseline, state))
        for line in shrunk:
            print(f"  tightened {line}")
        shrunk = []
    failures = problems + failures
    for line in failures:
        print(f"FAIL {line}")
    if shrunk:
        print("Improved since the baseline; lock it in with "
              "UPDATE_BASELINE=1 python3 scripts/checks/architecture.py:")
        for line in shrunk:
            print(f"  {line}")
    if failures or shrunk:
        print("See AGENTS.md \"Architecture guardrails\".")
        return 1
    print(f"architecture: ok ({len(state['functions'])} long functions, {len(state['files'])} large "
          f"files and {len(state['duplicates'])} duplicated blocks known, none grown)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
