#!/usr/bin/env python3
"""
scripts/clean_build_garbage.py
Build garbage & stale artifact cleaner respecting CARGO_TARGET_DIR without hardcoded developer paths.

Enforces:
1. Purges incremental caches across target profiles and architecture triples.
2. Purges *.dSYM debug symbol bundles in target deps/.
3. Purges stale test executable binaries in deps/ and reconciles .fingerprint.
4. Auto-recycles temporary test scratch directories without deleting the active CARGO_TARGET_DIR.
5. Prunes only known redundant workspace Cargo target directories (never walks .git/.agents/source).
"""

import argparse
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile


def _dir_size(path: str) -> int:
    total = 0
    for dp, _, fns in os.walk(path):
        for fn in fns:
            try:
                total += os.path.getsize(os.path.join(dp, fn))
            except OSError:
                pass
    return total


def clean_target_dir(target_dir: str) -> int:
    if not os.path.isdir(target_dir):
        return 0
    freed_bytes = 0

    # 1. Purge all incremental cache directories recursively
    for root, dirs, _ in os.walk(target_dir):
        for d in list(dirs):
            if d == "incremental":
                inc_path = os.path.join(root, d)
                freed_bytes += _dir_size(inc_path)
                shutil.rmtree(inc_path, ignore_errors=True)
                dirs.remove(d)

    # 2. Discover profile directories (debug, release, <triple>/debug, <triple>/release)
    profile_dirs = []
    for prof in ("debug", "release"):
        p = os.path.join(target_dir, prof)
        if os.path.isdir(p):
            profile_dirs.append(p)

    for entry in os.listdir(target_dir):
        sub = os.path.join(target_dir, entry)
        if os.path.isdir(sub) and entry not in ("debug", "release", "tmp", ".rustc_info.json"):
            for prof in ("debug", "release"):
                p = os.path.join(sub, prof)
                if os.path.isdir(p):
                    profile_dirs.append(p)

    for prof_dir in profile_dirs:
        deps_dir = os.path.join(prof_dir, "deps")
        fp_dir = os.path.join(prof_dir, ".fingerprint")
        if not os.path.isdir(deps_dir):
            continue

        removed_hashes = set()

        for entry in os.listdir(deps_dir):
            full_p = os.path.join(deps_dir, entry)
            if entry.endswith(".dSYM") and os.path.isdir(full_p):
                freed_bytes += _dir_size(full_p)
                shutil.rmtree(full_p, ignore_errors=True)
            elif os.path.isfile(full_p):
                _, ext = os.path.splitext(entry)
                if ext == "" and os.access(full_p, os.X_OK):
                    m = re.search(r"-([0-9a-f]{16})$", entry)
                    if m:
                        removed_hashes.add(m.group(1))
                    try:
                        freed_bytes += os.path.getsize(full_p)
                        os.remove(full_p)
                    except OSError:
                        pass

        # Reconcile .fingerprint so Cargo never sees a fingerprint whose artifact was removed
        if os.path.isdir(fp_dir):
            existing_dep_hashes = set()
            for entry in os.listdir(deps_dir):
                m = re.search(r"-([0-9a-f]{16})\.(rlib|rmeta|dylib|so|a)$", entry)
                if m:
                    existing_dep_hashes.add(m.group(1))

            for fp_entry in os.listdir(fp_dir):
                m = re.search(r"-([0-9a-f]{16})$", fp_entry)
                if not m:
                    continue
                h = m.group(1)
                fp_path = os.path.join(fp_dir, fp_entry)
                stem = fp_entry.rsplit("-", 1)[0].replace("-", "_")
                has_unhashed_artifact = (
                    os.path.exists(os.path.join(deps_dir, f"lib{stem}.rlib"))
                    or os.path.exists(os.path.join(deps_dir, f"lib{stem}.dylib"))
                    or os.path.exists(os.path.join(deps_dir, f"lib{stem}.a"))
                )
                if not has_unhashed_artifact and (
                    h in removed_hashes
                    or (
                        h not in existing_dep_hashes
                        and not os.path.isdir(os.path.join(prof_dir, "build", fp_entry))
                    )
                ):
                    shutil.rmtree(fp_path, ignore_errors=True)

    return freed_bytes


def clean_test_artifacts(root_dir: str, canonical_target: str) -> int:
    """Purges temporary test scratch directories and known redundant Cargo target directories."""
    freed_bytes = 0
    search_dirs = [
        os.path.join(canonical_target, "tmp"),
        os.path.join(root_dir, "build", "tmp"),
        os.path.join(tempfile.gettempdir(), "fabushi_tests"),
    ]
    for s_dir in search_dirs:
        if os.path.isdir(s_dir):
            freed_bytes += _dir_size(s_dir)
            shutil.rmtree(s_dir, ignore_errors=True)

    # Clean stray .cargo-target* directories at repo root
    for item in os.listdir(root_dir):
        if item.startswith(".cargo-target"):
            full_p = os.path.join(root_dir, item)
            if os.path.isdir(full_p) and os.path.abspath(full_p) != canonical_target:
                freed_bytes += _dir_size(full_p)
                shutil.rmtree(full_p, ignore_errors=True)

    # Prune only known redundant Cargo target locations that differ from active CARGO_TARGET_DIR
    known_target_candidates = [
        os.path.join(root_dir, "target"),
        os.path.join(root_dir, "source", "host", "target"),
        os.path.join(root_dir, "source", "desktop_core", "target"),
        os.path.join(root_dir, "source", "packages", "mahayana-rs", "target"),
    ]
    for cand in known_target_candidates:
        abs_cand = os.path.abspath(cand)
        if abs_cand != canonical_target and os.path.isdir(abs_cand):
            freed_bytes += _dir_size(abs_cand)
            shutil.rmtree(abs_cand, ignore_errors=True)

    return freed_bytes


def main() -> int:
    parser = argparse.ArgumentParser(description="Clean Cargo build garbage safely.")
    parser.add_argument("--repo-root", type=Path, help="Optional explicit repository root.")
    args = parser.parse_args()

    repo_root = (
        args.repo_root.resolve()
        if args.repo_root is not None
        else Path(__file__).resolve().parents[1]
    )
    if not (repo_root / "source").is_dir():
        print(
            f"[clean_build_garbage] Error: invalid repository root (missing source/): {repo_root}",
            file=sys.stderr,
        )
        return 2

    canonical_target = os.path.abspath(
        os.environ.get("CARGO_TARGET_DIR") or str(repo_root / "target")
    )
    total_freed = clean_target_dir(canonical_target)
    total_freed += clean_test_artifacts(str(repo_root), canonical_target)

    print(
        f"[clean_build_garbage] Freed {total_freed / (1024**2):.2f} MB in {repo_root} "
        f"(active target: {canonical_target})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
