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
VALID_FEATURE_SET = set(EXPECTED_FEATURE_IDS)

EXPECTED_C_ABI_SYMBOLS = [
    "fabushi_host_create",
    "fabushi_host_dispatch",
    "fabushi_host_poll_events",
    "fabushi_host_cancel",
    "fabushi_host_destroy",
    "fabushi_host_free_string",
]
MUTANT_STUB_SENTINEL_SYMBOL = "fabushi_e2e_is_mutant_stub"

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

ALLOWED_TIER_FILES: dict[str, set[str]] = {
    "tier1": {"tier1_auth_shell_messaging.rs", "tier1_agents_platform_hig.rs"},
    "tier2": {"tier2_auth_shell_messaging.rs", "tier2_agents_platform_hig.rs"},
    "tier3": {"tier3_pairwise_combos.rs"},
    "tier4": {"tier4_real_world_scenarios.rs"},
}
ALL_ALLOWED_TIER_FILES = set().union(*ALLOWED_TIER_FILES.values())

FEATURE_RPC_MATCHERS: dict[str, tuple[str, ...]] = {
    "F01": (
        "auth.register",
        "auth.login",
        "auth.get_kdf_params",
        "auth.upgrade_split_key",
        "auth.change_password",
        "session.current",
    ),
    "F02": ("auth.email.", "auth.phone."),
    "F03": ("auth.alipay.",),
    "F04": ("auth.passkey.", "auth.pair."),
    "F05": (
        "auth.two_factor.",
        "session.list_devices",
        "session.revoke_device",
        "session.revoke_other_devices",
    ),
    "F06": (
        "session.add_account",
        "session.list_accounts",
        "session.switch_account",
        "session.logout",
        "session.passcode.",
        "generation.",
    ),
    "F07": (
        "settings.get_shell_state",
        "settings.set_shell_state",
        "session.get_connection_status",
        "actions.navigate",
    ),
    "F08": ("chats.compose_candidates", "chats.compose_send"),
    "F09": (
        "chats.list",
        "chats.list_unread",
        "chats.create",
        "chats.open",
        "chats.pin_dialog",
        "chats.archive",
        "chats.mute",
        "chats.invites.",
        "chats.join",
        "folders.",
        "topics.",
    ),
    "F10": ("contacts.", "chats.send_as_list", "chats.prestart_bot", "chats.start_bot"),
    "F11": ("sync.", "outbox."),
    "F12": ("messages.send", "messages.history", "messages.compute_layout_groups", "messages.polls."),
    "F13": (
        "messages.edit",
        "messages.delete",
        "messages.forward",
        "messages.react",
        "messages.pins.",
        "chats.set_ttl",
        "chats.statistics",
        "chats.export",
        "chats.clear_history",
    ),
    "F14": ("messages.composer.", "messages.draft."),
    "F15": ("fabushi.commands.", "fabushi.context."),
    "F16": ("chats.inspector.", "chats.members.", "chats.shared_media", "fabushi.bots."),
    "F17": (
        "fabushi.get_status",
        "fabushi.list_toolsets",
        "fabushi.set_toolsets",
        "fabushi.toggle_toolset",
        "fabushi.kanban_",
        "fabushi.skills_",
        "fabushi.memory_",
        "fabushi.honcho_",
        "fabushi.cron_",
        "fabushi.approvals_",
        "fabushi.sessions_",
    ),
    "F18": ("multi_agent.",),
    "F19": ("magpie.",),
    "F20": ("coding.",),
    "F21": ("mcp.",),
    "F22": ("computers.", "remote.", "tailnet."),
    "F23": ("vault.",),
    "F24": ("miniapp.", "webmcp."),
    "F25": ("media.", "stories.", "stickers."),
    "F26": ("calls.", "talk."),
    "F27": ("search.",),
    "F28": ("e2ee.",),
    "F29": ("wallet.",),
    "F30": (
        "actions.list",
        "actions.schema",
        "actions.invoke",
        "actions.approve_gate",
        "actions.route_url",
        "push.",
    ),
    "F31": ("settings.theme_", "settings.accessibility_", "settings.locale_"),
    "F32": ("settings.sections_", "settings.get_all", "settings.patch", "settings.storage_"),
    "F33": ("proxy.", "settings.config_resolve", "settings.update_check", "settings.install_binary_update"),
    "F34": (
        "generation.",
        "settings.capability_ledger",
        "debug.trigger_panic",
        "channels.ocrm_",
        "channels.fbcp_",
        "settings.passport_",
        "wallet.ton_",
        "fbcp.",
        "fabushi.fbcp_",
        "totally.bogus_method",
        "nonexistent.",
    ),
}

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

# Construct forbidden tokens dynamically so this validator file itself contains zero literal hits.
FORBIDDEN_LOCAL_PATH = "/" + "Users" + "/"
FORBIDDEN_CODE_REGEXES: list[tuple[str, re.Pattern[str]]] = [
    ("ignored_test", re.compile(r"#\s*\[[^\]]*\b" + "ign" + "ore" + r"\b")),
    ("xctskip", re.compile(r"\b" + "XCT" + "Skip" + r"\b")),
    ("tautological_assert_true", re.compile(r"\bassert!\s*\(\s*true\s*(?:,|\))")),
    ("tautological_assert_eq_1_1", re.compile(r"\bassert_eq!\s*\(\s*1\s*,\s*1\s*(?:,|\))")),
    ("todo_macro", re.compile(r"\b" + "to" + r"do!\s*\(")),
    ("unimplemented_macro", re.compile(r"\b" + "unimplem" + r"ented!\s*\(")),
]
DISABLED_CFG_RE = re.compile(r"#\s*!?\s*\[\s*(?:cfg|cfg_attr)\s*\(")
FORBIDDEN_TIER_SHOULD_PANIC_RE = re.compile(
    r"#\s*!?\s*\[\s*(?:[A-Za-z0-9_]+::)*should_panic\b"
)
MUTANT_STUB_RE = re.compile(r"mutant[-_]stub", re.IGNORECASE)

CHAR_LITERAL_RE = re.compile(
    r"'(?:\\(?:[nrt0'\"\\]|x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f][0-9A-Fa-f_]*\})|[^'\\\n])'"
)
TEST_ATTR_EXACT_RE = re.compile(r"^#\s*\[\s*(?:[A-Za-z0-9_]+::)*test\s*\]$")
DISABLED_TEST_ATTR_RE = re.compile(
    r"^#\s*\[\s*(?:(?:[A-Za-z0-9_]+::)*(?:"
    + "ign"
    + r"ore|should_panic)\b|cfg\s*\(|cfg_attr\s*\()"
)
FN_HEADER_RE = re.compile(r"(?:pub(?:\s*\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)\s*\(")
CONNECT_PROD_CALL_RE = re.compile(r"\bconnect_production\s*\(\s*\)")
ASSERTION_CALL_RE = re.compile(
    r"\b(?:"
    r"assert!|"
    r"assert_eq!|"
    r"assert_ne!|"
    r"expect_err|"
    r"expect_err_with_gen|"
    r"verify_[a-z0-9_]+|"
    r"assert_no_sample_chats|"
    r"assert_not_hardcoded_localhost|"
    r"assert_dir_excludes_plaintext_secret"
    r")\s*\("
)
RPC_DISPATCH_SITE_RE = re.compile(
    r"\.\s*(?:expect_ok|expect_ok_with_meta|expect_ok_with_gen|expect_err|expect_err_with_gen|dispatch|dispatch_with_gen)\s*\("
)
RPC_DISPATCH_CALL_RE = re.compile(
    r"\.\s*(?:expect_ok|expect_ok_with_meta|expect_ok_with_gen|expect_err|expect_err_with_gen|dispatch|dispatch_with_gen)"
    r"\s*\(\s*\"([a-z_][a-z0-9_]*\.[a-z0-9_.]+)\""
)


def lex_rust_source(source: str) -> tuple[str, str]:
    """Single-pass stateful Rust lexer returning (no_comments, code_only) with 1:1 offsets."""
    n = len(source)
    no_comments: list[str] = [" "] * n
    code_only: list[str] = [" "] * n
    i = 0

    while i < n:
        ch = source[i]

        # 1. Line comment: //...
        if ch == "/" and i + 1 < n and source[i + 1] == "/":
            while i < n and source[i] != "\n":
                i += 1
            if i < n and source[i] == "\n":
                no_comments[i] = "\n"
                code_only[i] = "\n"
                i += 1
            continue

        # 2. Nested Rust block comment: /* ... /* ... */ ... */
        if ch == "/" and i + 1 < n and source[i + 1] == "*":
            depth = 1
            i += 2
            while i < n and depth > 0:
                if source[i] == "/" and i + 1 < n and source[i + 1] == "*":
                    depth += 1
                    i += 2
                elif source[i] == "*" and i + 1 < n and source[i + 1] == "/":
                    depth -= 1
                    i += 2
                else:
                    if source[i] == "\n":
                        no_comments[i] = "\n"
                        code_only[i] = "\n"
                    i += 1
            continue

        # 3. Raw string literal: r"...", r#"..."#, br#"..."#, cr#"..."#
        prev_is_ident = i > 0 and (source[i - 1].isalnum() or source[i - 1] == "_")
        raw_prefix = 0
        if not prev_is_ident:
            if source[i : i + 2] in ("br", "cr"):
                raw_prefix = 2
            elif ch == "r":
                raw_prefix = 1
        if raw_prefix > 0:
            j = i + raw_prefix
            hashes = 0
            while j < n and source[j] == "#":
                hashes += 1
                j += 1
            if j < n and source[j] == '"':
                closing = '"' + ("#" * hashes)
                end = source.find(closing, j + 1)
                stop = n if end == -1 else end + len(closing)
                for p in range(i, stop):
                    no_comments[p] = source[p]
                    if source[p] == "\n":
                        code_only[p] = "\n"
                i = stop
                continue

        # 4. Byte/C string literal (b"...", c"...") or regular string literal ("...")
        str_prefix = 0
        if not prev_is_ident and ch in ("b", "c") and i + 1 < n and source[i + 1] == '"':
            str_prefix = 1
        if ch == '"' or str_prefix == 1:
            if str_prefix:
                no_comments[i] = ch
                i += 1
            no_comments[i] = '"'
            i += 1
            while i < n:
                c = source[i]
                no_comments[i] = c
                if c == "\n":
                    code_only[i] = "\n"
                if c == "\\" and i + 1 < n:
                    nxt = source[i + 1]
                    no_comments[i + 1] = nxt
                    if nxt == "\n":
                        code_only[i + 1] = "\n"
                    i += 2
                    continue
                i += 1
                if c == '"':
                    break
            continue

        # 5. Char / byte literal ('a', '{', '\'', '\u{1F600}', b'{') vs Rust lifetimes ('a, 'static)
        if ch == "'" or (not prev_is_ident and ch == "b" and i + 1 < n and source[i + 1] == "'"):
            quote_pos = i + 1 if ch == "b" else i
            m = CHAR_LITERAL_RE.match(source, quote_pos)
            if m:
                for p in range(i, m.end()):
                    no_comments[p] = source[p]
                i = m.end()
                continue

        # 6. Normal code character
        no_comments[i] = ch
        code_only[i] = ch
        i += 1

    return "".join(no_comments), "".join(code_only)


def _match_balanced(text: str, open_idx: int, open_ch: str, close_ch: str) -> int:
    """Return index of matching closing delimiter or -1."""
    n = len(text)
    depth = 0
    i = open_idx
    while i < n:
        c = text[i]
        if c == open_ch:
            depth += 1
        elif c == close_ch:
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return -1


def parse_rust_test_functions(
    path: Path, no_comments: str, code_only: str
) -> tuple[dict[str, dict[str, Any]], list[str]]:
    """Parse every #[test] function and extract strictly its interior `{ ... }` body."""
    tests: dict[str, dict[str, Any]] = {}
    parse_errors: list[str] = []
    n = len(code_only)
    i = 0

    while i < n:
        # Collect a contiguous sequence of outer attributes `#[...]`
        if code_only[i] == "#":
            j = i + 1
            while j < n and code_only[j].isspace():
                j += 1
            if j < n and code_only[j] == "[":
                attrs: list[str] = []
                cur = i
                while cur < n:
                    while cur < n and code_only[cur].isspace():
                        cur += 1
                    if cur >= n or code_only[cur] != "#":
                        break
                    k = cur + 1
                    while k < n and code_only[k].isspace():
                        k += 1
                    if k >= n or code_only[k] != "[":
                        break
                    end_bracket = _match_balanced(code_only, k, "[", "]")
                    if end_bracket == -1:
                        parse_errors.append(f"unclosed attribute in {path.name}")
                        return tests, parse_errors
                    attrs.append(code_only[cur : end_bracket + 1].strip())
                    cur = end_bracket + 1

                is_test = any(TEST_ATTR_EXACT_RE.match(a) for a in attrs)
                if is_test:
                    while cur < n and code_only[cur].isspace():
                        cur += 1
                    fn_m = FN_HEADER_RE.match(code_only, cur)
                    if not fn_m:
                        line_no = code_only.count("\n", 0, i) + 1
                        parse_errors.append(
                            f"#[test] attribute without fn definition at {path.name}:{line_no}"
                        )
                        i = cur
                        continue

                    fn_name = fn_m.group(1)
                    start_line = code_only.count("\n", 0, cur) + 1

                    # Reject any disabled/conditional attribute attached to #[test]
                    bad_attrs = [a for a in attrs if DISABLED_TEST_ATTR_RE.match(a)]
                    if bad_attrs:
                        parse_errors.append(
                            f"FORBIDDEN disabled/conditional attribute on test `{fn_name}` in {path.name}:{start_line}: {bad_attrs}"
                        )
                        i = fn_m.end()
                        continue

                    # Match parameter list `(...)` then find body `{ ... }`
                    paren_open = fn_m.end() - 1
                    paren_close = _match_balanced(code_only, paren_open, "(", ")")
                    if paren_close == -1:
                        parse_errors.append(f"unclosed parameter list for `{fn_name}` in {path.name}:{start_line}")
                        i = fn_m.end()
                        continue

                    body_open = code_only.find("{", paren_close + 1)
                    if body_open == -1:
                        parse_errors.append(f"missing body brace for `{fn_name}` in {path.name}:{start_line}")
                        i = paren_close + 1
                        continue

                    body_close = _match_balanced(code_only, body_open, "{", "}")
                    if body_close == -1:
                        parse_errors.append(f"unclosed body brace for `{fn_name}` in {path.name}:{start_line}")
                        i = body_open + 1
                        continue

                    body_code_only = code_only[body_open + 1 : body_close]
                    body_no_comments = no_comments[body_open + 1 : body_close]
                    substantive_lines = sum(
                        1 for ln in body_no_comments.splitlines() if ln.strip()
                    )

                    dispatched_rpcs: list[str] = []
                    for call_m in RPC_DISPATCH_SITE_RE.finditer(body_code_only):
                        rpc_m = RPC_DISPATCH_CALL_RE.match(body_no_comments, call_m.start())
                        if rpc_m:
                            dispatched_rpcs.append(rpc_m.group(1))

                    if fn_name in tests:
                        parse_errors.append(f"duplicate Rust test function name `{fn_name}` in {path.name}")
                    tests[fn_name] = {
                        "file": path.as_posix(),
                        "filename": path.name,
                        "start_line": start_line,
                        "substantive_lines": substantive_lines,
                        "calls_connect_production": bool(CONNECT_PROD_CALL_RE.search(body_code_only)),
                        "has_assertion": bool(ASSERTION_CALL_RE.search(body_code_only)),
                        "dispatched_rpcs": dispatched_rpcs,
                    }
                    i = body_close + 1
                    continue

        i += 1

    return tests, parse_errors


def rpc_matches_feature(rpc: str, fid: str) -> bool:
    for pat in FEATURE_RPC_MATCHERS.get(fid, ()):
        if pat.endswith(".") or pat.endswith("_"):
            if rpc.startswith(pat):
                return True
        elif rpc == pat:
            return True
    return False


def max_bipartite_match(features: list[str], feature_to_rpcs: dict[str, set[str]]) -> int:
    """Compute maximum cardinality matching between declared features and distinct dispatched RPCs."""
    rpc_owner: dict[str, str] = {}

    def dfs(fid: str, visited: set[str]) -> bool:
        for rpc in sorted(feature_to_rpcs.get(fid, set())):
            if rpc in visited:
                continue
            visited.add(rpc)
            if rpc not in rpc_owner or dfs(rpc_owner[rpc], visited):
                rpc_owner[rpc] = fid
                return True
        return False

    matched = 0
    for fid in features:
        if dfs(fid, set()):
            matched += 1
    return matched


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
        return {"ok": False, "strict": strict, "counts": {}, "errors": errors, "warnings": warnings}

    try:
        matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        errors.append(f"failed to parse feature matrix {matrix_path}: {exc}")
        return {"ok": False, "strict": strict, "counts": {}, "errors": errors, "warnings": warnings}

    if not isinstance(matrix, dict):
        errors.append(
            f"feature matrix {matrix_path} must be a JSON object, got {type(matrix).__name__}"
        )
        return {"ok": False, "strict": strict, "counts": {}, "errors": errors, "warnings": warnings}

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

    # 2. Exact 6 C-ABI symbols + mutant_stub negative-oracle sentinel invariant
    mat_c_abi = matrix.get("required_c_abi_symbols", [])
    if mat_c_abi != EXPECTED_C_ABI_SYMBOLS:
        record(
            f"feature_matrix.json required_c_abi_symbols mismatch: expected {EXPECTED_C_ABI_SYMBOLS}, got {mat_c_abi}"
        )
    for sym in [*EXPECTED_C_ABI_SYMBOLS, MUTANT_STUB_SENTINEL_SYMBOL]:
        if sym not in infra_text:
            record(f"TEST_INFRA.md missing required C-ABI / sentinel symbol `{sym}`")

    ffi_client_path = repo_root / "tests/e2e/rust_ffi/src/ffi_client.rs"
    if not ffi_client_path.exists():
        record("tests/e2e/rust_ffi/src/ffi_client.rs is missing")
    else:
        ffi_no_comments, _ = lex_rust_source(ffi_client_path.read_text(encoding="utf-8"))
        for sym in [*EXPECTED_C_ABI_SYMBOLS, MUTANT_STUB_SENTINEL_SYMBOL]:
            if sym not in ffi_no_comments:
                record(f"ffi_client.rs missing required C-ABI / sentinel symbol `{sym}`")

    mutant_lib_path = repo_root / "tests/e2e/rust_ffi/mutant_stub/src/lib.rs"
    if not mutant_lib_path.exists():
        record("tests/e2e/rust_ffi/mutant_stub/src/lib.rs is missing")
    else:
        mutant_no_comments, _ = lex_rust_source(mutant_lib_path.read_text(encoding="utf-8"))
        if not re.search(r'extern\s+"C"\s+fn\s+fabushi_e2e_is_mutant_stub\s*\(', mutant_no_comments):
            record("mutant_stub/src/lib.rs must export `extern \"C\" fn fabushi_e2e_is_mutant_stub`")

    # 3. All 33 required RPC namespaces & 13 required UI namespaces
    mat_rpc = matrix.get("required_rpc_namespaces", [])
    if sorted(mat_rpc) != sorted(EXPECTED_RPC_NAMESPACES) or len(mat_rpc) != 33:
        record(f"feature_matrix.json must declare exact 33 required_rpc_namespaces (got {len(mat_rpc)})")

    mat_ui = matrix.get("required_ui_namespaces", [])
    if sorted(mat_ui) != sorted(EXPECTED_UI_NAMESPACES) or len(mat_ui) != 13:
        record(f"feature_matrix.json must declare exact 13 required_ui_namespaces (got {len(mat_ui)})")

    for ui_ns in EXPECTED_UI_NAMESPACES:
        if ui_ns not in infra_text:
            record(f"TEST_INFRA.md missing required UI accessibility namespace `{ui_ns}`")

    # 4. Parse all Tier 1-4 Rust test files via stateful lexer
    rust_tests_dir = repo_root / "tests/e2e/rust_ffi/tests"
    tier_files = sorted(rust_tests_dir.glob("tier*.rs"))
    all_tier_fns: dict[str, dict[str, Any]] = {}

    for tf in tier_files:
        if tf.name not in ALL_ALLOWED_TIER_FILES:
            record(f"UNEXPECTED tier test file outside allowed Tier 1-4 set: {tf.relative_to(repo_root)}")
        raw_text = tf.read_text(encoding="utf-8")
        no_comments, code_only = lex_rust_source(raw_text)

        if MUTANT_STUB_RE.search(no_comments):
            record(f"FORBIDDEN mutant_stub reference found in production tier file: {tf.relative_to(repo_root)}")
        if DISABLED_CFG_RE.search(code_only):
            record(f"FORBIDDEN #[cfg(...)] / #[cfg_attr(...)] attribute found in production tier file: {tf.relative_to(repo_root)}")
        if FORBIDDEN_TIER_SHOULD_PANIC_RE.search(code_only):
            record(f"FORBIDDEN #[should_panic] attribute found in production tier file: {tf.relative_to(repo_root)}")

        parsed, parse_errs = parse_rust_test_functions(tf, no_comments, code_only)
        for pe in parse_errs:
            record(pe)
        for fn_name, meta in parsed.items():
            if fn_name in all_tier_fns:
                record(f"duplicate Rust test function name across files: {fn_name}")
            all_tier_fns[fn_name] = meta

    dispatched_ns_counter: Counter[str] = Counter()
    for meta in all_tier_fns.values():
        for rpc in meta["dispatched_rpcs"]:
            ns = rpc.split(".", 1)[0]
            dispatched_ns_counter[ns] += 1

    rpc_counts: dict[str, int] = {}
    for ns in EXPECTED_RPC_NAMESPACES:
        hits = dispatched_ns_counter.get(ns, 0)
        rpc_counts[ns] = hits
        if hits == 0:
            record(f"RPC namespace `{ns}.*` has 0 dispatches across Tier 1-4 Rust tests")

    # 5. Enforce 1:1 unique test IDs, unique fn names, and strict tier-to-file placement
    seen_ids: dict[str, str] = {}
    seen_fns: dict[str, str] = {}

    def register_matrix_entry(tier_key: str, context: str, entry: dict[str, Any]) -> bool:
        test_id = str(entry.get("id", "")).strip()
        fn_name = str(entry.get("fn", "")).strip()
        ok = True
        if not test_id:
            record(f"{context} entry missing non-empty `id`")
            ok = False
        elif test_id in seen_ids:
            record(f"duplicate test ID `{test_id}` ({context} duplicates {seen_ids[test_id]})")
            ok = False
        else:
            seen_ids[test_id] = context

        if not fn_name:
            record(f"{context} test `{test_id}` missing non-empty `fn`")
            ok = False
        elif fn_name in seen_fns:
            record(f"duplicate Rust fn mapping `{fn_name}` ({context} duplicates {seen_fns[fn_name]})")
            ok = False
        else:
            seen_fns[fn_name] = context

        if fn_name and fn_name not in all_tier_fns:
            record(f"{context} test `{test_id}` -> missing Rust fn `{fn_name}`")
            return False
        if fn_name in all_tier_fns:
            actual_file = all_tier_fns[fn_name]["filename"]
            if actual_file not in ALLOWED_TIER_FILES[tier_key]:
                record(
                    f"{context} test `{test_id}` (`{fn_name}`) resides in `{actual_file}`, expected one of {sorted(ALLOWED_TIER_FILES[tier_key])}"
                )
                ok = False
        return ok

    t1_count = 0
    t2_count = 0
    for feat in features:
        fid = feat.get("id", "UNKNOWN")
        t1_list = feat.get("tier1_tests", [])
        t2_list = feat.get("tier2_tests", [])
        if len(t1_list) < 5:
            record(f"{fid} has only {len(t1_list)} Tier 1 tests (required >= 5)")
        if len(t2_list) < 5:
            record(f"{fid} has only {len(t2_list)} Tier 2 tests (required >= 5)")
        for entry in t1_list:
            if register_matrix_entry("tier1", f"{fid} Tier 1", entry):
                t1_count += 1
        for entry in t2_list:
            if register_matrix_entry("tier2", f"{fid} Tier 2", entry):
                t2_count += 1

    # 6. Tier 3 pairwise (>=34, valid F01..F34, degree >=2, AND body dispatches BOTH declared features!)
    t3_list = matrix.get("tier3_pairwise_tests", [])
    if len(t3_list) < 34:
        record(f"Tier 3 pairwise count is {len(t3_list)} (required >= 34)")
    pair_degree: Counter[str] = Counter()
    seen_pairs: set[tuple[str, str]] = set()
    t3_count = 0
    for entry in t3_list:
        tid = str(entry.get("id", "UNKNOWN"))
        pair = entry.get("features", [])
        fn_name = str(entry.get("fn", "")).strip()
        valid_entry = register_matrix_entry("tier3", f"Tier 3 `{tid}`", entry)
        if valid_entry:
            t3_count += 1

        if (
            len(pair) != 2
            or pair[0] not in VALID_FEATURE_SET
            or pair[1] not in VALID_FEATURE_SET
            or pair[0] == pair[1]
        ):
            record(f"Tier 3 entry `{tid}` must specify 2 distinct valid features in F01..F34: {pair}")
            continue

        fa, fb = pair[0], pair[1]
        canon = (fa, fb) if fa < fb else (fb, fa)
        if canon in seen_pairs:
            record(f"Tier 3 duplicate unordered pair: {canon}")
        seen_pairs.add(canon)
        pair_degree[fa] += 1
        pair_degree[fb] += 1

        if fn_name in all_tier_fns:
            rpcs = set(all_tier_fns[fn_name]["dispatched_rpcs"])
            hits_a = {r for r in rpcs if rpc_matches_feature(r, fa)}
            hits_b = {r for r in rpcs if rpc_matches_feature(r, fb)}
            if not hits_a:
                record(f"Tier 3 `{tid}` (`{fn_name}`) declares `{fa}` but dispatches 0 matching RPCs (found: {sorted(rpcs)})")
            if not hits_b:
                record(f"Tier 3 `{tid}` (`{fn_name}`) declares `{fb}` but dispatches 0 matching RPCs (found: {sorted(rpcs)})")
            if hits_a and hits_b and len(hits_a | hits_b) < 2:
                record(
                    f"Tier 3 `{tid}` (`{fn_name}`) double-counts a single shared RPC for both `{fa}` and `{fb}`: {sorted(hits_a | hits_b)}"
                )

    for fid in EXPECTED_FEATURE_IDS:
        if pair_degree[fid] < 2:
            record(f"Tier 3 pairwise coverage insufficient for {fid}: appears in {pair_degree[fid]} pairs (required >= 2)")

    # 7. Tier 4 scenarios (>=17, valid unique F01..F34, >=3 features, 100% body alignment & bipartite distinctness)
    t4_list = matrix.get("tier4_scenarios", [])
    if len(t4_list) < 17:
        record(f"Tier 4 scenario count is {len(t4_list)} (required >= 17)")
    t4_count = 0
    tier4_covered_features: set[str] = set()
    for entry in t4_list:
        sid = str(entry.get("id", "UNKNOWN"))
        f_list = entry.get("features", [])
        fn_name = str(entry.get("fn", "")).strip()
        valid_entry = register_matrix_entry("tier4", f"Tier 4 `{sid}`", entry)
        if valid_entry:
            t4_count += 1

        invalid_fids = [fid for fid in f_list if fid not in VALID_FEATURE_SET]
        if invalid_fids:
            record(f"Tier 4 scenario `{sid}` contains invalid feature IDs: {invalid_fids}")
            continue
        if len(f_list) != len(set(f_list)):
            record(f"Tier 4 scenario `{sid}` contains duplicate feature IDs: {f_list}")
            continue
        if len(f_list) < 3:
            record(f"Tier 4 scenario `{sid}` covers < 3 distinct features: {f_list}")
            continue

        tier4_covered_features.update(f_list)

        if fn_name in all_tier_fns:
            rpcs = set(all_tier_fns[fn_name]["dispatched_rpcs"])
            hits_by_fid: dict[str, set[str]] = {
                fid: {r for r in rpcs if rpc_matches_feature(r, fid)} for fid in f_list
            }
            missing_fids = [fid for fid in f_list if not hits_by_fid[fid]]
            if missing_fids:
                record(
                    f"Tier 4 scenario `{sid}` (`{fn_name}`) declares features {missing_fids} with 0 matching RPC dispatches (dispatched: {sorted(rpcs)})"
                )
            else:
                distinct_matched = max_bipartite_match(f_list, hits_by_fid)
                if distinct_matched < 3 or distinct_matched < len(f_list):
                    record(
                        f"Tier 4 scenario `{sid}` (`{fn_name}`) only matches {distinct_matched}/{len(f_list)} declared features with distinct RPCs"
                    )

    uncovered_t4 = sorted(VALID_FEATURE_SET - tier4_covered_features)
    if uncovered_t4:
        record(f"Tier 4 scenarios collectively omit required features: {uncovered_t4}")

    # 8. Total executable Tier 1-4 tests >= 391 and exact 1:1 bijection
    total_tier_tests = len(all_tier_fns)
    mapped_total = t1_count + t2_count + t3_count + t4_count
    if total_tier_tests < 391:
        record(f"Total Tier 1-4 executable Rust tests is {total_tier_tests} (required >= 391)")
    if len(seen_fns) != mapped_total or mapped_total != total_tier_tests:
        record(
            f"Matrix-to-Rust 1:1 bijection violated: seen_fns={len(seen_fns)}, mapped_sum={mapped_total}, actual_tier_fns={total_tier_tests}"
        )
    unmapped = sorted(set(all_tier_fns.keys()) - set(seen_fns.keys()))
    if unmapped:
        record(f"Unmapped Rust tests not listed in feature_matrix.json: {unmapped[:10]}")

    # 9. Substantive interior body, connect_production(), and strict call-syntax assertions
    for fn_name, meta in sorted(all_tier_fns.items()):
        if not meta["calls_connect_production"]:
            record(f"Tier test `{fn_name}` does not call connect_production()")
        if not meta["has_assertion"]:
            record(f"Tier test `{fn_name}` lacks substantive assertion")
        if meta["substantive_lines"] < 3:
            record(f"Tier test `{fn_name}` has only {meta['substantive_lines']} substantive lines (required >= 3)")

    # 10. Forbidden anti-patterns, local workstation home paths, and internal Cargo path deps
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
            _, sp_code_only = lex_rust_source(txt)
            for label, regex in FORBIDDEN_CODE_REGEXES:
                if regex.search(sp_code_only):
                    record(f"FORBIDDEN pattern `{label}` detected in {rel}")
        if sp.name == "Cargo.toml":
            for bad_dep in ("source/host", "source/desktop_core", "../host", "../../source"):
                if bad_dep in txt:
                    record(f"FORBIDDEN internal Cargo path dependency `{bad_dep}` in {rel}")

    # 11. Zero .gitignore-trapped filenames or target/ under tests/e2e/
    e2e_root = repo_root / "tests/e2e"
    for p in sorted(e2e_root.rglob("*")):
        rel = p.relative_to(e2e_root)
        if "target" in rel.parts:
            record(f"FORBIDDEN target/ directory inside tests/e2e/: {rel.as_posix()}")
        if p.is_file():
            for pat in GITIGNORE_TRAPPED_PATTERNS:
                if fnmatch.fnmatch(p.name, pat):
                    record(f"FORBIDDEN .gitignore-trapped filename under tests/e2e/: {rel.as_posix()}")

    # 12. Verify legacy tests/e2e/master_e2e_suite.swift preserved intact (required by project.yml:146)
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
            "c_abi_symbols_verified": len(EXPECTED_C_ABI_SYMBOLS),
            "rpc_namespaces_covered": sum(1 for v in rpc_counts.values() if v > 0),
            "rpc_namespaces_required": len(EXPECTED_RPC_NAMESPACES),
            "ui_namespaces_required": len(EXPECTED_UI_NAMESPACES),
            "tier1_tests": t1_count,
            "tier2_tests": t2_count,
            "tier3_pairwise_tests": t3_count,
            "tier4_scenarios": t4_count,
            "tier4_distinct_features_covered": len(tier4_covered_features),
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

    print(json.dumps(report.get("counts", {}), indent=2, sort_keys=True))
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
