#!/usr/bin/env python3
"""Strict opaque-box E2E coverage and anti-facade validator for Fabushi iOS v1.6.7.

Standard-library only. Designed to execute exclusively inside GitHub Actions CI.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import re
import sys
from collections import Counter
from pathlib import Path
from typing import Any

EXPECTED_FEATURE_IDS = [f"F{i:02d}" for i in range(1, 35)]
EXPECTED_RPC_NAMESPACES = [
    "auth",
    "vault",
    "session",
    "chats",
    "folders",
    "topics",
    "contacts",
    "messages",
    "sync",
    "outbox",
    "e2ee",
    "search",
    "media",
    "stories",
    "stickers",
    "fabushi",
    "multi_agent",
    "magpie",
    "coding",
    "mcp",
    "computers",
    "remote",
    "tailnet",
    "miniapp",
    "webmcp",
    "calls",
    "talk",
    "wallet",
    "settings",
    "proxy",
    "push",
    "actions",
    "generation",
]
EXPECTED_UI_NAMESPACES = [
    "fabushi.auth",
    "fabushi.shell",
    "fabushi.chatlist",
    "fabushi.compose",
    "fabushi.stage",
    "fabushi.composer",
    "fabushi.inspector",
    "fabushi.studio",
    "fabushi.magpie",
    "fabushi.remote",
    "fabushi.miniapp",
    "fabushi.call",
    "fabushi.settings",
]

GITIGNORE_TRAPPED_PATTERNS = [
    "build_suite.py",
    "gen_*.py",
    "generate_all.py",
    "remediate_*.py",
    "fix_*.py",
    "patch_*.py",
    "*.db",
    "*.db-wal",
    "*.db-shm",
]

# Build forbidden tokens dynamically so this validator file itself contains zero literal hits.
FORBIDDEN_LOCAL_PATH = "/" + "Users" + "/"
FORBIDDEN_CODE_PATTERNS: list[tuple[str, str]] = [
    ("ignored_test", "#" + "[ignore"),
    ("xctskip", "XCT" + "Skip"),
    ("tautological_assert_true", "assert!" + "(true)"),
    ("tautological_assert_eq_1_1", "assert_eq!" + "(1, 1)"),
    ("todo_macro", "to" + "do!("),
    ("unimplemented_macro", "unimplem" + "ented!("),
]

TEST_ATTR_RE = re.compile(r"^\s*#\[test\]\s*$")
FN_DEF_RE = re.compile(r"^\s*(?:pub\s+)?fn\s+([A-Za-z0-9_]+)\s*\(")


def scan_code_braces(line: str) -> tuple[int, int]:
    """Count `{` and `}` outside Rust string literals, raw strings, char literals, and `//` comments."""
    opens = 0
    closes = 0
    i = 0
    n = len(line)
    while i < n:
        ch = line[i]
        # Line comment
        if ch == "/" and i + 1 < n and line[i + 1] == "/":
            break
        # Raw string literal: r"..." or r#"..."# (including br#"..."#)
        if ch == "r" and (i == 0 or not (line[i - 1].isalnum() or line[i - 1] == "_")):
            j = i + 1
            hashes = 0
            while j < n and line[j] == "#":
                hashes += 1
                j += 1
            if j < n and line[j] == '"':
                j += 1
                closing = '"' + ("#" * hashes)
                end = line.find(closing, j)
                if end == -1:
                    break
                i = end + len(closing)
                continue
        # Regular string literal
        if ch == '"':
            i += 1
            while i < n:
                if line[i] == "\\":
                    i += 2
                    continue
                if line[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        # Char literal (avoid lifetimes like &'a str)
        if ch == "'" and i + 2 < n:
            if line[i + 1] == "\\" and i + 3 < n and line[i + 3] == "'":
                i += 4
                continue
            if line[i + 1] != "'" and line[i + 2] == "'":
                i += 3
                continue
        if ch == "{":
            opens += 1
        elif ch == "}":
            closes += 1
        i += 1
    return opens, closes


def parse_rust_test_functions(path: Path) -> dict[str, dict[str, Any]]:
    """Parse every #[test] function and its body lines via string/comment-aware brace tracking."""
    lines = path.read_text(encoding="utf-8").splitlines()
    tests: dict[str, dict[str, Any]] = {}
    i = 0
    n = len(lines)
    while i < n:
        if TEST_ATTR_RE.match(lines[i]):
            j = i + 1
            while j < n and (
                lines[j].strip() == ""
                or lines[j].strip().startswith("#[")
                or lines[j].strip().startswith("//")
            ):
                j += 1
            if j < n:
                m = FN_DEF_RE.match(lines[j])
                if m:
                    fn_name = m.group(1)
                    body_lines: list[str] = []
                    depth = 0
                    started = False
                    k = j
                    while k < n:
                        line = lines[k]
                        body_lines.append(line)
                        opens, closes = scan_code_braces(line)
                        if opens > 0:
                            started = True
                        depth += opens - closes
                        k += 1
                        if started and depth <= 0:
                            break
                    substantive = [
                        ln.strip()
                        for ln in body_lines[1:-1]
                        if ln.strip() and not ln.strip().startswith("//")
                    ]
                    body_text = "\n".join(body_lines)
                    tests[fn_name] = {
                        "file": path.as_posix(),
                        "start_line": j + 1,
                        "substantive_lines": len(substantive),
                        "calls_connect_production": "connect_production()" in body_text,
                        "has_assertion": any(
                            tok in body_text
                            for tok in (
                                "assert!(",
                                "assert_eq!(",
                                "assert_ne!(",
                                "expect_ok(",
                                "expect_ok_with_meta(",
                                "expect_err(",
                                "expect_err_with_gen(",
                                "verify_",
                                "assert_no_sample_chats(",
                                "assert_not_hardcoded_localhost(",
                                "assert_dir_excludes_plaintext_secret(",
                            )
                        ),
                    }
                    i = k
                    continue
        i += 1
    return tests


def check_coverage(repo_root: Path, matrix_path: Path, strict: bool) -> dict[str, Any]:
    errors: list[str] = []
    warnings: list[str] = []

    def record(msg: str) -> None:
        if strict:
            errors.append(msg)
        else:
            warnings.append(msg)

    if not matrix_path.exists():
        errors.append(f"feature matrix not found: {matrix_path}")
        return {"ok": False, "errors": errors, "warnings": warnings}

    matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
    infra_path = repo_root / "TEST_INFRA.md"
    infra_text = infra_path.read_text(encoding="utf-8") if infra_path.exists() else ""
    if not infra_path.exists():
        record("TEST_INFRA.md is missing")

    # 1. All 34 features F01..F34 in feature_matrix.json and TEST_INFRA.md
    features = matrix.get("features", [])
    feature_ids = [f.get("id") for f in features]
    if feature_ids != EXPECTED_FEATURE_IDS:
        record(
            f"feature_matrix.json feature IDs mismatch: expected F01..F34 ({len(EXPECTED_FEATURE_IDS)}), got {len(feature_ids)}"
        )
    for fid in EXPECTED_FEATURE_IDS:
        if f"`{fid}`" not in infra_text and fid not in infra_text:
            record(f"TEST_INFRA.md missing feature reference {fid}")

    # 2. All 33 required RPC namespaces & 13 required UI namespaces
    mat_rpc = matrix.get("required_rpc_namespaces", [])
    if sorted(mat_rpc) != sorted(EXPECTED_RPC_NAMESPACES) or len(mat_rpc) != 33:
        record(f"feature_matrix.json must declare exact 33 required_rpc_namespaces (got {len(mat_rpc)})")

    mat_ui = matrix.get("required_ui_namespaces", [])
    if sorted(mat_ui) != sorted(EXPECTED_UI_NAMESPACES) or len(mat_ui) != 13:
        record(f"feature_matrix.json must declare exact 13 required_ui_namespaces (got {len(mat_ui)})")

    for ui_ns in EXPECTED_UI_NAMESPACES:
        if ui_ns not in infra_text:
            record(f"TEST_INFRA.md missing required UI accessibility namespace `{ui_ns}`")

    # Parse all Tier 1-4 Rust test files
    rust_tests_dir = repo_root / "tests/e2e/rust_ffi/tests"
    tier_files = sorted(rust_tests_dir.glob("tier*.rs"))
    all_tier_fns: dict[str, dict[str, Any]] = {}
    all_tier_text_parts: list[str] = []

    for tf in tier_files:
        text = tf.read_text(encoding="utf-8")
        all_tier_text_parts.append(text)
        if "mutant_stub" in text:
            record(f"FORBIDDEN mutant_stub reference found in production tier file: {tf.relative_to(repo_root)}")
        parsed = parse_rust_test_functions(tf)
        for fn_name, meta in parsed.items():
            if fn_name in all_tier_fns:
                record(f"duplicate Rust test function name: {fn_name}")
            all_tier_fns[fn_name] = meta

    combined_tier_text = "\n".join(all_tier_text_parts)

    rpc_counts: dict[str, int] = {}
    for ns in EXPECTED_RPC_NAMESPACES:
        hits = len(re.findall(rf'"{re.escape(ns)}\.[A-Za-z0-9_.]+"', combined_tier_text))
        rpc_counts[ns] = hits
        if hits == 0:
            record(f"RPC namespace `{ns}.*` has 0 dispatches across Tier 1-4 Rust tests")

    # 4. Per-feature Tier 1 (>=5) & Tier 2 (>=5) verification
    t1_count = 0
    t2_count = 0
    referenced_fns: set[str] = set()
    for feat in features:
        fid = feat.get("id", "UNKNOWN")
        t1_list = feat.get("tier1_tests", [])
        t2_list = feat.get("tier2_tests", [])
        if len(t1_list) < 5:
            record(f"{fid} has only {len(t1_list)} Tier 1 tests (required >= 5)")
        if len(t2_list) < 5:
            record(f"{fid} has only {len(t2_list)} Tier 2 tests (required >= 5)")
        for entry in t1_list:
            fn_name = entry.get("fn", "")
            referenced_fns.add(fn_name)
            if fn_name not in all_tier_fns:
                record(f"{fid} Tier 1 test `{entry.get('id')}` -> missing Rust fn `{fn_name}`")
            else:
                t1_count += 1
        for entry in t2_list:
            fn_name = entry.get("fn", "")
            referenced_fns.add(fn_name)
            if fn_name not in all_tier_fns:
                record(f"{fid} Tier 2 test `{entry.get('id')}` -> missing Rust fn `{fn_name}`")
            else:
                t2_count += 1

    # 5. Tier 3 pairwise (>=34, every F01..F34 in >=2 pairs) & Tier 4 scenarios (>=17, >=3 features each)
    t3_list = matrix.get("tier3_pairwise_tests", [])
    if len(t3_list) < 34:
        record(f"Tier 3 pairwise count is {len(t3_list)} (required >= 34)")
    pair_degree: Counter[str] = Counter()
    seen_pairs: set[tuple[str, str]] = set()
    t3_count = 0
    for entry in t3_list:
        pair = entry.get("features", [])
        fn_name = entry.get("fn", "")
        referenced_fns.add(fn_name)
        if len(pair) != 2 or pair[0] == pair[1]:
            record(f"Tier 3 entry `{entry.get('id')}` must specify 2 distinct features: {pair}")
        else:
            canon = tuple(sorted(pair))
            if canon in seen_pairs:
                record(f"Tier 3 duplicate unordered pair: {canon}")
            seen_pairs.add(canon)
            pair_degree[pair[0]] += 1
            pair_degree[pair[1]] += 1
        if fn_name not in all_tier_fns:
            record(f"Tier 3 test `{entry.get('id')}` -> missing Rust fn `{fn_name}`")
        else:
            t3_count += 1

    for fid in EXPECTED_FEATURE_IDS:
        if pair_degree[fid] < 2:
            record(f"Tier 3 pairwise coverage insufficient for {fid}: appears in {pair_degree[fid]} pairs (required >= 2)")

    t4_list = matrix.get("tier4_scenarios", [])
    if len(t4_list) < 17:
        record(f"Tier 4 scenario count is {len(t4_list)} (required >= 17)")
    t4_count = 0
    for entry in t4_list:
        f_list = entry.get("features", [])
        fn_name = entry.get("fn", "")
        referenced_fns.add(fn_name)
        if len(set(f_list)) < 3:
            record(f"Tier 4 scenario `{entry.get('id')}` covers < 3 distinct features: {f_list}")
        if fn_name not in all_tier_fns:
            record(f"Tier 4 scenario `{entry.get('id')}` -> missing Rust fn `{fn_name}`")
        else:
            t4_count += 1

    # 6. Total executable Tier 1-4 tests >= 391
    total_tier_tests = len(all_tier_fns)
    if total_tier_tests < 391:
        record(f"Total Tier 1-4 executable Rust tests is {total_tier_tests} (required >= 391)")
    unmapped = sorted(set(all_tier_fns.keys()) - referenced_fns)
    if unmapped:
        record(f"Unmapped Rust tests not listed in feature_matrix.json: {unmapped[:10]}")

    # 7. Substantive body, connect_production(), and real assertions in EVERY Tier 1-4 test
    for fn_name, meta in sorted(all_tier_fns.items()):
        if not meta["calls_connect_production"]:
            record(f"Tier test `{fn_name}` does not call connect_production()")
        if not meta["has_assertion"]:
            record(f"Tier test `{fn_name}` lacks substantive assertion")
        if meta["substantive_lines"] < 3:
            record(f"Tier test `{fn_name}` has only {meta['substantive_lines']} substantive lines (required >= 3)")

    # 8. Forbidden anti-patterns, local workstation home paths, and internal Cargo path deps
    scan_paths: list[Path] = [
        infra_path,
        matrix_path,
        repo_root / "tests/e2e/runner.py",
        repo_root / "tests/e2e/spec_coverage_check.py",
        repo_root / ".github/workflows/ios-e2e-suite.yml",
    ]
    rust_ffi_root = repo_root / "tests/e2e/rust_ffi"
    if rust_ffi_root.exists():
        for p in sorted(rust_ffi_root.rglob("*")):
            if p.is_file() and p.suffix in {".rs", ".toml"} and "target" not in p.parts:
                scan_paths.append(p)

    for sp in scan_paths:
        if not sp.exists():
            continue
        rel = sp.relative_to(repo_root).as_posix()
        txt = sp.read_text(encoding="utf-8")
        if FORBIDDEN_LOCAL_PATH in txt:
            record(f"FORBIDDEN local workstation path detected in {rel}")
        if sp.suffix == ".rs":
            for label, needle in FORBIDDEN_CODE_PATTERNS:
                if needle in txt:
                    record(f"FORBIDDEN pattern `{label}` detected in {rel}")
        if sp.name == "Cargo.toml":
            for bad_dep in ("source/host", "source/desktop_core", "../host", "../../source"):
                if bad_dep in txt:
                    record(f"FORBIDDEN internal Cargo path dependency `{bad_dep}` in {rel}")

    # 9. Zero .gitignore-trapped filenames under tests/e2e/
    e2e_root = repo_root / "tests/e2e"
    for p in sorted(e2e_root.rglob("*")):
        rel = p.relative_to(e2e_root)
        if "target" in rel.parts:
            record(f"FORBIDDEN target/ directory inside tests/e2e/: {rel.as_posix()}")
        if p.is_file():
            for pat in GITIGNORE_TRAPPED_PATTERNS:
                if fnmatch.fnmatch(p.name, pat):
                    record(f"FORBIDDEN .gitignore-trapped filename under tests/e2e/: {rel.as_posix()}")

    # 10. Verify legacy tests/e2e/master_e2e_suite.swift preserved intact (required by project.yml:146)
    master_suite = e2e_root / "master_e2e_suite.swift"
    if not master_suite.exists():
        record("tests/e2e/master_e2e_suite.swift is missing (still required by mobile/ios/project.yml:146)")

    # Verify deleted legacy unreferenced challenger files stay deleted
    for legacy_removed in (
        "challenger_m6_core_adversarial.swift",
        "challenger_m6_iter2_adversarial.swift",
    ):
        if (e2e_root / legacy_removed).exists():
            record(f"Unreferenced legacy file still present: tests/e2e/{legacy_removed}")

    return {
        "ok": len(errors) == 0,
        "strict": strict,
        "counts": {
            "features": len(features),
            "rpc_namespaces_covered": sum(1 for v in rpc_counts.values() if v > 0),
            "rpc_namespaces_required": len(EXPECTED_RPC_NAMESPACES),
            "ui_namespaces_required": len(EXPECTED_UI_NAMESPACES),
            "tier1_tests": t1_count,
            "tier2_tests": t2_count,
            "tier3_pairwise_tests": t3_count,
            "tier4_scenarios": t4_count,
            "total_tier1_to_4_tests": total_tier_tests,
        },
        "rpc_namespace_hits": rpc_counts,
        "errors": errors,
        "warnings": warnings,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Fabushi iOS v1.6.7 E2E Spec Coverage Validator")
    parser.add_argument("--strict", action="store_true", help="Fail closed on any coverage or integrity violation")
    parser.add_argument(
        "--matrix",
        default="tests/e2e/feature_matrix.json",
        help="Path to feature_matrix.json relative to repo root or absolute",
    )
    parser.add_argument("--report", default="", help="Optional path to write JSON coverage report")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[2]
    matrix_path = Path(args.matrix)
    if not matrix_path.is_absolute():
        matrix_path = repo_root / matrix_path

    report = check_coverage(repo_root, matrix_path, strict=args.strict)

    if args.report:
        report_path = Path(args.report)
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    print(json.dumps(report["counts"], indent=2, sort_keys=True))
    if report["warnings"]:
        for w in report["warnings"]:
            print(f"WARNING: {w}", file=sys.stderr)
    if report["errors"]:
        for e in report["errors"]:
            print(f"ERROR: {e}", file=sys.stderr)
        return 1
    print("spec_coverage_check: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
