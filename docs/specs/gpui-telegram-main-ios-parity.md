# Fabushi Desktop main (`bhrumom/gpui-telegram`) -> Fabushi iOS Native Product Parity Specification

Status: active
Spec ID: IOS-GPUI-TELEGRAM-MAIN-001
Revision: 1
Updated: 2026-11-23
Target Repository: fabushiios/fabushi-ios (historical redirects: fabushi-ios/fabushi-ios, bhrumom/fabushi-ios)
Target Release: Fabushi v1.6.7 (`MARKETING_VERSION: 1.6.7`, `CURRENT_PROJECT_VERSION: 35`)
Canonical Policy: this file, `AGENTS.md`, `manifests/desktop-main-authority.json`
Superseded Specs: `docs/specs/fabushi-desktop-main-ios-parity.md`, `docs/specs/fabushi-desktop-pr20-ios-architecture-parity.md`
Historical Lineage: `bhrumom/fabushi-desktop@78d076cb8cc7d1ec157974cb9e91df9f599f0a71` (absorbed into `bhrumom/gpui-telegram/inherited/`)

---

## 1. Context, Goal & Live Authority (`IOS-MAIN-AUTH-01..08`)

### 1.1 Context & Problem
Previous iOS migration baselines tracked `bhrumom/fabushi-desktop@main` (legacy Electron architecture) and produced a Stratum A/B split-brain in `fabushiios/fabushi-ios` (`GrokMobileShell` + `mahayana-app-host-mobile` colliding with stubbed `source/host/src/dispatch.rs` and uninitialized Stratum B views). The live Desktop product has converged on **`bhrumom/gpui-telegram` (`Fabushi v1.6.7`)**, a pure-Rust domain workspace that absorbed `bhrumom/fabushi-desktop`'s domain crates into `inherited/`. Per `docs/specs/spec-first-ai-development.md` (amendment protocol), this Specification formally cuts over canonical iOS authority to **`bhrumom/gpui-telegram` branch `main`** before product code changes proceed.

### 1.2 Goal
Build, verify, and ship a standalone, 100% iOS-owned, Apple HIG-native iOS/iPadOS edition (`Fabushi`, `com.ombhrum.fabushi`) achieving full applicable product parity with **`bhrumom/gpui-telegram@main` (`Fabushi v1.6.7`)** while preserving `Coordinator / Host / Runner` ownership boundaries, canonical security/state isolation, existing working native Apple runners, and full honesty regarding upstream stubs and external blockers.

### 1.3 Live Authority Pin & Naming Reconciliation
- **Primary Live Desktop Authority**: `bhrumom/gpui-telegram` (GitHub repository ID `1449467779`), branch **`main`**.
- **Observed Baseline Snapshot (`2026-11-23`)**:
  - Release: **`Fabushi v1.6.7`** (published `2026-11-23T01:31:53Z`)
  - Commit SHA: `6141456e2a1121b78a6cdcd5da2a6d24c9c37e15`
  - Root Tree SHA: `836c181ac03585d0abc468f9d56a67a0dd20a9bf`
  - Non-truncated Tracked Inventory: **`11,342` blobs** (`3,056` under `crates/**` across 18 directories + `7,713` under `inherited/**` + `573` root/docs/scripts/tests/vendor), **`0` gitlinks**.
- **Verified Naming & Historical Lineage Notes**:
  - `bhrunom/gpui-telegram` -> **HTTP 404** (`users/bhrunom` does not exist; verified 1-character typo for `bhrumom/gpui-telegram`). Never accepted as an authority slug.
  - `bhrum/gpui-telegram` (GitHub repository ID `1456601492`, `archived: true`, frozen at `v1.3.0` commit `b436dcdca2c05584dd522301a3201441fb3b4e0e`) -> historical/research mirror only (referenced by unmerged PR #11/#12; missing v1.6.7 crates and features).
  - `bhrumom/fabushi-desktop` (GitHub repository ID `1375312893`, commit `78d076cb8cc7d1ec157974cb9e91df9f599f0a71`, tree `849eddbff71592e72b3b2389ddd1372fb0320055`) -> legacy Electron predecessor whose domain crates were absorbed into `bhrumom/gpui-telegram/inherited/`; superseded spec retained at `docs/specs/fabushi-desktop-main-ios-parity.md`.
  - PR #11, PR #12, PR #20 (`798cf51d`), Grok, and unmerged Desktop PRs -> historical/research only.
- **Machine Authority Lock**: `manifests/desktop-main-authority.json` (schema v3). Never parse Markdown for an authoritative SHA; every validation/CI/acceptance cycle reads the JSON machine lock and queries the live remote ref.

### 1.4 Authority Protocol (`IOS-MAIN-AUTH-01..08`)
- **`IOS-MAIN-AUTH-01`**: Resolve live Desktop main (`bhrumom/gpui-telegram` `refs/heads/main`) exact commit and root tree via recorded ref resolution (`git ls-remote` + non-truncated tree check) at the start of every CI/acceptance cycle, together with exact iOS HEAD.
- **`IOS-MAIN-AUTH-02`**: Read Desktop root `AGENTS.md`, `Cargo.toml`, active specs/contracts, shipping owners, and CI oracles; strictly distinguish shipping implementation from requirement-only planned items and upstream stubs.
- **`IOS-MAIN-AUTH-03`**: Exhaustively enumerate the full tracked Git tree (all blobs and gitlinks); fail closed if truncated (`truncated != false`), inaccessible, or if tracked counts/crate accounting drift.
- **`IOS-MAIN-AUTH-04`**: Diff against the prior authority snapshot (added, removed, renamed, blob-changed, contract/owner-changed, dependency-impacting files) before changing any status.
- **`IOS-MAIN-AUTH-05`**: Maintain the machine authority lock (`manifests/desktop-main-authority.json`) and inventory/ledger artifacts from that exact Desktop tree without hardcoding historical counts as permanent expectations.
- **`IOS-MAIN-AUTH-06`**: Record impacted responsibilities and fence changed/impacted rows against silent carry-forward of historical `verified` statuses.
- **`IOS-MAIN-AUTH-07`**: Carry forward unchanged rows only with explicit upstream owner/contract revalidation.
- **`IOS-MAIN-AUTH-08`**: Re-resolve live Desktop main at the **end** of every required CI/acceptance job and before final acceptance; if live `refs/heads/main != lock.commit`, fail closed for current-authority acceptance and rebaseline on a new exact iOS HEAD.
- **CI-Only Execution Mandate (`AGENTS.md`)**: All executable builds, tests, linters, generators, validators, benchmarks, and acceptance run **only in GitHub Actions or a user-authorized remote runtime**, never local Mac/Windows or an assistant container. Missing runner/account/device/signing = `blocked`, never `passed`.

---

## 2. Non-Goals & Architectural Constraints

### 2.1 Non-Goals
- No second standalone app, parallel Telegram-only app, or Human-only workspace.
- No shared external Desktop/iOS runtime repository or out-of-repo path/git Cargo dependency required at iOS build or runtime.
- No desktop GUI/GPU/EGL/Winit/daemon/CLI/WASM crates (`gpui`, `khronos-egl`, `gpui_telegram_desktop`, `fabushi_remote_daemon`, `gpui_telegram_cli`, `fabushi_miniapp_wasm`, `gpui_telegram_server`, `gpui_telegram_test_harness`) in the mobile Rust workspace.
- No prohibited iOS binary self-updater, local Chromium/node/shell subprocess sidecar execution, or kernel `utun` privileged daemon on iOS.
- No fabricated test results, local-only `TEST_READY.md` claims, `"status": "acknowledged"` FFI stubs, or fake sample chats.

### 2.2 Non-Negotiable Architectural Constraints
1. **Single Canonical Standalone App (`com.ombhrum.fabushi`)**: Eliminate Stratum A/B split-brain onto one unified Swift 6 + SwiftUI/UIKit + C-ABI Rust architecture.
2. **100% iOS-Owned Source & Build Inputs**: `source/desktop_core/**`, `source/host/**`, `mobile/ios/**`, manifests, and CI workflows are self-contained inside `fabushiios/fabushi-ios`.
3. **Coordinator / Host / Runner Boundaries**:
   - `FabushiCoordinator` (`@MainActor @Observable`) owns lifecycle, `accountScopeGeneration: UInt64` fencing, approval/lease routing, and store orchestration.
   - `FabushiRustHostActor` (`actor`, `Sendable`) owns the opaque C-ABI handle, serializes lifecycle transitions, and exposes typed async RPC + `AsyncStream<FabushiDomainEvent>`.
   - Native Apple Runners own hardware/OS-bound services (`Keychain`/`SecureEnclave`, `LocalAuthentication`, `ASAuthorization` Passkeys, `ASWebAuthenticationSession`, `URLSession` background CAS, `AVFoundation`/`AVAudioEngine`/VAD, `LiveKitWebRTC`, `CallKit`, `ReplayKit` `FabushiBroadcastUpload`, `WKWebView` `MiniAppWebMcpSurface` + `OfflineMiniAppPackageVerifier`, `StoreKit 2` `FabushiPayStoreKit`, `OfflineSpeechTranscriber`, `AppIntents`).
4. **Apple HIG Redesign**:
   - 4 `v1.6.7` themes (`Night`, `Tinted`, `Day`, `Classic`) with custom accent WCAG contrast guard.
   - Liquid Glass materials guarded cleanly under `#available(iOS 26, *)` with iOS 17–18 ultra-thin native material fallback.
   - Full Dynamic Type (zero fixed-size typography breaking accessibility scaling), VoiceOver labels/hints, Reduce Motion safety, and complete `zh-Hans` / `en` localization (`Localizable.xcstrings`).
   - Adaptive Shell: iPad 3-column `NavigationSplitView` + `.inspector`; iPhone `TabView` + `NavigationStack` + `.sheet` Inspector + floating PiP/HUD capsules.
5. **Existing-Owner-First**: Preserve working iOS-native owners (`LiveKitWebRTC`, `CallKit`, `FabushiBroadcastUpload`, `MiniAppWebMcpSurface`, `OfflineMiniAppPackageVerifier`, `FabushiPayStoreKit`, `VoiceRecorder`, `OfflineSpeechTranscriber`, Keychain security) and upgrade them cleanly rather than duplicating parallel owners.

---

## 3. Inventory, Vendored Core Ownership & iOS Sandbox Adaptations (`IOS-MAIN-INV-01..07`)

### 3.1 Full Tree & Crate Accounting (`IOS-MAIN-INV-01..07`)
- **`IOS-MAIN-INV-01`**: Account for 100% of tracked blobs (`11,342`) and gitlinks (`0`) from non-truncated `bhrumom/gpui-telegram@main` root tree `836c181ac03585d0abc468f9d56a67a0dd20a9bf`.
- **`IOS-MAIN-INV-02`**: Track all `3,056` blobs across all **18 directories** under `crates/**` without overlap or omission:
  - **11 Vendored Mobile Cargo Crates** (in `source/desktop_core/Cargo.toml` `[workspace].members`):
    1. `gpui_telegram_core`
    2. `gpui_telegram_proto`
    3. `gpui_telegram_client`
    4. `gpui_telegram_media`
    5. `gpui_telegram_webrtc`
    6. `gpui_telegram_agent` (incl. `assets/fixtures/**`)
    7. `gpui_telegram_coding`
    8. `magpie_gateway`
    9. `bot-browser` (pure crypto vault/`e2ee_sync`/`totp`; desktop keyring gated off iOS)
    10. `fabushi_remote_engine` (controller/codec/protocol/PTY mux; desktop `xcap` & Unix `openpty`/`fork`/`execve`/`termios` gated off iOS)
    11. `fabushi_miniapp` (protocol/WebMCP/assets)
  - **1 Non-Cargo Shared Source Directory** (vendored verbatim, in `[workspace].exclude` because upstream has no `Cargo.toml`):
    - `gpui_telegram_common` (`src/models/auth.rs`, 1 blob)
  - **5 Required Transitive `inherited/` Cargo Crates** (+ `inherited/packages/proto` data dir for `inherited/host/build.rs`):
    - `inherited/host` (`mahayana-host-runtime`), `inherited/mahayana-messaging` (`fabushi-messaging-core`), `inherited/node-agent-coordinator` (`mahayana-node-agent-coordinator`), `inherited/mahayana-rs/mahayana-computer` (`mahayana-computer`), `inherited/mahayana-rs/mahayana-host-protocol` (`mahayana-host-protocol`).
  - **6 Explicitly Excluded Desktop/Server/CLI/WASM/Harness Crates** (forbidden in `source/desktop_core` workspace and mobile dependency graph):
    - `gpui_telegram_desktop`, `fabushi_remote_daemon`, `gpui_telegram_cli`, `fabushi_miniapp_wasm`, `gpui_telegram_server`, `gpui_telegram_test_harness`.
- **`IOS-MAIN-INV-03`**: Account for `inherited/**` (`7,713` blobs) and all `8,286` non-`crates/**` tracked entries in full-tree closure before `--complete` acceptance.
- **`IOS-MAIN-INV-04`**: Enforce the **5 mandatory iOS sandbox adaptations** without degrading user-visible security or functionality:
  1. **Sandbox Paths & Configurable Server URL**: Store SQLite/WAL/CAS/Tantivy exclusively inside iOS App Container (`Application Support` / `Caches` / App Group where required) with zero hardcoded `localhost` endpoints (`DEFAULT_SERVER_URL` configurable, default `http://34.46.77.133:8080`).
  2. **`0xdead10cc` Prevention (Background SQLite WAL Checkpoint & POSIX Lock Release)**: On `ScenePhase` `.background` / `UIApplication.willResignActiveNotification`, flush pending writes, execute `PRAGMA wal_checkpoint(TRUNCATE)`, and release POSIX file locks inside a `UIApplication.beginBackgroundTask` guard so iOS never terminates the app with `0xdead10cc`.
  3. **Apple Native Security & Identity Storage**: Replace desktop OS keyring with iOS Keychain (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`) + `LocalAuthentication` (Face ID / Touch ID) + `ASAuthorization` Passkeys + `ASWebAuthenticationSession` OAuth callbacks, injecting unlocked vault keys into Rust `bot-browser` crypto primitives.
  4. **User-Space Mobile Controller Mode (No Kernel `utun` / Local `/bin/sh` / Local `xcap`)**: Remote Computer Workbench, Tailnet, Coding Agents, and Obscura Browser operate as first-class iOS mobile controllers talking to the user's remote desktop/cloud hosts; desktop `xcap` capture and `openpty`/`fork`/`execve`/`termios` are gated off `target_os = "ios"` (`ReplayKit` handles opt-in iOS broadcast share).
  5. **Apple Platform Entitlements & Manifest Hygiene**: Explicit `Info.plist` ATS policy, registered URL schemes (`fabushi`, `gpui-telegram`, `tg`, `ton`, `magpie`), declared background modes (`audio`, `voip`, `remote-notification`), and App Store privacy usage descriptions.
- **`IOS-MAIN-INV-05`**: No directory-level "covered" stamp may close unknown individual files in `--complete` mode.
- **`IOS-MAIN-INV-06`**: Requirement-only upstream items and upstream stubs stay tracked honestly (`blocked` / `upstream-planned`), never converted to `not-applicable` or fabricated `verified`.
- **`IOS-MAIN-INV-07`**: Never treat historical file counts (`7,943`, `7,951`, `8,014`, `11,237`, `11,342`) as fixed future constants; always detect and rebaseline drift against live `bhrumom/gpui-telegram@main`.

---

## 4. Complete 34-Feature Desktop Parity Matrix & Typed iOS Deltas

Every Desktop domain from `bhrumom/gpui-telegram@6141456e2a11` (34 Desktop features + 3 Governance/Unification/Release items = 37 total tracked items) is mapped to an authoritative iOS owner and milestone:

| # | Feature Domain | Desktop Status | Typed iOS Delta | Authoritative iOS Owner | Milestone |
|---|---|---|---|---|---|
| 1 | Authority Cutover, Parity Spec, Vendored `v1.6.7` Core, Fail-Closed FFI & Staged CI | `GOVERNANCE` | CI-only execution (`ubuntu-latest` + `macos-15` + `sccache`), 100% iOS-owned crates, zero `gpui`/`khronos-egl` | `AGENTS.md`, `manifests/desktop-main-authority.json`, `source/desktop_core/`, `source/host/`, `.github/workflows/` | M1 |
| 2 | Password Login, Registration & Split-Key V2 Bootstrap | `REAL SHIPPING (gpui-tg)` | iOS Keychain (`AfterFirstUnlockThisDeviceOnly`) + Rust Argon2id Split-Key V2 KEK derivation & transparent vault unlock | `source/desktop_core/crates/{gpui_telegram_client,bot-browser}`, `Presentation/Auth/` | M2 |
| 3 | Email OTP & Phone SMS Verification | `REAL SHIPPING (gpui-tg)` | Native SwiftUI `.oneTimeCode` autofill + country picker | `source/desktop_core/crates/gpui_telegram_client`, `Presentation/Auth/` | M2 |
| 4 | Alipay QR Login & Mobile OAuth Callback | `REAL SHIPPING (gpui-tg)` | `ASWebAuthenticationSession` + `fabushi://` URL scheme + iPad QR polling | `Core/Runners/`, `Presentation/Auth/` | M2 |
| 5 | Passkeys (WebAuthn/FIDO2) & QR Device Pairing | `REAL SHIPPING (gpui-tg)` | `ASAuthorizationPlatformPublicKeyCredentialProvider` + camera QR scanner (`pair/init/accept/poll/verify`) | `Core/Runners/`, `Presentation/Auth/` | M2 |
| 6 | Cloud 2FA Password & Active Device Sessions | `REAL SHIPPING (gpui-tg)` | Settings -> Privacy & Security -> Devices swipe-to-terminate | `source/desktop_core/crates/gpui_telegram_client`, `Presentation/Security/` | M2 |
| 7 | Multi-Account Switcher & Local Passcode/Face ID App Lock | `REAL SHIPPING (gpui-tg)` | Face ID / Touch ID (`LocalAuthentication`), background blur snapshot, generation-fenced isolated Keychain/SQLite | `Core/FabushiCoordinator`, `Presentation/Security/` | M2 |
| 8 | Apple HIG Design System & Unified Adaptive Shell | `REAL SHIPPING (gpui-tg)` | 4 v1.6.7 themes (`Night`/`Tinted`/`Day`/`Classic`), WCAG accent contrast guard, Liquid Glass (`#available(iOS 26, *)`), Dynamic Type, VoiceOver, `zh-Hans`/`en`, iPad 3-col `NavigationSplitView` + `.inspector` / iPhone `TabView` + `NavigationStack` + `.sheet` | `DesignSystem/`, `Presentation/Shell/`, `Resources/Localizable.xcstrings` | M2 |
| 9 | Grok Bot Multi-Select Compose (`To:` Picker) | `REAL SHIPPING (gpui-tg)` | Native Apple Messages-style `To:` chip bar with IME-safe multi-select human + bot autocomplete & atomic DM/group creation | `Presentation/Chats/`, `source/desktop_core/crates/gpui_telegram_client` | M3 |
| 10 | Unified Chat List, Folders, Archive, Pin, Mute & Forum Topics | `REAL SHIPPING (gpui-tg)` | Native `List` + folder pill bar + iOS swipe actions + context menu + pin stability | `Presentation/Chats/` | M3 |
| 11 | Contacts, Peer Resolution, Blocklist, Send-As & Prestart Bot | `REAL SHIPPING (gpui-tg)` | Native Contacts sheet + `@username` resolution + Send-As picker + Prestart Bot hero `Start` stage | `Presentation/Contacts/`, `Presentation/Chats/` | M3 |
| 12 | Atomic PTS/QTS Sync, Nonce Dedup & Offline SQLite WAL Outbox | `REAL SHIPPING (gpui-tg)` | Rust `gpui_telegram_client` monotonic PTS/WS/`SyncGetDifference`/24h nonce dedup + `NWPathMonitor` + `BGAppRefreshTask` + `0xdead10cc` WAL checkpoint | `source/desktop_core/crates/gpui_telegram_client`, `Core/`, `Presentation/Chats/` | M1 (core), M3 (UI) |
| 13 | Virtualized 120Hz Transcript & 11+ Canonical Message Cards | `REAL SHIPPING (gpui-tg)` | Bottom-anchored ProMotion transcript, 900s grouping + tails, date pills, 11+ cards (Text/MD/Math/Mermaid/Spoiler, Photo/Video, 5-bit Voice, File, Sticker, Album, Location, Poll, Todo, Service, Call, Story, Invoice, Inline Keyboard, `AgentThinkingBubble`, `ToolPermissionDock`, Computer card) | `Presentation/Conversation/` | M3 |
| 14 | Message Edit, Delete Tombstone, Forward, Reactions, Pins, TTL & Export | `REAL SHIPPING (gpui-tg)` | Swipe-to-reply, long-press reactions + `UIMenu`, multi-select toolbar, PTS-fenced reactions, `UIActivityViewController` JSON export & stats | `Presentation/Conversation/` | M3 |
| 15 | IME-Safe Canonical Composer, Drafts, Formatting, Voice Gesture & Send Options | `REAL SHIPPING (gpui-tg)` | `UITextView` `markedTextRange` CJK IME fencing, synced drafts, formatting bar, emoji/sticker/GIF picker, real `AVFoundation` hold/cancel/lock voice gesture, haptics, `PhotosPicker`/`UIDocumentPicker`/Camera, silent/scheduled send | `Presentation/Composer/` | M3 |
| 16 | 49+ Hermes/Fabushi Slash Commands & `@` Context Badges | `REAL SHIPPING (gpui-tg)` | Floating keyboard-accessory `/` autocomplete across 7 categories + `@file`/`@dir`/`@diff`/`@git:`/`@url` chips | `Presentation/Composer/`, `source/desktop_core/crates/gpui_telegram_agent` | M3 |
| 17 | Muse Context Inspector (Profile, Members, Shared Media, Bot/Computer/Coding Config) | `REAL SHIPPING (gpui-tg)` | Centered 80pt hero, 4-icon bar, Profile/Members/5-tab Shared Media/Settings, Bot Computers & Bot Config Editor (`320pt` iPad `.inspector`, iPhone push/`.sheet`) | `Presentation/Inspector/` | M3 |
| 18 | Unified Search (Tantivy Full-Text + Hybrid Memory + Bots/MiniApps) | `REAL SHIPPING (gpui-tg)` | Native `.searchable` + category filter pills + hybrid semantic/entity memory + SQLite FTS5 offline fallback | `source/desktop_core/crates/gpui_telegram_client`, `Presentation/Chats/` | M1 (core), M3 (UI) |
| 19 | E2EE Messaging (Double Ratchet, Sender Keys & Safety Numbers) | `PARTIAL / STUB (gpui-tg)` | Rust crypto core + Keychain keys + Secret/E2EE UI toggle & 4-emoji/QR Safety Number verification sheet | `source/desktop_core/crates/{gpui_telegram_core,gpui_telegram_client}`, `Presentation/Conversation/` | M1 (core), M3 (UI) |
| 20 | Chunked Media CAS, HTTP 206 Streaming, 5-Bit Voice, Lightbox, Editor, Stickers/GIFs, Stories & Instant View | `REAL SHIPPING (gpui-tg)` | SHA-256 resumable `URLSession` background transfers, HTTP 206 `AVPlayer`, 63-byte 5-bit waveform, Lightbox, PencilKit/crop editor, QuickLook, Stories, Instant View | `source/desktop_core/crates/gpui_telegram_media`, `Presentation/Media/` | M1 (core), M3 (UI) |
| 21 | Zero-Knowledge Split-Key V2 & X25519 Credential Vault + TOTP + Remote Obscura Browser | `REAL SHIPPING (gpui-tg)` | Native Keychain + Rust `bot-browser` Argon2id/X25519/ChaCha20-Poly1305 vault/TOTP; remote Obscura browser host control (sandbox-safe, no local Chromium) | `source/desktop_core/crates/bot-browser`, `Presentation/Security/` | M1 (core), M2 (UI) |
| 22 | Unified Hermes/Fabushi Agent Runtime, 29 Toolsets & 7-Tab Agent Studio | `REAL SHIPPING (gpui-tg)` | 7-tab `GlobalTasksWorkspace` (Tasks/Kanban/Subagents, `SKILL.md` Skills, `MEMORY`/`USER`/`SOUL` + Honcho + Quarantine, 29 Toolsets/MCP, 5-field Cron/Heartbeat, Sessions, Value-Bound Approvals) | `source/desktop_core/crates/gpui_telegram_agent`, `Presentation/AgentStudio/` | M4 |
| 23 | Multi-Agent Protocol (`FloorControlQueue`, 5-Hop `LoopBudget`, `SharedGroupWorkspace`, Grok `shouldReply`) | `REAL SHIPPING (gpui-tg)` | Live floor/typing pills, UserInterruption preempt, Workspace Inspector editor, Reduce-Motion-safe Grok persona avatars | `source/desktop_core/crates/gpui_telegram_agent`, `Presentation/AgentStudio/` | M4 |
| 24 | Pure-Rust Magpie AI Provider Gateway, Routing, Masking & Vault OAuth Sync | `REAL SHIPPING (gpui-tg)` | Multi-provider BYOK/presets, live `models.dev` catalog, Smart Routing Groups + circuit breakers, PII Privacy Masking + preview, `.env`/deep-link import, Zero-Knowledge Vault sync of desktop OAuth (`USER_VAULT_CATEGORY_AI_OAUTH`) + `ASWebAuthenticationSession` (no local `~/.claude` scan) | `source/desktop_core/crates/magpie_gateway`, `Presentation/Magpie/` | M4 |
| 25 | Third-Party Coding Agents (Claude Code, Codex, ACP) Mobile Client | `REAL SHIPPING (gpui-tg)` | BotFather NL creation, coding host probe, streaming diffs/worktrees/checkpoints/PRs, permission controls talking to user's remote desktop/cloud coding host via `CodingAgentBroker` (no local subprocess) | `source/desktop_core/crates/gpui_telegram_coding`, `Presentation/Coding/` | M4 |
| 26 | Official OAuth 2.1 Remote MCP Server Grants, QR-Poll Approval & 27-Tool Audit | `REAL SHIPPING (gpui-tg)` | Native iOS MCP Grants, PKCE/QR approval & redacted 27-tool `mcp_tool_audit` console + remote kill-switch | `source/desktop_core/crates/gpui_telegram_client`, `Presentation/MCP/` | M4 |
| 27 | UU-Remote Computer Workbench, Multi-Session PTY Terminal, Tailnet & Controlled Banner | `REAL SHIPPING (gpui-tg)` | First-class iOS Remote Controller (touch/trackpad/hotkeys/multi-display/privacy/power/file transfer) + multi-session PTY terminal + Cloud sandbox CRUD + Assist code + Tailnet + `<10ms` Emergency Takeover + Controlled status pill/Live Activity (unattended host daemon desktop-only; ReplayKit opt-in share only) | `source/desktop_core/crates/fabushi_remote_engine`, `Presentation/RemoteComputer/` | M5 |
| 28 | Telegram-Parity External Mini Apps, WebMCP Live Control, Floating PiP & 全球法布施 | `REAL SHIPPING (gpui-tg)` | `WKWebView` + isolated `WKWebsiteDataStore` + `telegram-web-app.js` + HMAC + ToS modal + live WebMCP bidirectional broker + draggable floating PiP capsule (in-place restore) + 全球法布施 Mini App (upgrading `MiniAppWebMcpSurface` + `OfflineMiniAppPackageVerifier`) | `source/desktop_core/crates/fabushi_miniapp`, `Presentation/MiniApps/` | M5 |
| 29 | 1:1 & Group Calls (`LiveKitWebRTC` + `CallKit` + `ReplayKit`) & Full-Duplex AI Talk Mode | `PARTIAL / STUB (gpui-tg)` | Unified Apple `CallOverlayView` wired to existing `LiveKitWebRTC` + `CallKit` + `FabushiBroadcastUpload`, 4-emoji E2EE fingerprint, history/rating + 160pt Talk Mode VAD `<60ms` barge-in (`AVAudioEngine` hardware AEC) + `OfflineSpeechTranscriber` | `source/desktop_core/crates/gpui_telegram_webrtc`, `Presentation/Calls/` | M5 |
| 30 | Full 26-Section Apple Settings Suite | `REAL SHIPPING (gpui-tg)` (22) / `PARTIAL` (Business/Passport/TON) | All 26 `SettingsSectionType` sections in native `List(.insetGrouped)` + search + real CAS/SQLite storage metrics & purge; honest capability notices for partial upstream sections | `Presentation/Settings/` | M5 |
| 31 | Economy, Stars Ledger, Star Gifts, Premium & StoreKit 2 | `REAL SHIPPING (gpui-tg)` (Stars/Alipay) / `PARTIAL` (TON/Apple IAP) | Stars balance/transactions/gifts, Redeem codes, Premium tiers, App Store Guideline 3.1.1 compliant **StoreKit 2** (`FabushiPayStoreKit`) + Restore Purchases + server entitlement sync | `Presentation/Economy/` | M5 |
| 32 | Network, Proxy (HTTP/SOCKS5/MTProto) & App Update Check | `REAL SHIPPING (gpui-tg)` | 4-tier config cascade, editable Server URL, proxy config + health probe, App Store / TestFlight version check (no prohibited binary self-update) | `source/desktop_core/crates/gpui_telegram_client`, `Presentation/Settings/` | M5 |
| 33 | Action Bus, App Intents / Siri Shortcuts & APNs Push Registration | `REAL SHIPPING` (Action Bus) / `STUB` (Server APNs HTTP/2) | Typed Action Bus mapped to iOS `AppIntents` / Siri Shortcuts & `fabushi://` URLs; real APNs token registration (`UNUserNotificationCenter` / `registerForRemoteNotifications`), server push delivery honestly tracked `blocked` | `AppIntents/`, `Core/`, `Presentation/Settings/` | M5 |
| 34 | Legacy / Planned Backlog Ledger (OCRM-001, FBCP-001, TDRP Shards, TON/Passport) | `REQUIREMENT-ONLY / PLANNED` | Explicit parity tracking of 141 unwired OCRM external channel adapters, FBCP D1, 467 unreferenced TDRP shard files, TON on-chain signing & Passport — NEVER fabricated | `docs/parity/`, `manifests/` | M1, M6 |
| 35 | Cross-Module Unification, Orphan & Legacy Cleanup, Provenance Clearance | `INTEGRATION` | Merge M3–M5, verify 0 orphaned views / 0 dead buttons, purge legacy Grok 0.18 reconstructed UI, update `NOTICE.md`/`docs/provenance/`, clean Swift 6 concurrency | `mobile/ios/`, `NOTICE.md`, `docs/provenance/` | M6 |
| 36 | Opaque-Box 4-Tier E2E Test Suite (`TEST_INFRA.md` / `TEST_READY.md`) | `E2E TRACK` | Requirement-driven Tier 1–4 E2E suite runnable strictly on GitHub Actions (`ubuntu-latest` + `macos-15`) | `TEST_INFRA.md`, `tests/e2e/`, `mobile/ios/FabushiUITests/` | E2E Track |
| 37 | Release Pipeline (`v1.6.7`), Signed Archive / TestFlight & GitHub Release | `RELEASE` | Bump `1.6.7` (build `35`), fix `ios-store-delivery.yml` `.broadcast` extension signing, add `ios-release.yml`, publish GitHub Release `v1.6.7` + execute TestFlight upload with exact-HEAD SHA-256 proof | `.github/workflows/ios-store-delivery.yml`, `.github/workflows/ios-release.yml`, `mobile/ios/project.yml` | M6, Final |

---

## 5. Honest Upstream Stubs & External Blockers Register (Blocker != Complete)

None of the following may ever be fabricated, mocked as production success, or silently marked `passed`:

| Blocker ID | Domain | Verified Upstream / External Gap | Honest iOS Disposition |
|---|---|---|---|
| `BLK-01` | Server APNs HTTP/2 Push Delivery | Upstream `gpui_telegram_server` `apns.rs` HTTP/2 push dispatcher is a server stub. | iOS client implements real `UNUserNotificationCenter` + `registerForRemoteNotifications` token registration; end-to-end server push receipt remains `blocked` until server APNs HTTP/2 ships. |
| `BLK-02` | WebRTC NAT Traversal / Server SFU & Two-Device Live Media | Upstream `gpui_telegram_webrtc` uses loopback SDP/ICE without production server NAT/SFU; CI lacks two physical iOS devices. | iOS preserves real `LiveKitWebRTC` + `CallKit` + `ReplayKit` owners; production SFU NAT traversal and two-device live call media remain `blocked` until server SFU + dual-device verification exist. |
| `BLK-03` | TON On-Chain Signing & Telegram Passport | Upstream marks TON on-chain signing and Telegram Passport partial/planned. | Exposed honestly as partial/planned capability notices in Settings/Wallet; never fabricated as live on-chain signing. |
| `BLK-04` | StoreKit 2 Server-Side Receipt / Entitlement Binding | Upstream server StoreKit 2 JWS receipt validation endpoint may be unprovisioned. | iOS implements real App Store compliant StoreKit 2 (`FabushiPayStoreKit`) + Restore Purchases; server entitlement verification remains `blocked` if server endpoint is absent. |
| `BLK-05` | Legacy / Planned Upstream Backlog (`OCRM-001`, `FBCP-001`, `TDRP`) | 141 unwired OCRM external channel adapters, FBCP D1, and 467 unreferenced TDRP shard files remain requirement-only/unwired upstream. | Tracked explicitly in parity ledger as `upstream-planned`; never stamped `verified` or silently converted to `not-applicable`. |
| `BLK-06` | Cloudflare `.fabapp` Production Signer Secrets | Production `.fabapp` Ed25519 signing keys/secrets may be absent from ordinary CI runs. | Offline verifier (`OfflineMiniAppPackageVerifier`) fails closed on unsigned/tampered bundles; production signer publishing remains `blocked` if secrets are unconfigured. |
| `BLK-07` | Physical Device Install/Upgrade/Rollback & `.broadcast` ASC Provisioning | GitHub Actions lacks physical iOS hardware and requires valid App Store Connect signing profiles for both main app and `FabushiBroadcastUpload` extension. | Tracked explicitly under `IOS-MAIN-AC-16..18`; `blocked` until signed archive, `.broadcast` provisioning, TestFlight upload, and physical device install/upgrade evidence pass on exact HEAD. |

---

## 6. Interface Contracts

### 6.1 `source/desktop_core` <-> `source/host` (Fail-Closed C-ABI FFI + JSON-RPC)
- **Exported C Symbols** (`source/host/include/mahayana_app_host.h`):
  - `fabushi_host_create(config_json: *const c_char) -> *mut c_void` (with unified compatibility aliases `mahayana_app_host_*` / `telegram_app_host_*` routed strictly to the same real engine — zero separate stub engine)
  - `fabushi_host_dispatch(handle: *mut c_void, request_json: *const c_char) -> *mut c_char`
  - `fabushi_host_poll_events(handle: *mut c_void, max_count: u32, timeout_ms: u32) -> *mut c_char`
  - `fabushi_host_cancel(handle: *mut c_void, request_id: *const c_char) -> bool`
  - `fabushi_host_destroy(handle: *mut c_void)`
  - `fabushi_host_free_string(ptr: *mut c_char)`
- **JSON-RPC Envelope**:
  - Request: `{"id": "<uuid>", "method": "<namespace>.<action>", "generation": <u64>, "account_id": "<id?>", "params": {...}}`
  - Success: `{"id": "<uuid>", "ok": true, "generation": <u64>, "result": {...}}`
  - Fail-Closed Error: `{"id": "<uuid>", "ok": false, "generation": <u64>, "error": {"code": "<typed_code>", "message": "<human_safe>", "retryable": <bool>}}`
- **Hard Invariants**: Unknown `method` MUST return `ok: false, error.code: "unknown_command"`. Every FFI entrypoint wrapped in `std::panic::catch_unwind`. Zero `"status": "acknowledged"` fallbacks. Zero hardcoded `localhost` URLs. Zero fake sample chats.
- **33 Required RPC Namespaces**:
  `auth.*`, `vault.*`, `session.*`, `chats.*`, `folders.*`, `topics.*`, `contacts.*`, `messages.*`, `sync.*`, `outbox.*`, `e2ee.*`, `search.*`, `media.*`, `stories.*`, `stickers.*`, `fabushi.*`, `multi_agent.*`, `magpie.*`, `coding.*`, `mcp.*`, `computers.*`, `remote.*`, `tailnet.*`, `miniapp.*`, `webmcp.*`, `calls.*`, `talk.*`, `wallet.*`, `settings.*`, `proxy.*`, `push.*`, `actions.*`, `generation.*`.

### 6.2 `source/host` <-> `mobile/ios/Fabushi/Core` (Swift 6 Strict Concurrency)
- `FabushiRustHostActor` (`actor`, `Sendable`): owns opaque C-ABI handle, serializes lifecycle (`initialize`, `switchAccount`, `logout`, `shutdown`), provides typed `dispatch<Req: Encodable & Sendable, Res: Decodable & Sendable>(method:params:) async throws -> Res`, exposes `events: AsyncStream<FabushiDomainEvent>`.
- `FabushiCoordinator` (`@MainActor @Observable`): single canonical coordinator owning `authState`, `accountScopeGeneration: UInt64`, active `ShellRoute`, and typed domain stores (`ChatStore`, `AgentStudioStore`, `RemoteComputerStore`, `MiniAppHostStore`, `CallSessionStore`, `SettingsStore`, `VaultStore`).
- **Generation Fencing Invariant**: Every async RPC checks `accountScopeGeneration` upon return and discards stale responses from prior accounts/sessions without mutating UI state.
- **Zero Mocks Invariant**: No production Swift bridge class may hold hardcoded fake phone numbers, fake sessions, fake Keychain keys, `Task.sleep` stubs, or `mockBridge: true` production paths.

### 6.3 `DesignSystem` & `Presentation/Shell` <-> Feature Modules (M2–M5)
- `FabushiTheme` (`@Observable` environment object) + `ShellNavigationRouter` (`@MainActor @Observable`).
- Module registration protocols defined in M2: `MessagingModuleProviding` (M3), `AgentStudioModuleProviding` (M4), `PlatformModulesProviding` (M5).
- Stable `accessibilityIdentifier` namespaces for opaque-box E2E tests: `fabushi.auth.*`, `fabushi.shell.*`, `fabushi.chatlist.*`, `fabushi.compose.*`, `fabushi.stage.*`, `fabushi.composer.*`, `fabushi.inspector.*`, `fabushi.studio.*`, `fabushi.magpie.*`, `fabushi.remote.*`, `fabushi.miniapp.*`, `fabushi.call.*`, `fabushi.settings.*`.

---

## 7. Ledger Policy (`IOS-MAIN-LEDGER-01..08`), Failure Modes & Edge Cases

### 7.1 Two-Phase Ledger Lifecycle & Status Rules (`IOS-MAIN-LEDGER-01..08`)
- **`IOS-MAIN-LEDGER-01`**: Two-phase file ledger mode recorded in `manifests/desktop-main-authority.json`:
  - Phase 1 (`fileLedgerMode: "legacy-predecessor-archived"`, M1a–M5): Legacy `bhrumom/fabushi-desktop@0ecf1326` per-file manifest/ledger/nonselected/impact chunks remain archived on disk as historical predecessor provenance; lock closure booleans (`baselineReady`, `dependenciesReviewed`, `completenessAccepted`, `acceptanceAccepted`) MUST remain `false`, and `--complete` UNCONDITIONALLY FAILS.
  - Phase 2 (`fileLedgerMode: "current-main-complete"`, required for M6+ and `--complete`): Full 1:1 `crates/**` + `nonselectedRegister` + `impactRegister` + `fullTreeSha256` + `selectedTreeSha256` regeneration bound to exact live `bhrumom/gpui-telegram@main`.
- **`IOS-MAIN-LEDGER-02`**: Status progression: `unreviewed -> understood (optional) -> mapped -> implemented -> verified` (or `not-applicable` solely via `not-applicable-with-replacement` with observable equivalent effect). Blockers are orthogonal; `blocked` is never complete.
- **`IOS-MAIN-LEDGER-03`**: Any changed or impacted row reverts to its last independently supported status and never inherits prior `verified`.
- **`IOS-MAIN-LEDGER-04`**: `implemented` requires real shipping target path/symbol and production composition; `verified` additionally requires current Desktop main oracle, current exact iOS HEAD, passing executable CI tests, artifact digest, and independent reviewer.
- **`IOS-MAIN-LEDGER-05`**: Impact propagation covers both direct changed files and affected sibling/consumer owners and protocols.
- **`IOS-MAIN-LEDGER-06`**: Anti-stub enforcement forbids `"status": "acknowledged"` fallbacks and fake sample chats in `source/host/src/**` (enforced via `--check-host-bridge` / `--complete` in M1a and unconditionally under `--strict` from M1b onward).
- **`IOS-MAIN-LEDGER-07`**: Anti-local-`TEST_READY.md` enforcement: root `TEST_READY.md` is rejected fail-closed if it contains local developer paths (`/Users/`, `/home/<user>/`, Windows `C:\Users\`), local execution markers, stale `bhrumom/fabushi-desktop` claims, or lacks GitHub Actions run URL/ID + 40-hex commit SHA provenance.
- **`IOS-MAIN-LEDGER-08`**: Legacy `bhrumom/fabushi-desktop` and PR #20 ledgers remain historical diagnostics only and never satisfy `bhrumom/gpui-telegram@main` acceptance.

### 7.2 Failure Modes & Edge Cases
- **CJK IME Composition**: `UITextView` `markedTextRange != nil` MUST suppress Enter-to-send and slash-command commit until IME composition commits.
- **iOS Background Suspension (`0xdead10cc`)**: WAL checkpoint + POSIX lock release MUST complete before app suspension; cold relaunch recovers SQLite WAL and offline outbox cleanly.
- **Account Switch / Logout Race**: Generation increment invalidates all in-flight RPCs, WebSocket streams, and cached actor callbacks; zero cross-account data leakage.
- **Offline Outbox & Nonce Dedup**: Disconnected sends queue durably in SQLite WAL with 24h `client_nonce` idempotency and jittered backoff on reconnect (`NWPathMonitor`).
- **Upstream Live Ref Drift**: If `bhrumom/gpui-telegram@main` advances mid-run, end-of-job `--authority-only` fails closed rather than falsely promoting stale parity.

---

## 8. Fail-Closed CI Gates (`IOS-MAIN-GATE-01..10`), Release / Migration / Rollback & Observability

### 8.1 Fail-Closed CI Gates (`IOS-MAIN-GATE-01..10`)
- **`IOS-MAIN-GATE-01`**: All builds, unit tests, integration tests, cross-compilation checks, UI/E2E suites, and validators run exclusively in GitHub Actions (`ubuntu-latest` + `macos-15` + `sccache`).
- **`IOS-MAIN-GATE-02`**: Every required workflow job checks out exact `IOS_EXPECTED_HEAD`, validates `git rev-parse HEAD == IOS_EXPECTED_HEAD`, and runs live `--authority-only` at BOTH job start and job end (`if: always()`).
- **`IOS-MAIN-GATE-03`**: Zero skipped required steps; skipped, cancelled, neutral, warning-only, or environment-blocked outcomes are never treated as `passed`.
- **`IOS-MAIN-GATE-04`**: Full non-truncated Git tree verification against `bhrumom/gpui-telegram@main` (`--desktop-checkout` or authenticated GitHub Git Trees API with `truncated == false`), asserting exact blob count, `0` gitlinks, `crates/**` + `inherited/**` counts, 18-directory crate closure, and SHA-256 tree digests.
- **`IOS-MAIN-GATE-05`**: 100% iOS-owned dependency fence: zero external path/git repo dependencies, zero escaping `../` or absolute paths, and zero forbidden `gpui`, `khronos-egl`, or excluded desktop crates in `source/desktop_core/**` or mobile iOS manifests.
- **`IOS-MAIN-GATE-06`**: `rust-desktop-core-ubuntu` compiles all workspace test targets (`--no-run`) and executes all deterministic offline Rust unit + integration tests across all vendored crates with `GPUI_OFFLINE_TEST=1` (zero deleted tests, zero weakened assertions).
- **`IOS-MAIN-GATE-07`**: `rust-desktop-core-ios-check` (`macos-15`) verifies `cargo check` for BOTH `aarch64-apple-ios-sim` AND `aarch64-apple-ios` across the entire `source/desktop_core` workspace and confirms zero `gpui`/`khronos-egl` nodes in the iOS `cargo tree`.
- **`IOS-MAIN-GATE-08`**: Staged CI expansion across milestones: M1a (authority + `desktop_core` Ubuntu tests + iOS target check) -> M1b (`source/host` C-ABI + `FabushiCore` Swift tests) -> M2–M6 (XcodeGen, Swift unit tests, Simulator XCUITest, E2E Tiers 1–4, unsigned/signed archive).
- **`IOS-MAIN-GATE-09`**: No test deletion, warning downgrade, or assertion relaxation to turn CI green; genuine defects are fixed at root cause on a new exact HEAD.
- **`IOS-MAIN-GATE-10`**: Complete evidence recording: workflow run ID, attempt, URL, exact `headSha`, per-job/step conclusions, test counts, and `sha256:` artifact digests (plus explicit negative evidence for blockers).

### 8.2 Release, Migration, Rollback & Observability
- **Release Target**: `MARKETING_VERSION: 1.6.7`, `CURRENT_PROJECT_VERSION: 35` (superseding `1.2.65` / `34`), publishing GitHub Release `v1.6.7` + signed TestFlight build via `ios-store-delivery.yml` (fixing `FabushiBroadcastUpload` extension signing profile handling) and `ios-release.yml`.
- **Upgrade & Rollback**: Preserve existing SQLite/Keychain user data across upgrades from prior installed builds with versioned schema migrations and fail-closed backup/rollback before destructive migration.
- **Observability**: Structured Rust `tracing` (redacted PII/secrets) bridged to Apple `os.Logger` subsystems (`com.ombhrum.fabushi.*`) with explicit health/sync/storage/audit diagnostics.

---

## 9. Normative Acceptance Criteria (`IOS-MAIN-AC-01..21`), Spec Compliance Record & Provenance

### 9.1 Release Definition of Done (`IOS-MAIN-AC-01..21`)
- `IOS-MAIN-AC-01`: Current live `bhrumom/gpui-telegram@main` exact commit and root tree confirmed before and after acceptance; `manifests/desktop-main-authority.json` consistent.
- `IOS-MAIN-AC-02`: All tracked files and submodule/dependency boundaries accounted; no partial or truncated inventory.
- `IOS-MAIN-AC-03`: Selected `crates/**` 100% file and responsibility ledger regenerated with current source blob identity (`fileLedgerMode == "current-main-complete"`).
- `IOS-MAIN-AC-04`: Requirement-only upstream work and upstream stubs distinguished from implemented shipping behavior and tracked honestly to closure.
- `IOS-MAIN-AC-05`: Entire applicable `bhrumom/gpui-telegram@main` (`v1.6.7`) product migrated; all mandatory responsibilities verified or legitimate `not-applicable-with-replacement`.
- `IOS-MAIN-AC-06`: Canonical existing-owner-first composition; single Apple HIG shell/root per product concept; zero Stratum A/B split-brain or duplicate stores.
- `IOS-MAIN-AC-07`: Native Swift 6 Coordinator / Host / Runner typed boundaries with real fail-closed C-ABI routing, cancellation, generation fencing, checkpoint, and recovery.
- `IOS-MAIN-AC-08`: Complete 100% iOS-owned source/build inputs; zero shared Fabushi runtime repository or forbidden desktop GUI/EGL dependencies.
- `IOS-MAIN-AC-09`: Login, Split-Key V2 vault, Passkeys/2FA/sessions, multi-account, messaging, E2EE, identity, and security continuity tested.
- `IOS-MAIN-AC-10`: Media CAS/streaming, 5-bit voice, offline ASR, calls/Talk Mode, Remote Workbench/PTY, background tasks, and platform capability parity tested.
- `IOS-MAIN-AC-11`: Agent Studio, Magpie, Coding Agents, Official MCP, Mini Apps/WebMCP/全球法布施, and StoreKit 2 billing/entitlement/restore journeys tested.
- `IOS-MAIN-AC-12`: Real API/service-side dependencies and provisioning verified; blocked external/upstream dependencies honestly marked `blocked` (never counted `passed`).
- `IOS-MAIN-AC-13`: Real Apple HIG UI functional, visual, 4-theme contrast, Dynamic Type, VoiceOver, `zh-Hans`/`en` localization, and iPhone/iPad adaptive flows covered.
- `IOS-MAIN-AC-14`: Rust unit/integration/contract tests, Swift unit/UI tests, opaque-box 4-tier E2E suite, fault/temporal/regression, and performance/soak verified on GitHub Actions.
- `IOS-MAIN-AC-15`: App lifecycle cold launch, foreground/background (`0xdead10cc`-safe WAL checkpoint), termination, relaunch, and data recovery proven.
- `IOS-MAIN-AC-16`: Signed device archive/export (including `FabushiBroadcastUpload` extension) and clean physical-device installation/launch passed.
- `IOS-MAIN-AC-17`: Update from previous accepted install and data migration rollback strategy/evidence passed.
- `IOS-MAIN-AC-18`: TestFlight distribution/install and App Store identity/signing/entitlement/privacy readiness passed for `v1.6.7` (build `35`).
- `IOS-MAIN-AC-19`: License, provenance, and legal notices (`NOTICE.md`, `docs/provenance/`) resolved for release.
- `IOS-MAIN-AC-20`: Independent reviewer verified exact source SHA, real CI job/step logs (0 skipped required steps), artifacts/digests, and compliance ledger.
- `IOS-MAIN-AC-21`: Final PR merge into `fabushiios/fabushi-ios` `main`, final `main` exact HEAD, and post-merge/release CI (`v1.6.7`) revalidated.

### 9.2 Provenance & References
- Upstream authority: `bhrumom/gpui-telegram@main` (`Fabushi v1.6.7`, Apache-2.0, `6141456e2a1121b78a6cdcd5da2a6d24c9c37e15`).
- Legacy lineage: `bhrumom/fabushi-desktop@78d076cb8cc7d1ec157974cb9e91df9f599f0a71`, `docs/specs/fabushi-desktop-main-ios-parity.md`, `docs/history/ios-desktop-pr20-spec-2026-10-08.md`.
- Licensing/notices: `NOTICE.md` and `docs/provenance/` (audited and finalized before `IOS-MAIN-AC-19`).

### 9.3 Spec Compliance Record (Initial Cutover State — Not Complete)

Allowed final statuses: `passed`, `blocked`, `not-applicable`. All criteria start `pending` at M1a authority cutover; zero premature `passed` claims are permitted.

| Criterion | Status | Evidence / Reason |
|---|---|---|
| `IOS-MAIN-AC-01` | `pending` | Cutover initialized in M1a; requires exact-HEAD CI start/end verification and final acceptance re-check. |
| `IOS-MAIN-AC-02` | `pending` | Non-truncated `11,342`-blob / `0`-gitlink tree check active in M1a; full closure pending M6. |
| `IOS-MAIN-AC-03` | `pending` | `fileLedgerMode: "legacy-predecessor-archived"` during M1a–M5; full `crates/**` ledger regeneration scheduled in M6. |
| `IOS-MAIN-AC-04` | `pending` | Honest stubs/blockers registered in Section 5; closure audit pending M6. |
| `IOS-MAIN-AC-05` | `pending` | Product feature parity implementation spans M2–M5. |
| `IOS-MAIN-AC-06` | `pending` | Unified Apple HIG shell and legacy split-brain removal span M2–M6. |
| `IOS-MAIN-AC-07` | `pending` | Fail-closed C-ABI FFI + Swift 6 Coordinator/Host/Runner implementation in M1b. |
| `IOS-MAIN-AC-08` | `pending` | Pure-Rust `source/desktop_core/` vendored in M1a; full standalone archive proof pending M6. |
| `IOS-MAIN-AC-09` | `pending` | Auth, Vault, Passkeys, Multi-Account, Messaging & E2EE implementation spans M2–M3. |
| `IOS-MAIN-AC-10` | `pending` | Media, Voice, Calls/Talk Mode, Remote Workbench/PTY implementation spans M3 & M5. |
| `IOS-MAIN-AC-11` | `pending` | Agent Studio, Magpie, Coding Agents, MCP, Mini Apps & StoreKit 2 span M4–M5. |
| `IOS-MAIN-AC-12` | `pending` | External/server dependencies tracked honestly in Section 5 (`BLK-01..06`). |
| `IOS-MAIN-AC-13` | `pending` | Apple HIG UI/UX, accessibility, localization & iPhone/iPad testing span M2–M6. |
| `IOS-MAIN-AC-14` | `pending` | Rust `desktop_core` CI starts in M1a; full Swift/UI/4-tier E2E suite spans M1b–Final. |
| `IOS-MAIN-AC-15` | `pending` | Lifecycle / `0xdead10cc` / recovery verification spans M1b–M6. |
| `IOS-MAIN-AC-16` | `pending` | Signed archive & physical device verification tracked under `BLK-07` / M6–Final. |
| `IOS-MAIN-AC-17` | `pending` | Upgrade & migration rollback verification pending M6–Final. |
| `IOS-MAIN-AC-18` | `pending` | TestFlight & App Store `v1.6.7` (build `35`) readiness pending M6–Final. |
| `IOS-MAIN-AC-19` | `pending` | `NOTICE.md` and `docs/provenance/` clearance scheduled in M6. |
| `IOS-MAIN-AC-20` | `pending` | Independent evidence review required on final release HEAD. |
| `IOS-MAIN-AC-21` | `pending` | Final PR merge to `main` and post-merge/release CI validation pending Final Milestone. |
