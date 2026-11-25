# Fabushi iOS — Agent Instructions

Repository: fabushiios/fabushi-ios (not historical redirects fabushi-ios/fabushi-ios or bhrumom/fabushi-ios).

## Mandatory active authority

The **only live desktop product/architecture upstream** is bhrumom/gpui-telegram branch main (verified release Fabushi v1.6.11; exact HEAD `cc56925f887be8f177642da9cd119d29108ddba0`, root tree `56de8e059b40526674b21943bbf520c56797b213`, `11381` blobs: `3085` `crates/**` + `7713` `inherited/**`, `0` gitlinks). The canonical iOS migration Spec is docs/specs/gpui-telegram-main-ios-parity.md and the machine lock is manifests/desktop-main-authority.json.

- Note on upstream naming & lineage: `bhrunom/gpui-telegram` is a verified HTTP 404 1-character typo for `bhrumom/gpui-telegram`. `bhrum/gpui-telegram` (archived `v1.3.0` mirror), `bhrumom/fabushi-desktop` (`78d076cb8cc7d1ec157974cb9e91df9f599f0a71`, legacy Electron predecessor absorbed into `bhrumom/gpui-telegram/inherited/`), and PR #11/#12/#20 are historical/lineage references only.

1. Read this AGENTS.md and the canonical iOS Spec before product-affecting work.
2. Resolve Desktop main (`bhrumom/gpui-telegram@main`) and iOS branch exact HEADs; read Desktop root AGENTS.md, current project SOURCE_OF_TRUTH and accepted active Specs/contracts, then current shipping owners and CI oracles.
3. Treat PR #11, PR #12, PR #20, archived `bhrum/gpui-telegram`, Grok and unmerged Desktop PRs as historical/research only. Their old SHA, ledger count and green jobs do not prove current main parity.
4. Distinguish Desktop requirement-only planned work and upstream/external stubs from real shipping implementation; both remain in final scope, but neither may be fabricated as implemented/verified.
5. Rebaseline all selected source and full tracked inventory against Desktop main; propagate owner/contract impacts, don't silently keep old verified rows.
6. Existing-owner-first: unified Fabushi shell/conversation/transcript/composer/profile/settings/Agent Studio/Remote/Mini Apps/Marketplace and typed capability differences. Preserve Coordinator/Host/Runner, canonical security/state and lifecycle boundaries. Do not introduce a second app, duplicate workspace, or shared Desktop/iOS runtime repository (iOS-owned pure-Rust domain crates vendored inside `source/desktop_core/` form the canonical standalone iOS runtime with zero external Desktop checkout/repo dependency).
7. Port product responsibility/effect, not filenames; use SwiftUI/UIKit/Apple native mechanisms (Apple HIG) where required; the source for all build/runtime inputs is 100% iOS-owned.
8. Blocker != complete. All required GitHub Actions and native-device/signed/TestFlight/release checks need exact-head proof, non-skipped steps and artifact digests.

## Spec-first development

**Discover -> Spec -> Architecture/Plan -> Implement -> Verify -> Spec Compliance Review -> Integrate/Deliver.** Read docs/specs/spec-first-ai-development.md. Repair an absent/stale contract before coding. No deletion, warning downgrade or assertion relaxation to make CI green.

## Build/test policy

All executable builds, tests, linters, generators, validators, benchmarks and acceptance run **only in GitHub Actions or a user-authorized remote runtime**, never local Mac/Windows or assistant container. Read-only investigation, Git/API editing are permitted. Missing runner/account/device/signing = blocked, not passed.

The full completion bar is IOS-MAIN-AC-01..21 in the active Spec. Current source acceptance requires Desktop main SHA at the start **and end** of the gate, matching Git root tree/authority lock, exact iOS SHA, successful real shipping paths, and independent evidence review.

