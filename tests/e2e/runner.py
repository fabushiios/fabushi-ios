#!/usr/bin/env python3
"""CI-native opaque-box E2E test orchestrator for Fabushi iOS v1.6.8.

Standard-library only. Designed to execute exclusively inside GitHub Actions CI
(`ubuntu-latest` / `macos-15`). Zero hardcoded workstation paths, zero mocks in
production mode.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

TIER_TEST_TARGETS: dict[str, list[str]] = {
    "tier1": ["tier1_auth_shell_messaging", "tier1_agents_platform_hig"],
    "tier2": ["tier2_auth_shell_messaging", "tier2_agents_platform_hig"],
    "tier3": ["tier3_pairwise_combos"],
    "tier4": ["tier4_real_world_scenarios"],
}


def shared_lib_ext() -> str:
    sys_name = platform.system().lower()
    if sys_name == "darwin":
        return "dylib"
    if sys_name == "windows":
        return "dll"
    return "so"


def resolve_path_abs(raw_path: str, repo_root: Path) -> Path:
    """Resolve relative/symlink paths to absolute canonical paths before spawning Cargo."""
    p = Path(raw_path)
    if not p.is_absolute():
        cwd_candidate = (Path.cwd() / p).resolve()
        if cwd_candidate.exists():
            return cwd_candidate
        p = repo_root / p
    return p.resolve()


def resolve_e2e_target_dir(repo_root: Path) -> Path:
    env_dir = os.environ.get("CARGO_TARGET_DIR", "").strip()
    if env_dir:
        p = Path(env_dir)
        return p.resolve() if p.is_absolute() else (repo_root / p).resolve()
    # Never default to tests/e2e/rust_ffi/target (which self-poisons spec_coverage_check.py --strict).
    return (repo_root / "target/e2e").resolve()


def build_strict_rustflags() -> str:
    existing = os.environ.get("RUSTFLAGS", "").strip()
    if not existing:
        return "-D warnings"
    tokens = existing.split()
    for idx in range(len(tokens) - 1):
        if tokens[idx] == "-D" and tokens[idx + 1] == "warnings":
            return existing
    return f"{existing} -D warnings"


def is_mutant_stub_path(lib_path: Path) -> bool:
    candidates = [lib_path.as_posix().lower()]
    try:
        candidates.append(lib_path.resolve().as_posix().lower())
    except OSError:
        pass
    return any("mutant_stub" in c or "mutant-stub" in c for c in candidates)


def run_step(
    name: str,
    cmd: list[str],
    cwd: Path,
    extra_env: dict[str, str] | None = None,
    unset_env: list[str] | None = None,
) -> dict[str, Any]:
    env = os.environ.copy()
    if unset_env:
        for key in unset_env:
            env.pop(key, None)
    if extra_env:
        env.update(extra_env)

    print(f"==> [{name}] {' '.join(cmd)}", flush=True)
    t0 = time.monotonic()
    proc = subprocess.run(cmd, cwd=str(cwd), env=env, check=False)
    duration_s = round(time.monotonic() - t0, 3)

    record = {
        "name": name,
        "command": cmd,
        "exit_code": proc.returncode,
        "duration_s": duration_s,
        "ok": proc.returncode == 0,
    }
    print(f"<== [{name}] exit={proc.returncode} duration={duration_s}s", flush=True)
    return record


def run_self_validate(
    repo_root: Path,
    mutant_lib_arg: str,
    baseline_lib_arg: str,
    skip_baseline_proof: bool,
    report_path: Path | None,
) -> tuple[int, list[dict[str, Any]], str | None, Path]:
    steps: list[dict[str, Any]] = []
    e2e_target_dir = resolve_e2e_target_dir(repo_root)
    manifest = str((repo_root / "tests/e2e/rust_ffi/Cargo.toml").resolve())
    mutant_manifest = str((repo_root / "tests/e2e/rust_ffi/mutant_stub/Cargo.toml").resolve())

    if skip_baseline_proof and baseline_lib_arg:
        msg = "FAIL-CLOSED: --skip-baseline-proof cannot be combined with --baseline-lib"
        print(msg, file=sys.stderr)
        return 2, steps, msg, e2e_target_dir

    cov_report = (
        str((report_path.parent / "runner_coverage_check.json").resolve())
        if report_path is not None
        else ""
    )
    cov_cmd = [
        sys.executable,
        str((repo_root / "tests/e2e/spec_coverage_check.py").resolve()),
        "--strict",
        "--matrix",
        str((repo_root / "tests/e2e/feature_matrix.json").resolve()),
    ]
    if cov_report:
        cov_cmd.extend(["--report", cov_report])

    s1 = run_step("spec_coverage_check", cov_cmd, repo_root)
    steps.append(s1)
    if not s1["ok"]:
        return s1["exit_code"], steps, "spec_coverage_check failed", e2e_target_dir

    strict_rust_env = {
        "CARGO_TARGET_DIR": str(e2e_target_dir),
        "RUSTFLAGS": build_strict_rustflags(),
    }

    s2 = run_step(
        "cargo_test_no_run_all_394_tests",
        ["cargo", "test", "--manifest-path", manifest, "--no-run"],
        repo_root,
        extra_env=strict_rust_env,
    )
    steps.append(s2)
    if not s2["ok"]:
        return s2["exit_code"], steps, "cargo test --no-run failed", e2e_target_dir

    s3 = run_step(
        "cargo_build_mutant_stub",
        ["cargo", "build", "--manifest-path", mutant_manifest],
        repo_root,
        extra_env=strict_rust_env,
    )
    steps.append(s3)
    if not s3["ok"]:
        return s3["exit_code"], steps, "cargo build mutant_stub failed", e2e_target_dir

    ext = shared_lib_ext()
    if mutant_lib_arg:
        mutant_lib = resolve_path_abs(mutant_lib_arg, repo_root)
    else:
        mutant_lib = (e2e_target_dir / "debug" / f"libfabushi_e2e_mutant_stub.{ext}").resolve()
    if not mutant_lib.exists():
        msg = f"ERROR: mutant_stub cdylib not found at {mutant_lib}"
        print(msg, file=sys.stderr)
        return 2, steps, msg, e2e_target_dir

    test_env: dict[str, str] = {
        "CARGO_TARGET_DIR": str(e2e_target_dir),
        "RUSTFLAGS": build_strict_rustflags(),
        "FABUSHI_MUTANT_STUB_LIB": str(mutant_lib),
    }
    test_cmd = [
        "cargo",
        "test",
        "--manifest-path",
        manifest,
        "--lib",
        "--test",
        "self_validate_antistub",
        "--",
        "--nocapture",
    ]

    if skip_baseline_proof:
        unset_env = ["FABUSHI_BASELINE_HOST_LIB", "FABUSHI_HOST_LIB"]
        test_env["FABUSHI_ALLOW_SKIP_BASELINE_PROOF"] = "1"
        test_env["FABUSHI_REQUIRE_BASELINE_PROOF"] = "0"
        test_cmd.extend(["--skip", "self_validate_baseline_8128951_host_negative_proof"])
    else:
        unset_env = ["FABUSHI_ALLOW_SKIP_BASELINE_PROOF", "FABUSHI_HOST_LIB"]
        baseline_lib_raw = baseline_lib_arg or os.environ.get("FABUSHI_BASELINE_HOST_LIB", "").strip()
        if not baseline_lib_raw:
            msg = (
                "FAIL-CLOSED: self-validate mode requires --baseline-lib or FABUSHI_BASELINE_HOST_LIB "
                "(or explicit --skip-baseline-proof)"
            )
            print(msg, file=sys.stderr)
            return 2, steps, msg, e2e_target_dir
        baseline_path = resolve_path_abs(baseline_lib_raw, repo_root)
        if not baseline_path.exists():
            msg = f"ERROR: baseline host library not found at {baseline_path}"
            print(msg, file=sys.stderr)
            return 2, steps, msg, e2e_target_dir
        test_env["FABUSHI_BASELINE_HOST_LIB"] = str(baseline_path)
        test_env["FABUSHI_REQUIRE_BASELINE_PROOF"] = "1"

    s4 = run_step(
        "cargo_test_self_validate_antistub",
        test_cmd,
        repo_root,
        extra_env=test_env,
        unset_env=unset_env,
    )
    steps.append(s4)
    err = None if s4["ok"] else "self_validate_antistub failed"
    return (0 if s4["ok"] else s4["exit_code"]), steps, err, e2e_target_dir


def run_production(
    repo_root: Path,
    tier: str,
    host_lib_arg: str,
) -> tuple[int, list[dict[str, Any]], str | None, Path]:
    steps: list[dict[str, Any]] = []
    e2e_target_dir = resolve_e2e_target_dir(repo_root)
    host_lib_str = host_lib_arg or os.environ.get("FABUSHI_HOST_LIB", "").strip()
    if not host_lib_str:
        msg = "FAIL-CLOSED: production mode requires --host-lib or FABUSHI_HOST_LIB"
        print(msg, file=sys.stderr)
        return 2, steps, msg, e2e_target_dir

    raw_host_path = Path(host_lib_str)
    host_lib = resolve_path_abs(host_lib_str, repo_root)
    if is_mutant_stub_path(raw_host_path) or is_mutant_stub_path(host_lib):
        msg = f"FAIL-CLOSED: production mode refuses mutant_stub library: {host_lib}"
        print(msg, file=sys.stderr)
        return 2, steps, msg, e2e_target_dir
    if not host_lib.exists():
        msg = f"FAIL-CLOSED: host library does not exist: {host_lib}"
        print(msg, file=sys.stderr)
        return 2, steps, msg, e2e_target_dir

    ordered_tiers = ["tier1", "tier2", "tier3", "tier4"] if tier == "all" else [tier]
    manifest = str((repo_root / "tests/e2e/rust_ffi/Cargo.toml").resolve())
    prod_env = {
        "FABUSHI_HOST_LIB": str(host_lib),
        "CARGO_TARGET_DIR": str(e2e_target_dir),
        "RUSTFLAGS": build_strict_rustflags(),
    }

    for current_tier in ordered_tiers:
        for target in TIER_TEST_TARGETS[current_tier]:
            step = run_step(
                f"{current_tier}:{target}",
                [
                    "cargo",
                    "test",
                    "--manifest-path",
                    manifest,
                    "--test",
                    target,
                    "--",
                    "--nocapture",
                    "--test-threads=4",
                ],
                repo_root,
                extra_env=prod_env,
            )
            steps.append(step)
            if not step["ok"]:
                return step["exit_code"], steps, f"{current_tier}:{target} failed", e2e_target_dir

    return 0, steps, None, e2e_target_dir


def main() -> int:
    parser = argparse.ArgumentParser(description="Fabushi iOS v1.6.8 Opaque-Box E2E Runner")
    parser.add_argument(
        "--mode",
        required=True,
        choices=["self-validate", "production"],
        help="Execution mode: self-validate (harness/anti-stub gate) or production (real host Tiers 1-4)",
    )
    parser.add_argument(
        "--tier",
        default="all",
        choices=["tier1", "tier2", "tier3", "tier4", "all"],
        help="Tier to run in production mode (sequential fail-fast when 'all')",
    )
    parser.add_argument("--host-lib", default="", help="Path to real libmahayana_app_host.{so,dylib}")
    parser.add_argument("--mutant-lib", default="", help="Optional override path to mutant_stub cdylib")
    parser.add_argument("--baseline-lib", default="", help="Path to baseline 8128951 libmahayana_app_host.so")
    parser.add_argument(
        "--skip-baseline-proof",
        action="store_true",
        help="Explicitly skip baseline 8128951 negative proof in post-M1 self-validate runs",
    )
    parser.add_argument("--report", default="", help="Optional JSON execution report output path")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[2]
    report_path = Path(args.report) if args.report else None
    t_start = time.monotonic()

    if args.mode == "self-validate":
        exit_code, steps, err_msg, target_dir = run_self_validate(
            repo_root,
            mutant_lib_arg=args.mutant_lib,
            baseline_lib_arg=args.baseline_lib,
            skip_baseline_proof=args.skip_baseline_proof,
            report_path=report_path,
        )
    else:
        exit_code, steps, err_msg, target_dir = run_production(
            repo_root,
            tier=args.tier,
            host_lib_arg=args.host_lib,
        )

    summary = {
        "mode": args.mode,
        "tier": args.tier,
        "ok": exit_code == 0,
        "exit_code": exit_code,
        "error": err_msg,
        "baseline_proof_skipped_explicitly": bool(args.mode == "self-validate" and args.skip_baseline_proof),
        "cargo_target_dir": str(target_dir),
        "total_duration_s": round(time.monotonic() - t_start, 3),
        "steps_executed": len(steps),
        "steps_passed": sum(1 for s in steps if s["ok"]),
        "steps": steps,
    }

    if report_path is not None:
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
