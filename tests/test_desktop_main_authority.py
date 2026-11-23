"""Run only in GitHub Actions: fail-closed authority checker unit contracts."""
import contextlib
import copy
import importlib.util
import io
import os
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
p = ROOT / "scripts/check-desktop-main-ios-architecture.py"
spec = importlib.util.spec_from_file_location("ios_desktop_main_authority", p)
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)


def make_valid_v3_lock(file_ledger_mode="current-main-complete"):
    return {
        "schemaVersion": 3,
        "authorityMode": "live-main",
        "repository": "bhrumom/gpui-telegram",
        "branch": "main",
        "commit": "a" * 40,
        "rootTreeSha": "b" * 40,
        "typoNote": {
            "requestedSlug": "bhrunom/gpui-telegram",
            "status": "http-404-verified-typo",
            "resolvedSlug": "bhrumom/gpui-telegram",
        },
        "legacyLineage": {
            "repository": "bhrumom/fabushi-desktop",
            "commit": "1" * 40,
            "rootTreeSha": "2" * 40,
            "lastInventoriedCommit": "3" * 40,
            "lastInventoriedRootTreeSha": "4" * 40,
            "lastInventoriedSelectedCount": 2,
            "lastInventoriedFullTrackedCount": 5,
        },
        "vendoredCoreCrates": ["gpui_telegram_core"],
        "vendoredSharedSourceDirs": ["gpui_telegram_common"],
        "excludedDesktopCrates": sorted(validator.EXCLUDED_DESKTOP_CRATES),
        "forbiddenMobileWorkspaceDeps": sorted(validator.FORBIDDEN_MOBILE_DEPS),
        "inventory": {
            "fileLedgerMode": file_ledger_mode,
            "selectedRoots": ["crates/**"],
            "inheritedRoots": ["inherited/**"],
            "selectedFileCount": 8,
            "inheritedFileCount": 1,
            "fullTrackedCount": 10,
            "gitlinkCount": 0,
            "nonselectedCount": 2,
            "treeTruncated": False,
            "fullTreeSha256": "e" * 64,
            "selectedTreeSha256": "c" * 64,
            "manifestIndex": "manifests/desktop-main-reference-index.json",
            "ledgerIndex": "docs/parity/desktop-main-index.json",
            "nonselectedRegister": "manifests/desktop-main/tracked-outside-selected.json",
            "impactRegister": "docs/parity/desktop-main-impact.json",
            "releaseEvidence": "docs/parity/desktop-main-release-evidence.json",
        },
        "baselineReady": False,
        "dependenciesReviewed": False,
        "completenessAccepted": False,
        "acceptanceAccepted": False,
    }


def make_base_tree_and_lock(file_ledger_mode="legacy-predecessor-archived"):
    lock = make_valid_v3_lock(file_ledger_mode=file_ledger_mode)
    crate_names = (
        lock["vendoredCoreCrates"]
        + lock["vendoredSharedSourceDirs"]
        + lock["excludedDesktopCrates"]
    )
    base_tree = {
        f"crates/{name}/src/lib.rs": ("100644", "blob", "a" * 40, 10)
        for name in crate_names
    }
    base_tree["inherited/host/src/lib.rs"] = ("100644", "blob", "b" * 40, 10)
    base_tree["README.md"] = ("100644", "blob", "c" * 40, 10)
    crates_blobs = {k: v for k, v in base_tree.items() if k.startswith("crates/")}
    lock["inventory"]["fullTreeSha256"] = validator.tree_digest(base_tree)
    lock["inventory"]["selectedTreeSha256"] = validator.tree_digest(crates_blobs)
    return base_tree, lock


class DesktopMainAuthorityTests(unittest.TestCase):
    def test_tree_digest_stable_and_identity_sensitive(self):
        a = {
            "crates/gpui_telegram_core/src/lib.rs": ("100644", "blob", "a" * 40, 12),
            "inherited/host/src/lib.rs": ("100644", "blob", "b" * 40, 3),
        }
        b = dict(reversed(list(a.items())))
        self.assertEqual(validator.tree_digest(a), validator.tree_digest(b))
        c = dict(a)
        c["inherited/host/src/lib.rs"] = ("100644", "blob", "c" * 40, 3)
        self.assertNotEqual(validator.tree_digest(a), validator.tree_digest(c))

    def test_stale_main_lock_rejected(self):
        errors = []
        data = make_valid_v3_lock(file_ledger_mode="current-main-complete")
        mf = {
            "authority": {
                "repository": data["repository"],
                "branch": "main",
                "commit": "d" * 40,
                "tree": data["rootTreeSha"],
                "mode": "live-main",
            },
            "fileCount": data["inventory"]["selectedFileCount"],
            "selectedTreeSha256": data["inventory"]["selectedTreeSha256"],
        }
        ld = {"source": mf["authority"], "rowCount": data["inventory"]["selectedFileCount"]}
        validator.verify_snapshot(data, mf, ld, errors)
        self.assertTrue(any("manifest authority drift" in e for e in errors), errors)

    def test_unreviewed_impact_never_verified(self):
        errors = []
        warnings = []
        row = {
            "implementation_status": "verified",
            "ios_disposition": "ios-adapted",
            "desktop_responsibility": "real",
            "desktop_visible_effect": "real",
            "ios_target_path": "README.md",
            "production_evidence": "a",
            "test_evidence": "b",
            "authority_review_state": "requires-main-responsibility-and-owner-review",
        }
        validator.check_status(row, "crates/gpui_telegram_core/src/lib.rs", False, errors, warnings, "a" * 40)
        self.assertTrue(any("impacted" in e for e in errors), errors)

    def test_wrong_upstream_repo_or_typo_slug_rejected(self):
        for bad_repo in ("bhrunom/gpui-telegram", "bhrum/gpui-telegram", "bhrumom/fabushi-desktop"):
            errors = []
            data = make_valid_v3_lock(file_ledger_mode="legacy-predecessor-archived")
            data["repository"] = bad_repo
            mf = {
                "authority": {
                    "repository": "bhrumom/fabushi-desktop",
                    "branch": "main",
                    "commit": "3" * 40,
                    "tree": "4" * 40,
                    "mode": "live-main",
                },
                "fileCount": 2,
            }
            ld = {"source": mf["authority"], "rowCount": 2}
            validator.verify_snapshot(data, mf, ld, errors)
            self.assertTrue(
                any("typo or historical repository" in e or "bhrumom/gpui-telegram@main" in e for e in errors),
                (bad_repo, errors),
            )

    def test_truncated_github_api_tree_rejected(self):
        tree_sha = "b" * 40

        def fake_fetch(_url):
            return {"sha": tree_sha, "truncated": True, "tree": []}

        with self.assertRaises(ValueError) as ctx:
            validator.github_api_tree("bhrumom/gpui-telegram", tree_sha, fetch_json=fake_fetch)
        self.assertIn("truncated", str(ctx.exception))

    def test_non_zero_gitlinks_or_blob_count_drift_rejected(self):
        base_tree, lock = make_base_tree_and_lock(file_ledger_mode="legacy-predecessor-archived")

        ok_errors = []
        validator.verify_tree_entries(base_tree, lock, ok_errors, actual_head=lock["commit"], actual_tree=lock["rootTreeSha"])
        self.assertEqual(ok_errors, [])

        # Drift 1: non-zero gitlink
        with_gitlink = dict(base_tree)
        with_gitlink["submod"] = ("160000", "commit", "d" * 40, None)
        errors = []
        validator.verify_tree_entries(with_gitlink, lock, errors, actual_head=lock["commit"], actual_tree=lock["rootTreeSha"])
        self.assertTrue(any("gitlink count drift" in e for e in errors), errors)

        # Drift 2: blob count drift
        drifted_blobs = dict(base_tree)
        drifted_blobs["EXTRA.md"] = ("100644", "blob", "e" * 40, 10)
        errors = []
        validator.verify_tree_entries(drifted_blobs, lock, errors, actual_head=lock["commit"], actual_tree=lock["rootTreeSha"])
        self.assertTrue(any("full tracked Desktop blob count drift" in e for e in errors), errors)

    def test_tree_digests_fail_closed_on_null_or_malformed(self):
        base_tree, lock = make_base_tree_and_lock(file_ledger_mode="legacy-predecessor-archived")

        # Null fullTreeSha256 must fail closed in verify_tree_entries AND verify_snapshot
        null_full = copy.deepcopy(lock)
        null_full["inventory"]["fullTreeSha256"] = None
        errors = []
        validator.verify_tree_entries(base_tree, null_full, errors, actual_head=lock["commit"], actual_tree=lock["rootTreeSha"])
        self.assertTrue(any("full tracked Desktop root tree inventory SHA256" in e for e in errors), errors)

        # Null selectedTreeSha256 must fail closed in verify_tree_entries
        null_sel = copy.deepcopy(lock)
        null_sel["inventory"]["selectedTreeSha256"] = None
        errors = []
        validator.verify_tree_entries(base_tree, null_sel, errors, actual_head=lock["commit"], actual_tree=lock["rootTreeSha"])
        self.assertTrue(any("selected crates/** inventory SHA256" in e for e in errors), errors)

        # Mismatched digest must fail closed
        wrong_digest = copy.deepcopy(lock)
        wrong_digest["inventory"]["fullTreeSha256"] = "f" * 64
        errors = []
        validator.verify_tree_entries(base_tree, wrong_digest, errors, actual_head=lock["commit"], actual_tree=lock["rootTreeSha"])
        self.assertTrue(any("full tracked Desktop root tree inventory SHA256 drift" in e for e in errors), errors)

        # verify_snapshot also rejects null digests in legacy-predecessor-archived mode
        mf = {
            "authority": {
                "repository": "bhrumom/fabushi-desktop",
                "branch": "main",
                "commit": "3" * 40,
                "tree": "4" * 40,
                "mode": "live-main",
            },
            "fileCount": 2,
        }
        ld = {"source": mf["authority"], "rowCount": 2}
        snap_errors = []
        validator.verify_snapshot(null_full, mf, ld, snap_errors)
        self.assertTrue(any("inventory.fullTreeSha256" in e for e in snap_errors), snap_errors)
        snap_errors_sel = []
        validator.verify_snapshot(null_sel, mf, ld, snap_errors_sel)
        self.assertTrue(any("inventory.selectedTreeSha256" in e for e in snap_errors_sel), snap_errors_sel)

    def test_forbidden_gpui_and_egl_deps_rejected_without_false_positive_on_gpui_telegram(self):
        with tempfile.TemporaryDirectory() as tmp:
            core_root = pathlib.Path(tmp)
            crate_dir = core_root / "crates/gpui_telegram_core"
            crate_dir.mkdir(parents=True)
            (crate_dir / "Cargo.toml").write_text('[package]\nname = "gpui_telegram_core"\nversion = "1.6.7"\n', encoding="utf-8")

            good_toml = core_root / "Cargo.toml"
            good_toml.write_text(
                '[workspace]\nmembers = ["crates/gpui_telegram_core"]\n'
                '[workspace.dependencies]\n'
                'gpui_telegram_core = { path = "crates/gpui_telegram_core" }\n'
                'serde = "1"\n',
                encoding="utf-8",
            )
            errors = []
            validator.verify_cargo_toml_hygiene(good_toml, core_root, validator.FORBIDDEN_MOBILE_DEPS, errors)
            self.assertEqual(errors, [])

            bad_toml = core_root / "BadCargo.toml"
            bad_toml.write_text(
                '[dependencies]\n'
                'gpui = "0.2.2"\n'
                'khronos-egl = "6.0.0"\n'
                'my_alias = { package = "gpui", version = "0.2.2" }\n',
                encoding="utf-8",
            )
            bad_errors = []
            validator.verify_cargo_toml_hygiene(bad_toml, core_root, validator.FORBIDDEN_MOBILE_DEPS, bad_errors)
            self.assertGreaterEqual(len(bad_errors), 3, bad_errors)
            self.assertTrue(any("gpui" in e for e in bad_errors))
            self.assertTrue(any("khronos-egl" in e for e in bad_errors))

    def test_external_or_escaping_cargo_path_dep_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            core_root = pathlib.Path(tmp) / "desktop_core"
            core_root.mkdir()
            bad_toml = core_root / "Cargo.toml"
            bad_toml.write_text(
                '[dependencies]\n'
                'ext_rel = { path = "../../../gpui-telegram/crates/gpui_telegram_core" }\n'
                'ext_abs = { path = "/Users/dev/gpui-telegram/crates/gpui_telegram_core" }\n'
                'ext_git = { git = "https://github.com/bhrumom/gpui-telegram.git" }\n'
                'ext_file_git = { git = "file:///tmp/repo" }\n',
                encoding="utf-8",
            )
            errors = []
            validator.verify_cargo_toml_hygiene(bad_toml, core_root, validator.FORBIDDEN_MOBILE_DEPS, errors)
            self.assertTrue(any("escapes" in e for e in errors), errors)
            self.assertTrue(any("absolute path dependency forbidden" in e for e in errors), errors)
            self.assertTrue(any("external Fabushi git dependency forbidden" in e for e in errors), errors)
            self.assertTrue(any("local file:// git dependency forbidden" in e for e in errors), errors)

    def test_cargo_toml_hygiene_rejects_patch_replace_target_normalized_names_and_file_git(self):
        with tempfile.TemporaryDirectory() as tmp:
            core_root = pathlib.Path(tmp) / "desktop_core"
            core_root.mkdir()

            # 1. [patch.crates-io] injection (escaping path, forbidden git, file:// git, forbidden package)
            patch_toml = core_root / "PatchCargo.toml"
            patch_toml.write_text(
                '[patch.crates-io]\n'
                'esc = { path = "../../outside" }\n'
                'org = { git = "git@github.com:bhrumom/gpui-telegram.git" }\n'
                'fgit = { git = "FILE:///tmp/evil-repo" }\n'
                'sneaky = { package = "GPUI", version = "0.2.2" }\n',
                encoding="utf-8",
            )
            patch_errors = []
            validator.verify_cargo_toml_hygiene(patch_toml, core_root, validator.FORBIDDEN_MOBILE_DEPS, patch_errors)
            self.assertTrue(any("patch.crates-io" in e and "escapes" in e for e in patch_errors), patch_errors)
            self.assertTrue(any("patch.crates-io" in e and "external Fabushi git" in e for e in patch_errors), patch_errors)
            self.assertTrue(any("patch.crates-io" in e and "file:// git" in e for e in patch_errors), patch_errors)
            self.assertTrue(any("patch.crates-io" in e and "forbidden mobile dependency" in e for e in patch_errors), patch_errors)

            # 2. [replace] injection
            replace_toml = core_root / "ReplaceCargo.toml"
            replace_toml.write_text(
                '[replace]\n'
                '"gpui:0.2.2" = { path = "../../outside" }\n'
                '"openssl-sys:0.9.100" = { git = "file:///tmp/ssl" }\n',
                encoding="utf-8",
            )
            replace_errors = []
            validator.verify_cargo_toml_hygiene(replace_toml, core_root, validator.FORBIDDEN_MOBILE_DEPS, replace_errors)
            self.assertTrue(any("[replace]" in e and "gpui:0.2.2" in e for e in replace_errors), replace_errors)
            self.assertTrue(any("[replace]" in e and "openssl-sys:0.9.100" in e for e in replace_errors), replace_errors)
            self.assertTrue(any("replace" in e and "escapes" in e for e in replace_errors), replace_errors)
            self.assertTrue(any("replace" in e and "file:// git" in e for e in replace_errors), replace_errors)

            # 3. [target.'cfg(...)'.dependencies] and Cargo crate-name normalization (- vs _, case)
            norm_toml = core_root / "NormCargo.toml"
            norm_toml.write_text(
                '[dependencies]\n'
                'openssl_sys = "0.9"\n'
                'GPUI = "0.2"\n'
                'Khronos-EGL = "6.0"\n'
                'gpui-telegram-desktop = "1.6.7"\n'
                '[target.\'cfg(target_os = "ios")\'.dependencies]\n'
                'OpenSSL_Sys = "0.9"\n',
                encoding="utf-8",
            )
            norm_errors = []
            validator.verify_cargo_toml_hygiene(norm_toml, core_root, validator.FORBIDDEN_MOBILE_DEPS, norm_errors)
            self.assertEqual(len(norm_errors), 5, norm_errors)
            self.assertTrue(any("openssl_sys" in e for e in norm_errors), norm_errors)
            self.assertTrue(any("GPUI" in e for e in norm_errors), norm_errors)
            self.assertTrue(any("Khronos-EGL" in e for e in norm_errors), norm_errors)
            self.assertTrue(any("gpui-telegram-desktop" in e for e in norm_errors), norm_errors)
            self.assertTrue(any("target.cfg(target_os = \"ios\").dependencies" in e for e in norm_errors), norm_errors)

    def test_desktop_core_workspace_rejects_escaping_or_excluded_members_and_scans_nested_target(self):
        with tempfile.TemporaryDirectory() as tmp:
            core_root = pathlib.Path(tmp) / "desktop_core"
            (core_root / "crates/gpui_telegram_core").mkdir(parents=True)
            (core_root / "crates/gpui_telegram_core/Cargo.toml").write_text(
                '[package]\nname = "gpui_telegram_core"\nversion = "1.6.7"\n', encoding="utf-8"
            )
            (core_root / "crates/gpui_telegram_common").mkdir(parents=True)

            lock = make_valid_v3_lock()
            valid_ws = (
                '[workspace]\n'
                'members = ["crates/gpui_telegram_core"]\n'
                '[workspace.package]\n'
                'edition = "2024"\n'
                '[profile.dev]\n'
                'debug = 0\n'
                'incremental = false\n'
                '[profile.test]\n'
                'debug = 0\n'
                'incremental = false\n'
            )
            (core_root / "Cargo.toml").write_text(valid_ws, encoding="utf-8")

            # Top-level target/ should be ignored even if a stale Cargo.toml exists inside it
            (core_root / "target/debug").mkdir(parents=True)
            (core_root / "target/debug/Cargo.toml").write_text('[dependencies]\ngpui = "0.2"\n', encoding="utf-8")

            ok_errors = []
            validator.verify_desktop_core_workspace(core_root, lock, ok_errors)
            self.assertEqual(ok_errors, [])

            # Escaping, absolute, and excluded-crate members (including ./ prefix and hyphenated spelling) fail closed
            bad_ws = valid_ws.replace(
                'members = ["crates/gpui_telegram_core"]',
                'members = ["crates/gpui_telegram_core", "../../outside", "/tmp/evil_crate", "./crates/gpui_telegram_desktop", "crates/gpui-telegram-server"]',
            )
            (core_root / "Cargo.toml").write_text(bad_ws, encoding="utf-8")
            ws_errors = []
            validator.verify_desktop_core_workspace(core_root, lock, ws_errors)
            self.assertTrue(any("workspace member escapes" in e for e in ws_errors), ws_errors)
            self.assertTrue(any("workspace member absolute path forbidden" in e for e in ws_errors), ws_errors)
            self.assertTrue(any("./crates/gpui_telegram_desktop" in e for e in ws_errors), ws_errors)
            self.assertTrue(any("crates/gpui-telegram-server" in e for e in ws_errors), ws_errors)

            # Nested crate named "target" under crates/ MUST be scanned (not skipped by top-level target skip)
            (core_root / "Cargo.toml").write_text(valid_ws, encoding="utf-8")
            (core_root / "crates/target").mkdir(parents=True)
            (core_root / "crates/target/Cargo.toml").write_text(
                '[package]\nname = "nested_target"\nversion = "0.1.0"\n[dependencies]\ngpui = "0.2"\n',
                encoding="utf-8",
            )
            nested_errors = []
            validator.verify_desktop_core_workspace(core_root, lock, nested_errors)
            self.assertTrue(any("forbidden mobile dependency 'gpui'" in e for e in nested_errors), nested_errors)

    def test_invalid_local_test_ready_rejected_and_ci_verified_accepted(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            # Absent TEST_READY.md passes cleanly
            errors = []
            validator.verify_test_ready_integrity(root, errors)
            self.assertEqual(errors, [])

            # Invalid local developer paths & missing CI provenance fail closed
            (root / "TEST_READY.md").write_text(
                "Run locally at /Users/bhrum/Documents/ChatGPT/fabushi-ios\nSigned off by: worker_m6_iter3_impl\n",
                encoding="utf-8",
            )
            bad_errors = []
            validator.verify_test_ready_integrity(root, bad_errors)
            self.assertTrue(any("local host path" in e for e in bad_errors), bad_errors)
            self.assertTrue(any("GitHub Actions run provenance" in e for e in bad_errors), bad_errors)

            # Valid CI-verified TEST_READY.md with anchored GitHub Actions URL and non-zero 40-hex SHA passes
            (root / "TEST_READY.md").write_text(
                "# E2E Test Readiness\n"
                "Exact HEAD: 6141456e2a1121b78a6cdcd5da2a6d24c9c37e15\n"
                "GitHub Actions Run: https://github.com/fabushiios/fabushi-ios/actions/runs/45114476960\n",
                encoding="utf-8",
            )
            good_errors = []
            validator.verify_test_ready_integrity(root, good_errors)
            self.assertEqual(good_errors, [])

            # Valid CI-verified TEST_READY.md with explicit numeric GITHUB_RUN_ID and non-zero 40-hex SHA passes
            (root / "TEST_READY.md").write_text(
                "# E2E Test Readiness\n"
                "Exact HEAD: f14b4c4c732fa145452a74b4d647e35f4098e62a\n"
                "GITHUB_RUN_ID=45114476960\n",
                encoding="utf-8",
            )
            good_id_errors = []
            validator.verify_test_ready_integrity(root, good_id_errors)
            self.assertEqual(good_id_errors, [])

    def test_test_ready_rejects_spoofed_urls_placeholders_zero_sha_local_variants_and_head_mismatch(self):
        valid_sha = "f14b4c4c732fa145452a74b4d647e35f4098e62a"
        valid_url = "https://github.com/fabushiios/fabushi-ios/actions/runs/45114476960"
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            provenance_failures = [
                f"Run: https://spoof.invalid/github.com/fabushiios/fabushi-ios/actions/runs/TBD\nSHA: {valid_sha}\n",
                f"Run: https://evil.example/https://github.com/fabushiios/fabushi-ios/actions/runs/45114476960\nSHA: {valid_sha}\n",
                f"Run: https://github.com/fabushiios/fabushi-ios/actions/runs/TBD\nSHA: {valid_sha}\n",
                f"Run: https://github.com/fabushiios/fabushi-ios/actions/runs/0\nSHA: {valid_sha}\n",
                f"GITHUB_RUN_ID: none\nSHA: {valid_sha}\n",
                f"No GITHUB_RUN_ID (executed locally)\nSHA: {valid_sha}\n",
                f"Run: {valid_url}\nSHA: {'0' * 40}\n",
            ]
            for idx, payload in enumerate(provenance_failures):
                (root / "TEST_READY.md").write_text(payload, encoding="utf-8")
                errs = []
                validator.verify_test_ready_integrity(root, errs)
                self.assertTrue(errs, f"case {idx} unexpectedly passed: {payload!r}")

            local_and_stale_payloads = [
                f"Run: {valid_url}\nSHA: {valid_sha}\nPath: C:/Users/dev/fabushi-ios\n",
                f"Run: {valid_url}\nSHA: {valid_sha}\nPath: c:\\users\\dev\\fabushi-ios\n",
                f"Run: {valid_url}\nSHA: {valid_sha}\nExecuted in /home/dev\n",
                f"Run: {valid_url}\nSHA: {valid_sha}\nWorkspace ~/repo\n",
                f"Run: {valid_url}\nSHA: {valid_sha}\nBinary: target/e2e/master_e2e_runner\n",
                f"Run: {valid_url}\nSHA: {valid_sha}\nUpstream: bhrunom/gpui-telegram\n",
                f"Run: {valid_url}\nSHA: {valid_sha}\nUpstream: bhrum/gpui-telegram\n",
            ]
            for idx, payload in enumerate(local_and_stale_payloads):
                (root / "TEST_READY.md").write_text(payload, encoding="utf-8")
                errs = []
                validator.verify_test_ready_integrity(root, errs)
                self.assertTrue(errs, f"local/stale case {idx} unexpectedly passed: {payload!r}")

            # complete=True enforces IOS_EXPECTED_HEAD binding when env var is set
            (root / "TEST_READY.md").write_text(
                f"Run: {valid_url}\nSHA: {valid_sha}\n", encoding="utf-8"
            )
            with mock.patch.dict(os.environ, {"IOS_EXPECTED_HEAD": valid_sha}):
                errs_ok = []
                validator.verify_test_ready_integrity(root, errs_ok, complete=True)
                self.assertEqual(errs_ok, [])
            with mock.patch.dict(os.environ, {"IOS_EXPECTED_HEAD": "1" * 40}):
                errs_mismatch = []
                validator.verify_test_ready_integrity(root, errs_mismatch, complete=True)
                self.assertTrue(any("IOS_EXPECTED_HEAD" in e for e in errs_mismatch), errs_mismatch)

    def test_release_evidence_rejects_null_ios_commit_when_env_unset_and_empty_release_gates(self):
        real_lock = validator.document(validator.LOCK)
        real_mf = validator.document(validator.MANIFEST_INDEX)
        real_ld = validator.document(validator.LEDGER_INDEX)
        real_release = validator.document(validator.ROOT / "docs/parity/desktop-main-release-evidence.json")
        orig_document = validator.document

        # Case 1: status="passed" with iosCommit=None while IOS_EXPECTED_HEAD is unset MUST fail closed
        mutated_ac = copy.deepcopy(real_release)
        mutated_ac["acceptanceCriteria"][0].update({
            "status": "passed",
            "iosCommit": None,
            "evidence": ["fake-evidence"],
            "independentReviewer": "reviewer",
        })

        def fake_doc_ac(path):
            if pathlib.Path(path).name == "desktop-main-release-evidence.json":
                return mutated_ac
            return orig_document(path)

        env_without_head = {k: v for k, v in os.environ.items() if k != "IOS_EXPECTED_HEAD"}
        with mock.patch.dict(os.environ, env_without_head, clear=True), \
             mock.patch.object(validator, "document", side_effect=fake_doc_ac):
            errs = []
            warns = []
            validator.verify_files(real_lock, real_mf, real_ld, errs, warns, False, None)
            self.assertTrue(any("not tied to current iOS exact head" in e for e in errs), errs)

        # Case 2: empty releaseGates: {} MUST fail closed in BOTH non-complete (--strict) and --complete
        mutated_empty_gates = copy.deepcopy(real_release)
        mutated_empty_gates["releaseGates"] = {}

        def fake_doc_empty(path):
            if pathlib.Path(path).name == "desktop-main-release-evidence.json":
                return mutated_empty_gates
            return orig_document(path)

        with mock.patch.object(validator, "document", side_effect=fake_doc_empty):
            for complete_flag in (False, True):
                errs = []
                warns = []
                validator.verify_files(real_lock, real_mf, real_ld, errs, warns, complete_flag, None)
                self.assertTrue(
                    any("releaseGates must match exact 13 canonical keys" in e for e in errs),
                    (complete_flag, errs),
                )

    def test_host_anti_stub_whitespace_insensitive_across_all_host_rs_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            host_src = root / "source/host/src"
            host_src.mkdir(parents=True)
            (host_src / "dispatch.rs").write_text('pub fn dispatch() -> &str { r#"{"ok":false}"# }\n', encoding="utf-8")
            (host_src / "other.rs").write_text('pub fn helper() -> i32 { 42 }\n', encoding="utf-8")

            ok_errors = []
            validator.verify_host_anti_stub(root, ok_errors)
            self.assertEqual(ok_errors, [])

            # Standard spacing in dispatch.rs fails closed
            (host_src / "dispatch.rs").write_text('let s = r#"{"status": "acknowledged"}"#;\n', encoding="utf-8")
            errs_spaced = []
            validator.verify_host_anti_stub(root, errs_spaced)
            self.assertTrue(any("dispatch.rs" in e and "acknowledged" in e for e in errs_spaced), errs_spaced)

            # Compact whitespace variant in a DIFFERENT .rs file under source/host/src/ fails closed
            (host_src / "dispatch.rs").write_text('pub fn dispatch() {}\n', encoding="utf-8")
            (host_src / "other.rs").write_text('let s = r#"{"status":"acknowledged"}"#;\n', encoding="utf-8")
            errs_compact = []
            validator.verify_host_anti_stub(root, errs_compact)
            self.assertTrue(any("other.rs" in e and "acknowledged" in e for e in errs_compact), errs_compact)

            # sample_chats in another .rs file fails closed
            (host_src / "other.rs").write_text('let chats = sample_chats();\n', encoding="utf-8")
            errs_sample = []
            validator.verify_host_anti_stub(root, errs_sample)
            self.assertTrue(any("other.rs" in e and "sample_chats" in e for e in errs_sample), errs_sample)

    def test_strict_authority_only_invokes_verify_ios_owners_and_host_cargo_hygiene(self):
        real_lock = validator.document(validator.LOCK)

        # verify_ios_owners inspects source/host/Cargo.toml via verify_cargo_toml_hygiene cleanly
        real_hygiene = validator.verify_cargo_toml_hygiene
        owner_errors = []
        with mock.patch.object(validator, "verify_cargo_toml_hygiene", wraps=real_hygiene) as spy_hygiene:
            validator.verify_ios_owners(owner_errors, real_lock)
        self.assertEqual(owner_errors, [])
        checked_paths = [call.args[0] for call in spy_hygiene.call_args_list]
        self.assertIn(validator.ROOT / "source/host/Cargo.toml", checked_paths)

        # --strict --authority-only genuinely dispatches verify_ios_owners
        real_verify_owners = validator.verify_ios_owners
        with mock.patch.object(sys, "argv", ["check-desktop-main-ios-architecture.py", "--strict", "--authority-only"]), \
             mock.patch.object(validator, "live_head", return_value=real_lock["commit"]), \
             mock.patch.object(validator, "verify_ios_owners", wraps=real_verify_owners) as spy_owners, \
             contextlib.redirect_stdout(io.StringIO()):
            rc = validator.run()
        self.assertEqual(rc, 0)
        spy_owners.assert_called_once()

    def test_complete_mode_rejects_archived_legacy_file_ledger_mode(self):
        # Non-complete with closure flags=False passes snapshot validation
        data = make_valid_v3_lock(file_ledger_mode="legacy-predecessor-archived")
        mf = {
            "authority": {
                "repository": "bhrumom/fabushi-desktop",
                "branch": "main",
                "commit": "3" * 40,
                "tree": "4" * 40,
                "mode": "live-main",
            },
            "fileCount": 2,
        }
        ld = {"source": mf["authority"], "rowCount": 2}
        errors = []
        validator.verify_snapshot(data, mf, ld, errors)
        self.assertEqual(errors, [])

        # Setting baselineReady=True while archived fails closed
        bad_flag_data = make_valid_v3_lock(file_ledger_mode="legacy-predecessor-archived")
        bad_flag_data["baselineReady"] = True
        flag_errors = []
        validator.verify_snapshot(bad_flag_data, mf, ld, flag_errors)
        self.assertTrue(any("premature closure flag" in e for e in flag_errors), flag_errors)

        # --complete mode unconditionally rejects legacy-predecessor-archived
        real_lock = validator.document(validator.LOCK)
        real_mf = validator.document(validator.MANIFEST_INDEX)
        real_ld = validator.document(validator.LEDGER_INDEX)
        complete_errors = []
        complete_warnings = []
        validator.verify_files(real_lock, real_mf, real_ld, complete_errors, complete_warnings, True, None)
        self.assertTrue(
            any("requires inventory.fileLedgerMode == 'current-main-complete'" in e for e in complete_errors),
            complete_errors,
        )


if __name__ == "__main__":
    unittest.main()
