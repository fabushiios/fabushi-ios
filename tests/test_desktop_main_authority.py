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
                "Exact HEAD: 57a62ef92be2fa816704d7337aca3254dc8f943e\n"
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

        # verify_ios_owners inspects source/host/Cargo.toml and invokes verify_host_anti_stub cleanly
        real_hygiene = validator.verify_cargo_toml_hygiene
        real_anti_stub = validator.verify_host_anti_stub
        owner_errors = []
        with mock.patch.object(validator, "verify_cargo_toml_hygiene", wraps=real_hygiene) as spy_hygiene, \
             mock.patch.object(validator, "verify_host_anti_stub", wraps=real_anti_stub) as spy_anti_stub:
            validator.verify_ios_owners(owner_errors, real_lock)
        self.assertEqual(owner_errors, [])
        checked_paths = [call.args[0] for call in spy_hygiene.call_args_list]
        self.assertIn(validator.ROOT / "source/host/Cargo.toml", checked_paths)
        spy_anti_stub.assert_called_once()

        # --strict --authority-only genuinely dispatches verify_ios_owners + verify_host_anti_stub
        real_verify_owners = validator.verify_ios_owners
        with mock.patch.object(sys, "argv", ["check-desktop-main-ios-architecture.py", "--strict", "--authority-only"]), \
             mock.patch.object(validator, "live_head", return_value=real_lock["commit"]), \
             mock.patch.object(validator, "verify_ios_owners", wraps=real_verify_owners) as spy_owners, \
             mock.patch.object(validator, "verify_host_anti_stub", wraps=real_anti_stub) as spy_stub_cli, \
             contextlib.redirect_stdout(io.StringIO()):
            rc = validator.run()
        self.assertEqual(rc, 0)
        spy_owners.assert_called_once()
        spy_stub_cli.assert_called_once()

        # Negative sub-assertion: stub error in verify_host_anti_stub causes --strict to fail closed
        def inject_stub_error(_root, errs):
            errs.append("injected stub error")

        with mock.patch.object(sys, "argv", ["check-desktop-main-ios-architecture.py", "--strict", "--authority-only"]), \
             mock.patch.object(validator, "live_head", return_value=real_lock["commit"]), \
             mock.patch.object(validator, "verify_host_anti_stub", side_effect=inject_stub_error), \
             contextlib.redirect_stdout(io.StringIO()):
            rc_fail = validator.run()
        self.assertEqual(rc_fail, 1)

    def test_host_cargo_manifests_missing_and_recursive_subcrate_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "source/host").mkdir(parents=True)

            # Case 1: missing source/host/Cargo.toml fails closed
            errs_missing = []
            validator.verify_host_cargo_manifests(root, validator.FORBIDDEN_MOBILE_DEPS, errs_missing)
            self.assertTrue(
                any("missing C-ABI host manifest: source/host/Cargo.toml" in e for e in errs_missing),
                errs_missing,
            )

            # Case 2: valid root manifest + top-level source/host/target/ skipped, nested subcrate rejected
            (root / "source/host/Cargo.toml").write_text(
                '[package]\nname = "ok"\nversion = "1.0.0"\n[dependencies]\nserde = "1"\n',
                encoding="utf-8",
            )
            (root / "source/host/target/debug").mkdir(parents=True)
            (root / "source/host/target/debug/Cargo.toml").write_text(
                '[package]\nname = "build-scratch"\nversion = "0.1.0"\n[dependencies]\ngpui = "0.2"\n',
                encoding="utf-8",
            )
            errs_ok = []
            validator.verify_host_cargo_manifests(root, validator.FORBIDDEN_MOBILE_DEPS, errs_ok)
            self.assertEqual(errs_ok, [])

            # Nested subcrate under source/host/subcrate/Cargo.toml with forbidden dep fails closed
            (root / "source/host/subcrate").mkdir(parents=True)
            (root / "source/host/subcrate/Cargo.toml").write_text(
                '[package]\nname = "nested"\nversion = "0.1.0"\n[dependencies]\ngpui = "0.2"\n',
                encoding="utf-8",
            )
            errs_nested = []
            validator.verify_host_cargo_manifests(root, validator.FORBIDDEN_MOBILE_DEPS, errs_nested)
            self.assertTrue(any("forbidden mobile dependency" in e for e in errs_nested), errs_nested)

            # Nested target dir inside a subcrate is NOT skipped
            (root / "source/host/subcrate/Cargo.toml").unlink()
            (root / "source/host/sub/target").mkdir(parents=True)
            (root / "source/host/sub/target/Cargo.toml").write_text(
                '[package]\nname = "tricky"\nversion = "0.1.0"\n[dependencies]\nkhronos-egl = "6"\n',
                encoding="utf-8",
            )
            errs_tricky = []
            validator.verify_host_cargo_manifests(root, validator.FORBIDDEN_MOBILE_DEPS, errs_tricky)
            self.assertTrue(any("forbidden mobile dependency" in e for e in errs_tricky), errs_tricky)

    def test_cargo_toml_hygiene_rejects_underscore_dep_aliases_and_replace_at_separator(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            toml_path = root / "Cargo.toml"

            # Underscore dev_dependencies / build_dependencies at top-level and under target.*
            toml_path.write_text(
                '[package]\nname = "alias-test"\nversion = "0.1.0"\n'
                '[dev_dependencies]\ngpui = "0.2"\n'
                '[build_dependencies]\nopenssl_sys = "0.9"\n'
                '[target.\'cfg(target_os = "ios")\'.dev_dependencies]\n'
                'evil = { path = "../../outside" }\n'
                '[target.\'cfg(target_os = "ios")\'.build_dependencies]\n'
                'local_git = { git = "git+file:///tmp/evil" }\n',
                encoding="utf-8",
            )
            errs_alias = []
            validator.verify_cargo_toml_hygiene(
                toml_path, root, validator.FORBIDDEN_MOBILE_DEPS, errs_alias, require_path_exists=False
            )
            self.assertTrue(any("[dev_dependencies]" in e for e in errs_alias), errs_alias)
            self.assertTrue(any("[build_dependencies]" in e for e in errs_alias), errs_alias)
            self.assertTrue(any("escapes" in e for e in errs_alias), errs_alias)
            self.assertTrue(any("file://" in e for e in errs_alias), errs_alias)

            # Cargo 1.77+ [replace] '@' PkgId separator & forbidden replace source URLs
            toml_path.write_text(
                '[package]\nname = "replace-test"\nversion = "0.1.0"\n'
                '[replace]\n'
                '"gpui@0.2.2" = { version = "0.2.2" }\n'
                '"openssl_sys@0.9.100" = { version = "0.9.100" }\n'
                '"https://github.com/bhrumom/gpui-telegram#0.1.0" = { version = "0.1.0" }\n'
                '"git+file:///tmp/evil#serde@1.0" = { version = "1.0" }\n',
                encoding="utf-8",
            )
            errs_rep = []
            validator.verify_cargo_toml_hygiene(
                toml_path, root, validator.FORBIDDEN_MOBILE_DEPS, errs_rep, require_path_exists=False
            )
            self.assertTrue(any("gpui@0.2.2" in e for e in errs_rep), errs_rep)
            self.assertTrue(any("openssl_sys@0.9.100" in e for e in errs_rep), errs_rep)
            self.assertTrue(any("forbidden replace source URL" in e and "bhrumom" in e for e in errs_rep), errs_rep)
            self.assertTrue(any("forbidden replace source URL" in e and "file://" in e for e in errs_rep), errs_rep)

    def test_release_evidence_rejects_premature_final_acceptance_in_non_complete_mode(self):
        real_lock = validator.document(validator.LOCK)
        real_release = validator.document(validator.ROOT / "docs/parity/desktop-main-release-evidence.json")

        # Positive control: finalAcceptance=false passes in complete=False
        errs_ok = []
        validator.verify_release_evidence(real_release, real_lock, errs_ok, complete=False)
        self.assertEqual(errs_ok, [])

        # Negative: finalAcceptance=true while archived / pending ACs / mismatched finalIosCommit MUST fail closed
        mutated = copy.deepcopy(real_release)
        mutated["finalAcceptance"] = True
        mutated["finalIosCommit"] = None
        errs_bad = []
        with mock.patch.dict(os.environ, {"IOS_EXPECTED_HEAD": "a" * 40}):
            validator.verify_release_evidence(mutated, real_lock, errs_bad, complete=False)
        self.assertTrue(any("premature finalAcceptance=true" in e for e in errs_bad), errs_bad)
        self.assertTrue(any("finalIosCommit" in e for e in errs_bad), errs_bad)
        self.assertTrue(any("IOS-MAIN-AC-01" in e for e in errs_bad), errs_bad)

    def test_host_anti_stub_rejects_escaped_quotes_and_localhost_without_scanning_tests_dir(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            host_src = root / "source/host/src"
            host_tests = root / "source/host/tests"
            host_src.mkdir(parents=True)
            host_tests.mkdir(parents=True)

            (host_src / "dispatch.rs").write_text('pub fn dispatch() -> &str { "{}" }\n', encoding="utf-8")
            # Integration tests under source/host/tests/ may safely contain negative anti-stub assertions
            (host_tests / "anti_stub.rs").write_text(
                'assert!(!resp.contains(r#"{"status": "acknowledged"}"#));\n'
                'assert!(!resp.contains("sample_chats"));\n'
                'assert!(!resp.contains("https://127.0.0.1:8443"));\n',
                encoding="utf-8",
            )
            errs_clean = []
            validator.verify_host_anti_stub(root, errs_clean)
            self.assertEqual(errs_clean, [])

            # Escaped JSON quotes inside source/host/src/ MUST fail closed
            (host_src / "dispatch.rs").write_text(
                r'let s = "{\"status\":\"acknowledged\"}";' + "\n", encoding="utf-8"
            )
            errs_esc = []
            validator.verify_host_anti_stub(root, errs_esc)
            self.assertTrue(any("acknowledged" in e for e in errs_esc), errs_esc)

            # Hardcoded localhost:8443 inside source/host/src/ MUST fail closed
            (host_src / "dispatch.rs").write_text(
                'let url = "https://127.0.0.1:8443";\n', encoding="utf-8"
            )
            errs_local = []
            validator.verify_host_anti_stub(root, errs_local)
            self.assertTrue(any("127.0.0.1:8443" in e for e in errs_local), errs_local)

    def test_test_ready_complete_mode_rejects_unset_or_malformed_ios_expected_head(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            sha = "a" * 40
            (root / "TEST_READY.md").write_text(
                f"Verified on https://github.com/fabushiios/fabushi-ios/actions/runs/987654321 for {sha}\n",
                encoding="utf-8",
            )
            env_without_head = {k: v for k, v in os.environ.items() if k != "IOS_EXPECTED_HEAD"}
            with mock.patch.dict(os.environ, env_without_head, clear=True):
                errs_unset = []
                validator.verify_test_ready_integrity(root, errs_unset, complete=True)
                self.assertTrue(any("valid 40-hex IOS_EXPECTED_HEAD" in e for e in errs_unset), errs_unset)

            with mock.patch.dict(os.environ, {"IOS_EXPECTED_HEAD": "not-a-sha"}):
                errs_malformed = []
                validator.verify_test_ready_integrity(root, errs_malformed, complete=True)
                self.assertTrue(any("valid 40-hex IOS_EXPECTED_HEAD" in e for e in errs_malformed), errs_malformed)

            with mock.patch.dict(os.environ, {"IOS_EXPECTED_HEAD": sha}):
                errs_ok = []
                validator.verify_test_ready_integrity(root, errs_ok, complete=True)
                self.assertEqual(errs_ok, [])

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

    def test_cargo_replace_rejects_path_file_url_without_hash_and_url_version_shorthand(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            toml_path = root / "Cargo.toml"
            toml_path.write_text(
                '[package]\nname = "replace-edge"\nversion = "0.1.0"\n'
                '[replace]\n'
                '"path+file:///tmp/evil#serde@1.0.0" = { version = "1.0.0" }\n'
                '"git+file:///tmp/evil@1.0.0" = { version = "1.0.0" }\n'
                '"https://github.com/bhrumom/other@1.0.0" = { version = "1.0.0" }\n'
                '"https://github.com/someone/gpui#0.2.2" = { version = "0.2.2" }\n'
                '"https://github.com/someone/openssl-sys.git@0.9.100" = { version = "0.9.100" }\n',
                encoding="utf-8",
            )
            errs = []
            validator.verify_cargo_toml_hygiene(
                toml_path, root, validator.FORBIDDEN_MOBILE_DEPS, errs, require_path_exists=False
            )
            self.assertTrue(any("path+file://" in e for e in errs), errs)
            self.assertTrue(any("git+file:///tmp/evil@1.0.0" in e for e in errs), errs)
            self.assertTrue(any("bhrumom/other@1.0.0" in e for e in errs), errs)
            self.assertTrue(any("someone/gpui#0.2.2" in e and "forbidden mobile dependency" in e for e in errs), errs)
            self.assertTrue(
                any("openssl-sys.git@0.9.100" in e and "forbidden mobile dependency" in e for e in errs),
                errs,
            )

    def test_host_anti_stub_rejects_local_paths_in_clean_build_garbage(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "source/host/src").mkdir(parents=True)
            (root / "source/host/src/dispatch.rs").write_text('pub fn dispatch() {}\n', encoding="utf-8")
            (root / "scripts").mkdir(parents=True)

            for idx, bad_line in enumerate([
                'TARGET = "/Users/dev/project/target"\n',
                'TARGET = "/home/alice/project/target"\n',
                'TARGET = "C:\\Users\\dev\\target"\n',
                'TARGET = "~/project/target"\n',
            ]):
                (root / "scripts/clean_build_garbage.py").write_text(bad_line, encoding="utf-8")
                errs = []
                validator.verify_host_anti_stub(root, errs)
                self.assertTrue(
                    any("clean_build_garbage.py" in e for e in errs),
                    f"case {idx} unexpectedly passed: {bad_line!r} -> {errs}",
                )


    def test_cargo_replace_url_userinfo_at_parsing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            toml_path = root / "Cargo.toml"
            toml_path.write_text(
                '[package]\nname = "replace-userinfo"\nversion = "0.1.0"\n'
                '[replace]\n'
                '"ssh://git@github.com/someone/gpui@0.2.2" = { version = "0.2.2" }\n'
                '"https://user@example.com/someone/gpui@0.2.2" = { version = "0.2.2" }\n'
                '"ssh://git@github.com/someone/openssl-sys.git@0.9.100" = { version = "0.9.100" }\n',
                encoding="utf-8",
            )
            errs = []
            validator.verify_cargo_toml_hygiene(
                toml_path, root, validator.FORBIDDEN_MOBILE_DEPS, errs, require_path_exists=False
            )
            self.assertTrue(
                any("ssh://git@github.com/someone/gpui@0.2.2" in e and "forbidden mobile dependency" in e for e in errs),
                errs,
            )
            self.assertTrue(
                any("https://user@example.com/someone/gpui@0.2.2" in e and "forbidden mobile dependency" in e for e in errs),
                errs,
            )
            self.assertTrue(
                any("ssh://git@github.com/someone/openssl-sys.git@0.9.100" in e and "forbidden mobile dependency" in e for e in errs),
                errs,
            )

    def test_ios_swift_anti_mock_rejects_fake_accounts_keys_mock_bridge_and_skips_tests(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            core_dir = root / "mobile/ios/Fabushi/Core"
            bridge_dir = root / "mobile/ios/Fabushi/Bridge"
            tests_dir = root / "mobile/ios/FabushiTests"
            core_dir.mkdir(parents=True)
            bridge_dir.mkdir(parents=True)
            tests_dir.mkdir(parents=True)

            (core_dir / "FabushiRustHostActor.swift").write_text("public actor FabushiRustHostActor {}\n", encoding="utf-8")
            (core_dir / "FabushiCoordinator.swift").write_text("public final class FabushiCoordinator {}\n", encoding="utf-8")
            (bridge_dir / "AccountScopeManager.swift").write_text("public actor AccountScopeManager {}\n", encoding="utf-8")
            # Negative fixtures inside FabushiTests MUST NOT trigger production Swift anti-mock errors
            (tests_dir / "NegativeFixtureTests.swift").write_text(
                'let a = "acc_primary"; let k = Data(repeating: 0xAA, count: 32); let m = mockBridge\n',
                encoding="utf-8",
            )
            (root / "mobile/ios/project.yml").write_text(
                "NSAppTransportSecurity:\n  NSAllowsArbitraryLoads: false\n  NSAllowsLocalNetworking: true\n",
                encoding="utf-8",
            )

            clean_errs = []
            validator.verify_ios_swift_anti_mock(root, clean_errs)
            self.assertEqual(clean_errs, [])

            for label, bad_snippet, expected_substr in [
                ("acc_primary", 'let a = "acc_primary"\n', "acc_primary/acc_work"),
                ("acc_work", 'let b = "acc_work"\n', "acc_primary/acc_work"),
                ("0xAA", "let k = Data(repeating: 0xAA, count: 32)\n", "0xAA/0xBB"),
                ("0xbb", "let k = Data(repeating: 0xbb, count: 32)\n", "0xAA/0xBB"),
                ("mockBridge", "func upload(mockBridge: Bool = false) {}\n", "mockBridge/mockChunks/mock_media_"),
                ("mockChunks", "func download(mockChunks: [Data]? = nil) {}\n", "mockBridge/mockChunks/mock_media_"),
                ("mock_media_", 'let id = "mock_media_1234"\n', "mockBridge/mockChunks/mock_media_"),
            ]:
                (bridge_dir / "AccountScopeManager.swift").write_text(bad_snippet, encoding="utf-8")
                errs = []
                validator.verify_ios_swift_anti_mock(root, errs)
                self.assertTrue(
                    any(expected_substr in e for e in errs),
                    f"case {label} unexpectedly passed: {errs}",
                )

            # Missing required Core owner fails closed
            (bridge_dir / "AccountScopeManager.swift").write_text("public actor AccountScopeManager {}\n", encoding="utf-8")
            (core_dir / "FabushiCoordinator.swift").unlink()
            missing_errs = []
            validator.verify_ios_swift_anti_mock(root, missing_errs)
            self.assertTrue(
                any("FabushiCoordinator.swift" in e for e in missing_errs),
                missing_errs,
            )

    def test_ios_swift_anti_mock_enforces_project_yml_ats_and_excludes_master_e2e(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            core_dir = root / "mobile/ios/Fabushi/Core"
            core_dir.mkdir(parents=True)
            (core_dir / "FabushiRustHostActor.swift").write_text("public actor FabushiRustHostActor {}\n", encoding="utf-8")
            (core_dir / "FabushiCoordinator.swift").write_text("public final class FabushiCoordinator {}\n", encoding="utf-8")

            yml = root / "mobile/ios/project.yml"

            # Missing NSAllowsLocalNetworking: true fails closed
            yml.write_text("NSAppTransportSecurity:\n  NSAllowsArbitraryLoads: false\n", encoding="utf-8")
            errs_local = []
            validator.verify_ios_swift_anti_mock(root, errs_local)
            self.assertTrue(any("NSAllowsLocalNetworking: true" in e for e in errs_local), errs_local)

            # NSAllowsArbitraryLoads: true fails closed
            yml.write_text(
                "NSAppTransportSecurity:\n  NSAllowsArbitraryLoads: true\n  NSAllowsLocalNetworking: true\n",
                encoding="utf-8",
            )
            errs_arb = []
            validator.verify_ios_swift_anti_mock(root, errs_arb)
            self.assertTrue(any("NSAllowsArbitraryLoads: true" in e for e in errs_arb), errs_arb)

            # Re-introducing master_e2e_suite.swift in project.yml fails closed
            yml.write_text(
                "NSAppTransportSecurity:\n  NSAllowsArbitraryLoads: false\n  NSAllowsLocalNetworking: true\n"
                "sources:\n  - path: ../../tests/e2e/master_e2e_suite.swift\n",
                encoding="utf-8",
            )
            errs_e2e = []
            validator.verify_ios_swift_anti_mock(root, errs_e2e)
            self.assertTrue(any("master_e2e_suite.swift" in e for e in errs_e2e), errs_e2e)

    def test_strict_authority_only_invokes_verify_ios_swift_anti_mock(self):
        real_lock = validator.document(validator.LOCK)
        real_swift_check = validator.verify_ios_swift_anti_mock

        with mock.patch.object(sys, "argv", ["check-desktop-main-ios-architecture.py", "--strict", "--authority-only"]), \
             mock.patch.object(validator, "live_head", return_value=real_lock["commit"]), \
             mock.patch.object(validator, "verify_ios_swift_anti_mock", wraps=real_swift_check) as spy_swift, \
             contextlib.redirect_stdout(io.StringIO()):
            rc = validator.run()
        self.assertEqual(rc, 0)
        spy_swift.assert_called_once()

        def inject_swift_mock_error(_root, errs):
            errs.append("injected swift anti-mock error")

        with mock.patch.object(sys, "argv", ["check-desktop-main-ios-architecture.py", "--strict", "--authority-only"]), \
             mock.patch.object(validator, "live_head", return_value=real_lock["commit"]), \
             mock.patch.object(validator, "verify_ios_swift_anti_mock", side_effect=inject_swift_mock_error), \
             contextlib.redirect_stdout(io.StringIO()):
            rc_fail = validator.run()
        self.assertEqual(rc_fail, 1)


if __name__ == "__main__":
    unittest.main()
