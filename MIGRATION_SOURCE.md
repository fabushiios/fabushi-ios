# Migration source — live Desktop main (`bhrumom/gpui-telegram`)

Status: active / incomplete (policy and source snapshot are not final acceptance)
Canonical Spec: `docs/specs/gpui-telegram-main-ios-parity.md`
Machine authority: `manifests/desktop-main-authority.json`

- Desktop repository: `bhrumom/gpui-telegram`
- Desktop live branch: `main` (`authorityPinMode: "fixed-baseline"` per user directive `2026-11-25T12:28:00Z`)
- Desktop fixed baseline snapshot at `2026-11-25`: `Fabushi v1.6.15` (`2ca3fffd32c9cb5b9aa8b7c4d5092b6b901914d2`, annotated tag `v1.6.15` object SHA `5a0446c2a0ba5ba7484c169e406b8ba140527f1f` tagged `2026-11-25T12:19:32Z`; latest published GitHub Release `v1.6.14`)
- Desktop snapshot root tree: `82ab74d1bfa95addfba162330e3cf22048b90900`
- Desktop tracked inventory at this snapshot: `11,406` blobs (`3,110` `crates/**` across 19 directories + `7,713` `inherited/**` + `583` root/docs/scripts/tests/vendor), `0` gitlinks
- Target repository: `fabushiios/fabushi-ios` (historical redirects: `fabushi-ios/fabushi-ios`, `bhrumom/fabushi-ios`)
- Target release: `Fabushi v1.6.15` (`MARKETING_VERSION: 1.6.15`, `CURRENT_PROJECT_VERSION: 35`)

Never parse this Markdown for an authoritative SHA. Each migration/validation/acceptance cycle reads the JSON machine lock (`manifests/desktop-main-authority.json`) and strictly verifies the exact pinned upstream commit SHA, root tree SHA, non-truncated tracked inventory, and SHA-256 tree digests (in `fixed-baseline` mode, advancing upstream `main` commits are logged informatively without invalidating the user-locked `v1.6.15` baseline).

Naming and historical lineage notes:
- `bhrunom/gpui-telegram` returns HTTP 404 (`users/bhrunom` does not exist) and is a verified 1-character typo for `bhrumom/gpui-telegram`.
- `bhrum/gpui-telegram` (archived `v1.3.0`, `b436dcdca2c05584dd522301a3201441fb3b4e0e`) is an archived intermediate mirror referenced by historical unmerged PR #11/#12 and is historical/research only.
- `bhrumom/fabushi-desktop` (`78d076cb8cc7d1ec157974cb9e91df9f599f0a71`, tree `849eddbff71592e72b3b2389ddd1372fb0320055`) is the legacy Electron predecessor whose domain crates were absorbed verbatim into `bhrumom/gpui-telegram/inherited/`; its superseded parity spec remains at `docs/specs/fabushi-desktop-main-ios-parity.md`.
- Previous PR #20 SHA `798cf51d96cb1cb98cf657af212bb47274ccb701` and its 7,943-row inventory are historical only; old MIGRATION_SOURCE notes are archived at `docs/history/ios-migration-source-pr20-2026-10-08.md`. Grok and unmerged Desktop PRs are not alternative authorities. iOS maintains its standalone Coordinator/Host/Runner, iOS-owned vendored pure-Rust core (`source/desktop_core/`), and native Apple platform replacements; no shared runtime repository is introduced.

Full source and product migration is incomplete until all `IOS-MAIN-AC-01..21` gates are independently evidenced on final exact HEAD. No stale success artifact, local-only claim, or policy-only commit can replace that verification.

