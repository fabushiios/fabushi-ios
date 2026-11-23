#!/usr/bin/env python3
"""Strict opaque-box E2E coverage and anti-facade validator for Fabushi iOS v1.6.8.

Standard-library only. Designed to execute exclusively inside GitHub Actions CI.
"""

from __future__ import annotations

import argparse
import fnmatch
import io
import json
import re
import sys
import tokenize
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

EXPECTED_SCHEMA_VERSION_V168 = "fabushi.ios.e2e.feature_matrix.v168"
EXPECTED_UPSTREAM_REPO = "bhrumom/gpui-telegram"
EXPECTED_UPSTREAM_BRANCH = "main"
EXPECTED_UPSTREAM_COMMIT_V168 = "424aa33cf67a11a778da2dd3b8cca7ea35072b07"
EXPECTED_UPSTREAM_TREE_V168 = "181a98ba298a49e64ae0ce5b6ac0160616134708"
EXPECTED_UPSTREAM_RELEASE_V168 = "Fabushi v1.6.8"
EXPECTED_UPSTREAM_TRACKED_BLOBS = 11351
EXPECTED_UPSTREAM_CRATES_BLOBS = 3063
EXPECTED_UPSTREAM_INHERITED_BLOBS = 7713
EXPECTED_UPSTREAM_GITLINKS = 0
EXPECTED_IOS_MARKETING_VERSION = "1.6.8"
EXPECTED_IOS_BUILD_NUMBER = 35

EXPECTED_SWIFT_TIER_CLASSES: dict[str, tuple[str, int]] = {
    "tier1": ("FabushiV167Tier1UITests", 34),
    "tier2": ("FabushiV167Tier2UITests", 16),
    "tier3": ("FabushiV167Tier3UITests", 10),
    "tier4": ("FabushiV167Tier4UITests", 8),
}
SWIFT_CLASS_TO_TIER: dict[str, str] = {
    cls_name: tier_key for tier_key, (cls_name, _) in EXPECTED_SWIFT_TIER_CLASSES.items()
}
MIN_SWIFT_TOTAL_TESTS = 68
ALLOWED_SWIFT_BASE_CLASSES = {"FabushiE2EUITestCase", "FabushiV167BaseUITestCase", "XCTestCase"}

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

SWIFT_TYPE_DECL_RE = re.compile(r"\b(class|extension)\s+([A-Za-z_][A-Za-z0-9_]*)\b")
SWIFT_ENUM_DECL_RE = re.compile(r"\benum\s+([A-Za-z_][A-Za-z0-9_]*)\b")
SWIFT_FUNC_DECL_RE = re.compile(r"\bfunc\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(")
SWIFT_DECL_MODIFIERS: frozenset[str] = frozenset(
    {
        "private",
        "fileprivate",
        "internal",
        "package",
        "public",
        "open",
        "final",
        "static",
        "class",
        "override",
        "required",
        "convenience",
        "dynamic",
        "indirect",
        "lazy",
        "mutating",
        "nonmutating",
        "optional",
        "nonisolated",
        "isolated",
        "distributed",
        "async",
        "actor",
        "consuming",
        "borrowing",
        "sending",
        "prefix",
        "postfix",
        "infix",
    }
)
SWIFT_FORBIDDEN_TEST_MODIFIERS: frozenset[str] = frozenset(
    {"private", "fileprivate", "static", "class"}
)
SWIFT_FORBIDDEN_TYPE_MODIFIERS: frozenset[str] = frozenset({"private", "fileprivate"})
SWIFT_ASSERTION_CALL_RE = re.compile(
    r"\b(?:"
    r"XCTAssert(?:True|False|Equal|NotEqual|Nil|NotNil|GreaterThan|GreaterThanOrEqual|LessThan|LessThanOrEqual|ThrowsError|NoThrow)?|"
    r"XCTFail|"
    r"assert[A-Z][A-Za-z0-9_]*|"
    r"verify[A-Z][A-Za-z0-9_]*|"
    r"require[A-Z][A-Za-z0-9_]*"
    r")\s*\("
)
# Strict non-alphanumeric boundaries on BOTH sides: prevents `#F01ABC` / `F12px` false matches.
SWIFT_FEATURE_TAG_RE = re.compile(
    r"(?<![A-Za-z0-9])(F(?:0[1-9]|[12][0-9]|3[0-4]))(?![A-Za-z0-9])"
)
SWIFT_ACCESSIBILITY_REF_RE = re.compile(
    r"\bFabushiAccessibilityIDs\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)\b"
)
SWIFT_STATIC_LET_STR_RE = re.compile(
    r'\bstatic\s+let\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?::\s*String\s*)?=\s*"([^"\n]+)"'
)
SWIFT_UI_NS_LITERAL_RE = re.compile(
    r"(?<![A-Za-z0-9_])("
    + "|".join(re.escape(ns) for ns in EXPECTED_UI_NAMESPACES)
    + r")\.[A-Za-z0-9_.]+"
)
SWIFT_SILENT_SKIP_WAIT_RE = re.compile(r"\bif\s+[^{}\n]*\bwaitForExistence\s*\(")

SWIFT_FORBIDDEN_CODE_REGEXES: list[tuple[str, re.Pattern[str]]] = [
    ("conditional_compilation_if", re.compile("#" + r"\s*(?:if|elseif)\b")),
    ("testable_import", re.compile(r"@test" + r"able\s+import\b")),
    (
        "import_fabushi",
        re.compile(
            r"\bimport\s+(?:(?:typealias|struct|class|enum|protocol|let|var|func|actor)\s+)?Fabu"
            + r"shi\b"
        ),
    ),
    ("xctskip", re.compile(r"\b" + "XCT" + r"Skip(?:If|Unless)?\b")),
    (
        "tautological_xctassert_true",
        re.compile(
            r"\bXCTAssert(?:True)?\s*\(\s*\(*\s*"
            r"(?:true|!\s*\(*\s*false\s*\)*|(\d+(?:\.\d+)?|true|false)\s*==\s*\1)"
            r"\s*\)*\s*(?:,|\))"
        ),
    ),
    (
        "tautological_xctassert_false",
        re.compile(
            r"\bXCTAssertFalse\s*\(\s*\(*\s*"
            r"(?:false|!\s*\(*\s*true\s*\)*|(\d+(?:\.\d+)?|true|false)\s*!=\s*\1)"
            r"\s*\)*\s*(?:,|\))"
        ),
    ),
    (
        "tautological_xctassert_nil",
        re.compile(r"\bXCTAssertNil\s*\(\s*\(*\s*nil\s*\)*\s*(?:,|\))"),
    ),
    (
        "tautological_xctassert_notnil_literal",
        re.compile(
            r"\bXCTAssertNotNil\s*\(\s*\(*\s*(?:true|false|\d+(?:\.\d+)?)\s*\)*\s*(?:,|\))"
        ),
    ),
    (
        "sync_setup_teardown_with_error",
        re.compile(r"\bfunc\s+(?:setUpWithError|tearDownWithError)\s*\("),
    ),
    (
        "sync_override_setup_teardown",
        re.compile(
            r"\boverride\b(?:\s+(?:public|open|internal|package|fileprivate|private|class|static|final|dynamic|nonisolated))*"
            r"\s+func\s+(?:setUp|tearDown)\s*\(\s*\)"
            r"(?:\s+throws)?(?:\s*->\s*(?:Void|\(\s*\)))?\s*\{"
        ),
    ),
    (
        "nspredicate_block_or_closure",
        re.compile(r"\bNSPredicate(?:\s*\.\s*init)?\s*(?:\(\s*block\s*:|\{)"),
    ),
    (
        "runloop_current_run",
        re.compile(
            r"\bRunLoop\s*\.\s*(?:current|main)\s*\.\s*run\b|\bCFRunLoopRun(?:InMode)?\b"
        ),
    ),
    (
        "xctwaiter_sync_wait",
        re.compile(r"\bXCTWaiter(?:\s*\.\s*init)?\s*(?:\([^)]*\))?\s*\.\s*wait\s*\("),
    ),
]

SWIFT_ASSERT_CALL_SITE_RE = re.compile(
    r"\b(XCTAssert(?:True)?|XCTAssertEqual|XCTAssertNotNil|XCTAssertGreaterThanOrEqual|XCTAssertLessThanOrEqual)\s*\("
)
SWIFT_STRING_LIT_RE = re.compile(r'^(?:"(?:\\.|[^"\\\n])*"|(#+)"[^"\n]*"\1)$')
SWIFT_SIMPLE_LITERAL_OR_PATH_RE = re.compile(
    r"^(?:true|false|nil|-?\d+(?:\.\d+)?|[A-Za-z_][A-Za-z0-9_.]*)$"
)
SWIFT_STRING_SELF_EQ_RE = re.compile(
    r'^("(?:\\.|[^"\\\n])*"|(#+)"[^"\n]*"\2)\s*==\s*\1$'
)
SWIFT_ZERO_LIT_RE = re.compile(r"^0+(?:\.0+)?$")


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


def lex_swift_source(source: str) -> tuple[str, str, list[str]]:
    """Stack-based stateful Swift 6 lexer returning (no_comments, code_only, string_literals).

    Preserves exact 1:1 character offsets and newlines. Handles `//`, nested `/* /* */ */`,
    single/multiline strings, raw `#..."..."#` strings, and nested `\\(...)` / `\\#(...)` interpolations.
    """
    n = len(source)
    no_comments: list[str] = [" "] * n
    code_only: list[str] = [" "] * n
    string_literals: list[str] = []
    # Stack frames: ["str", hashes, multiline, list[str]] or ["interp", paren_depth]
    stack: list[list[Any]] = []
    i = 0

    while i < n:
        # Inside a string literal
        if stack and stack[-1][0] == "str":
            frame = stack[-1]
            hashes: int = frame[1]
            multiline: bool = frame[2]
            lit_buf: list[str] = frame[3]
            esc_prefix = "\\" + ("#" * hashes)
            closing = ('"""' if multiline else '"') + ("#" * hashes)

            if source.startswith(esc_prefix, i):
                after = i + len(esc_prefix)
                if after < n and source[after] == "(":
                    for p in range(i, after + 1):
                        no_comments[p] = source[p]
                    code_only[after] = "("
                    stack.append(["interp", 1])
                    i = after + 1
                    continue
                stop = min(n, after + 1)
                for p in range(i, stop):
                    ch_esc = source[p]
                    no_comments[p] = ch_esc
                    lit_buf.append(ch_esc)
                    if ch_esc == "\n":
                        code_only[p] = "\n"
                i = stop
                continue

            if source.startswith(closing, i):
                stop = i + len(closing)
                for p in range(i, stop):
                    no_comments[p] = source[p]
                string_literals.append("".join(lit_buf))
                stack.pop()
                i = stop
                continue

            ch = source[i]
            no_comments[i] = ch
            if ch == "\n":
                code_only[i] = "\n"
                if not multiline:
                    string_literals.append("".join(lit_buf))
                    stack.pop()
                else:
                    lit_buf.append(ch)
            else:
                lit_buf.append(ch)
            i += 1
            continue

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

        # 2. Nested Swift block comment: /* ... /* ... */ ... */
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

        # 3. String literal opening: "...", """...""", #"..."#, #"""..."""#, etc.
        if ch == "#" or ch == '"':
            j = i
            hashes = 0
            while j < n and source[j] == "#":
                hashes += 1
                j += 1
            if j < n and source[j] == '"':
                multiline = False
                if source[j : j + 3] == '"""':
                    k = j + 3
                    while k < n and source[k] in (" ", "\t"):
                        k += 1
                    if k < n and source[k] in ("\n", "\r"):
                        multiline = True
                open_len = 3 if multiline else 1
                stop = j + open_len
                for p in range(i, stop):
                    no_comments[p] = source[p]
                stack.append(["str", hashes, multiline, []])
                i = stop
                continue

        # 4. Track interpolation paren balance
        if stack and stack[-1][0] == "interp":
            if ch == "(":
                stack[-1][1] += 1
            elif ch == ")":
                stack[-1][1] -= 1
                no_comments[i] = ch
                code_only[i] = ch
                if stack[-1][1] == 0:
                    stack.pop()
                i += 1
                continue

        no_comments[i] = ch
        code_only[i] = ch
        i += 1

    return "".join(no_comments), "".join(code_only), string_literals


def _match_balanced_backward(
    text: str, close_idx: int, bound: int, open_ch: str, close_ch: str
) -> int:
    depth = 0
    idx = close_idx
    while idx >= bound:
        ch = text[idx]
        if ch == close_ch:
            depth += 1
        elif ch == open_ch:
            depth -= 1
            if depth == 0:
                return idx
        idx -= 1
    return -1


def _extract_swift_decl_modifiers(code_only: str, decl_idx: int, bound: int) -> list[str]:
    """Walk backward across newlines from `func`/`class`/`extension` collecting attached modifiers."""
    modifiers: list[str] = []
    pos = decl_idx
    while pos > bound:
        while pos > bound and code_only[pos - 1].isspace():
            pos -= 1
        if pos <= bound:
            break
        ch = code_only[pos - 1]
        if ch == ")":
            open_p = _match_balanced_backward(code_only, pos - 1, bound, "(", ")")
            if open_p == -1:
                break
            cur = open_p
            while cur > bound and code_only[cur - 1].isspace():
                cur -= 1
            ident_end = cur
            while cur > bound and (
                code_only[cur - 1].isalnum() or code_only[cur - 1] in ("_", ".")
            ):
                cur -= 1
            ident = code_only[cur:ident_end]
            if cur > bound and code_only[cur - 1] == "@":
                pos = cur - 1
                continue
            if ident in SWIFT_DECL_MODIFIERS and (cur == bound or code_only[cur - 1] != "."):
                modifiers.append(ident)
                pos = cur
                continue
            break
        if ch.isalnum() or ch == "_":
            ident_end = pos
            cur = pos
            while cur > bound and (code_only[cur - 1].isalnum() or code_only[cur - 1] == "_"):
                cur -= 1
            tok = code_only[cur:ident_end]
            attr_cur = cur
            while attr_cur > bound and (
                code_only[attr_cur - 1].isalnum() or code_only[attr_cur - 1] in ("_", ".")
            ):
                attr_cur -= 1
            if attr_cur > bound and code_only[attr_cur - 1] == "@":
                pos = attr_cur - 1
                continue
            if cur > bound and code_only[cur - 1] == ".":
                break
            if tok in SWIFT_DECL_MODIFIERS:
                modifiers.append(tok)
                pos = cur
                continue
            break
        break
    modifiers.reverse()
    return modifiers


def _split_top_level_arg_spans(
    code_only: str, inner_start: int, inner_end: int
) -> list[tuple[int, int]]:
    spans: list[tuple[int, int]] = []
    depth = 0
    arg_start = inner_start
    for idx in range(inner_start, inner_end):
        ch = code_only[idx]
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth = max(0, depth - 1)
        elif ch == "," and depth == 0:
            spans.append((arg_start, idx))
            arg_start = idx + 1
    spans.append((arg_start, inner_end))
    return spans


def _strip_balanced_outer_parens(expr: str) -> str:
    s = expr.strip()
    while len(s) >= 2 and s[0] == "(" and _match_balanced(s, 0, "(", ")") == len(s) - 1:
        s = s[1:-1].strip()
    return s


def _find_swift_tautological_or_vacuous_asserts(
    no_comments: str, code_only: str
) -> list[tuple[str, int]]:
    """Inspect Swift XCTAssert calls using balanced arg spans on both code_only and no_comments."""
    hits: list[tuple[str, int]] = []
    for m in SWIFT_ASSERT_CALL_SITE_RE.finditer(code_only):
        call_name = m.group(1)
        paren_open = m.end() - 1
        paren_close = _match_balanced(code_only, paren_open, "(", ")")
        if paren_close == -1:
            continue
        spans = _split_top_level_arg_spans(code_only, paren_open + 1, paren_close)
        args_code = [_strip_balanced_outer_parens(code_only[s:e]) for s, e in spans]
        args_nc = [_strip_balanced_outer_parens(no_comments[s:e]) for s, e in spans]
        line_no = code_only.count("\n", 0, m.start()) + 1

        if call_name == "XCTAssertEqual" and len(args_nc) >= 2:
            lhs, rhs = args_nc[0], args_nc[1]
            if lhs and lhs == rhs and (
                SWIFT_STRING_LIT_RE.match(lhs)
                or (SWIFT_SIMPLE_LITERAL_OR_PATH_RE.match(lhs) and "(" not in args_code[0])
            ):
                hits.append(("tautological_xctassert_equal", line_no))
        elif call_name in ("XCTAssert", "XCTAssertTrue") and len(args_nc) >= 1:
            if SWIFT_STRING_SELF_EQ_RE.match(args_nc[0]):
                hits.append(("tautological_xctassert_true_string_eq", line_no))
        elif call_name == "XCTAssertNotNil" and len(args_nc) >= 1:
            if SWIFT_STRING_LIT_RE.match(args_nc[0]):
                hits.append(("tautological_xctassert_notnil_string", line_no))
        elif call_name == "XCTAssertGreaterThanOrEqual" and len(args_code) >= 2:
            if SWIFT_ZERO_LIT_RE.match(args_code[1]):
                hits.append(("vacuous_xctassert_ge_zero", line_no))
        elif call_name == "XCTAssertLessThanOrEqual" and len(args_code) >= 2:
            if SWIFT_ZERO_LIT_RE.match(args_code[0]):
                hits.append(("vacuous_xctassert_zero_le", line_no))
    return hits


def _extract_accessibility_id_symbol_map(
    lexed_files: list[tuple[Path, str, str, str, str, list[str]]]
) -> dict[tuple[str, str], str]:
    """Map (NestedEnumName, memberName) -> canonical `fabushi.<ns>` strictly for real element IDs."""
    symbol_to_ns: dict[tuple[str, str], str] = {}
    for _, _, _, no_comments, code_only, _ in lexed_files:
        m_root = re.search(r"\benum\s+FabushiAccessibilityIDs\b", code_only)
        if not m_root:
            continue
        root_open = code_only.find("{", m_root.end())
        if root_open == -1:
            continue
        root_close = _match_balanced(code_only, root_open, "{", "}")
        if root_close == -1:
            continue
        j = root_open + 1
        depth = 1
        while j < root_close:
            ch = code_only[j]
            if ch == "{":
                depth += 1
                j += 1
                continue
            if ch == "}":
                depth -= 1
                j += 1
                continue
            if depth == 1:
                m_enum = SWIFT_ENUM_DECL_RE.match(code_only, j)
                if m_enum and (
                    j == root_open + 1 or not (code_only[j - 1].isalnum() or code_only[j - 1] == "_")
                ):
                    enum_name = m_enum.group(1)
                    e_open = code_only.find("{", m_enum.end(), root_close)
                    if e_open == -1:
                        j = m_enum.end()
                        continue
                    e_close = _match_balanced(code_only, e_open, "{", "}")
                    if e_close == -1 or e_close >= root_close:
                        break
                    enum_nc = no_comments[e_open + 1 : e_close]
                    for m_let in SWIFT_STATIC_LET_STR_RE.finditer(enum_nc):
                        member_name = m_let.group(1)
                        val = m_let.group(2)
                        for ns in EXPECTED_UI_NAMESPACES:
                            if val.startswith(f"{ns}."):
                                symbol_to_ns[(enum_name, member_name)] = ns
                                break
                    j = e_close + 1
                    continue
            j += 1
    return symbol_to_ns


def _extract_swift_ui_ns_counts(
    body_code: str, body_raw: str, id_symbol_to_ns: dict[tuple[str, str], str]
) -> Counter[str]:
    """Count UI namespace references inside a Swift method body (resolved symbolic IDs + literals)."""
    counts: Counter[str] = Counter()
    for m_ref in SWIFT_ACCESSIBILITY_REF_RE.finditer(body_code):
        ns = id_symbol_to_ns.get((m_ref.group(1), m_ref.group(2)))
        if ns:
            counts[ns] += 1
    _, _, body_literals = lex_swift_source(body_raw)
    for lit in body_literals:
        for m_lit in SWIFT_UI_NS_LITERAL_RE.finditer(lit):
            counts[m_lit.group(1)] += 1
    return counts


def _strip_python_comments(source: str) -> str:
    """Return Python source with `#` comment tokens stripped via stdlib tokenize."""
    tokens: list[str] = []
    for tok in tokenize.generate_tokens(io.StringIO(source).readline):
        if tok.type == tokenize.COMMENT:
            continue
        tokens.append(tok.string)
    return " ".join(tokens)


def parse_swift_v167_files(
    v167_files: list[Path], repo_root: Path
) -> tuple[dict[str, dict[str, dict[str, Any]]], dict[str, int], dict[str, int], list[str]]:
    """Parse V167 Swift 6 XCUITest classes, extensions, helpers, and top-level `func test*` methods.

    Returns (tier_methods_by_class, class_decl_counts, tier_ui_namespace_hits, parse_errors).
    """
    tier_methods: dict[str, dict[str, dict[str, Any]]] = {
        cls_name: {} for cls_name in SWIFT_CLASS_TO_TIER
    }
    class_decl_counts: dict[str, int] = {cls_name: 0 for cls_name in SWIFT_CLASS_TO_TIER}
    tier_ui_namespace_hits: dict[str, int] = {ns: 0 for ns in EXPECTED_UI_NAMESPACES}
    parse_errors: list[str] = []

    # Stage 1: Lex every V167 Swift file & run file-wide anti-pattern / tautology scans
    lexed_files: list[tuple[Path, str, str, str, str, list[str]]] = []
    for sf in v167_files:
        rel = sf.relative_to(repo_root).as_posix()
        raw_text = sf.read_text(encoding="utf-8")
        no_comments, code_only, literals = lex_swift_source(raw_text)
        lexed_files.append((sf, rel, raw_text, no_comments, code_only, literals))

        for label, regex in SWIFT_FORBIDDEN_CODE_REGEXES:
            if regex.search(code_only):
                parse_errors.append(f"FORBIDDEN Swift 6 pattern `{label}` detected in {rel}")
        for label, line_no in _find_swift_tautological_or_vacuous_asserts(no_comments, code_only):
            parse_errors.append(f"FORBIDDEN Swift assertion `{label}` detected in {rel}:{line_no}")

    # Stage 2: Resolve FabushiAccessibilityIDs symbol map & non-test helper namespace closure
    id_symbol_to_ns = _extract_accessibility_id_symbol_map(lexed_files)
    defined_id_nss = set(id_symbol_to_ns.values())
    missing_catalog_nss = sorted(set(EXPECTED_UI_NAMESPACES) - defined_id_nss)
    if missing_catalog_nss:
        parse_errors.append(
            f"FabushiAccessibilityIDs is missing real element IDs for namespaces: {missing_catalog_nss}"
        )

    base_helper_ns: dict[str, set[str]] = {}
    base_helper_bodies: dict[str, str] = {}
    tier_helper_ns: dict[str, dict[str, set[str]]] = {cls: {} for cls in SWIFT_CLASS_TO_TIER}
    tier_helper_bodies: dict[str, dict[str, str]] = {cls: {} for cls in SWIFT_CLASS_TO_TIER}

    def _collect_methods_in_body(
        raw_text: str, no_comments: str, code_only: str, open_brace: int, close_brace: int
    ) -> list[tuple[str, int, int, int, int, int]]:
        found: list[tuple[str, int, int, int, int, int]] = []
        j = open_brace + 1
        inner_depth = 1
        while j < close_brace:
            ch_in = code_only[j]
            if ch_in == "{":
                inner_depth += 1
                j += 1
                continue
            if ch_in == "}":
                inner_depth -= 1
                j += 1
                continue
            if inner_depth == 1:
                m_fn = SWIFT_FUNC_DECL_RE.match(code_only, j)
                if m_fn and (
                    j == open_brace + 1
                    or not (code_only[j - 1].isalnum() or code_only[j - 1] == "_")
                ):
                    fn_name = m_fn.group(1)
                    paren_open = m_fn.end() - 1
                    paren_close = _match_balanced(code_only, paren_open, "(", ")")
                    if paren_close == -1:
                        j = m_fn.end()
                        continue
                    body_open = code_only.find("{", paren_close + 1, close_brace)
                    if body_open == -1:
                        j = paren_close + 1
                        continue
                    body_close = _match_balanced(code_only, body_open, "{", "}")
                    if body_close == -1 or body_close >= close_brace:
                        break
                    found.append((fn_name, j, paren_open, paren_close, body_open, body_close))
                    j = body_close + 1
                    continue
            j += 1
        return found

    type_blocks: list[tuple[str, str, str, str, str, str, str, int, int, int]] = []
    for _, rel, raw_text, no_comments, code_only, _ in lexed_files:
        n = len(code_only)
        i = 0
        brace_depth = 0
        while i < n:
            c = code_only[i]
            if c == "{":
                brace_depth += 1
                i += 1
                continue
            if c == "}":
                brace_depth = max(0, brace_depth - 1)
                i += 1
                continue
            if brace_depth == 0:
                m_type = SWIFT_TYPE_DECL_RE.match(code_only, i)
                if m_type and (i == 0 or not (code_only[i - 1].isalnum() or code_only[i - 1] == "_")):
                    kind = m_type.group(1)
                    type_name = m_type.group(2)
                    open_brace = code_only.find("{", m_type.end())
                    if open_brace == -1:
                        i = m_type.end()
                        continue
                    close_brace = _match_balanced(code_only, open_brace, "{", "}")
                    if close_brace == -1:
                        parse_errors.append(f"unclosed `{kind} {type_name}` body in {rel}")
                        break
                    type_blocks.append(
                        (rel, raw_text, no_comments, code_only, kind, type_name, code_only[m_type.end() : open_brace], i, open_brace, close_brace)
                    )
                    if type_name in {"FabushiE2EUITestCase", "FabushiV167BaseUITestCase"} or type_name in SWIFT_CLASS_TO_TIER:
                        for fn_name, _, _, _, body_open, body_close in _collect_methods_in_body(
                            raw_text, no_comments, code_only, open_brace, close_brace
                        ):
                            if not fn_name.startswith("test"):
                                b_code = code_only[body_open + 1 : body_close]
                                b_raw = raw_text[body_open + 1 : body_close]
                                nss = set(_extract_swift_ui_ns_counts(b_code, b_raw, id_symbol_to_ns).keys())
                                if type_name in SWIFT_CLASS_TO_TIER:
                                    tier_helper_ns[type_name].setdefault(fn_name, set()).update(nss)
                                    tier_helper_bodies[type_name][fn_name] = (
                                        tier_helper_bodies[type_name].get(fn_name, "") + "\n" + b_code
                                    )
                                else:
                                    base_helper_ns.setdefault(fn_name, set()).update(nss)
                                    base_helper_bodies[fn_name] = (
                                        base_helper_bodies.get(fn_name, "") + "\n" + b_code
                                    )
                    i = close_brace + 1
                    continue
            i += 1

    def _propagate_helpers(ns_map: dict[str, set[str]], body_map: dict[str, str]) -> None:
        max_iters = len(ns_map) + 1
        for _ in range(max_iters):
            changed = False
            for caller, body_c in body_map.items():
                caller_set = ns_map.setdefault(caller, set())
                for callee, callee_nss in ns_map.items():
                    if callee == caller or not callee_nss:
                        continue
                    if not callee_nss.issubset(caller_set) and re.search(
                        rf"\b{re.escape(callee)}\s*\(", body_c
                    ):
                        caller_set.update(callee_nss)
                        changed = True
            if not changed:
                break

    _propagate_helpers(base_helper_ns, base_helper_bodies)
    combined_helpers_by_tier: dict[str, dict[str, set[str]]] = {}
    for cls_name in SWIFT_CLASS_TO_TIER:
        merged_ns = {k: set(v) for k, v in base_helper_ns.items()}
        merged_bodies = dict(base_helper_bodies)
        for k, v in tier_helper_ns[cls_name].items():
            merged_ns.setdefault(k, set()).update(v)
        for k, b in tier_helper_bodies[cls_name].items():
            merged_bodies[k] = merged_bodies.get(k, "") + "\n" + b
        _propagate_helpers(merged_ns, merged_bodies)
        combined_helpers_by_tier[cls_name] = merged_ns

    # Stage 3: Extract classes, extensions, and top-level `func test*()` methods
    for rel, raw_text, no_comments, code_only, kind, type_name, header_tail, type_idx, open_brace, close_brace in type_blocks:
        if kind == "class" and type_name in class_decl_counts:
            class_decl_counts[type_name] += 1
            sup_m = re.search(r":\s*([A-Za-z_][A-Za-z0-9_]*)", header_tail)
            if not sup_m or sup_m.group(1) not in ALLOWED_SWIFT_BASE_CLASSES:
                parse_errors.append(
                    f"Swift tier class `{type_name}` in {rel} must inherit from one of {sorted(ALLOWED_SWIFT_BASE_CLASSES)}"
                )

        track_tier_class = type_name in SWIFT_CLASS_TO_TIER
        forbid_tests_in_base = type_name in {"FabushiE2EUITestCase", "FabushiV167BaseUITestCase"}
        if not (track_tier_class or forbid_tests_in_base):
            continue

        type_line = code_only.count("\n", 0, type_idx) + 1
        type_mods = set(_extract_swift_decl_modifiers(code_only, type_idx, 0))
        bad_type_mods = sorted(type_mods & SWIFT_FORBIDDEN_TYPE_MODIFIERS)
        if bad_type_mods:
            parse_errors.append(
                f"FORBIDDEN visibility modifier {bad_type_mods} on `{kind} {type_name}` ({rel}:{type_line}); XCTest cannot discover private/fileprivate classes or extensions"
            )

        for fn_name, j, paren_open, paren_close, body_open, body_close in _collect_methods_in_body(
            raw_text, no_comments, code_only, open_brace, close_brace
        ):
            if not fn_name.startswith("test"):
                continue
            start_line = code_only.count("\n", 0, j) + 1
            if forbid_tests_in_base:
                parse_errors.append(
                    f"FORBIDDEN `func {fn_name}` inside base class `{type_name}` ({rel}:{start_line}); base class must declare 0 test methods"
                )
                continue

            fn_mods = set(_extract_swift_decl_modifiers(code_only, j, open_brace + 1))
            bad_fn_mods = sorted(fn_mods & SWIFT_FORBIDDEN_TEST_MODIFIERS)
            if bad_fn_mods:
                parse_errors.append(
                    f"FORBIDDEN visibility/static modifier {bad_fn_mods} on XCTest method `{type_name}.{fn_name}` ({rel}:{start_line})"
                )
            params_inner = code_only[paren_open + 1 : paren_close].strip()
            if params_inner:
                parse_errors.append(
                    f"XCTest method `{type_name}.{fn_name}` ({rel}:{start_line}) must take empty `()` parameter list"
                )
            sig_tail = code_only[paren_close + 1 : body_open]
            if "->" in sig_tail:
                parse_errors.append(
                    f"XCTest method `{type_name}.{fn_name}` ({rel}:{start_line}) must return Void (no `->`)"
                )

            body_code = code_only[body_open + 1 : body_close]
            body_nc = no_comments[body_open + 1 : body_close]
            body_raw = raw_text[body_open + 1 : body_close]
            sub_lines = sum(1 for ln in body_nc.splitlines() if ln.strip())
            has_assert = bool(SWIFT_ASSERTION_CALL_RE.search(body_code))
            if SWIFT_SILENT_SKIP_WAIT_RE.search(body_code):
                parse_errors.append(
                    f"FORBIDDEN silent-skip `if ...waitForExistence(...)` branch inside `{type_name}.{fn_name}` ({rel}:{start_line})"
                )
            name_features = sorted(set(SWIFT_FEATURE_TAG_RE.findall(fn_name)))
            direct_ui_counts = _extract_swift_ui_ns_counts(body_code, body_raw, id_symbol_to_ns)
            for ns_k, cnt_v in direct_ui_counts.items():
                tier_ui_namespace_hits[ns_k] = tier_ui_namespace_hits.get(ns_k, 0) + cnt_v
            effective_ui_ns = set(direct_ui_counts.keys())
            for callee, callee_nss in combined_helpers_by_tier[type_name].items():
                if callee_nss and re.search(rf"\b{re.escape(callee)}\s*\(", body_code):
                    effective_ui_ns.update(callee_nss)

            if fn_name in tier_methods[type_name]:
                parse_errors.append(
                    f"duplicate Swift XCTest method `{type_name}.{fn_name}` in {rel}:{start_line}"
                )
            tier_methods[type_name][fn_name] = {
                "file": rel,
                "start_line": start_line,
                "substantive_lines": sub_lines,
                "has_assertion": has_assert,
                "features": name_features,
                "direct_ui_ns_counts": dict(direct_ui_counts),
                "direct_ui_namespaces": sorted(direct_ui_counts.keys()),
                "effective_ui_namespaces": sorted(effective_ui_ns),
            }

    return tier_methods, class_decl_counts, tier_ui_namespace_hits, parse_errors


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
        repo_root / "tests/e2e/simulator_loopback_server.py",
        repo_root / ".github/workflows/ios-e2e-suite.yml",
    ]
    rust_ffi_root = repo_root / "tests/e2e/rust_ffi"
    if rust_ffi_root.exists():
        for p in sorted(rust_ffi_root.rglob("*")):
            if p.is_file() and p.suffix in {".rs", ".toml"} and "target" not in p.parts:
                scan_paths.append(p)

    swift_v167_root = repo_root / "mobile/ios/FabushiUITests/V167"
    swift_v167_files: list[Path] = []
    if swift_v167_root.exists():
        swift_v167_files = sorted(swift_v167_root.rglob("*.swift"))
        scan_paths.extend(swift_v167_files)

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

    # 11. Zero .gitignore-trapped filenames, __pycache__/*.pyc, or target/ under tests/e2e/
    e2e_root = repo_root / "tests/e2e"
    for p in sorted(e2e_root.rglob("*")):
        rel = p.relative_to(e2e_root)
        if "target" in rel.parts:
            record(f"FORBIDDEN target/ directory inside tests/e2e/: {rel.as_posix()}")
        if "__pycache__" in rel.parts or p.suffix == ".pyc":
            record(f"FORBIDDEN Python bytecode artifact inside tests/e2e/: {rel.as_posix()}")
        if p.is_file():
            for pat in GITIGNORE_TRAPPED_PATTERNS:
                if fnmatch.fnmatch(p.name, pat):
                    record(f"FORBIDDEN .gitignore-trapped filename under tests/e2e/: {rel.as_posix()}")

    # 12. Verify preserved legacy files (comment-stripped), simulator_loopback_server.py, and live v1.6.8 authority lock
    master_suite = e2e_root / "master_e2e_suite.swift"
    if not master_suite.exists():
        record("tests/e2e/master_e2e_suite.swift is missing (still required by mobile/ios/project.yml:146)")
    else:
        ms_nc, _, _ = lex_swift_source(master_suite.read_text(encoding="utf-8"))
        for req_tok in (
            "public enum MasterE2ERunner",
            "Tier1FeatureCoverageSuite",
            "Tier2BoundaryCornerCasesSuite",
            "Tier3CrossFeatureSuite",
            "Tier4RealWorldScenariosSuite",
        ):
            if req_tok not in ms_nc:
                record(f"tests/e2e/master_e2e_suite.swift missing preserved symbol `{req_tok}`")

    global_dharma_path = repo_root / "mobile/ios/FabushiUITests/GlobalDharmaJourneyUITests.swift"
    if not global_dharma_path.exists():
        record("mobile/ios/FabushiUITests/GlobalDharmaJourneyUITests.swift is missing")
    else:
        gd_nc, _, _ = lex_swift_source(global_dharma_path.read_text(encoding="utf-8"))
        required_host_flag = 'app.launchEnvironment["FABUSHI_FEATURE_HOST_TEST"] = "0"'
        if required_host_flag not in gd_nc:
            record(
                "GlobalDharmaJourneyUITests.swift missing required CI literal `app.launchEnvironment[\"FABUSHI_FEATURE_HOST_TEST\"] = \"0\"`"
            )
        for req_sym in (
            "class GlobalDharmaJourneyUITests",
            "testGlobalDharmaMarketplaceBotWebMcpCommerceJourney",
            "testPrepareGlobalDharmaOfflineAcceptance",
            "testGlobalDharmaOfflineColdStartHostWebMcpAndPersistence",
        ):
            if req_sym not in gd_nc:
                record(f"GlobalDharmaJourneyUITests.swift missing preserved symbol `{req_sym}`")

    legacy_ui_path = repo_root / "mobile/ios/FabushiUITests/FabushiUITests.swift"
    if not legacy_ui_path.exists():
        record("mobile/ios/FabushiUITests/FabushiUITests.swift is missing")
    else:
        legacy_nc, _, _ = lex_swift_source(legacy_ui_path.read_text(encoding="utf-8"))
        if "class FabushiUITests" not in legacy_nc:
            record("mobile/ios/FabushiUITests/FabushiUITests.swift missing `class FabushiUITests`")

    loopback_py = e2e_root / "simulator_loopback_server.py"
    if not loopback_py.exists():
        record("tests/e2e/simulator_loopback_server.py is missing")
    else:
        try:
            lb_nc = _strip_python_comments(loopback_py.read_text(encoding="utf-8"))
        except tokenize.TokenError as exc:
            record(f"tests/e2e/simulator_loopback_server.py failed Python tokenization: {exc}")
            lb_nc = ""
        for req_flag in (
            "--self-test",
            "--url-file",
            "--port-file",
            "/__e2e__/reset",
            "/__e2e__/seed",
            "/__e2e__/fault",
            "/__e2e__/state",
            "/api/v1/ws",
            "Sec-WebSocket-Accept",
        ):
            if req_flag not in lb_nc:
                record(f"tests/e2e/simulator_loopback_server.py missing required flag/route `{req_flag}`")

    # Verify full live Fabushi v1.6.8 authority lock in feature_matrix.json and TEST_INFRA.md
    if matrix.get("schema_version") != EXPECTED_SCHEMA_VERSION_V168:
        record(
            f"feature_matrix.json schema_version must be `{EXPECTED_SCHEMA_VERSION_V168}`, got `{matrix.get('schema_version')}`"
        )
    auth_obj = matrix.get("upstream_authority", {})
    if auth_obj.get("repo") != EXPECTED_UPSTREAM_REPO:
        record(
            f"feature_matrix.json upstream_authority.repo must be `{EXPECTED_UPSTREAM_REPO}`, got `{auth_obj.get('repo')}`"
        )
    if auth_obj.get("branch") != EXPECTED_UPSTREAM_BRANCH:
        record(
            f"feature_matrix.json upstream_authority.branch must be `{EXPECTED_UPSTREAM_BRANCH}`, got `{auth_obj.get('branch')}`"
        )
    if auth_obj.get("commit") != EXPECTED_UPSTREAM_COMMIT_V168:
        record(
            f"feature_matrix.json upstream_authority.commit must be {EXPECTED_UPSTREAM_COMMIT_V168}, got {auth_obj.get('commit')}"
        )
    if auth_obj.get("root_tree") != EXPECTED_UPSTREAM_TREE_V168:
        record(
            f"feature_matrix.json upstream_authority.root_tree must be {EXPECTED_UPSTREAM_TREE_V168}, got {auth_obj.get('root_tree')}"
        )
    if auth_obj.get("release") != EXPECTED_UPSTREAM_RELEASE_V168:
        record(
            f"feature_matrix.json upstream_authority.release must be `{EXPECTED_UPSTREAM_RELEASE_V168}`, got `{auth_obj.get('release')}`"
        )
    for blob_key, expected_val in (
        ("tracked_blobs", EXPECTED_UPSTREAM_TRACKED_BLOBS),
        ("crates_blobs", EXPECTED_UPSTREAM_CRATES_BLOBS),
        ("inherited_blobs", EXPECTED_UPSTREAM_INHERITED_BLOBS),
        ("gitlinks", EXPECTED_UPSTREAM_GITLINKS),
    ):
        actual_val = auth_obj.get(blob_key)
        if type(actual_val) is not int or actual_val != expected_val:
            record(
                f"feature_matrix.json upstream_authority.{blob_key} must be exact int {expected_val}, got {actual_val!r}"
            )
    ios_rel = matrix.get("target_ios_release", {})
    if (
        ios_rel.get("marketing_version") != EXPECTED_IOS_MARKETING_VERSION
        or type(ios_rel.get("build_number")) is not int
        or ios_rel.get("build_number") != EXPECTED_IOS_BUILD_NUMBER
    ):
        record(
            f"feature_matrix.json target_ios_release must be {EXPECTED_IOS_MARKETING_VERSION} / build {EXPECTED_IOS_BUILD_NUMBER}, got {ios_rel}"
        )
    for req_infra_tok in (
        EXPECTED_UPSTREAM_COMMIT_V168,
        EXPECTED_UPSTREAM_TREE_V168,
        EXPECTED_UPSTREAM_RELEASE_V168,
        "simulator_loopback_server.py",
    ):
        if req_infra_tok not in infra_text:
            record(f"TEST_INFRA.md missing v1.6.8 authority / loopback token `{req_infra_tok}`")

    # Verify deleted legacy unreferenced challenger files stay deleted
    for legacy_removed in (
        "challenger_m6_core_adversarial.swift",
        "challenger_m6_iter2_adversarial.swift",
    ):
        if (e2e_root / legacy_removed).exists():
            record(f"Unreferenced legacy file still present: tests/e2e/{legacy_removed}")

    # 13. Swift 6 V167 XCUITest suite validation (4 tier classes, >= 68 tests, 13 UI namespaces in Tier bodies, 34 features, 1:1 matrix bijection)
    swift_ui_namespace_hits: dict[str, int] = {ns: 0 for ns in EXPECTED_UI_NAMESPACES}
    swift_tier_counts: dict[str, int] = {t: 0 for t in EXPECTED_SWIFT_TIER_CLASSES}
    swift_tier_classes: dict[str, int] = {cls: 0 for cls in SWIFT_CLASS_TO_TIER}
    swift_covered_features: set[str] = set()
    swift_total_tests = 0

    if not swift_v167_root.exists() or not swift_v167_files:
        record("mobile/ios/FabushiUITests/V167/ is missing or contains 0 .swift files")
    else:
        parsed_swift, cls_decls, tier_ui_namespace_hits, swift_errs = parse_swift_v167_files(
            swift_v167_files, repo_root
        )
        for se in swift_errs:
            record(se)

        for tier_key, (cls_name, min_count) in EXPECTED_SWIFT_TIER_CLASSES.items():
            if cls_decls.get(cls_name, 0) != 1:
                record(
                    f"Expected exactly 1 declaration of `class {cls_name}:` under mobile/ios/FabushiUITests/V167/, found {cls_decls.get(cls_name, 0)}"
                )
            methods_map = parsed_swift.get(cls_name, {})
            count = len(methods_map)
            swift_tier_counts[tier_key] = count
            swift_tier_classes[cls_name] = count
            swift_total_tests += count
            if count < min_count:
                record(
                    f"Swift {cls_name} has only {count} genuine `func test*` methods (required >= {min_count})"
                )
            for fn_name, meta in sorted(methods_map.items()):
                if meta["substantive_lines"] < 3:
                    record(
                        f"Swift test `{cls_name}.{fn_name}` ({meta['file']}:{meta['start_line']}) has only {meta['substantive_lines']} substantive lines (required >= 3)"
                    )
                if not meta["has_assertion"]:
                    record(
                        f"Swift test `{cls_name}.{fn_name}` ({meta['file']}:{meta['start_line']}) lacks a substantive assertion"
                    )
                if not meta["direct_ui_namespaces"]:
                    record(
                        f"Swift test `{cls_name}.{fn_name}` ({meta['file']}:{meta['start_line']}) references 0 direct `fabushi.*` UI identifiers"
                    )
                swift_covered_features.update(meta["features"])

        if swift_total_tests < MIN_SWIFT_TOTAL_TESTS:
            record(
                f"Total Swift 6 V167 XCUITests is {swift_total_tests} (required >= {MIN_SWIFT_TOTAL_TESTS})"
            )

        for ui_ns in EXPECTED_UI_NAMESPACES:
            hits = tier_ui_namespace_hits.get(ui_ns, 0)
            swift_ui_namespace_hits[ui_ns] = hits
            if hits == 0:
                record(
                    f"Swift V167 Tier 1-4 test bodies have 0 UI identifier references to required namespace `{ui_ns}`"
                )

        uncovered_swift_features = sorted(VALID_FEATURE_SET - swift_covered_features)
        if uncovered_swift_features:
            record(
                f"Swift V167 XCUITests omit required features: {uncovered_swift_features}"
            )

        # Cross-check 1:1 bijection + per-test features/ui_namespaces alignment with feature_matrix.json `swift_uitest_suite`
        swift_matrix = matrix.get("swift_uitest_suite")
        if not isinstance(swift_matrix, dict):
            record("feature_matrix.json missing `swift_uitest_suite` registration object")
        else:
            tiers_obj = swift_matrix.get("tiers", {})
            seen_ui_ids: set[str] = set()
            valid_ui_ns_set = set(EXPECTED_UI_NAMESPACES)
            feature_id_to_ui_ns = {
                f["id"]: set(f.get("ui_namespaces", []))
                for f in features
                if isinstance(f, dict) and "id" in f
            }
            for tier_key, (cls_name, min_count) in EXPECTED_SWIFT_TIER_CLASSES.items():
                t_obj = tiers_obj.get(tier_key)
                if not isinstance(t_obj, dict):
                    record(f"feature_matrix.json swift_uitest_suite.tiers missing `{tier_key}`")
                    continue
                if t_obj.get("class") != cls_name:
                    record(
                        f"swift_uitest_suite.tiers.{tier_key}.class must be `{cls_name}`, got `{t_obj.get('class')}`"
                    )
                declared_tests = t_obj.get("tests", [])
                if len(declared_tests) < min_count:
                    record(
                        f"swift_uitest_suite.tiers.{tier_key}.tests has {len(declared_tests)} entries (required >= {min_count})"
                    )
                declared_methods: set[str] = set()
                actual_methods = set(parsed_swift.get(cls_name, {}).keys())
                for entry in declared_tests:
                    uid = str(entry.get("id", "")).strip()
                    method = str(entry.get("method", "")).strip()
                    feats = entry.get("features", [])
                    nss = entry.get("ui_namespaces", [])
                    if not uid:
                        record(f"swift_uitest_suite {tier_key} entry missing `id`")
                    elif uid in seen_ui_ids:
                        record(f"duplicate swift_uitest_suite test id `{uid}`")
                    else:
                        seen_ui_ids.add(uid)
                    if not method:
                        record(f"swift_uitest_suite `{uid}` missing `method`")
                    elif method in declared_methods:
                        record(f"duplicate swift_uitest_suite method `{cls_name}.{method}`")
                    else:
                        declared_methods.add(method)
                    if not feats or any(f not in VALID_FEATURE_SET for f in feats):
                        record(f"swift_uitest_suite `{uid}` has invalid features: {feats}")
                    elif len(feats) != len(set(feats)):
                        record(f"swift_uitest_suite `{uid}` contains duplicate features: {feats}")
                    if not nss or any(ns not in valid_ui_ns_set for ns in nss):
                        record(f"swift_uitest_suite `{uid}` has invalid ui_namespaces: {nss}")
                    elif len(nss) != len(set(nss)):
                        record(f"swift_uitest_suite `{uid}` contains duplicate ui_namespaces: {nss}")
                    method_meta = parsed_swift.get(cls_name, {}).get(method)
                    if method_meta is not None:
                        actual_feats = set(method_meta["features"])
                        declared_feats = set(feats)
                        if declared_feats != actual_feats:
                            record(
                                f"swift_uitest_suite `{uid}` (`{cls_name}.{method}`) declares features {sorted(declared_feats)}, "
                                f"but Swift test method tags are {sorted(actual_feats)}"
                            )
                        effective_nss = set(method_meta["effective_ui_namespaces"])
                        missing_nss = sorted(set(nss) - effective_nss)
                        if missing_nss:
                            record(
                                f"swift_uitest_suite `{uid}` (`{cls_name}.{method}`) declares ui_namespaces {missing_nss} "
                                f"not exercised in Swift test body (direct={method_meta['direct_ui_namespaces']}, effective={method_meta['effective_ui_namespaces']})"
                            )
                        if tier_key == "tier1":
                            for fid in feats:
                                expected_feat_nss = feature_id_to_ui_ns.get(fid, set())
                                if expected_feat_nss and not (effective_nss & expected_feat_nss):
                                    record(
                                        f"swift_uitest_suite Tier 1 `{uid}` (`{cls_name}.{method}`) covers `{fid}` "
                                        f"but exercises none of its canonical UI namespaces {sorted(expected_feat_nss)} (exercised: {sorted(effective_nss)})"
                                    )
                missing_in_swift = sorted(declared_methods - actual_methods)
                unregistered_in_matrix = sorted(actual_methods - declared_methods)
                if missing_in_swift:
                    record(
                        f"swift_uitest_suite `{cls_name}` declares methods missing from Swift source: {missing_in_swift[:10]}"
                    )
                if unregistered_in_matrix:
                    record(
                        f"Swift `{cls_name}` defines methods missing from feature_matrix.json swift_uitest_suite: {unregistered_in_matrix[:10]}"
                    )

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
            "swift_v167_files": len(swift_v167_files),
            "swift_v167_tier1_tests": swift_tier_counts["tier1"],
            "swift_v167_tier2_tests": swift_tier_counts["tier2"],
            "swift_v167_tier3_tests": swift_tier_counts["tier3"],
            "swift_v167_tier4_tests": swift_tier_counts["tier4"],
            "swift_v167_total_tests": swift_total_tests,
            "swift_ui_namespaces_covered": sum(1 for v in swift_ui_namespace_hits.values() if v > 0),
            "swift_features_covered": len(swift_covered_features),
        },
        "rpc_namespace_hits": rpc_counts,
        "swift_ui_namespace_hits": swift_ui_namespace_hits,
        "swift_tier_classes": swift_tier_classes,
        "errors": errors,
        "warnings": warnings,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Fabushi iOS v1.6.8 E2E Spec Coverage Validator")
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
