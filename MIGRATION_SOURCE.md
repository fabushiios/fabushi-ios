# Migration source — live Desktop main (`bhrumom/gpui-telegram`)

Status: active / incomplete (policy and source snapshot are not final acceptance)
Canonical Spec: `docs/specs/gpui-telegram-main-ios-parity.md`
Machine authority: `manifests/desktop-main-authority.json`

- Desktop repository: `bhrumom/gpui-telegram`
- Desktop live branch: `main`
- Desktop observed release at `2026-11-23`: `Fabushi v1.6.7` (`6141456e2a1121b78a6cdcd5da2a6d24c9c37e15`)
- Desktop snapshot root tree: `836c181ac03585d0abc468f9d56a67a0dd20a9bf`
- Desktop tracked inventory at this snapshot: `11,342` blobs (`3,056` `crates/**` across 18 directories + `7,713` `inherited/**` + `573` root/docs/scripts/tests/vendor), `0` gitlinks
- Target repository: `fabushiios/fabushi-ios` (historical redirects: `fabushi-ios/fabushi-ios`, `bhrumom/fabushi-ios`)
- Target release: `Fabushi v1.6.7` (`MARKETING_VERSION: 1.6.7`, `CURRENT_PROJECT_VERSION: 35`)

Never parse this Markdown for an authoritative SHA. Each migration/validation/acceptance cycle reads the JSON machine lock and resolves the live Desktop main exact ref. A new main SHA requires rebaseline before current parity can be accepted, including selected source manifest, full tracked tree, dependency/owner-impact review, ledger and exact iOS evidence.

Naming and historical lineage notes:
- `bhrunom/gpui-telegram` returns HTTP 404 (`users/bhrunom` does not exist) and is a verified 1-character typo for `bhrumom/gpui-telegram`.
- `bhrum/gpui-telegram` (archived `v1.3.0`, `b436dcdca2c05584dd522301a3201441fb3b4e0e`) is an archived intermediate mirror referenced by historical unmerged PR #11/#12 and is historical/research only.
- `bhrumom/fabushi-desktop` (`78d076cb8cc7d1ec157974cb9e91df9f599f0a71`, tree `849eddbff71592e72b3b2389ddd1372fb0320055`) is the legacy Electron predecessor whose domain crates were absorbed verbatim into `bhrumom/gpui-telegram/inherited/`; its superseded parity spec remains at `docs/specs/fabushi-desktop-main-ios-parity.md`.
- Previous PR #20 SHA `798cf51d96cb1cb98cf657af212bb47274ccb701` and its 7,943-row inventory are historical only; old MIGRATION_SOURCE notes are archived at `docs/history/ios-migration-source-pr20-2026-10-08.md`. Grok and unmerged Desktop PRs are not alternative authorities. iOS maintains its standalone Coordinator/Host/Runner, iOS-owned vendored pure-Rust core (`source/desktop_core/`), and native Apple platform replacements; no shared runtime repository is introduced.

Full source and product migration is incomplete until all `IOS-MAIN-AC-01..21` gates are independently evidenced on final exact HEAD. No stale success artifact, local-only claim, or policy-only commit can replace that verification.

