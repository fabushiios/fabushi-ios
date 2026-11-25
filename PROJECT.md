# Project: Fabushi iOS — Full `bhrumom/gpui-telegram` (`v1.6.9`) Parity, Apple HIG UI/UX Redesign & New Release

## Upstream Authority & Reconciliation

- **Original User Request** (`.agents/teamwork/ORIGINAL_REQUEST.md`, `2026-11-23T03:18:47Z`):
  `全面对齐bhrunom/gpui-telegram，使用苹果的设计风格去全面重构UIUX，全面实现桌面端的所有功能，发布全新版本`
- **Verified Live Upstream Authority**:
  - `bhrunom/gpui-telegram` -> **HTTP 404** (`users/bhrunom` does not exist; verified 1-character typo for `bhrumom/gpui-telegram`).
  - **Primary Live Upstream**: `bhrumom/gpui-telegram` (GitHub repo ID `1449467779`, branch `main`), exact HEAD `57a62ef92be2fa816704d7337aca3254dc8f943e`, root tree `733c8dcd0e65be3d1829517bba5ecae7b622cb70`, **`Fabushi v1.6.9`** (advanced from `v1.6.8` `424aa33cf67a` at `2026-11-23T09:33:44Z`: macOS TCC stale-csreq recovery, Gemini HUD binding, popup spam, desktop hardening; `11,353` blobs: `3,064` `crates/**` + `7,713` `inherited/**`, `0` gitlinks).
  - **Archived Intermediate Mirror**: `bhrum/gpui-telegram` (ID `1456601492`, `archived: true`, frozen at `v1.3.0` `b436dcdc`, 26 commits behind; previously referenced by unmerged PRs #11/#12).
  - **Legacy Electron Predecessor**: `bhrumom/fabushi-desktop` (ID `1375312893`, inactive since `2026-11-06`, HEAD `78d076cb8cc7`, `v1.2.78`, 9,196 blobs; domain crates verbatim absorbed into `bhrumom/gpui-telegram/inherited/`).
  - **Target iOS Repository**: `fabushiios/fabushi-ios` (ID `1375312995`; `bhrumom/fabushi-ios` and `fabushi-ios/fabushi-ios` HTTP 301 redirect here), baseline `main` HEAD `8128951a71503a6f109ce0cb37618d022fe8e79a`.
- **Governance & Execution Policy (`AGENTS.md` + `docs/specs/spec-first-ai-development.md:28`)**:
  1. **Spec-First Cutover (M1)**: Per `docs/specs/spec-first-ai-development.md:28`, formally amend `AGENTS.md`, canonical parity Spec (`docs/specs/gpui-telegram-main-ios-parity.md`), `manifests/desktop-main-authority.json`, architecture checker, and CI workflows to pin live `bhrumom/gpui-telegram@main` (`57a62ef92be2` `v1.6.9`) BEFORE product code edits, while retaining full traceability to `bhrumom/fabushi-desktop@78d076cb` legacy/backlog items.
  2. **CI-Only Execution Policy (NON-NEGOTIABLE)**: ALL executable builds, tests, linters, generators, validators, benchmarks, and acceptance runs MUST execute **exclusively in GitHub Actions (`ubuntu-latest` / `macos-15`) or a user-authorized remote runtime** — NEVER locally in the assistant container or local host.
  3. **Zero Fabrication / Blocker != Complete**: Upstream stubs (`gpui_telegram_server` APNs HTTP/2 dispatcher, `gpui_telegram_webrtc` loopback SDP/ICE vs real NAT traversal, TON on-chain signing, OCRM-001 141 unwired external channel adapters) and external hardware/service blockers (physical iOS device install/upgrade, two-device live call media, Cloudflare `.fabapp` signer / StoreKit server bindings, `.broadcast` extension signing profile if ASC auto-signing is denied) MUST be explicitly tracked as `blocked`/`upstream-planned` and NEVER faked as verified.
  4. **Target New Release Version**: `MARKETING_VERSION: 1.6.9`, `CURRENT_PROJECT_VERSION: 35` (aligned 1:1 with live `bhrumom/gpui-telegram` `Fabushi v1.6.9`, superseding `1.2.65` / `34`), publishing GitHub Release `v1.6.9` + signed TestFlight upload via `ios-store-delivery.yml` (subject to ASC signing verification).

## Architecture

### Single Canonical iOS Architecture (Eliminating Stratum A/B Split-Brain)
Current `main` (`8128951`) collides legacy Stratum A (`GrokMobileShell` + `mahayana-app-host-mobile`) with broken Stratum B (`TelegramRootShellView` + stub `source/host/src/dispatch.rs` + 9/10 orphaned views + uninitialized `TelegramBridgeRuntime`). We unify onto **one canonical standalone iOS architecture** preserving `Coordinator / Host / Runner` and existing working iOS-native owners (`LiveKitWebRTC` + `CallKit` + `FabushiBroadcastUpload`, `MiniAppWebMcpSurface` + `OfflineMiniAppPackageVerifier`, `FabushiPayStoreKit`, `VoiceRecorder` + `OfflineSpeechTranscriber`, Keychain security):

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Apple HIG Native Presentation Layer (SwiftUI + UIKit, Swift 6, iOS 17+) │
│  ├─ DesignSystem (4 v1.6.7 Themes: Night/Tinted/Day/Classic, Liquid     │
│  │    Glass #available(iOS 26,*), Dynamic Type, VoiceOver, zh-Hans/en)  │
│  ├─ Unified Adaptive Shell (iPad 3-col NavigationSplitView + .inspector │
│  │    / iPhone TabView + NavigationStack + .sheet Inspector + PiP/HUD)  │
│  ├─ Feature Surfaces: Auth/Vault, Chats/Compose, Stage/Composer, Muse   │
│  │    Inspector, Media/Stories/IV, 7-Tab Agent Studio, Magpie/MCP/      │
│  │    Coding, Remote Workbench/PTY, Mini Apps/WebMCP/全球法布施,        │
│  │    Calls/Talk Mode, 26-Section Settings, Wallet/StoreKit 2           │
├─────────────────────────────────────────────────────────────────────────┤
│ Swift 6 Coordinator / Host / Runner Layer (mobile/ios/Fabushi/Core/)    │
│  ├─ FabushiCoordinator (generation-fenced lifecycle, account isolation, │
│  │    approval & lease enforcement, network/background task routing)    │
│  ├─ FabushiRustHostActor (typed Swift 6 wrapper over C-ABI FFI,         │
│  │    AsyncStream event bus, zero mocks, fail-closed errors)            │
│  ├─ Native Apple Runners (Keychain/SecureEnclave, LocalAuthentication,  │
│  │    ASAuthorization Passkeys, ASWebAuthenticationSession, URLSession  │
│  │    background CAS, AVFoundation/AudioEngine/VAD, LiveKitWebRTC,      │
│  │    CallKit, ReplayKit, WKWebView/WebMCP, StoreKit 2, AppIntents)     │
├─────────────────────────────────────────────────────────────────────────┤
│ Fail-Closed C-ABI Rust Host Bridge (source/host/ -> libmahayana_app_host)│
│  ├─ Zero stubs, zero "acknowledged" fallback, panic-safe catch_unwind   │
│  ├─ 100% iOS-owned vendored bhrumom/gpui-telegram@6141456e pure-Rust    │
│  │    crates (source/desktop_core/crates/*): core, proto, client,       │
│  │    media, webrtc, agent, coding-protocol, magpie_gateway,            │
│  │    bot-browser (vault/e2ee_sync/totp), fabushi_remote_engine,        │
│  │    fabushi_miniapp (protocol/WebMCP)                                 │
└─────────────────────────────────────────────────────────────────────────┘
```

## Feature Inventory

Every Survey feature (34 Desktop domains + 3 Governance/HIG/Release domains = 37 items) is assigned to an explicit milestone (0 unassigned):

| # | Feature | Description | Desktop Status | Typed iOS Delta | Milestone | Source |
|---|---------|-------------|----------------|-----------------|-----------|--------|
| 1 | Authority Cutover, Spec & CI Foundation | Amend `AGENTS.md`, canonical parity Spec, `manifests/desktop-main-authority.json`, checker, `.gitignore`, vendored `v1.6.7` crates, fail-closed C-ABI FFI, staged GitHub Actions CI | `GOVERNANCE` | CI-only execution (`ubuntu-latest` + `macos-15` + `sccache`), 100% iOS-owned crates | M1 | survey_1, survey_3 |
| 2 | Password Login, Registration & Split-Key V2 Bootstrap | Username/password auth + Argon2id Split-Key V2 KEK derivation & transparent E2EE vault unlock | `REAL SHIPPING (gpui-tg)` | iOS Keychain (`AfterFirstUnlockThisDeviceOnly`) + Rust Argon2id | M2 | survey_2 #1 |
| 3 | Email OTP & Phone SMS Verification | Email OTP registration & Phone SMS verification wizard | `REAL SHIPPING (gpui-tg)` | Native SwiftUI `.oneTimeCode` autofill + country picker | M2 | survey_2 #2 |
| 4 | Alipay QR Login & Mobile OAuth Callback | Alipay QR login polling + OAuth callback (`fabushi://`) | `REAL SHIPPING (gpui-tg)` | `ASWebAuthenticationSession` + `fabushi://` URL scheme + iPad QR | M2 | survey_2 #3 |
| 5 | Passkeys (WebAuthn/FIDO2) & QR Device Pairing | Passwordless FIDO2 Passkeys + cross-device QR pairing (`pair/init/accept/poll/verify`) | `REAL SHIPPING (gpui-tg)` | `ASAuthorizationPlatformPublicKeyCredentialProvider` + camera QR scanner | M2 | survey_2 #4 |
| 6 | Cloud 2FA Password & Active Device Sessions | Cloud 2FA CRUD + inspect & terminate active device sessions | `REAL SHIPPING (gpui-tg)` | Settings -> Privacy & Security -> Devices swipe-to-terminate | M2 | survey_2 #5 |
| 7 | Multi-Account Switcher & Local Passcode App Lock | Up to 3/6 accounts with isolated state/generation fencing + idle auto-lock PIN | `REAL SHIPPING (gpui-tg)` | Face ID / Touch ID (`LocalAuthentication`), background blur snapshot, isolated Keychain/SQLite | M2 | survey_2 #6 |
| 8 | Apple HIG Design System & Unified Adaptive Shell | 4 v1.6.7 themes (`Night`/`Tinted`/`Day`/`Classic`), custom accent WCAG contrast guard, Liquid Glass, Dynamic Type, VoiceOver, `zh-Hans`/`en`, single adaptive iPhone/iPad shell | `REAL SHIPPING (gpui-tg)` | iPad 3-col `NavigationSplitView` + `.inspector`; iPhone `TabView` + `NavigationStack` + `.sheet`; 0 fixed fonts | M2 | survey_2 #7, #31; survey_3 |
| 9 | Grok Bot Multi-Select Compose (`To:` Picker) | Inline Apple Messages `New Message` stage with multi-select human + bot chips, group title, atomic DM/group creation | `REAL SHIPPING (gpui-tg)` | Native iOS Messages-style `To:` chip bar + IME-safe autocomplete | M3 | survey_2 #8 |
| 10 | Unified Chat List, Folders, Archive, Pin, Mute & Forum Topics | Single list for Humans/Bots/Groups/Channels/Saved/Topics, folder pills, swipe actions, pin stability | `REAL SHIPPING (gpui-tg)` | Native `List` + folder pill bar + iOS swipe actions + context menu | M3 | survey_2 #9 |
| 11 | Contacts, Peer Resolution, Blocklist, Send-As & Prestart Bot | Contacts CRUD, `@username` resolve, blocklist, Send-As identity, virtual Prestart Bot hero card | `REAL SHIPPING (gpui-tg)` | Native Contacts sheet + Send-As picker + Bot hero `Start` stage | M3 | survey_2 #10 |
| 12 | Atomic PTS/QTS Sync, Nonce Dedup & Offline Outbox | Monotonic PTS log, WS real-time push + `SyncGetDifference` + `snapshot_required`, 24h `client_nonce` dedup, SQLite WAL outbox w/ jittered backoff | `REAL SHIPPING (gpui-tg)` | Rust `gpui_telegram_client` core + `NWPathMonitor` + `BGAppRefreshTask` | M1 (core), M3 (UI) | survey_2 #11 |
| 13 | Virtualized Transcript & 11+ Canonical Message Cards | Bottom-anchored 120Hz transcript, 900s bubble grouping + tails, date pills, 11+ cards (Text/MD/Math/Mermaid/Spoiler, Photo/Video, 5-bit Voice, File, Sticker, Album, Location, Poll, Todo, Service, Call, Story, Invoice, Inline Keyboard, `AgentThinkingBubble`, `ToolPermissionDock`, Computer card) | `REAL SHIPPING (gpui-tg)` | Smooth SwiftUI/UIKit transcript, Dynamic Type, long-press reactions + `UIMenu`, ProMotion | M3 | survey_2 #12 |
| 14 | Message Edit, Delete Tombstone, Forward, Reactions, Pins, TTL & Export | In-place edit, revoke-for-everyone tombstone, multi-forward, PTS-fenced reactions, pins, auto-delete TTL, clear/leave, JSON export & chat stats | `REAL SHIPPING (gpui-tg)` | Swipe-to-reply, multi-select toolbar, `UIActivityViewController` export | M3 | survey_2 #13 |
| 15 | IME-Safe Canonical Composer, Drafts, Formatting, Voice Gesture & Send Options | `UITextView` CJK IME fencing, synced per-chat drafts, formatting bar, emoji/sticker/GIF picker, real `AVFoundation` hold/cancel/lock voice gesture, silent/scheduled send | `REAL SHIPPING (gpui-tg)` | `UITextView` (`markedTextRange`), Apple pill bar, haptics, `PhotosPicker`/`UIDocumentPicker`/Camera | M3 | survey_2 #14 |
| 16 | 49+ Hermes/Fabushi Slash Commands & `@` Context Badges | Autocomplete `/` command palette across 7 categories + `@file`/`@dir`/`@diff`/`@git:`/`@url` chips | `REAL SHIPPING (gpui-tg)` | Floating keyboard-accessory slash palette + Files/clipboard/remote-path `@` picker | M3 | survey_2 #15 |
| 17 | Muse Context Inspector (Profile, Members, Shared Media, Bot/Computer/Coding Config) | Centered 80pt hero, 4-icon bar, Profile/Members/5-tab Shared Media/Settings, Bot Computers & Bot Config Editor subpages | `REAL SHIPPING (gpui-tg)` | iPad 320pt `.inspector`, iPhone contact-header push/`.sheet` | M3 | survey_2 #16 |
| 18 | Unified Search (Tantivy Full-Text + Hybrid Memory + Bots/MiniApps) | Global & in-chat search with category pills + hybrid semantic/entity memory + local offline FTS5 | `REAL SHIPPING (gpui-tg)` | Native `.searchable` + filter chips + SQLite FTS5 offline fallback | M1 (core), M3 (UI) | survey_2 #27 |
| 19 | E2EE Messaging (Double Ratchet, Sender Keys & Safety Numbers) | PreKey server endpoints, Double Ratchet 1:1, Sender Keys group, 4-emoji + QR safety number verification | `PARTIAL / STUB (gpui-tg)` | Rust crypto core + Keychain keys + Secret/E2EE UI toggle & Safety Number sheet | M1 (core), M3 (UI) | survey_2 #28 |
| 20 | Chunked Media CAS, HTTP 206 Streaming, 5-Bit Voice, Lightbox, Editor, Stickers/GIFs, Stories & Instant View | SHA-256 resumable upload, 206 `AVPlayer` streaming, 63-byte 5-bit waveform, Lightbox, Editor, Stickers/GIFs, Stories, Instant View | `REAL SHIPPING (gpui-tg)` | `URLSession` background transfers, `AVPlayer`/`AVAudioEngine`, PencilKit/crop editor, QuickLook | M1 (core), M3 (UI) | survey_2 #25 |
| 21 | Zero-Knowledge Split-Key V2 & X25519 Credential Vault + TOTP (+ Remote Obscura Browser) | E2EE cloud credential vault (`password`/`browser_cookie`/`ai_oauth`), X25519 cross-device sync approval, TOTP authenticator, remote Obscura browser control | `REAL SHIPPING (gpui-tg)` | Native Keychain + Rust `bot-browser::e2ee_sync` vault/TOTP; remote browser host control (sandbox-safe) | M1 (core), M2 (UI) | survey_2 #23 |
| 22 | Unified Hermes/Fabushi Agent Runtime, 29 Toolsets & 7-Tab Agent Studio | `GlobalTasksWorkspace`: Tasks/Kanban/Subagents, `SKILL.md` Skills, `MEMORY`/`USER`/`SOUL` + Honcho + Quarantine, 29 Toolsets/MCP, 5-field Cron/Heartbeat, Sessions, Value-Bound Approvals | `REAL SHIPPING (gpui-tg)` | Dedicated Studio tab/sidebar section, native Kanban/Markdown editors, approval quick-actions | M4 | survey_2 #17 |
| 23 | Multi-Agent Protocol (Floor Control, 5-Hop Loop Budget, Shared Workspace, Grok `shouldReply`) | `FloorControlQueue` (`UserInterruption` preempt), 5-hop `LoopBudget`, revision-fenced `SharedGroupWorkspace`, Grok animated persona avatars | `REAL SHIPPING (gpui-tg)` | Live floor/typing pills, Workspace Inspector editor, Reduce-Motion-safe Grok avatars | M4 | survey_2 #18 |
| 24 | Pure-Rust Magpie AI Provider Gateway, Routing, Masking & Vault OAuth Sync | Multi-provider presets/BYOK, live `models.dev` catalog, Smart Routing Groups + circuit breakers, PII Privacy Masking + preview, `.env`/deep-link import, usage/cost | `REAL SHIPPING (gpui-tg)` | Full Magpie Settings UI + Zero-Knowledge Vault sync of desktop OAuth (`USER_VAULT_CATEGORY_AI_OAUTH`) + `ASWebAuthenticationSession` (no local `~/.claude` scan) | M4 | survey_2 #19 |
| 25 | Third-Party Coding Agents (Claude Code, Codex, ACP) Mobile Client | BotFather NL bot creation, coding host probe, streaming diffs/worktrees/checkpoints/PRs, permission controls via `CodingAgentBroker` | `REAL SHIPPING (gpui-tg)` | Mobile chat/diff/checkpoint/approval client talking to user's remote desktop/cloud coding host (no local subprocess) | M4 | survey_2 #20 |
| 26 | Official OAuth 2.1 Remote MCP Server Grants & Audit | Manage OAuth 2.1 PKCE grants/scopes, approve QR-poll login, inspect redacted 27-tool `mcp_tool_audit`, remote kill-switch | `REAL SHIPPING (gpui-tg)` | Native iOS MCP Grants, QR approval & Audit console | M4 | survey_2 #21 |
| 27 | UU-Remote Computer Workbench, PTY Terminal, Tailnet & Controlled Banner | Remote desktop stream + touch/trackpad/hotkeys/multi-display/privacy/power/file transfer, multi-session PTY terminal, Cloud sandbox CRUD, Assist code, Tailnet, `<10ms` Emergency Takeover, Controlled status indicator | `REAL SHIPPING (gpui-tg)` | First-class iOS Remote Controller + PTY Terminal + Live Activity; unattended host daemon desktop-only (ReplayKit opt-in share only) | M5 | survey_2 #22 |
| 28 | Telegram-Parity External Mini Apps, WebMCP Live Control, Floating PiP & 全球法布施 | `WKWebView` + isolated `WKWebsiteDataStore` + `telegram-web-app.js` + HMAC + ToS modal + live WebMCP bidirectional broker + draggable floating PiP capsule (in-place restore) + 全球法布施 WASM Mini App | `REAL SHIPPING (gpui-tg)` | Upgrade `MiniAppWebMcpSurface` + `OfflineMiniAppPackageVerifier`; zero false success claims | M5 | survey_2 #24 |
| 29 | 1:1 & Group Calls (`LiveKitWebRTC` + `CallKit` + `ReplayKit`) & Full-Duplex AI Talk Mode | Unified Apple `CallOverlayView` wired to real `LiveKitWebRTC` + `CallKit` + `FabushiBroadcastUpload`, 4-emoji E2EE fingerprint, call history/rating + 160pt Talk Mode VAD `<60ms` barge-in (`AVAudioEngine` hardware AEC) + offline ASR | `PARTIAL / STUB (gpui-tg)` | Preserve Stratum A `LiveKitWebRTC`/`CallKit`/`ReplayKit`; mark server-side SFU/NAT & two-device physical test honestly | M5 | survey_2 #26 |
| 30 | Full 26-Section Apple Settings Suite | All 26 `SettingsSectionType` sections in native `List(.insetGrouped)` + search + real CAS/SQLite storage metrics & purge | `REAL SHIPPING (gpui-tg)` (22) / `PARTIAL` (Business/Passport/TON) | Native iOS Settings + iPad master-detail; honest capability notices for partial upstream sections | M5 | survey_2 #32 |
| 31 | Economy, Stars Ledger, Star Gifts, Premium & StoreKit 2 | Stars balance/transactions/gifts, Redeem codes, Premium tiers, App Store compliant **StoreKit 2** (`FabushiPayStoreKit`) + Restore Purchases + server entitlement sync | `REAL SHIPPING (gpui-tg)` (Stars/Alipay) / `PARTIAL` (TON/Apple IAP) | App Store Guideline 3.1.1 compliant StoreKit 2 IAP; track StoreKit server binding blocker honestly | M5 | survey_2 #29 |
| 32 | Network, Proxy (HTTP/SOCKS5/MTProto) & App Update Check | 4-tier config cascade, editable Server URL, proxy config + health probe, App Store / TestFlight version check (no prohibited binary self-update) | `REAL SHIPPING (gpui-tg)` | `URLSessionConfiguration` / Rust `reqwest` proxy; App Store/TestFlight update banner | M5 | survey_2 #33 |
| 33 | Action Bus, App Intents / Siri Shortcuts & Push Registration | Typed Action Bus mapped to iOS App Intents / Siri Shortcuts & `fabushi://` URLs; real APNs token registration (honestly tracking upstream `apns.rs` server stub) | `REAL SHIPPING` (Action Bus) / `STUB` (Server APNs HTTP/2) | Native App Intents + `UNUserNotificationCenter` / `UIApplication.registerForRemoteNotifications`; server APNs tracked `blocked` | M5 | survey_2 #30 |
| 34 | Legacy / Planned Backlog Ledger (OCRM-001, FBCP-001, TDRP Shards, TON/Passport) | Explicit parity ledger tracking of 141 unwired OCRM channel adapters, FBCP D1, 467 unreferenced TDRP shard files, TON on-chain signing & Passport — NEVER fabricated | `REQUIREMENT-ONLY / PLANNED` | Recorded honestly in parity ledger with `upstream-planned`/`server-blocked` disposition | M1, M6 | survey_2 #34 |
| 35 | Cross-Module Unification, Orphan & Legacy Cleanup, Provenance Clearance | Merge M3–M5, verify 0 orphaned views / 0 dead buttons, purge legacy Grok 0.18 reconstructed UI & update `NOTICE.md`/`docs/provenance/` | `INTEGRATION` | Clean `project.yml`, zero Swift 6 concurrency warnings, clean provenance | M6 | survey_1, survey_3 |
| 36 | Opaque-Box 4-Tier E2E Test Suite (`TEST_INFRA.md` / `TEST_READY.md`) | Requirement-driven Tier 1–4 E2E suite (>=5 Tier 1 & >=5 Tier 2 per feature, pairwise Tier 3, >=17 Tier 4 scenarios) runnable on GitHub Actions | `E2E TRACK` | Automated Rust + Swift + XCUITest CI harness | E2E Track | Project Pattern |
| 37 | Release Pipeline (`v1.6.7`), Signed Archive / TestFlight & GitHub Release | Bump `1.6.7` (build `35`), fix `ios-store-delivery.yml` `.broadcast` extension signing, add `ios-release.yml`, publish GitHub Release `v1.6.7` + execute TestFlight upload with exact-HEAD proof | `RELEASE` | Exact-HEAD GitHub Actions proof + SHA256 digests; honest blocker report if ASC/device constrained | M6, Final Milestone | ORIGINAL_REQUEST, survey_1, survey_3 |

## Milestones

| # | Name | Scope | Dependencies | Branch / Worktree | Status |
|---|------|-------|--------------|-------------------|--------|
| M1 | Governance Cutover, `v1.6.9` Rust Core Sync, Fail-Closed FFI & Staged CI | Spec-first `AGENTS.md`/Spec/Lock/checker cutover to `bhrumom/gpui-telegram@57a62ef92be2` (`v1.6.9`), `.gitignore`, vendored pure-Rust `v1.6.9` crates, zero-stub `source/host/` C-ABI bridge, Swift 6 `Core/` & `Bridge/` runtime, green GitHub Actions CI | none | `feat/m1-v167-core-ffi-ci` | **DONE** (`0401cce637f61f8480b17cbe23c9ac14cab3c244`, M1a+M1b+M1c 5/5 unanimous, Runs `45294879540` & `45294879442` Att. 2: `8/8` jobs / `133/133` steps / `0` skipped, `966/966` XCTests, `1,148` Rust core, `47/47` host FFI, `29/29` authority, 3 verified artifact SHA-256 digests; `.agents/teamwork/sub_orch_m1c_gen3/handoff.md`) |
| M2 | Apple HIG Design System, Unified Adaptive Shell, Auth & Security Vault | `DesignSystem/` (4 themes, Liquid Glass, Dynamic Type, VoiceOver, `zh-Hans`/`en`), unified iPhone/iPad `Presentation/Shell/`, full `AuthWizardView` (Password/Split-Key V2/Email/Phone/Alipay/Passkeys/QR Pair/2FA), Multi-Account, Face ID/Passcode Lock, Zero-Knowledge Vault & TOTP | M1 | `feat/m2-apple-hig-shell-auth` | IN_PROGRESS (`eca3fcc7dd002102fd4038dd469f8f3e` `sub_orch_m2_gen3` [Gen 5 post-restart recovery]; **M2a DONE 5/5 unanimous `5a49dd3941a9`** [Runs `45365226443` & `45365226532`, `8/8` jobs / `133/133` steps / `0` skipped, `991/991` XCTests, `0` warnings]; **M2b Step b SEALED** [`.agents/teamwork/sub_orch_m2_gen2/iter1_worker_m2b_resume/handoff.md`, `54f8dee5`, Runs `45403560050` & `45403560027` `8/8` GREEN, `133/133` steps, `1,004/1,004` XCTests, `0` M2 warnings, `3/3` artifacts verified]; **5-Agent Parallel Verification Wave ACTIVE**) |
| M3 | Unified Messaging, Grok Multi-Select Compose, Transcript, Composer, Inspector & Media | Chat List/Folders/Topics/Contacts/Search, Grok Multi-Select `To:` Compose, 11+ Message Cards & Actions, E2EE Safety Numbers, IME-Safe Composer + 49 Slash Commands + Voice Gesture, Muse Context Inspector, Media Lightbox/Editor/Stories/Instant View | M2 | `feat/m3-messaging-composer-media` | PLANNED |
| M4 | Hermes/Fabushi 7-Tab Agent Studio, Multi-Agent Protocol, Magpie AI Gateway, Coding Agents & Official MCP | 7-tab `GlobalTasksWorkspace`, Multi-Agent Floor/LoopBudget/Workspace, Magpie AI Providers/Routing/Masking/Vault OAuth, Coding Agents mobile client, Official OAuth 2.1 MCP grants & audit | M2 | `feat/m4-agent-studio-magpie-mcp` | PLANNED |
| M5 | Remote Computer Workbench/PTY, Mini Apps/WebMCP/全球法布施, Calls/Talk Mode & 26-Section Settings/StoreKit 2 | UU-Remote iOS Controller + PTY Terminal + Emergency Takeover, `WKWebView` Mini Apps + WebMCP + PiP + 全球法布施, unified `LiveKitWebRTC`/`CallKit`/`ReplayKit` Calls + Talk Mode VAD Orb, full 26-section Apple Settings, StoreKit 2/Stars, Proxy, App Intents & APNs registration | M2 | `feat/m5-remote-miniapp-calls-settings` | PLANNED |
| M6 | Unification, Cleanup, Provenance, CI & Release Pipeline (`v1.6.9`) | Merge M3–M5, purge legacy/orphaned split-brain & Grok reconstructed UI, bump `1.6.9` (build `35`), fix `ios-store-delivery.yml` `.broadcast` signing, add `ios-release.yml`, update parity/evidence ledgers, 100% green CI | M3, M4, M5 | `feat/m6-integration-release-v167` | PLANNED |
| E2E | Opaque-Box 4-Tier E2E Testing Track | Requirement-driven `TEST_INFRA.md`, Tier 1–4 E2E tests (`tests/e2e/**`, `mobile/ios/FabushiUITests/**`), publishes `TEST_READY.md` | none (parallel, `/home/antigravity/fabushiios/worktrees/e2e`) | `feat/e2e-suite-v167` | **DONE** (`0455e574ba1b`, Run `45153072457`, 462 tests, 5/5 unanimous, `TEST_READY.md` published) |
| Final | Final Milestone (Phase 1 E2E Tiers 1–4 -> Phase 2 Tier 5 Adversarial -> Release `v1.6.9`) | 100% E2E pass across Tiers 1–4 on GitHub Actions, Tier 5 white-box adversarial hardening until 0 gaps, merge to `main`, publish GitHub Release `v1.6.9` + run `ios-store-delivery.yml` | M6, E2E (`TEST_READY.md`) | `main` | PLANNED |

## Interface Contracts

### 1. `source/desktop_core` ↔ `source/host` (C-ABI FFI + JSON-RPC Contract)
- **Exported C Symbols** (`source/host/include/mahayana_app_host.h`):
  - `fabushi_host_create(config_json: *const c_char) -> *mut c_void` (with legacy `mahayana_app_host_*` / `telegram_app_host_*` compatibility aliases routed to the exact same real engine — zero separate stub engine!)
  - `fabushi_host_dispatch(handle: *mut c_void, request_json: *const c_char) -> *mut c_char`
  - `fabushi_host_poll_events(handle: *mut c_void, max_count: u32, timeout_ms: u32) -> *mut c_char`
  - `fabushi_host_cancel(handle: *mut c_void, request_id: *const c_char) -> bool`
  - `fabushi_host_destroy(handle: *mut c_void)`
  - `fabushi_host_free_string(ptr: *mut c_char)`
- **Envelope Schema**:
  - Request: `{"id": "<uuid>", "method": "<namespace>.<action>", "generation": <u64>, "account_id": "<id?>", "params": {...}}`
  - Response (Success): `{"id": "<uuid>", "ok": true, "generation": <u64>, "result": {...}}`
  - Response (Error — FAIL CLOSED): `{"id": "<uuid>", "ok": false, "generation": <u64>, "error": {"code": "<typed_code>", "message": "<human_safe>", "retryable": <bool>}}`
  - **Hard Invariant**: Unknown `method` MUST return `ok: false, error.code: "unknown_command"`. Every FFI entrypoint wrapped in `std::panic::catch_unwind`. Zero hardcoded localhost URLs (`DEFAULT_SERVER_URL` configurable, default `http://34.46.77.133:8080`). Zero fake sample chats.
- **Required RPC Namespaces**:
  `auth.*`, `vault.*`, `session.*`, `chats.*`, `folders.*`, `topics.*`, `contacts.*`, `messages.*`, `sync.*`, `outbox.*`, `e2ee.*`, `search.*`, `media.*`, `stories.*`, `stickers.*`, `fabushi.*` (Hermes commands/toolsets/tasks/kanban/skills/memory/soul/cron/sessions/approvals), `multi_agent.*`, `magpie.*`, `coding.*`, `mcp.*`, `computers.*`, `remote.*`, `tailnet.*`, `miniapp.*`, `webmcp.*`, `calls.*`, `talk.*`, `wallet.*`, `settings.*`, `proxy.*`, `push.*`, `actions.*`, `generation.*`.

### 2. `source/host` ↔ `mobile/ios/Fabushi/Core` (Swift 6 Strict-Concurrency Contract)
- `FabushiRustHostActor` (`actor`, `Sendable`): owns opaque C-ABI handle, serializes lifecycle (`initialize`, `switchAccount`, `logout`, `shutdown`), provides typed `dispatch<Req: Encodable & Sendable, Res: Decodable & Sendable>(method:params:) async throws -> Res`, exposes `events: AsyncStream<FabushiDomainEvent>`.
- `FabushiCoordinator` (`@MainActor @Observable`): single canonical app coordinator owning `authState`, `accountScopeGeneration: UInt64`, active `ShellRoute`, `ChatStore`, `AgentStudioStore`, `RemoteComputerStore`, `MiniAppHostStore`, `CallSessionStore`, `SettingsStore`, `VaultStore`.
- **Generation Fencing Invariant**: Every async RPC checks `accountScopeGeneration` on return; stale responses from previous account/session are dropped immediately without mutating UI state.
- **Zero Mocks Invariant**: No Swift bridge class may hold hardcoded fake phone numbers, fake sessions, fake Keychain keys, `Task.sleep` cache stubs, or `mockBridge: true` production paths.

### 3. `DesignSystem` & `Presentation/Shell` ↔ Feature Modules (M3, M4, M5)
- `FabushiTheme` (`@Observable` environment object): exposes semantic tokens (`surfacePrimary`, `surfaceSecondary`, `surfaceElevated`, `surfaceTopBar`, `bubbleIncoming`, `bubbleOutgoingGradient`, `separatorHairline`, `accentColor`, `accentForegroundColor` with WCAG contrast guard, `bubbleCornerRadius`, `wallpaper`, `locale`).
- `ShellNavigationRouter` (`@MainActor @Observable`): typed routes (`chats(chatId:messageId?)`, `compose`, `agentStudio(tab:)`, `calls`, `remoteComputer(nodeId:)`, `miniApp(id:)`, `settings(section:)`) + inspector presentation (`inspectorTarget: InspectorTarget?`) + floating PiP overlay state (`activeMiniAppPip`, `activeCallPip`, `activeRemoteControlStatus`).
- **Feature Module Registration Protocols** (defined in M2 so M3/M4/M5 implement independently with zero merge collisions):
  - `MessagingModuleProviding` (implemented by M3): provides `ChatSidebarView`, `ConversationStageView`, `MultiSelectComposeView`, `ContextInspectorView`, `UniversalSearchView`, `MediaViewerOverlay`, `StoriesOverlay`, `InstantViewOverlay`.
  - `AgentStudioModuleProviding` (implemented by M4): provides `AgentStudioWorkspaceView`, `MagpieAiProvidersSettingsView`, `OfficialMcpConsoleView`, `CodingAgentThreadAccessories`, `MultiAgentRoomAccessories`.
  - `PlatformModulesProviding` (implemented by M5): provides `RemoteComputerWorkbenchView`, `RemoteControlledPillView`, `MiniAppHostSheetView`, `MiniAppFloatingPipView`, `CallAndTalkOverlayView`, `CallsTabView`, `SettingsRootView`, `WalletAndPremiumViews`.

### 4. Implementation Track ↔ E2E Testing Track
- Opaque-box E2E tests interact strictly via:
  1. Rust `source/host` public C-ABI / JSON-RPC contract (exercising real SQLite WAL, Tantivy, crypto, HTTP/WS loopback test server — zero stubs),
  2. Public Swift `FabushiCoordinator` / `FabushiRustHostActor` contracts,
  3. Stable `accessibilityIdentifier` contracts in SwiftUI/UIKit (`fabushi.auth.*`, `fabushi.shell.*`, `fabushi.chatlist.*`, `fabushi.compose.*`, `fabushi.stage.*`, `fabushi.composer.*`, `fabushi.inspector.*`, `fabushi.studio.*`, `fabushi.magpie.*`, `fabushi.remote.*`, `fabushi.miniapp.*`, `fabushi.call.*`, `fabushi.settings.*`).
- All E2E tests MUST execute cleanly inside GitHub Actions (`ubuntu-latest` + `macos-15` iOS Simulator) with exit code 0 and non-skipped steps.

## Code Layout & Exclusive Write Ownership

| Directory / Files | Exclusive Owner | Notes |
|-------------------|-----------------|-------|
| `AGENTS.md`, `README.md`, `MIGRATION_SOURCE.md`, `.gitignore`, `docs/specs/gpui-telegram-main-ios-parity.md`, `manifests/**`, `scripts/check-desktop-main-ios-architecture.py`, `scripts/build-mahayana-ios.sh`, `.github/workflows/ios-gpui-telegram-ci.yml`, `.github/workflows/ios-desktop-main-architecture.yml` | **M1** (final release tweaks in M6) | Spec-first authority lock & CI foundation |
| `source/desktop_core/**`, `source/host/**` | **M1** | 100% iOS-owned vendored `bhrumom/gpui-telegram@6141456e` pure-Rust core + zero-stub C-ABI bridge |
| `mobile/ios/Fabushi/Core/**`, `mobile/ios/Fabushi/Bridge/**` | **M1** | Swift 6 Coordinator/Host/Runner + real Rust/Keychain/SQLite/NWPathMonitor bridges |
| `mobile/ios/Fabushi/DesignSystem/**`, `mobile/ios/Fabushi/Presentation/Shell/**`, `mobile/ios/Fabushi/Presentation/Auth/**`, `mobile/ios/Fabushi/Presentation/Security/**`, `mobile/ios/Fabushi/Resources/Localizable.xcstrings` | **M2** | Apple HIG tokens/themes/components, adaptive iPhone/iPad Shell, Auth Wizard, Multi-Account, Vault/Passcode |
| `mobile/ios/Fabushi/Presentation/Chats/**`, `Presentation/Conversation/**`, `Presentation/Composer/**`, `Presentation/Inspector/**`, `Presentation/Media/**`, `Presentation/Contacts/**` | **M3** | Unified messaging, Grok multi-select compose, 11+ cards, IME composer + 49 slash commands, Muse inspector, media/stories/IV |
| `mobile/ios/Fabushi/Presentation/AgentStudio/**`, `Presentation/MCP/**`, `Presentation/Magpie/**`, `Presentation/Coding/**` | **M4** | 7-tab Hermes/Fabushi Studio, Multi-Agent UI, Magpie AI Gateway UI, Coding Agents client, Official MCP OAuth 2.1 |
| `mobile/ios/Fabushi/Presentation/RemoteComputer/**`, `Presentation/MiniApps/**`, `Presentation/Calls/**`, `Presentation/Settings/**`, `Presentation/Economy/**`, `mobile/ios/Fabushi/AppIntents/**` | **M5** | UU-Remote controller/PTY, Mini Apps/WebMCP/PiP/全球法布施, LiveKitWebRTC/CallKit/Talk Mode, 26-section Settings, StoreKit 2, App Intents |
| `mobile/ios/Fabushi/FabushiApp.swift`, `FabushiAppDelegate.swift`, `mobile/ios/project.yml`, `NOTICE.md`, `docs/provenance/**`, `docs/parity/**`, `.github/workflows/ios-store-delivery.yml`, `.github/workflows/ios-release.yml` | **M1** (bootstrap), **M6** (unification, cleanup, version `1.6.7`, release) | Entrypoint unification & release packaging |
| `mobile/ios/FabushiTests/**` | **M1–M6** (scoped per module subdirectory `FabushiTests/<Module>/`) | Module unit & integration tests |
| `TEST_INFRA.md`, `TEST_READY.md`, `tests/e2e/**`, `mobile/ios/FabushiUITests/**` | **E2E Testing Track** (isolated worktree `feat/e2e-suite-v167`) | Opaque-box 4-tier E2E test infrastructure & suite |
| `.agents/teamwork/**` | **Agent Metadata Only** | NEVER committed to git; gitignored in M1 |
