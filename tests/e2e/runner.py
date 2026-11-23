#!/usr/bin/env python3
"""CI-native opaque-box E2E test orchestrator for Fabushi iOS v1.6.7.

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


def resolve_e2e_target_dir(repo_root: Path) -> Path:
    env_dir = os.environ.get("CARGO_TARGET_DIR", "").strip()
    if env_dir:
        return Path(env_dir)
    return repo_root / "tests/e2e/rust_ffi/target"


def run_step(
    name: str,
    cmd: list[str],
    cwd: Path,
    extra_env: dict[str, str] | None = None,
) -> dict[str, Any]:
    env = os.environ.copy()
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
    report_path: Path | None,
) -> tuple[int, list[dict[str, Any]]]:
    steps: list[dict[str, Any]] = []
    manifest = str(repo_root / "tests/e2e/rust_ffi/Cargo.toml")
    mutant_manifest = str(repo_root / "tests/e2e/rust_ffi/mutant_stub/Cargo.toml")

    cov_report = (
        str(report_path.parent / "runner_coverage_check.json")
        if report_path is not None
        else ""
    )
    cov_cmd = [
        sys.executable,
        str(repo_root / "tests/e2e/spec_coverage_check.py"),
        "--strict",
        "--matrix",
        str(repo_root / "tests/e2e/feature_matrix.json"),
    ]
    if cov_report:
        cov_cmd.extend(["--report", cov_report])

    s1 = run_step("spec_coverage_check", cov_cmd, repo_root)
    steps.append(s1)
    if not s1["ok"]:
        return s1["exit_code"], steps

    strict_rust_env = {"RUSTFLAGS": "-D warnings"}

    s2 = run_step(
        "cargo_test_no_run_all_394_tests",
        ["cargo", "test", "--manifest-path", manifest, "--no-run"],
        repo_root,
        extra_env=strict_rust_env,
    )
    steps.append(s2)
    if not s2["ok"]:
        return s2["exit_code"], steps

    s3 = run_step(
        "cargo_build_mutant_stub",
        ["cargo", "build", "--manifest-path", mutant_manifest],
        repo_root,
        extra_env=strict_rust_env,
    )
    steps.append(s3)
    if not s3["ok"]:
        return s3["exit_code"], steps

    ext = shared_lib_ext()
    if mutant_lib_arg:
        mutant_lib = Path(mutant_lib_arg)
    else:
        mutant_lib = (
            resolve_e2e_target_dir(repo_root)
            / "debug"
            / f"libfabushi_e2e_mutant_stub.{ext}"
        )
    if not mutant_lib.exists():
        print(f"ERROR: mutant_stub cdylib not found at {mutant_lib}", file=sys.stderr)
        return 2, steps

    test_env = {
        "RUSTFLAGS": "-D warnings",
        "FABUSHI_MUTANT_STUB_LIB": str(mutant_lib),
    }
    baseline_lib = baseline_lib_arg or os.environ.get("FABUSHI_BASELINE_HOST_LIB", "").strip()
    if baseline_lib:
        baseline_path = Path(baseline_lib)
        if not baseline_path.exists():
            print(f"ERROR: baseline host library not found at {baseline_path}", file=sys.stderr)
            return 2, steps
        test_env["FABUSHI_BASELINE_HOST_LIB"] = str(baseline_path)
        test_env["FABUSHI_REQUIRE_BASELINE_PROOF"] = "1"

    s4 = run_step(
        "cargo_test_self_validate_antistub",
        [
            "cargo",
            "test",
            "--manifest-path",
            manifest,
            "--lib",
            "--test",
            "self_validate_antistub",
            "--",
            "--nocapture",
        ],
        repo_root,
        extra_env=test_env,
    )
    steps.append(s4)
    return (0 if s4["ok"] else s4["exit_code"]), steps


def run_production(
    repo_root: Path,
    tier: str,
    host_lib_arg: str,
) -> tuple[int, list[dict[str, Any]]]:
    steps: list[dict[str, Any]] = []
    host_lib_str = host_lib_arg or os.environ.get("FABUSHI_HOST_LIB", "").strip()
    if not host_lib_str:
        print(
            "FAIL-CLOSED: production mode requires --host-lib or FABUSHI_HOST_LIB",
            file=sys.stderr,
        )
        return 2, steps

    host_lib = Path(host_lib_str)
    if "mutant_stub" in host_lib.as_posix().lower():
        print(
            f"FAIL-CLOSED: production mode refuses mutant_stub library: {host_lib}",
            file=sys.stderr,
        )
        return 2, steps
    if not host_lib.exists():
        print(f"FAIL-CLOSED: host library does not exist: {host_lib}", file=sys.stderr)
        return 2, steps

    ordered_tiers = ["tier1", "tier2", "tier3", "tier4"] if tier == "all" else [tier]
    manifest = str(repo_root / "tests/e2e/rust_ffi/Cargo.toml")
    prod_env = {
        "FABUSHI_HOST_LIB": str(host_lib.resolve()),
        "RUSTFLAGS": "-D warnings",
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
                return step["exit_code"], steps

    return 0, steps


def main() -> int:
    parser = argparse.ArgumentParser(description="Fabushi iOS v1.6.7 Opaque-Box E2E Runner")
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
    parser.add_argument("--report", default="", help="Optional JSON execution report output path")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[2]
    report_path = Path(args.report) if args.report else None
    t_start = time.monotonic()

    if args.mode == "self-validate":
        exit_code, steps = run_self_validate(
            repo_root,
            mutant_lib_arg=args.mutant_lib,
            baseline_lib_arg=args.baseline_lib,
            report_path=report_path,
        )
    else:
        exit_code, steps = run_production(
            repo_root,
            tier=args.tier,
            host_lib_arg=args.host_lib,
        )

    summary = {
        "mode": args.mode,
        "tier": args.tier,
        "ok": exit_code == 0,
        "exit_code": exit_code,
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
