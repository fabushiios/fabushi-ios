"""Run only in GitHub Actions: fail-closed authority checker unit contracts."""
import importlib.util
import pathlib
import tempfile
import unittest

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
            "fullTreeSha256": "e" * 64 if file_ledger_mode == "current-main-complete" else None,
            "selectedTreeSha256": "c" * 64 if file_ledger_mode == "current-main-complete" else None,
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
        lock = make_valid_v3_lock(file_ledger_mode="legacy-predecessor-archived")
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
                'ext_git = { git = "https://github.com/bhrumom/gpui-telegram.git" }\n',
                encoding="utf-8",
            )
            errors = []
            validator.verify_cargo_toml_hygiene(bad_toml, core_root, validator.FORBIDDEN_MOBILE_DEPS, errors)
            self.assertTrue(any("escapes" in e for e in errors), errors)
            self.assertTrue(any("absolute path dependency forbidden" in e for e in errors), errors)
            self.assertTrue(any("external Fabushi git dependency forbidden" in e for e in errors), errors)

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

            # Valid CI-verified TEST_READY.md with GitHub Actions URL and 40-hex commit SHA passes
            (root / "TEST_READY.md").write_text(
                "# E2E Test Readiness\n"
                "Exact HEAD: 6141456e2a1121b78a6cdcd5da2a6d24c9c37e15\n"
                "GitHub Actions Run: https://github.com/fabushiios/fabushi-ios/actions/runs/9999999999\n",
                encoding="utf-8",
            )
            good_errors = []
            validator.verify_test_ready_integrity(root, good_errors)
            self.assertEqual(good_errors, [])

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
