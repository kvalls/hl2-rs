#!/usr/bin/env python3
"""On-demand LLVM line-coverage check for Bevy-free workspace crates (stdlib only).

Runs `cargo test --locked` with `-C instrument-coverage` on PURE_CRATES into `target/coverage/`,
merges `.profraw` profiles with `llvm-profdata`, summarizes production-file coverage via `llvm-cov`,
and fails only if any crate drops more than `tolerance` percentage points below `coverage-baseline.json`.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BASELINE = Path(__file__).with_name("coverage-baseline.json")
PURE_CRATES = (
    "modkit-core",
    "source-assets",
    "source-ui",
    "hl2-simulation",
    "hl2-ui",
    "hl2-advanced-options",
    "hl2-debug",
)
DEFAULT_TOLERANCE = 2.0
TEST_PATH_RE = re.compile(r"(?:^|/)(?:tests\.rs|[^/]+_tests\.rs|tests/|plugins/[^/]+/src/bevy/)")
IGNORE_FILENAME_REGEX = (
    r"([/\\]\.cargo[/\\]|[/\\]rustc[/\\]|tests\.rs$|_tests\.rs$|[/\\]tests[/\\]"
    r"|plugins[/\\][^/\\]+[/\\]src[/\\]bevy[/\\])"
)


def find_llvm_tools(sysroot: Path | None = None) -> tuple[Path, Path] | None:
    """Locate `llvm-profdata` and `llvm-cov` inside rustup's `llvm-tools` component."""
    if sysroot is None:
        raw = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
        sysroot = Path(raw)
    suffix = ".exe" if os.name == "nt" else ""
    rustlib = sysroot / "lib" / "rustlib"
    if not rustlib.is_dir():
        return None
    profdata = next(rustlib.glob(f"*/bin/llvm-profdata{suffix}"), None)
    llvm_cov = next(rustlib.glob(f"*/bin/llvm-cov{suffix}"), None)
    if not (profdata and llvm_cov):
        return None
    return profdata, llvm_cov


def _crate_for_file(file_path: str, repo: Path, crates: tuple[str, ...]) -> str | None:
    norm = file_path.replace("\\", "/")
    repo_norm = repo.as_posix().rstrip("/") + "/"
    if norm.startswith(repo_norm):
        rel = norm[len(repo_norm) :]
    else:
        idx = -1
        for marker in ("crates/", "plugins/"):
            pos = norm.find(marker)
            if pos != -1 and (idx == -1 or pos < idx):
                idx = pos
        if idx == -1:
            return None
        rel = norm[idx:]
    parts = rel.split("/")
    if len(parts) < 4 or parts[0] not in ("crates", "plugins") or parts[1] not in crates or parts[2] != "src":
        return None
    if TEST_PATH_RE.search(rel):
        return None
    return parts[1]


def _relative_label(file_path: str, repo: Path) -> str:
    norm = file_path.replace("\\", "/")
    repo_norm = repo.as_posix().rstrip("/") + "/"
    if norm.startswith(repo_norm):
        return norm[len(repo_norm) :]
    return norm


def summarize_export(
    export_json: dict,
    repo: Path,
    crates: tuple[str, ...] = PURE_CRATES,
) -> tuple[dict[str, dict], list[dict]]:
    """Aggregate line coverage per crate and collect per-file uncovered line counts."""
    totals = {c: {"covered": 0, "count": 0} for c in crates}
    file_rows = []
    for entry in export_json.get("data", []):
        for file_entry in entry.get("files", []):
            filename = file_entry.get("filename", "")
            crate = _crate_for_file(filename, repo, crates)
            if crate is None:
                continue
            lines = file_entry.get("summary", {}).get("lines", {})
            covered = int(lines.get("covered", 0))
            count = int(lines.get("count", 0))
            totals[crate]["covered"] += covered
            totals[crate]["count"] += count
            if count > covered:
                file_rows.append({
                    "crate": crate,
                    "file": _relative_label(filename, repo),
                    "covered": covered,
                    "count": count,
                    "uncovered": count - covered,
                    "percent": round(100.0 * covered / count, 2) if count else 100.0,
                })
    summary = {}
    for crate in crates:
        cov = totals[crate]["covered"]
        cnt = totals[crate]["count"]
        if cnt:
            summary[crate] = {
                "covered": cov,
                "count": cnt,
                "percent": round(100.0 * cov / cnt, 2),
            }
    file_rows.sort(key=lambda row: (-row["uncovered"], row["file"]))
    return summary, file_rows


def check_baseline(
    summary: dict[str, dict],
    baseline: dict[str, dict],
    tolerance: float = DEFAULT_TOLERANCE,
) -> list[str]:
    """Fail if any baselined crate drops more than `tolerance` percentage points."""
    failures = []
    for crate, base_stats in sorted(baseline.get("crates", {}).items()):
        base_pct = float(base_stats.get("percent", 0.0))
        if crate not in summary:
            failures.append(f"{crate}: no coverage data collected (was {base_pct:.2f}%)")
            continue
        now_pct = float(summary[crate]["percent"])
        if now_pct + tolerance < base_pct:
            failures.append(
                f"{crate}: line coverage {now_pct:.2f}% dropped more than {tolerance:.2f}% "
                f"below baseline {base_pct:.2f}%"
            )
    return failures


def raise_baseline(baseline: dict, summary: dict[str, dict]) -> dict:
    """Return baseline with each crate's percent ratcheted upward to current when higher."""
    old_crates = baseline.get("crates", {})
    out = {}
    for crate in sorted(set(old_crates) | set(summary)):
        old = old_crates.get(crate)
        now = summary.get(crate)
        if now is None:
            out[crate] = old
        elif old is None or float(now["percent"]) >= float(old.get("percent", 0.0)):
            out[crate] = now
        else:
            out[crate] = old
    return {"crates": out}


def collect_coverage(profdata: Path, llvm_cov: Path) -> dict:
    """Run instrumented unit tests on PURE_CRATES and return `llvm-cov export` JSON."""
    pkg_args = [arg for crate in PURE_CRATES for arg in ("-p", crate)]
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(REPO / "target" / "coverage")
    existing_flags = env.get("RUSTFLAGS", "").strip()
    env["RUSTFLAGS"] = f"{existing_flags} -C instrument-coverage".strip()
    with tempfile.TemporaryDirectory(prefix="hl2rs-cov-") as tmp:
        tmp_dir = Path(tmp)
        env["LLVM_PROFILE_FILE"] = str(tmp_dir / "hl2rs-%p-%m.profraw")
        build = subprocess.run(
            ["cargo", "test", "--locked", "--no-run", "--message-format=json", *pkg_args],
            cwd=REPO,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        if build.returncode != 0:
            sys.stderr.write(build.stderr)
            raise RuntimeError(f"instrumented cargo build failed ({build.returncode})")
        executables = []
        seen = set()
        for line in build.stdout.splitlines():
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            if msg.get("reason") == "compiler-artifact" and msg.get("profile", {}).get("test"):
                exe = msg.get("executable")
                if exe and exe not in seen:
                    seen.add(exe)
                    executables.append(exe)
        if not executables:
            raise RuntimeError("no test executables found in cargo build output")
        run = subprocess.run(
            ["cargo", "test", "--locked", "--no-fail-fast", *pkg_args],
            cwd=REPO,
            env=env,
            check=False,
        )
        if run.returncode != 0:
            raise RuntimeError(f"instrumented tests failed ({run.returncode})")
        profraws = sorted(tmp_dir.glob("*.profraw"))
        if not profraws:
            raise RuntimeError("no .profraw files generated")
        merged = tmp_dir / "coverage.profdata"
        merge = subprocess.run(
            [str(profdata), "merge", "-sparse", *[str(p) for p in profraws], "-o", str(merged)],
            capture_output=True,
            text=True,
            check=False,
        )
        if merge.returncode != 0:
            sys.stderr.write(merge.stderr)
            raise RuntimeError("llvm-profdata merge failed")
        cmd = [
            str(llvm_cov),
            "export",
            f"--instr-profile={merged}",
            "--summary-only",
            f"--ignore-filename-regex={IGNORE_FILENAME_REGEX}",
            executables[0],
            *[arg for exe in executables[1:] for arg in ("--object", exe)],
        ]
        exported = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if exported.returncode != 0:
            sys.stderr.write(exported.stderr)
            raise RuntimeError("llvm-cov export failed")
        return json.loads(exported.stdout)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Check line coverage of Bevy-free workspace crates.")
    parser.add_argument("--tolerance", type=float, default=DEFAULT_TOLERANCE)
    parser.add_argument("--top", type=int, default=15)
    parser.add_argument("--allow-missing", action="store_true")
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args(argv)

    tools = find_llvm_tools()
    if tools is None:
        print("coverage: rustup component llvm-tools is not installed (run: rustup component add llvm-tools)")
        return 0 if args.allow_missing else 2

    export_json = collect_coverage(*tools)
    summary, file_rows = summarize_export(export_json, REPO, PURE_CRATES)
    print("Pure-crate production line coverage:")
    for crate in PURE_CRATES:
        stats = summary.get(crate)
        if stats:
            print(f"  {crate:<22} {stats['percent']:6.2f}%  ({stats['covered']}/{stats['count']})")
    if args.top and file_rows:
        print(f"\nTop {min(args.top, len(file_rows))} files by uncovered lines:")
        for row in file_rows[: args.top]:
            print(f"  {row['uncovered']:5d} uncovered  {row['percent']:5.1f}%  {row['file']}")

    if args.record:
        BASELINE.write_text(json.dumps({"crates": summary}, indent=1, sort_keys=True) + "\n", encoding="utf-8")
        print(f"\ncoverage: recorded baseline in {BASELINE.name}")
        return 0

    baseline = json.loads(BASELINE.read_text(encoding="utf-8")) if BASELINE.exists() else {}
    if os.environ.get("UPDATE_BASELINE") == "1":
        BASELINE.write_text(
            json.dumps(raise_baseline(baseline, summary), indent=1, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(f"\ncoverage: updated {BASELINE.name}")
        return 0

    failures = check_baseline(summary, baseline, args.tolerance)
    for line in failures:
        print(f"FAIL {line}")
    if failures:
        return 1
    print(f"\ncoverage: ok (within {args.tolerance:.2f}% tolerance)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
