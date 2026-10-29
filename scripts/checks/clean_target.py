#!/usr/bin/env python3
"""
clean_target.py - Target directory maintenance and build remnant cleaner for HL2RS.

Modes:
  --stale (default):
      Prunes superseded object files (.rcgu.o), old .rlib archives, stale test
      runner executables, and orphaned incremental compilation sessions for
      workspace crates. Keeps third-party crate caches intact. Holds Cargo's
      debug/release compatibility locks throughout pruning; skips cleanup
      when the target is busy or compatible locking is unavailable.
  --workspace:
      Runs `cargo clean -p <workspace-packages>` to completely reset workspace
      crates while preserving all 3rd-party dependencies.
  --dry-run:
      Reports what files/directories would be deleted and total space reclaimed
      without deleting anything.
"""

import argparse
import collections
import os
import re
import shutil
import subprocess
import sys

from target_lock import cargo_locks


def find_repo_root():
    """Locate HL2RS repository root containing Cargo.toml."""
    current = os.path.abspath(os.path.dirname(__file__))
    while current and current != os.path.dirname(current):
        if os.path.isfile(os.path.join(current, "Cargo.toml")):
            return current
        current = os.path.dirname(current)
    return os.getcwd()


def get_workspace_crates(repo_root):
    """Read workspace members from Cargo.toml or use standard defaults."""
    cargo_toml = os.path.join(repo_root, "Cargo.toml")
    crates = []
    if os.path.isfile(cargo_toml):
        with open(cargo_toml, "r", encoding="utf-8") as f:
            content = f.read()
        m = re.search(r"members\s*=\s*\[(.*?)\]", content, re.DOTALL)
        if m:
            raw_members = m.group(1)
            for item in re.findall(r'"([^"]+)"', raw_members):
                cname = os.path.basename(item)
                crates.append(cname)
    if not crates:
        crates = ["hl2-bevy", "hl2-ui", "hl2-simulation", "modkit-core", "source-assets"]
    return crates


def format_bytes(num_bytes):
    """Format bytes into human-readable string."""
    if num_bytes >= 1024 * 1024 * 1024:
        return f"{num_bytes / (1024 * 1024 * 1024):.2f} GiB"
    elif num_bytes >= 1024 * 1024:
        return f"{num_bytes / (1024 * 1024):.1f} MiB"
    elif num_bytes >= 1024:
        return f"{num_bytes / 1024:.1f} KiB"
    return f"{num_bytes} B"


def clean_workspace(repo_root, crates, dry_run=False):
    """Run `cargo clean -p ...` for workspace crates."""
    cmd = ["cargo", "clean"]
    for c in crates:
        cmd.extend(["-p", c])
    if dry_run:
        cmd.append("--dry-run")

    print(f"==> Running: {' '.join(cmd)}")
    result = subprocess.run(cmd, cwd=repo_root)
    return result.returncode == 0


def incremental_candidates(target_dir, crate_underscores):
    """Find superseded workspace caches; dependencies are never candidates."""
    dirs_to_remove = []
    inc_dir = os.path.join(target_dir, "debug", "incremental")
    if os.path.isdir(inc_dir):
        inc_pattern = re.compile(r"^([a-zA-Z0-9_]+)-([0-9a-zA-Z]+)$")
        by_crate = collections.defaultdict(list)
        for entry in os.listdir(inc_dir):
            full_path = os.path.join(inc_dir, entry)
            if not os.path.isdir(full_path):
                continue
            m = inc_pattern.match(entry)
            if m and m.group(1) in crate_underscores:
                crate_name = m.group(1)
                # Compute newest mtime in dir
                mtime = os.path.getmtime(full_path)
                for root, _, files in os.walk(full_path):
                    for f in files:
                        try:
                            mt = os.path.getmtime(os.path.join(root, f))
                            if mt > mtime:
                                mtime = mt
                        except OSError:
                            pass
                by_crate[crate_name].append((full_path, mtime))

        for crate_name, dir_list in by_crate.items():
            dir_list.sort(key=lambda x: x[1], reverse=True)
            # Retain the newest workspace cache for each crate.
            for dpath, _ in dir_list[1:]:
                try:
                    sz = sum(
                        os.path.getsize(os.path.join(r, f))
                        for r, _, fs in os.walk(dpath)
                        for f in fs
                    )
                except OSError:
                    sz = 0
                dirs_to_remove.append((dpath, sz, crate_name))

    return dirs_to_remove


def clean_stale(repo_root, target_dir, crates, keep_active_versions=2, dry_run=False):
    """Prune only while holding Cargo-compatible locks; busy targets are skipped."""
    if not os.path.isdir(target_dir):
        print(f"Target directory not found: {target_dir}")
        return True
    with cargo_locks(target_dir) as acquired:
        if not acquired:
            return True
        return _prune_stale(repo_root, target_dir, crates, keep_active_versions, dry_run)


def _prune_stale(repo_root, target_dir, crates, keep_active_versions=2, dry_run=False):
    """Prune workspace artifacts; the caller holds the target locks."""
    if not os.path.isdir(target_dir):
        print(f"Target directory not found: {target_dir}")
        return True

    crate_underscores = [c.replace("-", "_") for c in crates]

    files_to_remove = []

    dirs_to_remove = incremental_candidates(target_dir, crate_underscores)

    # 2. Prune stale deps artifacts (debug & release)
    file_pattern = re.compile(r"^(?:lib)?([a-zA-Z0-9_]+)-([0-9a-fA-F]{16})(?:\..*)?$")

    for profile in ["debug", "release"]:
        deps_dir = os.path.join(target_dir, profile, "deps")
        if not os.path.isdir(deps_dir):
            continue

        by_crate_hash = collections.defaultdict(lambda: collections.defaultdict(list))
        for fname in os.listdir(deps_dir):
            fpath = os.path.join(deps_dir, fname)
            m = file_pattern.match(fname)
            if m:
                crate_name = m.group(1)
                if crate_name in crate_underscores:
                    h = m.group(2)
                    try:
                        sz = os.path.getsize(fpath)
                        mt = os.path.getmtime(fpath)
                        by_crate_hash[crate_name][h].append((fpath, sz, mt))
                    except OSError:
                        pass

        for crate_name, hashes in by_crate_hash.items():
            hash_stats = []
            for h, flist in hashes.items():
                max_mt = max(x[2] for x in flist)
                tot_sz = sum(x[1] for x in flist)
                hash_stats.append((h, max_mt, tot_sz, flist))
            hash_stats.sort(key=lambda x: x[1], reverse=True)

            keep_count = 1 if profile == "release" else keep_active_versions
            for h, max_mt, tot_sz, flist in hash_stats[keep_count:]:
                for fpath, sz, mt in flist:
                    files_to_remove.append((fpath, sz, crate_name, profile))

    total_inc_bytes = sum(x[1] for x in dirs_to_remove)
    total_deps_bytes = sum(x[1] for x in files_to_remove)
    total_bytes = total_inc_bytes + total_deps_bytes

    action_label = "Would clean" if dry_run else "Cleaning"
    print(f"==> {action_label} stale build artifacts for workspace crates:")
    print(f"    - Incremental session directories: {len(dirs_to_remove)} ({format_bytes(total_inc_bytes)})")
    print(f"    - Stale deps/ object files & binaries: {len(files_to_remove)} ({format_bytes(total_deps_bytes)})")
    print(f"    - Total space {'reclaimable' if dry_run else 'reclaimed'}: {format_bytes(total_bytes)}")

    if dry_run:
        print("    [dry-run: no files were deleted]")
        return True

    # Delete directories
    deleted_dirs = 0
    for dpath, _, _ in dirs_to_remove:
        try:
            shutil.rmtree(dpath, ignore_errors=True)
            deleted_dirs += 1
        except OSError as e:
            print(f"Warning: Failed to remove directory {dpath}: {e}", file=sys.stderr)

    # Delete files
    deleted_files = 0
    for fpath, _, _, _ in files_to_remove:
        try:
            os.remove(fpath)
            deleted_files += 1
        except OSError:
            pass

    print(f"==> Successfully removed {deleted_dirs} incremental directories and {deleted_files} stale files.")
    return True


def main():
    parser = argparse.ArgumentParser(
        description="Workspace cache cleanup. Stale pruning holds Cargo-compatible locks and skips busy or unsupported targets."
    )
    group = parser.add_mutually_exclusive_group()
    group.add_argument(
        "--stale",
        action="store_true",
        default=True,
        help="Prune superseded hashes, old test binaries, and dead incremental sessions (default).",
    )
    group.add_argument(
        "--workspace",
        action="store_true",
        help="Run cargo clean for workspace crates only (keeps Bevy/wgpu/Rapier cached).",
    )
    parser.add_argument(
        "-n",
        "--dry-run",
        action="store_true",
        help="Display what would be deleted without deleting anything.",
    )
    parser.add_argument(
        "--target-dir",
        default=None,
        help="Path to cargo target directory (defaults to HL2RS/target).",
    )

    args = parser.parse_args()

    repo_root = find_repo_root()
    crates = get_workspace_crates(repo_root)
    target_dir = args.target_dir or os.environ.get("CARGO_TARGET_DIR") or os.path.join(repo_root, "target")
    if args.target_dir is None and os.environ.get("CARGO_BUILD_TARGET"):
        target_dir = os.path.join(target_dir, os.environ["CARGO_BUILD_TARGET"])

    if args.workspace:
        success = clean_workspace(repo_root, crates, dry_run=args.dry_run)
    else:
        success = clean_stale(repo_root, target_dir, crates, dry_run=args.dry_run)

    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()
