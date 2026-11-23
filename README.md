# Fabushi iOS

Standalone native Apple HIG iOS product of the canonical Fabushi Desktop **main** architecture (`bhrumom/gpui-telegram`, `Fabushi v1.6.9`), owned and built entirely in **`fabushiios/fabushi-ios`**. Legacy lineage descends from `bhrumom/fabushi-desktop` (`78d076cb8cc7d1ec157974cb9e91df9f599f0a71`, absorbed into `bhrumom/gpui-telegram/inherited/`), historical `bhrumom/fabushi`, archived mirror `bhrum/gpui-telegram` (`v1.3.0`), and PR #11/#12/#20, none of which are live completion authorities. Note: `bhrunom/gpui-telegram` is a verified HTTP 404 typo for `bhrumom/gpui-telegram`.

- Target repository: `fabushiios/fabushi-ios` (historical redirects: `fabushi-ios/fabushi-ios`, `bhrumom/fabushi-ios`)
- Live upstream: `bhrumom/gpui-telegram@main` (`Fabushi v1.6.9`)
- Canonical migration Spec: `docs/specs/gpui-telegram-main-ios-parity.md`
- Machine authority lock: `manifests/desktop-main-authority.json`
- Current status: implementation in progress; exact-main source completeness and all native release gates (`IOS-MAIN-AC-01..21`) must pass.

Only GitHub Actions or a user-approved remote runtime runs executable build/test/validation. No shared Fabushi runtime repository or external Desktop source checkout is required at iOS build/runtime — all vendored pure-Rust core crates (`source/desktop_core/`), C-ABI host bridge (`source/host/`), and SwiftUI/UIKit sources (`mobile/ios/`) are 100% iOS-owned.

