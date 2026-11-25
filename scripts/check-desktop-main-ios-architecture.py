#!/usr/bin/env python3
"""Fail-closed live Desktop-main source/ownership authority validation for Fabushi iOS.

Validates the recorded snapshot against the pinned Git tree and the live
bhrumom/gpui-telegram@main ref in GitHub Actions. No historical PR #20 SHA or
legacy file count is treated as a permanent acceptance target.
--complete is intentionally stronger than ordinary inventory validation.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from collections import Counter
from pathlib import Path

try:
    import tomllib
except ImportError:  # pragma: no cover
    tomllib = None

ROOT = Path(__file__).resolve().parents[1]
LOCK = ROOT / "manifests/desktop-main-authority.json"
MANIFEST_INDEX = ROOT / "manifests/desktop-main-reference-index.json"
LEDGER_INDEX = ROOT / "docs/parity/desktop-main-index.json"
ACTIVE_SPEC = ROOT / "docs/specs/gpui-telegram-main-ios-parity.md"
SUPERSEDED_DESKTOP_SPEC = ROOT / "docs/specs/fabushi-desktop-main-ios-parity.md"
HISTORICAL_SPEC = ROOT / "docs/specs/fabushi-desktop-pr20-ios-architecture-parity.md"
HISTORICAL_SPECS = [SUPERSEDED_DESKTOP_SPEC, HISTORICAL_SPEC]
UPSTREAM_REPO = "bhrumom/gpui-telegram"
UPSTREAM_URL = "https://github.com/bhrumom/gpui-telegram.git"
TYPO_OR_HISTORICAL_REPOS = {
    "bhrunom/gpui-telegram",
    "bhrum/gpui-telegram",
    "bhrumom/fabushi-desktop",
}
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
DISPOSITIONS = {"unreviewed", "direct-port", "ios-adapted", "not-applicable-with-replacement"}
STATUSES = {"unreviewed", "understood", "mapped", "implemented", "verified", "not-applicable"}
AUTHORITY_PIN_MODES = {"live-main", "fixed-baseline"}
EXCLUDED_DESKTOP_CRATES = {
    "gpui_telegram_desktop",
    "fabushi_remote_daemon",
    "gpui_telegram_cli",
    "fabushi_miniapp_wasm",
    "gpui_telegram_server",
    "gpui_telegram_test_harness",
}
FORBIDDEN_MOBILE_DEPS = {
    "gpui",
    "khronos-egl",
    "khronos_egl",
    "openssl-sys",
} | EXCLUDED_DESKTOP_CRATES
FORBIDDEN_EXTERNAL_REPO_STRINGS = [
    "bhrumom/fabushi-platform-core",
    "../fabushi-platform-core",
    "../fabushi-desktop",
    "../gpui-telegram",
    "github.com/bhrumom/fabushi-desktop",
    "github.com/bhrumom/gpui-telegram",
    "github.com/bhrum/gpui-telegram",
    "github.com/bhrunom/gpui-telegram",
]
FORBIDDEN_GIT_ORG_RE = re.compile(r"(?:github\.com[:/])?(bhrumom|bhrum|bhrunom)/", re.IGNORECASE)
FORBIDDEN_FILE_GIT_RE = re.compile(r"^(?:(?:git|registry|sparse|path)\+)?file://", re.IGNORECASE)
WINDOWS_ABS_RE = re.compile(r"^[A-Za-z]:[\\/]")
LOCAL_PATH_PATTERNS = [
    re.compile(r"/Users/"),
    re.compile(r"/home/[^/\s]+(?:/|$|\s)"),
    re.compile(r"[A-Za-z]:[/\\][Uu]sers[/\\]"),
    re.compile(r"(?:^|[\s\x22\x27(`:=])~/"),
]
TEST_READY_ACTIONS_URL_RE = re.compile(
    r"(?:^|[\s(\[<\x22\x27`])https://github\.com/fabushiios/fabushi-ios/actions/runs/[1-9][0-9]*(?:\b|/)"
)
TEST_READY_RUN_ID_RE = re.compile(r"\bGITHUB_RUN_ID\s*[:=]\s*[1-9][0-9]*\b")
HOST_STUB_ACK_RE = re.compile(r'\\?"status\\?"\s*:\s*\\?"acknowledged\\?"')
HOST_HARDCODED_LOCALHOST_RE = re.compile(r'https?://127\.0\.0\.1:8443')
SWIFT_FAKE_ACCOUNT_RE = re.compile(r'"acc_primary"|"acc_work"')
SWIFT_FAKE_CRYPTO_KEY_RE = re.compile(
    r'(?:Data|\[UInt8\])\s*\(\s*repeating\s*:\s*(?:UInt8\s*\(\s*)?0x(?:AA|BB)\b',
    re.IGNORECASE,
)
SWIFT_MOCK_BRIDGE_RE = re.compile(r'\bmockBridge\b|\bmockChunks\b|"mock_media_')
PROJECT_ATS_SECTION_RE = re.compile(r'(?:^|\s)NSAppTransportSecurity\s*:', re.MULTILINE)
PROJECT_ATS_ARBITRARY_FALSE_RE = re.compile(
    r'(?:^|[\s{,])NSAllowsArbitraryLoads\s*:\s*["\']?(?:false|no|off|0)["\']?(?=\s|$|[},])',
    re.IGNORECASE | re.MULTILINE,
)
PROJECT_ATS_ARBITRARY_TRUE_RE = re.compile(
    r'(?:^|[\s{,])NSAllowsArbitraryLoads\s*:\s*["\']?(?:true|yes|on|1)["\']?(?=\s|$|[},])',
    re.IGNORECASE | re.MULTILINE,
)
PROJECT_ATS_LOCAL_TRUE_RE = re.compile(
    r'(?:^|[\s{,])NSAllowsLocalNetworking\s*:\s*["\']?(?:true|yes|on|1)["\']?(?=\s|$|[},])',
    re.IGNORECASE | re.MULTILINE,
)
PROJECT_TESTS_E2E_PATH_RE = re.compile(r'(?:^|[\s/"\'])tests/e2e(?:/|\b|$)')
DEP_TABLE_KEYS = (
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "dev_dependencies",
    "build_dependencies",
)
EXPECTED_RELEASE_GATES = frozenset({
    "rust-host-and-coordinator",
    "swift-unit-and-ui",
    "native-lifecycle-restart-recovery",
    "protected-global-dharma-real-account",
    "signed-device-archive-export",
    "physical-device-install-launch",
    "installed-app-upgrade-data-migration",
    "testflight-install-and-app-store-validation",
    "accessibility-localization-iPhone-iPad",
    "security-privacy-credential-isolation",
    "performance-fault-soak",
    "licensing-provenance-legal-review",
    "post-merge-main-exact-head-verification",
})


def _norm_crate_name(name: str) -> str:
    return name.strip().lower().replace("-", "_")


def document(path):
    return json.loads(path.read_text(encoding="utf-8"))


def authority_pin_mode(lock):
    mode = lock.get("authorityPinMode")
    return "live-main" if mode is None else mode


def tree_digest(records):
    """SHA-256 of canonical full Git tracked identities (including symlink/gitlink)."""
    stream = "".join(
        f"{path}\0{mode}\0{kind}\0{sha}\0{size if size is not None else '-'}\n"
        for path, (mode, kind, sha, size) in sorted(records.items())
    )
    return hashlib.sha256(stream.encode("utf-8")).hexdigest()


def git_tree(checkout):
    raw = subprocess.check_output(
        ["git", "ls-tree", "-r", "-l", "-z", "HEAD"], cwd=checkout
    )
    result = {}
    for line in raw.split(b"\0"):
        if not line:
            continue
        identity, name = line.split(b"\t", 1)
        mode, kind, sha, size = identity.decode("ascii").split()
        path = name.decode("utf-8")
        if path in result:
            raise ValueError(f"duplicate Git tree identity: {path}")
        result[path] = (mode, kind, sha, None if size == "-" else int(size))
    return result


def live_head(url=UPSTREAM_URL):
    env = {**os.environ, "GIT_TERMINAL_PROMPT": "0"}
    result = subprocess.check_output(
        ["git", "ls-remote", "--heads", url, "refs/heads/main"],
        stderr=subprocess.PIPE,
        timeout=60,
        env=env,
    ).decode("ascii", "strict").strip()
    if not result:
        raise ValueError("Desktop main ref could not be resolved")
    sha, ref = result.split()
    if ref != "refs/heads/main" or not HEX40.fullmatch(sha):
        raise ValueError(f"unexpected Desktop main ref: {result!r}")
    return sha


def github_api_json(url):
    headers = {
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "fabushi-ios-authority-checker",
    }
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    last_exc = None
    for attempt in range(2):
        req = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                return json.loads(resp.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            last_exc = exc
            if exc.code in {500, 502, 503, 504} and attempt == 0:
                time.sleep(2)
                continue
            raise ValueError(f"GitHub API request failed for {url}: {exc}") from exc
        except urllib.error.URLError as exc:
            last_exc = exc
            if attempt == 0:
                time.sleep(2)
                continue
            raise ValueError(f"GitHub API request failed for {url}: {exc}") from exc
    raise ValueError(f"GitHub API request failed for {url}: {last_exc}") from last_exc


def github_api_commit_tree(repo, commit_sha, fetch_json=None):
    fetcher = fetch_json or github_api_json
    data = fetcher(f"https://api.github.com/repos/{repo}/git/commits/{commit_sha}")
    if data.get("sha") != commit_sha:
        raise ValueError(f"GitHub API commit SHA mismatch: {data.get('sha')!r} != {commit_sha!r}")
    tree_sha = (data.get("tree") or {}).get("sha")
    if not HEX40.fullmatch(tree_sha or ""):
        raise ValueError(f"GitHub API commit response missing valid tree SHA: {tree_sha!r}")
    return tree_sha


def github_api_verify_pinned_commit(repo, commit_sha, expected_tree_sha, fetch_json=None):
    """Prove pinned commit exists in upstream repo with exact commit SHA and root tree SHA."""
    fetcher = fetch_json or github_api_json
    data = fetcher(f"https://api.github.com/repos/{repo}/commits/{commit_sha}")
    actual_sha = data.get("sha")
    if actual_sha != commit_sha:
        raise ValueError(
            f"pinned Desktop commit SHA mismatch: got={actual_sha!r} expected={commit_sha!r}"
        )
    tree_sha = ((data.get("commit") or {}).get("tree") or {}).get("sha")
    if not HEX40.fullmatch(tree_sha or ""):
        raise ValueError(f"pinned Desktop commit missing valid commit.tree.sha: {tree_sha!r}")
    if tree_sha != expected_tree_sha:
        raise ValueError(
            f"pinned Desktop root tree SHA mismatch: got={tree_sha!r} expected={expected_tree_sha!r}"
        )
    return actual_sha, tree_sha


def github_api_tree(repo, tree_sha, fetch_json=None):
    fetcher = fetch_json or github_api_json
    data = fetcher(f"https://api.github.com/repos/{repo}/git/trees/{tree_sha}?recursive=1")
    if data.get("truncated") is not False:
        raise ValueError(f"GitHub API Git tree response is truncated or non-boolean: {data.get('truncated')!r}")
    if data.get("sha") != tree_sha:
        raise ValueError(f"GitHub API tree SHA mismatch: {data.get('sha')!r} != {tree_sha!r}")
    result = {}
    for entry in data.get("tree", []):
        kind = entry.get("type")
        if kind == "tree":
            continue
        path = entry.get("path")
        mode = entry.get("mode")
        sha = entry.get("sha")
        size = entry.get("size") if kind == "blob" else None
        if not path or not mode or not HEX40.fullmatch(sha or ""):
            raise ValueError(f"malformed GitHub API tree entry: {entry!r}")
        if path in result:
            raise ValueError(f"duplicate GitHub API tree identity: {path}")
        result[path] = (mode, kind, sha, None if size is None else int(size))
    return result


def verify_tree_entries(real, lock, errors, actual_head=None, actual_tree=None):
    source_sha = lock.get("commit")
    root_tree = lock.get("rootTreeSha")
    if actual_head is not None and actual_head != source_sha:
        errors.append(f"Desktop tree identity drift: commit={actual_head} expected={source_sha}")
    if actual_tree is not None and actual_tree != root_tree:
        errors.append(f"Desktop tree identity drift: tree={actual_tree} expected={root_tree}")

    inv = lock.get("inventory", {})
    blobs = {p: v for p, v in real.items() if v[1] == "blob"}
    gitlinks = {p: v for p, v in real.items() if v[1] == "commit"}
    crates_blobs = {p: v for p, v in blobs.items() if p.startswith("crates/")}
    inherited_blobs = {p: v for p, v in blobs.items() if p.startswith("inherited/")}

    if len(gitlinks) != inv.get("gitlinkCount", 0):
        errors.append(f"Desktop tree gitlink count drift: {len(gitlinks)} != {inv.get('gitlinkCount', 0)}")
    if len(blobs) != inv.get("fullTrackedCount"):
        errors.append(f"full tracked Desktop blob count drift: {len(blobs)} != {inv.get('fullTrackedCount')}")
    if len(crates_blobs) != inv.get("selectedFileCount"):
        errors.append(f"selected crates/** blob count drift: {len(crates_blobs)} != {inv.get('selectedFileCount')}")
    if inv.get("inheritedFileCount") is not None and len(inherited_blobs) != inv.get("inheritedFileCount"):
        errors.append(f"inherited/** blob count drift: {len(inherited_blobs)} != {inv.get('inheritedFileCount')}")
    if len(blobs) - len(crates_blobs) != inv.get("nonselectedCount"):
        errors.append(
            f"nonselected blob count drift: {len(blobs) - len(crates_blobs)} != {inv.get('nonselectedCount')}"
        )

    actual_crate_dirs = {p.split("/")[1] for p in crates_blobs if len(p.split("/")) > 2}
    vendored = set(lock.get("vendoredCoreCrates", []))
    shared = set(lock.get("vendoredSharedSourceDirs", []))
    excluded = set(lock.get("excludedDesktopCrates", []))
    if (vendored & excluded) or (shared & excluded) or (vendored & shared):
        errors.append("authority lock vendored/shared/excluded crate sets overlap")
    expected_crate_dirs = vendored | shared | excluded
    if actual_crate_dirs != expected_crate_dirs:
        missing = sorted(expected_crate_dirs - actual_crate_dirs)
        extra = sorted(actual_crate_dirs - expected_crate_dirs)
        errors.append(f"crates/* directory accounting drift: missing={missing} extra={extra}")

    computed_full = tree_digest(real)
    computed_selected = tree_digest(crates_blobs)
    expected_full = inv.get("fullTreeSha256")
    expected_selected = inv.get("selectedTreeSha256")
    if not isinstance(expected_full, str) or not HEX64.fullmatch(expected_full):
        errors.append(
            f"full tracked Desktop root tree inventory SHA256 missing or invalid 64-hex digest: {expected_full!r}"
        )
    elif computed_full != expected_full:
        errors.append(
            f"full tracked Desktop root tree inventory SHA256 drift: got={computed_full} expected={expected_full}"
        )
    if not isinstance(expected_selected, str) or not HEX64.fullmatch(expected_selected):
        errors.append(
            f"selected crates/** inventory SHA256 missing or invalid 64-hex digest: {expected_selected!r}"
        )
    elif computed_selected != expected_selected:
        errors.append(
            f"selected crates/** inventory SHA256 drift: got={computed_selected} expected={expected_selected}"
        )
    return computed_full, computed_selected


def check_status(row, path, complete, errors, warnings, locked_commit):
    status = (row.get("implementation_status") or "").strip()
    disposition = (row.get("ios_disposition") or "").strip()
    target = (row.get("ios_target_path") or "").strip()
    if status not in STATUSES:
        errors.append(f"{path}: invalid status: {status!r}")
    if disposition not in DISPOSITIONS:
        errors.append(f"{path}: invalid disposition: {disposition!r}")
    if status in {"mapped", "implemented", "verified", "not-applicable"}:
        if row.get("desktop_responsibility") in (None, "", "pending-review"):
            errors.append(f"{path}: reviewed row missing a real responsibility")
        if row.get("desktop_visible_effect") in (None, "", "pending-review"):
            errors.append(f"{path}: reviewed row missing a product effect")
    if status in {"implemented", "verified"}:
        target_paths = [t.strip() for t in target.split(";") if t.strip()]
        if not target_paths or any(not (ROOT / t).is_file() for t in target_paths):
            errors.append(f"{path}: {status} has no actual iOS source file: {target}")
        if not (row.get("production_evidence") or "").strip():
            errors.append(f"{path}: {status} missing production evidence")
    if status == "verified" and not (row.get("test_evidence") or "").strip():
        errors.append(f"{path}: verified missing test evidence")
    if status == "not-applicable":
        if disposition != "not-applicable-with-replacement":
            errors.append(f"{path}: N/A status without replacement disposition")
        if not (row.get("ios_platform_delta") or "").strip():
            errors.append(f"{path}: N/A without platform and product-effect replacement")
    if row.get("authority_review_state") in {
        "pending", "requires-current-main-review", "requires-main-responsibility-and-owner-review"
    } and status in {"verified", "implemented"}:
        errors.append(f"{path}: impacted main row improperly promoted to {status}")

    if complete:
        if status not in {"verified", "not-applicable"}:
            errors.append(f"{path}: incomplete mandatory responsibility status={status}")
        if row.get("authority_review_state") not in {"current-main-reviewed", "current-main-verified"}:
            errors.append(f"{path}: final source owner/contract revalidation not documented")
        if status == "verified":
            for field in (
                "responsibility_id", "ios_target_symbol", "production_entrypoint",
                "oracle_ids", "ios_target_commit", "workflow_run", "run_attempt",
                "job", "artifact_id", "artifact_digest", "independent_reviewer",
            ):
                if not row.get(field):
                    errors.append(f"{path}: verified lacks current-head evidence field {field}")
            if row.get("desktop_review_commit") != locked_commit:
                errors.append(f"{path}: verified source review not bound to current Desktop main")
            expected_ios_head = os.environ.get("IOS_EXPECTED_HEAD")
            row_ios_commit = row.get("ios_target_commit")
            if (
                not isinstance(row_ios_commit, str)
                or not HEX40.fullmatch(row_ios_commit)
                or not expected_ios_head
                or not HEX40.fullmatch(expected_ios_head)
                or row_ios_commit != expected_ios_head
            ):
                errors.append(f"{path}: verified evidence not bound to current iOS HEAD")
    elif status in {"verified", "not-applicable"} and row.get("desktop_review_commit") != locked_commit:
        warnings.append(f"{path}: legacy {status} is HISTORICAL, not current-main verified")


def verify_snapshot(lock, manifest_index, ledger_index, errors):
    if lock.get("schemaVersion") != 3 or lock.get("authorityMode") != "live-main":
        errors.append("missing v3 live-main authority lock")
    pin_mode = authority_pin_mode(lock)
    if pin_mode not in AUTHORITY_PIN_MODES:
        errors.append(
            f"invalid authorityPinMode: {pin_mode!r}; expected one of {sorted(AUTHORITY_PIN_MODES)}"
        )
    elif pin_mode == "fixed-baseline":
        directive = lock.get("pinnedByUserDirective")
        if not isinstance(directive, str) or not directive.strip():
            errors.append("fixed-baseline authorityPinMode requires non-empty pinnedByUserDirective")
    sha, tree = lock.get("commit"), lock.get("rootTreeSha")
    if not HEX40.fullmatch(sha or "") or not HEX40.fullmatch(tree or ""):
        errors.append("authority lock missing exact 40-hex commit and root tree")
    repo = lock.get("repository")
    if repo in TYPO_OR_HISTORICAL_REPOS:
        errors.append(f"authority lock uses typo or historical repository {repo!r}; expected {UPSTREAM_REPO!r}")
    elif repo != UPSTREAM_REPO or lock.get("branch") != "main":
        errors.append(f"authority lock is not {UPSTREAM_REPO}@main")

    typo = lock.get("typoNote", {})
    if (
        typo.get("requestedSlug") != "bhrunom/gpui-telegram"
        or typo.get("resolvedSlug") != UPSTREAM_REPO
        or typo.get("status") != "http-404-verified-typo"
    ):
        errors.append("authority lock missing verified bhrunom/gpui-telegram 404 typoNote")

    lineage = lock.get("legacyLineage", {})
    if (
        lineage.get("repository") != "bhrumom/fabushi-desktop"
        or not HEX40.fullmatch(lineage.get("commit") or "")
        or not HEX40.fullmatch(lineage.get("rootTreeSha") or "")
    ):
        errors.append("authority lock missing valid bhrumom/fabushi-desktop legacyLineage")

    vendored = lock.get("vendoredCoreCrates")
    shared = lock.get("vendoredSharedSourceDirs", [])
    excluded = lock.get("excludedDesktopCrates")
    if not isinstance(vendored, list) or not vendored:
        errors.append("authority lock missing non-empty vendoredCoreCrates")
        vendored = []
    if not isinstance(shared, list):
        errors.append("authority lock vendoredSharedSourceDirs must be a list")
        shared = []
    if not isinstance(excluded, list) or not excluded:
        errors.append("authority lock missing non-empty excludedDesktopCrates")
        excluded = []
    if (set(vendored) & set(excluded)) or (set(shared) & set(excluded)) or (set(vendored) & set(shared)):
        errors.append("authority lock crate sets overlap")
    if not EXCLUDED_DESKTOP_CRATES.issubset(set(excluded)):
        errors.append("authority lock excludedDesktopCrates missing required excluded desktop crates")

    inventory = lock.get("inventory", {})
    ledger_mode = inventory.get("fileLedgerMode")
    if ledger_mode not in {"legacy-predecessor-archived", "current-main-complete"}:
        errors.append(f"invalid inventory.fileLedgerMode: {ledger_mode!r}")
    if inventory.get("selectedRoots") != ["crates/**"]:
        errors.append("selected source roots drift: expected ['crates/**']")
    if inventory.get("treeTruncated") is not False:
        errors.append("authority lock must record treeTruncated=false")
    if inventory.get("gitlinkCount") != 0:
        errors.append("authority lock must record gitlinkCount=0")
    sel_cnt = inventory.get("selectedFileCount")
    non_cnt = inventory.get("nonselectedCount")
    full_cnt = inventory.get("fullTrackedCount")
    if not all(isinstance(x, int) and x >= 0 for x in (sel_cnt, non_cnt, full_cnt)):
        errors.append("authority lock inventory counts must be non-negative integers")
    elif sel_cnt + non_cnt != full_cnt:
        errors.append("authority lock selectedFileCount + nonselectedCount != fullTrackedCount")
    if not isinstance(inventory.get("fullTreeSha256"), str) or not HEX64.fullmatch(inventory["fullTreeSha256"]):
        errors.append("authority lock must record valid 64-hex inventory.fullTreeSha256")
    if not isinstance(inventory.get("selectedTreeSha256"), str) or not HEX64.fullmatch(inventory["selectedTreeSha256"]):
        errors.append("authority lock must record valid 64-hex inventory.selectedTreeSha256")

    if ledger_mode != "current-main-complete":
        for flag in ("baselineReady", "dependenciesReviewed", "completenessAccepted", "acceptanceAccepted"):
            if lock.get(flag) is True:
                errors.append(f"premature closure flag {flag}=true while fileLedgerMode={ledger_mode!r}")

    for key, value in (
        ("manifestIndex", "manifests/desktop-main-reference-index.json"),
        ("ledgerIndex", "docs/parity/desktop-main-index.json"),
        ("nonselectedRegister", "manifests/desktop-main/tracked-outside-selected.json"),
        ("impactRegister", "docs/parity/desktop-main-impact.json"),
        ("releaseEvidence", "docs/parity/desktop-main-release-evidence.json"),
    ):
        if inventory.get(key) != value:
            errors.append(f"authority lock file route drift: {key}")

    if ledger_mode == "current-main-complete":
        for value, idx in (("manifest", manifest_index), ("ledger", ledger_index)):
            source = idx.get("authority" if value == "manifest" else "source", {})
            for k, expected in (
                ("repository", lock.get("repository")),
                ("branch", "main"),
                ("commit", sha),
                ("tree", tree),
                ("mode", "live-main"),
            ):
                if source.get(k) != expected:
                    errors.append(f"{value} authority drift: {k}={source.get(k)!r}; expected {expected!r}")
        if manifest_index.get("fileCount") != inventory.get("selectedFileCount"):
            errors.append("manifest row count does not agree with authority lock")
        if ledger_index.get("rowCount") != inventory.get("selectedFileCount"):
            errors.append("ledger row count does not agree with authority lock")
        if not HEX64.fullmatch(str(inventory.get("selectedTreeSha256") or "")):
            errors.append("current-main-complete requires non-null 64-hex selectedTreeSha256")
        elif manifest_index.get("selectedTreeSha256") != inventory.get("selectedTreeSha256"):
            errors.append("manifest inventory digest differs from authority lock")
    elif ledger_mode == "legacy-predecessor-archived":
        leg_repo = lineage.get("repository")
        leg_commit = lineage.get("lastInventoriedCommit")
        leg_tree = lineage.get("lastInventoriedRootTreeSha")
        leg_count = lineage.get("lastInventoriedSelectedCount")
        for value, idx in (("manifest", manifest_index), ("ledger", ledger_index)):
            source = idx.get("authority" if value == "manifest" else "source", {})
            for k, expected in (
                ("repository", leg_repo),
                ("branch", "main"),
                ("commit", leg_commit),
                ("tree", leg_tree),
                ("mode", "live-main"),
            ):
                if source.get(k) != expected:
                    errors.append(f"{value} authority drift (legacy archived): {k}={source.get(k)!r}; expected {expected!r}")
        if manifest_index.get("fileCount") != leg_count:
            errors.append("archived legacy manifest row count does not agree with legacyLineage")
        if ledger_index.get("rowCount") != leg_count:
            errors.append("archived legacy ledger row count does not agree with legacyLineage")

    required_docs = [
        ROOT / "AGENTS.md",
        ROOT / "README.md",
        ROOT / "MIGRATION_SOURCE.md",
        ACTIVE_SPEC,
        SUPERSEDED_DESKTOP_SPEC,
        HISTORICAL_SPEC,
        ROOT / "docs/specs/spec-first-ai-development.md",
        ROOT / "docs/parity/desktop-main-release-evidence.json",
    ]
    for file in required_docs:
        if not file.is_file():
            errors.append(f"missing canonical authority document: {file}")

    agents_path = ROOT / "AGENTS.md"
    if agents_path.is_file():
        agents_text = agents_path.read_text(encoding="utf-8")
        for token in [
            "fabushiios/fabushi-ios",
            "bhrumom/gpui-telegram",
            "docs/specs/gpui-telegram-main-ios-parity.md",
            "manifests/desktop-main-authority.json",
            "only in GitHub Actions or a user-authorized remote runtime",
            "IOS-MAIN-AC-01..21",
        ]:
            if token not in agents_text:
                errors.append(f"AGENTS.md missing normative token {token}")

    if ACTIVE_SPEC.is_file():
        active = ACTIVE_SPEC.read_text(encoding="utf-8")
        for token in [
            "bhrumom/gpui-telegram",
            "bhrunom/gpui-telegram",
            "bhrumom/fabushi-desktop",
            "manifests/desktop-main-authority.json",
            "IOS-MAIN-AUTH-01",
            "IOS-MAIN-INV-01",
            "IOS-MAIN-LEDGER-01",
            "IOS-MAIN-GATE-01",
            "IOS-MAIN-AC-01",
            "IOS-MAIN-AC-21",
        ]:
            if token not in active:
                errors.append(f"canonical iOS Spec missing normative token {token}")

    if SUPERSEDED_DESKTOP_SPEC.is_file():
        superseded = SUPERSEDED_DESKTOP_SPEC.read_text(encoding="utf-8")
        for token in ["Status: superseded", "docs/specs/gpui-telegram-main-ios-parity.md"]:
            if token not in superseded:
                errors.append(f"superseded fabushi-desktop Spec missing token {token}")


def verify_release_evidence(release, lock, errors, complete):
    source_sha = lock.get("commit")
    if release.get("desktopRepository") != lock.get("repository"):
        errors.append(
            f"release acceptance matrix desktopRepository mismatch: {release.get('desktopRepository')!r} != {lock.get('repository')!r}"
        )
    if release.get("desktopSourceCommit") != source_sha or release.get("desktopRootTree") != lock.get("rootTreeSha"):
        errors.append("release acceptance matrix is not bound to current Desktop main")
    expected_ac = {f"IOS-MAIN-AC-{i:02d}" for i in range(1, 22)}
    entries = release.get("acceptanceCriteria", [])
    actual_ac = [r.get("id") for r in entries]
    if len(actual_ac) != 21 or set(actual_ac) != expected_ac:
        errors.append("release matrix must contain exactly 21 stable acceptance criterion IDs")
    expected_ios_head = os.environ.get("IOS_EXPECTED_HEAD")
    for r in entries:
        if r.get("status") == "passed":
            if not r.get("evidence") or not r.get("independentReviewer"):
                errors.append(f"{r.get('id')}: passed claim lacks evidence or independent reviewer")
            ios_commit = r.get("iosCommit")
            if (
                not isinstance(ios_commit, str)
                or not HEX40.fullmatch(ios_commit)
                or not expected_ios_head
                or not HEX40.fullmatch(expected_ios_head)
                or ios_commit != expected_ios_head
            ):
                errors.append(f"{r.get('id')}: passed claim not tied to current iOS exact head")
    release_gates = release.get("releaseGates")
    if not isinstance(release_gates, dict) or set(release_gates.keys()) != EXPECTED_RELEASE_GATES:
        present_keys = set(release_gates.keys()) if isinstance(release_gates, dict) else set()
        missing_gates = sorted(EXPECTED_RELEASE_GATES - present_keys)
        extra_gates = sorted(present_keys - EXPECTED_RELEASE_GATES)
        errors.append(
            f"release matrix releaseGates must match exact 13 canonical keys "
            f"(missing={missing_gates} extra={extra_gates})"
        )
        release_gates_iter = {}
    else:
        release_gates_iter = release_gates
    for key, gate in release_gates_iter.items():
        if not isinstance(gate, dict):
            errors.append(f"{key}: release gate entry must be an object")
            continue
        if gate.get("status") == "passed":
            required = [
                "iosCommit", "desktopCommit", "workflowRun", "runAttempt", "job",
                "requiredSteps", "artifactId", "artifactDigest", "independentReviewer",
            ]
            for field in required:
                if not gate.get(field):
                    errors.append(f"{key}: passed release gate lacks {field}")
            gate_ios_commit = gate.get("iosCommit")
            if (
                gate.get("desktopCommit") != source_sha
                or not isinstance(gate_ios_commit, str)
                or not HEX40.fullmatch(gate_ios_commit)
                or not expected_ios_head
                or not HEX40.fullmatch(expected_ios_head)
                or gate_ios_commit != expected_ios_head
            ):
                errors.append(f"{key}: passed release gate source identity mismatch")
            if not re.fullmatch(r"sha256:[0-9a-f]{64}", gate.get("artifactDigest") or ""):
                errors.append(f"{key}: passed release artifact digest not sha256")
            if gate.get("requiredSteps") and any(
                step.get("conclusion") != "success" or step.get("status") != "completed"
                for step in gate["requiredSteps"]
            ):
                errors.append(f"{key}: passed release gate has missing, skipped or failed required step")
    if release.get("finalAcceptance") is True:
        if lock.get("inventory", {}).get("fileLedgerMode") != "current-main-complete":
            errors.append(
                "release acceptance matrix sets premature finalAcceptance=true while "
                f"fileLedgerMode={lock.get('inventory', {}).get('fileLedgerMode')!r}"
            )
        final_ios_commit = release.get("finalIosCommit")
        if (
            not isinstance(final_ios_commit, str)
            or not HEX40.fullmatch(final_ios_commit)
            or not expected_ios_head
            or not HEX40.fullmatch(expected_ios_head)
            or final_ios_commit != expected_ios_head
        ):
            errors.append(
                "release acceptance matrix sets finalAcceptance=true without matching 40-hex finalIosCommit == IOS_EXPECTED_HEAD"
            )
        for r in entries:
            if r.get("status") != "passed":
                errors.append(
                    f"release acceptance matrix sets finalAcceptance=true while {r.get('id')} status={r.get('status')!r}"
                )
        for name, gate in release_gates_iter.items():
            if not isinstance(gate, dict) or gate.get("status") != "passed":
                errors.append(
                    f"release acceptance matrix sets finalAcceptance=true while release gate {name!r} is not passed"
                )
    if complete:
        if release.get("finalAcceptance") is not True:
            errors.append("final independent iOS release acceptance not recorded")
        for r in entries:
            if r.get("status") != "passed":
                errors.append(f"{r.get('id')}: mandatory release criterion not accepted")
        for name, gate in release_gates_iter.items():
            if not isinstance(gate, dict) or gate.get("status") != "passed":
                errors.append(f"release gate pending/blocked: {name}")


def verify_files(lock, manifest_index, ledger_index, errors, warnings, complete, checkout, verify_api_tree=False):
    source_sha = lock.get("commit")
    inv = lock.get("inventory", {})
    ledger_mode = inv.get("fileLedgerMode")
    lineage = lock.get("legacyLineage", {})

    if complete and ledger_mode != "current-main-complete":
        errors.append(
            f"--complete requires inventory.fileLedgerMode == 'current-main-complete'; got {ledger_mode!r}"
        )

    is_legacy_archived = ledger_mode == "legacy-predecessor-archived"
    if is_legacy_archived:
        warnings.append(
            "per-file manifest/ledger chunks remain archived bhrumom/fabushi-desktop predecessor lineage "
            "pending M6 full crates/** ledger regeneration; --complete remains fail-closed blocked"
        )

    expected_selected_count = (
        lineage.get("lastInventoriedSelectedCount") if is_legacy_archived else inv.get("selectedFileCount")
    )
    expected_nonselected_count = (
        (lineage.get("lastInventoriedFullTrackedCount", 0) - lineage.get("lastInventoriedSelectedCount", 0))
        if is_legacy_archived
        else inv.get("nonselectedCount")
    )
    expected_ledger_commit = lineage.get("lastInventoriedCommit") if is_legacy_archived else source_sha
    expected_ledger_tree = lineage.get("lastInventoriedRootTreeSha") if is_legacy_archived else lock.get("rootTreeSha")

    recorded = {}
    for group in manifest_index.get("groups", []):
        path = ROOT / group["path"]
        if not path.is_file():
            errors.append(f"missing Desktop-main manifest: {group['path']}")
            continue
        block = document(path)
        rows = block.get("files", [])
        if block.get("fileCount") != len(rows) or group.get("fileCount") != len(rows):
            errors.append(f"manifest chunk row count mismatch: {group['path']}")
        if not HEX40.fullmatch(block.get("sourceCommit") or ""):
            errors.append(f"missing sourceCommit provenance: {group['path']}")
        for row in rows:
            p = row.get("path")
            if p in recorded:
                errors.append(f"duplicate manifest source path {p}")
            recorded[p] = row

    ledgers = {}
    statuses, dispositions = Counter(), Counter()
    for group in ledger_index.get("chunks", []):
        path = ROOT / group["path"]
        if not path.is_file():
            errors.append(f"missing Desktop-main parity ledger: {group['path']}")
            continue
        block = document(path)
        rows = block.get("rows", [])
        if block.get("rowCount") != len(rows) or group.get("rowCount") != len(rows):
            errors.append(f"ledger chunk row count mismatch: {group['path']}")
        if not HEX40.fullmatch(block.get("sourceCommit") or ""):
            errors.append(f"missing ledger sourceCommit provenance: {group['path']}")
        for row in rows:
            p = row.get("desktop_path")
            if p in ledgers:
                errors.append(f"duplicate ledger Desktop source path {p}")
            ledgers[p] = row
            statuses[row.get("implementation_status")] += 1
            dispositions[row.get("ios_disposition")] += 1
            check_status(row, p, False if is_legacy_archived else complete, errors, warnings, expected_ledger_commit)

    if len(recorded) != expected_selected_count:
        errors.append(f"selected source count mismatch: {len(recorded)} != {expected_selected_count}")
    if len(ledgers) != expected_selected_count:
        errors.append(f"selected parity row count mismatch: {len(ledgers)} != {expected_selected_count}")
    if set(recorded) != set(ledgers):
        errors.append(
            f"manifest/ledger paths drift: missing={list(set(recorded)-set(ledgers))[:5]} "
            f"extra={list(set(ledgers)-set(recorded))[:5]}"
        )
    for path, row in recorded.items():
        if ledgers.get(path, {}).get("desktop_blob_sha") != row.get("blobSha"):
            errors.append(f"{path}: manifest/ledger Git blob mismatch")
        if is_legacy_archived:
            if not (path.startswith("frontend/") or path.startswith("source/")):
                errors.append(f"{path}: legacy archived selected root contamination")
        else:
            if not path.startswith("crates/"):
                errors.append(f"{path}: selected root contamination (expected crates/**)")
        if not HEX40.fullmatch(row.get("blobSha") or ""):
            errors.append(f"{path}: invalid exact Git blob SHA")

    unselected = document(ROOT / inv["nonselectedRegister"])
    outside = {}
    if unselected.get("count") != len(unselected.get("entries", [])):
        errors.append("nonselected tracked entry count mismatch")
    for r in unselected.get("entries", []):
        p = r.get("path")
        if p in outside:
            errors.append(f"duplicate nonselected path: {p}")
        outside[p] = r
        if is_legacy_archived:
            if p in recorded or p.startswith("frontend/") or p.startswith("source/"):
                errors.append(f"nonselected register contains legacy selected path: {p}")
        else:
            if p in recorded or p.startswith("crates/"):
                errors.append(f"nonselected register contains selected path: {p}")
    if len(outside) != expected_nonselected_count:
        errors.append(f"nonselected tracked count differs from expected: {len(outside)} != {expected_nonselected_count}")

    impact = document(ROOT / inv["impactRegister"])
    if impact.get("currentCommit") != expected_ledger_commit or impact.get("fullTree") != expected_ledger_tree:
        errors.append("impact register is not bound to expected authority commit/tree")
    if impact.get("directCount") != len([x for x in impact.get("directChanges", []) if x["change"] != "metadata-corrected"]):
        errors.append("impact direct-change count inconsistent")
    for change in impact.get("directChanges", []):
        if change["change"] in {"added", "blob-changed"}:
            row = ledgers.get(change["path"])
            if row is None or row.get("authority_review_state") not in {
                "pending", "requires-current-main-review", "requires-main-responsibility-and-owner-review"
            }:
                errors.append(f"changed source not re-review-fenced: {change['path']}")
            if row and row.get("implementation_status") in {"verified", "implemented"}:
                errors.append(f"changed source incorrectly accepted: {change['path']}")

    computed_digests = (None, None)
    real = None
    if checkout is not None:
        actual_head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout).decode().strip()
        actual_tree = subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], cwd=checkout).decode().strip()
        real = git_tree(checkout)
        computed_digests = verify_tree_entries(real, lock, errors, actual_head=actual_head, actual_tree=actual_tree)
    elif verify_api_tree:
        api_tree_sha = github_api_commit_tree(lock["repository"], source_sha)
        real = github_api_tree(lock["repository"], lock["rootTreeSha"])
        computed_digests = verify_tree_entries(real, lock, errors, actual_head=source_sha, actual_tree=api_tree_sha)

    if real is not None and not is_legacy_archived:
        if set(real) != set(recorded) | set(outside):
            errors.append("full Desktop tree has missing/unexpected tracked paths")
        for path, row in recorded.items():
            got = real.get(path)
            if not got or got[1] != "blob" or got[2] != row["blobSha"] or got[3] != row.get("size"):
                errors.append(f"{path}: blob/size does not match live main exact Git tree")
        for path, row in outside.items():
            got = real.get(path)
            if not got or got != (row.get("mode"), row.get("type"), row.get("sha"), row.get("size")):
                errors.append(f"{path}: nonselected Git identity differs from live main")

    release_path = ROOT / "docs/parity/desktop-main-release-evidence.json"
    if not release_path.is_file():
        errors.append("missing durable current-main release acceptance/evidence matrix")
    else:
        release = document(release_path)
        verify_release_evidence(release, lock, errors, complete)

    if complete:
        for key in ("baselineReady", "dependenciesReviewed", "completenessAccepted", "acceptanceAccepted"):
            if lock.get(key) is not True:
                errors.append(f"release gate not closed: {key}")
        if not HEX64.fullmatch(str(inv.get("fullTreeSha256") or "")):
            errors.append("--complete requires non-null 64-hex inventory.fullTreeSha256")
        if not HEX64.fullmatch(str(inv.get("selectedTreeSha256") or "")):
            errors.append("--complete requires non-null 64-hex inventory.selectedTreeSha256")
        if impact.get("fullReviewAccepted") is not True or impact.get("dependencyReview", {}).get("status") != "verified":
            errors.append("upstream changed-owner/protocol impact review not closed")
        for row in outside.values():
            if row.get("coverage_status") not in {"verified", "non-runtime-accounted"}:
                errors.append(f"nonselected tracked item unreviewed: {row.get('path')}")
        if checkout is None:
            errors.append("--complete requires a pinned actual Desktop Git checkout")
    return statuses, dispositions, computed_digests


def _iter_dependency_tables(parsed):
    for key in DEP_TABLE_KEYS:
        tbl = parsed.get(key)
        if isinstance(tbl, dict):
            yield key, tbl
    ws = parsed.get("workspace")
    if isinstance(ws, dict) and isinstance(ws.get("dependencies"), dict):
        yield "workspace.dependencies", ws["dependencies"]
    targets = parsed.get("target")
    if isinstance(targets, dict):
        for cfg, body in targets.items():
            if not isinstance(body, dict):
                continue
            for key in DEP_TABLE_KEYS:
                tbl = body.get(key)
                if isinstance(tbl, dict):
                    yield f"target.{cfg}.{key}", tbl
    patches = parsed.get("patch")
    if isinstance(patches, dict):
        for source, tbl in patches.items():
            if isinstance(tbl, dict):
                yield f"patch.{source}", tbl
    replace_tbl = parsed.get("replace")
    if isinstance(replace_tbl, dict):
        yield "replace", replace_tbl


def verify_cargo_toml_hygiene(
    cargo_toml_path,
    core_root,
    forbidden_deps,
    errors,
    require_path_exists=True,
):
    text = cargo_toml_path.read_text(encoding="utf-8")
    try:
        rel_display = str(cargo_toml_path.relative_to(ROOT))
    except ValueError:
        rel_display = str(cargo_toml_path)

    for forbidden in FORBIDDEN_EXTERNAL_REPO_STRINGS:
        if forbidden in text:
            errors.append(f"{rel_display}: introduces forbidden cross-repo dependency string: {forbidden}")

    if tomllib is None:  # pragma: no cover
        errors.append("Python 3.11+ tomllib is required to validate Cargo.toml manifests")
        return

    try:
        parsed = tomllib.loads(text)
    except Exception as exc:
        errors.append(f"{rel_display}: failed to parse Cargo.toml: {exc}")
        return

    core_root_resolved = core_root.resolve()
    forbidden_set = {_norm_crate_name(name) for name in forbidden_deps}

    for table_name, deps in _iter_dependency_tables(parsed):
        if table_name.startswith("patch."):
            patch_source = table_name[len("patch."):].strip()
            if FORBIDDEN_GIT_ORG_RE.search(patch_source) or FORBIDDEN_FILE_GIT_RE.match(patch_source):
                errors.append(
                    f"{rel_display} [{table_name}]: forbidden patch source URL: {patch_source!r}"
                )
        for dep_key, spec in deps.items():
            raw_dep_name = dep_key
            if table_name == "replace":
                dep_key_s = dep_key.strip()
                if "#" in dep_key_s:
                    replace_source, pkg_spec = dep_key_s.rsplit("#", 1)
                    replace_source_s = replace_source.strip()
                    if FORBIDDEN_GIT_ORG_RE.search(replace_source_s) or FORBIDDEN_FILE_GIT_RE.match(replace_source_s):
                        errors.append(
                            f"{rel_display} [replace]: forbidden replace source URL: {replace_source!r}"
                        )
                    pkg_spec_s = pkg_spec.strip()
                    if pkg_spec_s and pkg_spec_s[0].isdigit():
                        # Cargo URL-only '#version' shorthand: crate name is final URL path segment
                        url_base = replace_source_s.split("?", 1)[0].rstrip("/")
                        seg = url_base.rsplit("/", 1)[-1]
                        if seg.endswith(".git"):
                            seg = seg[:-4]
                        raw_dep_name = seg
                    else:
                        raw_dep_name = re.split(r"[:@]", pkg_spec_s, maxsplit=1)[0]
                else:
                    if FORBIDDEN_GIT_ORG_RE.search(dep_key_s) or FORBIDDEN_FILE_GIT_RE.match(dep_key_s):
                        errors.append(
                            f"{rel_display} [replace]: forbidden replace source URL: {dep_key!r}"
                        )
                    if "://" in dep_key_s:
                        # Cargo 1.77+ URL with '@version' and no '#'
                        # Extract the final path segment first so URL userinfo '@' (e.g. ssh://git@github.com/...)
                        # is never mistaken for the '@version' separator.
                        url_part = dep_key_s.split("?", 1)[0].rstrip("/")
                        seg = url_part.rsplit("/", 1)[-1].split("@", 1)[0]
                        if seg.endswith(".git"):
                            seg = seg[:-4]
                        raw_dep_name = seg
                    else:
                        raw_dep_name = re.split(r"[:@]", dep_key_s, maxsplit=1)[0]
            pkg_name = raw_dep_name
            if isinstance(spec, dict) and isinstance(spec.get("package"), str):
                pkg_name = spec["package"]
            if _norm_crate_name(raw_dep_name) in forbidden_set or _norm_crate_name(pkg_name) in forbidden_set:
                errors.append(
                    f"{rel_display} [{table_name}]: forbidden mobile dependency {dep_key!r} (package={pkg_name!r})"
                )
            if isinstance(spec, dict):
                dep_path = spec.get("path")
                if isinstance(dep_path, str):
                    if dep_path.startswith("/") or WINDOWS_ABS_RE.match(dep_path) or Path(dep_path).is_absolute():
                        errors.append(
                            f"{rel_display} [{table_name}.{dep_key}]: absolute path dependency forbidden: {dep_path!r}"
                        )
                    else:
                        resolved = (cargo_toml_path.parent / dep_path).resolve()
                        try:
                            resolved.relative_to(core_root_resolved)
                        except ValueError:
                            errors.append(
                                f"{rel_display} [{table_name}.{dep_key}]: path dependency escapes {core_root_resolved}: {dep_path!r}"
                            )
                        else:
                            if require_path_exists and not resolved.exists():
                                errors.append(
                                    f"{rel_display} [{table_name}.{dep_key}]: path dependency target does not exist: {dep_path!r}"
                                )
                git_url = spec.get("git")
                if isinstance(git_url, str):
                    if FORBIDDEN_GIT_ORG_RE.search(git_url):
                        errors.append(
                            f"{rel_display} [{table_name}.{dep_key}]: external Fabushi git dependency forbidden: {git_url!r}"
                        )
                    if FORBIDDEN_FILE_GIT_RE.match(git_url.strip()):
                        errors.append(
                            f"{rel_display} [{table_name}.{dep_key}]: local file:// git dependency forbidden: {git_url!r}"
                        )


def verify_desktop_core_workspace(core_root, lock, errors):
    root_toml = core_root / "Cargo.toml"
    if not core_root.is_dir():
        errors.append("missing iOS-owned desktop_core directory: source/desktop_core")
        return
    if not root_toml.is_file():
        errors.append("missing iOS-owned workspace manifest: source/desktop_core/Cargo.toml")
        return

    forbidden_deps = set(FORBIDDEN_MOBILE_DEPS)
    if lock and isinstance(lock.get("forbiddenMobileWorkspaceDeps"), list):
        forbidden_deps |= set(lock["forbiddenMobileWorkspaceDeps"])
    excluded_crates = set(EXCLUDED_DESKTOP_CRATES)
    if lock and isinstance(lock.get("excludedDesktopCrates"), list):
        excluded_crates |= set(lock["excludedDesktopCrates"])
    forbidden_norm = {_norm_crate_name(name) for name in forbidden_deps}
    excluded_norm = {_norm_crate_name(name) for name in excluded_crates}

    verify_cargo_toml_hygiene(root_toml, core_root, forbidden_deps, errors, require_path_exists=True)
    try:
        parsed = tomllib.loads(root_toml.read_text(encoding="utf-8"))
    except Exception:
        return
    ws = parsed.get("workspace", {})
    members = ws.get("members", [])
    excludes = set(ws.get("exclude", []))
    ws_pkg = ws.get("package", {})
    if ws_pkg.get("edition") != "2024":
        errors.append(f"source/desktop_core/Cargo.toml: workspace.package.edition must be '2024', got {ws_pkg.get('edition')!r}")
    for profile_name in ("dev", "test"):
        prof = (parsed.get("profile") or {}).get(profile_name, {})
        if prof.get("debug") != 0 or prof.get("incremental") is not False:
            errors.append(f"source/desktop_core/Cargo.toml: profile.{profile_name} must enforce debug=0 and incremental=false")

    core_root_resolved = core_root.resolve()
    normalized_members = set()
    if not isinstance(members, list):
        errors.append("source/desktop_core/Cargo.toml: workspace.members must be a list")
        members = []
    for member in members:
        if not isinstance(member, str) or not member.strip():
            errors.append(f"source/desktop_core/Cargo.toml: invalid workspace member entry: {member!r}")
            continue
        if any(ch in member for ch in "*?[]"):
            errors.append(f"source/desktop_core/Cargo.toml: wildcard workspace member forbidden: {member!r}")
            continue
        if member.startswith("/") or WINDOWS_ABS_RE.match(member) or Path(member).is_absolute():
            errors.append(f"source/desktop_core/Cargo.toml: workspace member absolute path forbidden: {member!r}")
            continue
        resolved_member = (core_root / member).resolve()
        try:
            rel_member_path = resolved_member.relative_to(core_root_resolved)
        except ValueError:
            errors.append(
                f"source/desktop_core/Cargo.toml: workspace member escapes {core_root_resolved}: {member!r}"
            )
            continue
        normalized_members.add(rel_member_path.as_posix())
        if any(
            _norm_crate_name(part) in excluded_norm or _norm_crate_name(part) in forbidden_norm
            for part in rel_member_path.parts
        ):
            errors.append(
                f"source/desktop_core/Cargo.toml: workspace member includes excluded desktop crate: {member!r}"
            )
        member_toml = resolved_member / "Cargo.toml"
        if member_toml.is_file():
            try:
                member_parsed = tomllib.loads(member_toml.read_text(encoding="utf-8"))
                pkg_name = (member_parsed.get("package") or {}).get("name")
                if isinstance(pkg_name, str) and (
                    _norm_crate_name(pkg_name) in excluded_norm
                    or _norm_crate_name(pkg_name) in forbidden_norm
                ):
                    errors.append(
                        f"source/desktop_core/Cargo.toml: workspace member {member!r} declares excluded desktop package {pkg_name!r}"
                    )
            except Exception as exc:
                errors.append(f"source/desktop_core/{rel_member_path.as_posix()}/Cargo.toml: failed to parse: {exc}")

    if lock:
        for crate in lock.get("vendoredCoreCrates", []):
            rel_member = f"crates/{crate}"
            if rel_member not in normalized_members:
                errors.append(f"source/desktop_core/Cargo.toml missing required vendored member: {rel_member}")
            if not (core_root / rel_member / "Cargo.toml").is_file():
                errors.append(f"missing vendored crate manifest: source/desktop_core/{rel_member}/Cargo.toml")
        for shared_dir in lock.get("vendoredSharedSourceDirs", []):
            if not (core_root / "crates" / shared_dir).is_dir():
                errors.append(f"missing vendored shared source directory: source/desktop_core/crates/{shared_dir}")

    crates_dir = core_root / "crates"
    if crates_dir.is_dir():
        for child in sorted(crates_dir.iterdir()):
            if _norm_crate_name(child.name) in excluded_norm:
                errors.append(f"excluded desktop crate directory present under source/desktop_core/crates/{child.name}")

    excluded_paths = [(core_root / ex).resolve() for ex in excludes]
    for toml_file in sorted(core_root.rglob("Cargo.toml")):
        rel_parts = toml_file.relative_to(core_root).parts
        if rel_parts and rel_parts[0] == "target":
            continue
        if toml_file == root_toml:
            continue
        is_excluded = any(
            toml_file.resolve() == ep or ep in toml_file.resolve().parents
            for ep in excluded_paths
        )
        verify_cargo_toml_hygiene(
            toml_file,
            core_root,
            forbidden_deps,
            errors,
            require_path_exists=not is_excluded,
        )


def verify_test_ready_integrity(root, errors, complete=False):
    test_ready = root / "TEST_READY.md"
    if not test_ready.is_file():
        return
    text = test_ready.read_text(encoding="utf-8")
    for pat in LOCAL_PATH_PATTERNS:
        if pat.search(text):
            errors.append(f"TEST_READY.md contains forbidden local host path matching {pat.pattern!r}")
    for marker in (
        "cd /Users/",
        "worker_m6_iter3_impl",
        "target/e2e/master_e2e_runner",
        "bhrumom/fabushi-desktop",
        "bhrunom/gpui-telegram",
        "bhrum/gpui-telegram",
    ):
        if marker in text:
            errors.append(f"TEST_READY.md contains forbidden stale/local-only marker: {marker!r}")
    has_actions_url = bool(TEST_READY_ACTIONS_URL_RE.search(text))
    has_run_id = bool(TEST_READY_RUN_ID_RE.search(text))
    all_shas = re.findall(r"\b[0-9a-f]{40}\b", text)
    if "0" * 40 in all_shas:
        errors.append("TEST_READY.md contains forbidden all-zero 40-hex commit SHA")
    valid_shas = [sha for sha in all_shas if sha != "0" * 40]
    has_sha = bool(valid_shas)
    if not ((has_actions_url or has_run_id) and has_sha):
        errors.append(
            "TEST_READY.md lacks required GitHub Actions run provenance "
            "(anchored actions run URL with numeric run ID or numeric GITHUB_RUN_ID) "
            "and non-zero 40-hex commit SHA"
        )
    if complete:
        expected_head = os.environ.get("IOS_EXPECTED_HEAD")
        if not expected_head or not HEX40.fullmatch(expected_head):
            errors.append(
                "complete mode requires valid 40-hex IOS_EXPECTED_HEAD to verify TEST_READY.md binding"
            )
        elif not re.search(rf"\b{re.escape(expected_head)}\b", text):
            errors.append(
                f"TEST_READY.md does not reference current IOS_EXPECTED_HEAD ({expected_head})"
            )


def verify_host_anti_stub(root, errors):
    host_src = root / "source/host/src"
    dispatch = host_src / "dispatch.rs"
    if not dispatch.is_file():
        errors.append("missing C-ABI host dispatch owner: source/host/src/dispatch.rs")
        return
    for rs_file in sorted(host_src.rglob("*.rs")):
        text = rs_file.read_text(encoding="utf-8")
        rel = rs_file.relative_to(root).as_posix()
        if HOST_STUB_ACK_RE.search(text):
            errors.append(
                f'{rel} contains forbidden stub/fake fallback matching "status": "acknowledged"'
            )
        if "sample_chats" in text:
            errors.append(f"{rel} contains forbidden stub/fake fallback: sample_chats")
        if HOST_HARDCODED_LOCALHOST_RE.search(text):
            errors.append(f"{rel} contains forbidden hardcoded localhost stub URL (127.0.0.1:8443)")
    cleaner = root / "scripts/clean_build_garbage.py"
    if cleaner.is_file():
        cleaner_text = cleaner.read_text(encoding="utf-8")
        for pat in LOCAL_PATH_PATTERNS:
            if pat.search(cleaner_text):
                errors.append(
                    f"scripts/clean_build_garbage.py contains forbidden local host path matching {pat.pattern}"
                )


def _strip_yaml_comments(text: str) -> str:
    cleaned_lines = []
    for line in text.splitlines():
        in_single = False
        in_double = False
        out = []
        for ch in line:
            if ch == "'" and not in_double:
                in_single = not in_single
            elif ch == '"' and not in_single:
                in_double = not in_double
            elif ch == "#" and not in_single and not in_double:
                break
            out.append(ch)
        cleaned_lines.append("".join(out))
    return "\n".join(cleaned_lines)


def verify_ios_swift_anti_mock(root, errors):
    app_root = root / "mobile/ios/Fabushi"
    frontend_root = root / "frontend"
    required_core_owners = [
        "mobile/ios/Fabushi/Core/FabushiRustHostActor.swift",
        "mobile/ios/Fabushi/Core/FabushiCoordinator.swift",
    ]
    for relative in required_core_owners:
        if not (root / relative).is_file():
            errors.append(f"missing canonical Swift Core owner: {relative}")

    swift_roots = []
    if app_root.is_dir():
        swift_roots.append(app_root)
    else:
        errors.append("missing iOS Swift app source root: mobile/ios/Fabushi")
    if frontend_root.is_dir():
        swift_roots.append(frontend_root)

    for scan_root in swift_roots:
        for swift_file in sorted(scan_root.rglob("*.swift")):
            text = swift_file.read_text(encoding="utf-8")
            rel = swift_file.relative_to(root).as_posix()
            if SWIFT_FAKE_ACCOUNT_RE.search(text):
                errors.append(
                    f"{rel} contains forbidden hardcoded fake account literal (acc_primary/acc_work)"
                )
            if SWIFT_FAKE_CRYPTO_KEY_RE.search(text):
                errors.append(
                    f"{rel} contains forbidden hardcoded fake crypto key (Data(repeating: 0xAA/0xBB, ...))"
                )
            if SWIFT_MOCK_BRIDGE_RE.search(text):
                errors.append(
                    f"{rel} contains forbidden production mock bypass (mockBridge/mockChunks/mock_media_)"
                )

    project_yml = root / "mobile/ios/project.yml"
    if not project_yml.is_file():
        errors.append("missing XcodeGen spec: mobile/ios/project.yml")
    else:
        yml_text = _strip_yaml_comments(project_yml.read_text(encoding="utf-8"))
        if not PROJECT_ATS_SECTION_RE.search(yml_text):
            errors.append("mobile/ios/project.yml missing NSAppTransportSecurity configuration")
        if PROJECT_ATS_ARBITRARY_TRUE_RE.search(yml_text):
            errors.append("mobile/ios/project.yml must not enable NSAllowsArbitraryLoads: true")
        if not PROJECT_ATS_ARBITRARY_FALSE_RE.search(yml_text):
            errors.append("mobile/ios/project.yml must enforce NSAllowsArbitraryLoads: false")
        if not PROJECT_ATS_LOCAL_TRUE_RE.search(yml_text):
            errors.append("mobile/ios/project.yml must enable NSAllowsLocalNetworking: true")
        if "master_e2e_suite.swift" in yml_text or PROJECT_TESTS_E2E_PATH_RE.search(yml_text):
            errors.append("mobile/ios/project.yml must not compile standalone tests/e2e sources (master_e2e_suite.swift)")


def verify_host_cargo_manifests(root, forbidden_deps, errors):
    host_dir = root / "source/host"
    host_toml = host_dir / "Cargo.toml"
    if not host_toml.is_file():
        errors.append("missing C-ABI host manifest: source/host/Cargo.toml")
        return
    for toml_file in sorted(host_dir.rglob("Cargo.toml")):
        rel_parts = toml_file.relative_to(host_dir).parts
        if rel_parts and rel_parts[0] == "target":
            continue
        verify_cargo_toml_hygiene(toml_file, root, forbidden_deps, errors, require_path_exists=True)


def verify_ios_owners(errors, lock=None, complete=False):
    """Keep preexisting shipping-path architecture regression fences and standalone iOS core checks intact."""
    required_roots = [
        "frontend",
        "source/desktop_core",
        "source/ios-main",
        "source/ios-preload",
        "source/ios-dev-controls",
        "source/mahayana-agent-coordinator",
        "source/host",
        "source/local-exec-daemon",
        "source/box-exec-daemon",
        "source/internal",
        "source/packages",
        "source/shared",
    ]
    for relative in required_roots:
        if not (ROOT / relative).is_dir():
            errors.append(f"missing iOS architecture root: {relative}")

    if not (ROOT / "mobile/ios/Fabushi/FabushiApp.swift").is_file():
        errors.append("missing iOS application bootstrap")

    # The iOS product must remain standalone. These are source/build dependency checks,
    # not documentation/provenance string bans.
    dependency_files = [
        ROOT / "mobile/ios/project.yml",
        ROOT / "source/packages/mahayana-rs/Cargo.toml",
    ]
    for dependency_file in dependency_files:
        if not dependency_file.is_file():
            continue
        text = dependency_file.read_text(encoding="utf-8")
        for forbidden in FORBIDDEN_EXTERNAL_REPO_STRINGS:
            if forbidden in text:
                errors.append(
                    f"{dependency_file.relative_to(ROOT)} introduces forbidden cross-repo runtime dependency: {forbidden}"
                )

    forbidden_deps = set(FORBIDDEN_MOBILE_DEPS)
    if lock and isinstance(lock.get("forbiddenMobileWorkspaceDeps"), list):
        forbidden_deps |= set(lock["forbiddenMobileWorkspaceDeps"])
    verify_host_cargo_manifests(ROOT, forbidden_deps, errors)
    verify_host_anti_stub(ROOT, errors)
    verify_ios_swift_anti_mock(ROOT, errors)

    verify_desktop_core_workspace(ROOT / "source/desktop_core", lock, errors)
    verify_test_ready_integrity(ROOT, errors, complete=complete)

    # Preserve the shipping ownership chain established by PR #3 while it is re-audited.
    preload = ROOT / "source/ios-preload/preload.swift"
    if preload.is_file():
        text = preload.read_text(encoding="utf-8")
        if "IOSCoordinatorPortClient" not in text:
            errors.append("iOS preload does not use its renderer-facing coordinator-port client")
        for forbidden in ["CoordinatorControlPortClient", "main.dispatch("]:
            if forbidden in text:
                errors.append(f"iOS preload bypasses Coordinator ownership: {forbidden}")

    app = ROOT / "mobile/ios/Fabushi/FabushiApp.swift"
    if app.is_file():
        text = app.read_text(encoding="utf-8")
        for forbidden in ["MahayanaHost(", "MahayanaCoordinator("]:
            if forbidden in text:
                errors.append(f"FabushiApp.swift directly constructs runtime owner: {forbidden}")

    # Desktop packaged acceptance reads one canonical agent-avatar responsibility.
    avatar_path = ROOT / "frontend/src/recovered/features/agent-info/ClothGhostAvatar.swift"
    summary_path = ROOT / "frontend/src/recovered/features/agent-info/MobileBotSummary.swift"
    bot_service_path = ROOT / "frontend/src/production/GrokMobileBotService.swift"
    bot_chat_path = ROOT / "frontend/src/recovered/features/conversation/MobileBotChat.swift"
    group_members_path = ROOT / "frontend/src/production/MobileBotGroupMembers.swift"
    home_path = ROOT / "frontend/src/production/GrokMobileShell+Home.swift"

    if not avatar_path.is_file():
        errors.append("missing canonical native Agent avatar owner")
    else:
        avatar_text = avatar_path.read_text(encoding="utf-8")
        for token in [
            "internal struct MobileAgentAvatar: View",
            "internal func mobileAgentAvatarKind",
            "internal func mobileResolvePersonaColor",
            "internal func mobileResolvePersonaShape",
            "AvatarImagePolicy.data(fromImageDataURL:",
            "MobileOnboardingCharacter(",
        ]:
            if token not in avatar_text:
                errors.append(f"canonical native Agent avatar contract drift: missing {token}")

    if not summary_path.is_file():
        errors.append("missing canonical native Agent summary projection")
    elif "let avatarState: MobileAgentAvatarState" not in summary_path.read_text(encoding="utf-8"):
        errors.append("native Agent summary dropped canonical avatar activity state")

    if not bot_service_path.is_file():
        errors.append("missing canonical native Bot roster adapter")
    else:
        service_text = bot_service_path.read_text(encoding="utf-8")
        for token in [
            'currentActivity: row["currentActivity"]',
            'isSharedRoom: row["isSharedRoom"] as? Bool ?? false',
            "avatarState: canonical.avatarState",
            "avatarDataURL: canonical.avatarDataURL",
        ]:
            if token not in service_text:
                errors.append(f"native Bot roster no longer preserves avatar authority: missing {token}")

    if not bot_chat_path.is_file():
        errors.append("missing native Bot chat surface")
    else:
        bot_chat_text = bot_chat_path.read_text(encoding="utf-8")
        if bot_chat_text.count("MobileAgentAvatar(bot: bot") < 4:
            errors.append(
                "native Bot chat no longer reuses canonical MobileAgentAvatar across visible Agent surfaces"
            )
        if "ClothGhostAvatar(botId: bot.id" in bot_chat_text:
            errors.append("native Bot chat bypasses canonical MobileAgentAvatar")

    if not group_members_path.is_file():
        errors.append("missing native group-member Agent surface")
    else:
        group_text = group_members_path.read_text(encoding="utf-8")
        for token in ["MobileAgentAvatar(bot: member", "MobileAgentAvatar(bot: candidate"]:
            if token not in group_text:
                errors.append(f"native group-member surface bypasses canonical avatar owner: missing {token}")
        if "ClothGhostAvatar(botId: member.id" in group_text or "ClothGhostAvatar(botId: candidate.id" in group_text:
            errors.append("native group-member surface bypasses canonical MobileAgentAvatar")

    if not home_path.is_file():
        errors.append("missing native Agent home surface")
    else:
        home_text = home_path.read_text(encoding="utf-8")
        if home_text.count("MobileAgentAvatar(bot: bot") < 2:
            errors.append("native Agent home rows no longer reuse canonical MobileAgentAvatar")
        if "ClothGhostAvatar(botId: bot.id" in home_text:
            errors.append("native Agent home rows bypass canonical MobileAgentAvatar")

    # Native owner composition and device security remain part of strict architecture.
    rust_host = ROOT / "source/packages/mahayana-rs/mahayana-host/src/lib.rs"
    if rust_host.is_file():
        source = rust_host.read_text(encoding="utf-8")
        for token in ["ModelCredentialResolver", "credential_client.session_token()", "ModelProviderMode::UserConfiguredRemote"]:
            if token not in source:
                errors.append(f"missing native credential separation contract: {token}")
        for bad in ["HostAuthExtension", "ProductionTeamRulesResolver"]:
            if bad in source:
                errors.append(f"forbidden secondary Cursor HostAuth owner in iOS: {bad}")


def run():
    parser = argparse.ArgumentParser()
    parser.add_argument("--strict", action="store_true", help="verify shipping owner composition and fail closed on authority drift")
    parser.add_argument("--complete", action="store_true", help="require full current-main responsibility, release, and evidence closure")
    parser.add_argument("--authority-only", action="store_true", help="fast start/end live main authority check (not a complete source audit)")
    parser.add_argument("--desktop-checkout", type=Path, help="pinned Desktop-main checkout; required for source tree acceptance")
    parser.add_argument("--verify-api-tree", action="store_true", help="verify non-truncated full Git tree via GitHub REST API")
    parser.add_argument("--check-host-bridge", action="store_true", help="enforce fail-closed anti-stub checks across source/host/src/**/*.rs")
    args = parser.parse_args()
    errors = []
    warnings = []
    try:
        for p in [LOCK, MANIFEST_INDEX, LEDGER_INDEX]:
            if not p.is_file():
                errors.append(f"missing canonical machine authority file: {p}")
        if errors:
            raise ValueError("; ".join(errors))
        lock, manifest, ledger = (document(x) for x in (LOCK, MANIFEST_INDEX, LEDGER_INDEX))
        verify_snapshot(lock, manifest, ledger, errors)
        pin_mode = authority_pin_mode(lock)
        if pin_mode == "fixed-baseline":
            if (
                lock.get("repository") == UPSTREAM_REPO
                and HEX40.fullmatch(lock.get("commit") or "")
                and HEX40.fullmatch(lock.get("rootTreeSha") or "")
            ):
                try:
                    github_api_verify_pinned_commit(
                        lock["repository"], lock["commit"], lock["rootTreeSha"]
                    )
                except ValueError as exc:
                    errors.append(str(exc))
            try:
                observed_live = live_head()
                if observed_live != lock.get("commit"):
                    print(
                        f"INFO: fixed-baseline pin active (pinned={lock.get('commit')}, "
                        f"live_main={observed_live}, directive={lock.get('pinnedByUserDirective')})"
                    )
            except (OSError, ValueError, subprocess.SubprocessError) as exc:
                print(f"INFO: live main head lookup skipped in fixed-baseline mode ({exc})")
        elif pin_mode == "live-main":
            observed = live_head()
            if observed != lock.get("commit"):
                errors.append(f"STALE Desktop main lock: recorded={lock.get('commit')} live={observed}")
        expected = os.environ.get("IOS_EXPECTED_HEAD")
        if expected:
            own_head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip()
            if own_head != expected:
                errors.append(f"iOS exact-head checkout differs from event source: {own_head} != {expected}")
        statuses = dispositions = None
        computed_digests = (None, None)
        if args.authority_only and args.complete:
            errors.append("--complete is incompatible with --authority-only")
        elif not args.authority_only:
            checkout = args.desktop_checkout.resolve() if args.desktop_checkout else None
            use_api_tree = args.verify_api_tree or (args.strict and checkout is None)
            statuses, dispositions, computed_digests = verify_files(
                lock,
                manifest,
                ledger,
                errors,
                warnings,
                args.complete,
                checkout,
                verify_api_tree=use_api_tree,
            )
            if args.strict or args.complete:
                verify_ios_owners(errors, lock, complete=args.complete)
        else:
            if args.strict:
                verify_ios_owners(errors, lock, complete=False)
            if args.verify_api_tree:
                api_tree_sha = github_api_commit_tree(lock["repository"], lock["commit"])
                real = github_api_tree(lock["repository"], lock["rootTreeSha"])
                computed_digests = verify_tree_entries(
                    real, lock, errors, actual_head=lock["commit"], actual_tree=api_tree_sha
                )
        if args.check_host_bridge and not (args.strict or args.complete):
            verify_host_anti_stub(ROOT, errors)
        if errors:
            for message in errors[:250]:
                print("ERROR:", message)
            if len(errors) > 250:
                print(f"ERROR: {len(errors)-250} additional errors (truncated)")
            return 1
        if statuses is not None:
            print(
                f"Desktop main ({lock['repository']}) -> iOS source authority {lock['commit']} "
                f"tree={lock['rootTreeSha']} tracked={lock['inventory']['fullTrackedCount']} "
                f"selected={lock['inventory']['selectedFileCount']} "
                f"fileLedgerMode={lock['inventory']['fileLedgerMode']} "
                f"statuses={dict(statuses)} dispositions={dict(dispositions)}"
            )
        if computed_digests[0] is not None:
            print(
                f"TREE_DIGESTS fullTreeSha256={computed_digests[0]} "
                f"selectedTreeSha256={computed_digests[1]}"
            )
        for message in warnings[:15]:
            print("HISTORICAL:", message)
        if len(warnings) > 15:
            print(f"HISTORICAL: {len(warnings)-15} additional row statuses are not current-main acceptance")
        mode_label = (
            f"fixed-baseline Desktop pin ({lock['commit'][:12]})"
            if pin_mode == "fixed-baseline"
            else "live Desktop main authority"
        )
        print(
            f"PASS: {mode_label}"
            + (" and source/ledger identity" if not args.authority_only else "")
            + "; full product/release completeness is NOT implied"
        )
        return 0
    except (OSError, ValueError, KeyError, subprocess.SubprocessError, json.JSONDecodeError) as exc:
        print("ERROR: fail-closed authority validation:", exc)
        return 1


if __name__ == "__main__":
    sys.exit(run())
