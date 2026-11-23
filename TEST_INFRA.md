# Fabushi iOS `v1.6.8` — Opaque-Box 4-Tier E2E Testing Infrastructure (`TEST_INFRA.md`)

## 1. Upstream Authority & Opaque-Box Philosophy

- **Verified Live Upstream Authority**: `bhrumom/gpui-telegram` branch `main`, exact commit `424aa33cf67a11a778da2dd3b8cca7ea35072b07`, root tree `181a98ba298a49e64ae0ce5b6ac0160616134708`, release **`Fabushi v1.6.8`** (`11,351` tracked blobs: `3,063` `crates/**` + `7,713` `inherited/**`, `0` gitlinks). Target iOS release: **`1.6.8` / build `35`**.
- **CI-Only Execution Policy (`AGENTS.md`)**: Every build, compilation, static validation, Rust FFI E2E test, Python loopback self-test, Swift 6 typecheck, and iOS Simulator XCUITest run executes **exclusively in GitHub Actions (`ubuntu-latest` and `macos-15`)** — never locally on developer workstations or assistant containers.
- **100% Opaque-Box Architecture**:
  - `tests/e2e/rust_ffi/` is a standalone Rust Edition 2024 Cargo workspace with **zero Cargo path dependencies** on `source/host` or `source/desktop_core`.
  - Production Rust tests dynamically `dlopen` the compiled native host library (`libmahayana_app_host.so` on Linux, `libmahayana_app_host.dylib` on macOS) via `libloading` using `FABUSHI_HOST_LIB`.
  - Every single one of the **394** Tier 1–4 Rust E2E tests calls `FabushiE2EHarness::connect_production()`, which enforces a **3-layer fail-closed anti-stub gate** (canonicalized path check, `fabushi_e2e_is_mutant_stub` sentinel symbol check via `dlsym`, and startup behavioral `unknown_command` probe) and fails closed if `FABUSHI_HOST_LIB` is unset, missing, or lacks any of the 6 canonical `fabushi_host_*` symbols. There is **zero mock fallback** in Tiers 1–4.
  - `mobile/ios/FabushiUITests/V167/**` provides **68** Swift 6.0 strict-concurrency (`-swift-version 6 -strict-concurrency=complete -warnings-as-errors`) opaque-box XCUITests importing ONLY `Foundation`, `UIKit`, and `XCTest` (**zero `@testable import Fabushi`**), backed by `tests/e2e/simulator_loopback_server.py`. Combined production E2E suite total: **394 Rust + 68 Swift = 462 executable E2E tests**.
  - `tests/e2e/rust_ffi/mutant_stub/` is strictly a negative-oracle `cdylib` exporting `fabushi_e2e_is_mutant_stub`, used exclusively by `tests/self_validate_antistub.rs` to prove our contract assertions catch stub/facade defects (alongside an empirical negative proof against the actual compiled baseline `8128951` `libmahayana_app_host.so`).

---

## 2. Canonical C-ABI FFI & JSON-RPC Contract (`PROJECT.md § Interface Contracts #1`)

### 2.1 Required 6 Exported C-ABI Symbols (`source/host/include/mahayana_app_host.h`)

```c
void*  fabushi_host_create(const char* config_json);
char*  fabushi_host_dispatch(void* handle, const char* request_json);
char*  fabushi_host_poll_events(void* handle, uint32_t max_count, uint32_t timeout_ms);
bool   fabushi_host_cancel(void* handle, const char* request_id);
void   fabushi_host_destroy(void* handle);
void   fabushi_host_free_string(char* ptr);
```

**Negative-Oracle Sentinel Symbol (FORBIDDEN in Production Hosts)**:
```c
uint32_t fabushi_e2e_is_mutant_stub(void);
```

- **3-Layer Fail-Closed Production Anti-Stub Gate (`FfiHostClient::load_production` / `FabushiE2EHarness::connect_production`)**:
  1. **Layer 1 (Canonicalized Path Check)**: Checks both raw path and `std::fs::canonicalize(lib_path)` for `mutant_stub` or `mutant-stub` (blocking symlinks and relative traversal bypasses before `dlopen`).
  2. **Layer 2 (`dlsym` Sentinel Symbol Refusal)**: Probes `library.get(b"fabushi_e2e_is_mutant_stub\0")` BEFORE `fabushi_host_create` and immediately rejects any shared library exporting the negative-oracle sentinel even if copied or renamed to a neutral filename.
  3. **Layer 3 (Startup Behavioral Anti-Stub Probe)**: Immediately dispatches synthetic unknown method `__fabushi_e2e_probe__.unknown_command_guard.<uuid>` and asserts `ok == false`, `error.code == "unknown_command"`, `error.retryable == false`, and zero `"acknowledged"` stub token before returning the handle to any test.
- Every returned `char*` MUST be freed exclusively by calling `fabushi_host_free_string(ptr)` from the same loaded library (panic-safe RAII guard in `ffi_client.rs`, zero cross-allocator frees).
- Strict lifetime drop ordering: `fabushi_host_destroy(handle)` completes before `libloading::Library` unmaps the shared library (`dlclose`) and before the ephemeral `LoopbackServer` or isolated `TempDir` (`app_data_dir`) drops.
- Every C-ABI entrypoint MUST be wrapped in `std::panic::catch_unwind` (verified in both debug and release profiles; a panic returns `ok: false, error.code: "host_panic"` and leaves the host handle intact).

### 2.2 Strict Request / Response / Event Envelopes

- **Request**:
  ```json
  {
    "id": "<uuid-v4>",
    "method": "<namespace>.<action>",
    "generation": 1,
    "account_id": "<optional-account-id>",
    "params": {}
  }
  ```
- **Success Response**:
  ```json
  {
    "id": "<echoed-uuid-v4>",
    "ok": true,
    "generation": 1,
    "result": {}
  }
  ```
- **Error Response (FAIL-CLOSED)**:
  ```json
  {
    "id": "<echoed-uuid-v4>",
    "ok": false,
    "generation": 1,
    "error": {
      "code": "<typed_snake_case_code>",
      "message": "<human_safe_redacted_message>",
      "retryable": false
    }
  }
  ```
- **Hard Anti-Stub Invariants**:
  1. Unrecognized `method` MUST return `ok: false, error.code: "unknown_command", error.retryable: false` — NEVER fail-open `{"status": "acknowledged"}`.
  2. Fresh unauthenticated `chats.list` MUST return `[]` (`total: 0`) — ZERO hardcoded `sample_chats` (`"Saved Messages"` id `100` / `"Fabushi Community"` id `200`).
  3. Default fallback `server_url` MUST be `"http://34.46.77.133:8080"` (`DEFAULT_SERVER_URL`) — ZERO hardcoded `"https://127.0.0.1:8443"`.
  4. Requests with stale `generation < current_generation` MUST fail closed with `error.code: "stale_generation"` without mutating state or sending network traffic; harness generation synchronizes monotonically via atomic `fetch_max` across all success, error, and event-poll envelopes.

### 2.3 Ephemeral Real-Socket Loopback Protocol Servers (`src/loopback_server.rs` & `tests/e2e/simulator_loopback_server.py`)

- **Rust FFI Loopback Server (`tests/e2e/rust_ffi/src/loopback_server.rs`)**:
  - Binds `std::net::TcpListener::bind("127.0.0.1:0")` per test with clean wake-on-drop shutdown joining both the accept loop and per-connection worker threads.
  - Implements UTF-8 safe byte-slice prefix matching (`normalize_request_target`, `extract_bearer_token`), RFC 3986 percent-decoded query parameter extraction (`extract_query_param`), strict non-empty credential validation on `/api/v1/auth/{register,login,phone/verify}`, Bearer revocation (`401`) enforcement across ALL `/api/v1/media/*` routes (binary, range, HEAD, and JSON), deadline-bounded multi-packet `Content-Length` and **RFC 9112 `Transfer-Encoding: chunked`** decoding (with `Expect: 100-continue`, 32 MiB cap -> `413`, 64-trailer cap -> `400`), real binary media CAS SHA-256 storage + **HTTP 206/416**, stateful shared tables, strict **fail-closed `404`** on unrecognized/sibling routes, and buffered **RFC 6455 WebSocket** framing.
- **iOS Simulator Loopback Server (`tests/e2e/simulator_loopback_server.py`)**:
  - Standard-library only Python 3 `ThreadingHTTPServer` (`127.0.0.1:0`, HTTP/1.1, `Connection: close`, `Expect: 100-continue`, RFC 9112 chunked decoding, thread-safe `RLock` never held across `time.sleep`, deadlock-free `SIGTERM`/`SIGINT` shutdown).
  - Publishes bound port/URL atomically (`mkstemp` + `fsync` + `os.replace`) via `--port-file` and `--url-file` strictly AFTER socket bind and listen.
  - Mirrors all stateful `/api/v1/*` routes from `loopback_server.rs` (including all 4 hardening rules, `v1.6.8` remote node ScreenCaptureKit/Accessibility permission fields, MiniApps catalog, and fail-closed `404` catch-all).
  - Exposes a typed `/__e2e__/*` control plane (`POST /__e2e__/reset`, `POST /__e2e__/seed`, `POST /__e2e__/fault`, `DELETE /__e2e__/fault`, `GET /__e2e__/state`) immune to `offline`/faults/delays and excluded from recorded requests, consumed by Swift 6 `FabushiLoopbackClient`.
  - Provides `--self-test [--report <path>]` running **10 deterministic checks** in CI on both `ubuntu-latest` and `macos-15`.

---

## 3. Complete 34-Feature Coverage Matrix (`F01..F34`)

Machine-readable lock: [`tests/e2e/feature_matrix.json`](tests/e2e/feature_matrix.json).

- **All 33 Required RPC Namespaces**: `auth.*`, `vault.*`, `session.*`, `chats.*`, `folders.*`, `topics.*`, `contacts.*`, `messages.*`, `sync.*`, `outbox.*`, `e2ee.*`, `search.*`, `media.*`, `stories.*`, `stickers.*`, `fabushi.*`, `multi_agent.*`, `magpie.*`, `coding.*`, `mcp.*`, `computers.*`, `remote.*`, `tailnet.*`, `miniapp.*`, `webmcp.*`, `calls.*`, `talk.*`, `wallet.*`, `settings.*`, `proxy.*`, `push.*`, `actions.*`, `generation.*`.
- **All 13 Required `fabushi.*` Accessibility Namespaces**: `fabushi.auth.*`, `fabushi.shell.*`, `fabushi.chatlist.*`, `fabushi.compose.*`, `fabushi.stage.*`, `fabushi.composer.*`, `fabushi.inspector.*`, `fabushi.studio.*`, `fabushi.magpie.*`, `fabushi.remote.*`, `fabushi.miniapp.*`, `fabushi.call.*`, `fabushi.settings.*`.

| Feature | Domain (`PROJECT.md` / `survey_2`) | RPC Namespaces | `fabushi.*` UI Namespaces | Upstream Status | Tier 1 | Tier 2 |
|---|---|---|---|---|---:|---:|
| `F01` | Password Login, Registration & Split-Key V2 Bootstrap (#2 / #1) | `auth.*`, `vault.*`, `session.*` | `fabushi.auth.*` | `REAL SHIPPING` | 5 | 5 |
| `F02` | Email OTP Registration & Phone SMS Verification (#3 / #2) | `auth.*` | `fabushi.auth.*` | `REAL SHIPPING` | 5 | 5 |
| `F03` | Alipay QR Login & Mobile OAuth Callback (#4 / #3) | `auth.*` | `fabushi.auth.*` | `REAL SHIPPING` | 5 | 5 |
| `F04` | Passkeys (WebAuthn/FIDO2) & QR Device Pairing (#5 / #4) | `auth.*` | `fabushi.auth.*`, `fabushi.settings.*` | `REAL SHIPPING` | 5 | 5 |
| `F05` | Cloud 2FA Password & Active Device Sessions (#6 / #5) | `auth.*`, `session.*` | `fabushi.auth.*`, `fabushi.settings.*` | `REAL SHIPPING` | 5 | 5 |
| `F06` | Multi-Account Switcher, Generation Fence & Passcode Lock (#7 / #6) | `session.*`, `generation.*` | `fabushi.shell.*`, `fabushi.auth.*`, `fabushi.settings.*` | `REAL SHIPPING` | 5 | 5 |
| `F07` | Unified Adaptive Apple Shell (iPad 3-Col / iPhone Tab+Stack+Sheet) (#8 / #7) | `settings.*`, `session.*`, `actions.*` | `fabushi.shell.*` | `REAL SHIPPING` | 5 | 5 |
| `F08` | Grok Bot Multi-Select Compose (`To:` Picker) (#9 / #8) | `chats.*` | `fabushi.compose.*` | `REAL SHIPPING` | 5 | 5 |
| `F09` | Unified Chat List, Folders, Archive, Pin, Mute & Forum Topics (#10 / #9) | `chats.*`, `folders.*`, `topics.*` | `fabushi.chatlist.*` | `REAL SHIPPING` | 5 | 5 |
| `F10` | Contacts, Peer Resolution, Blocklist, Send-As & Prestart Bot (#11 / #10) | `contacts.*`, `chats.*` | `fabushi.chatlist.*`, `fabushi.shell.*`, `fabushi.composer.*`, `fabushi.stage.*` | `REAL SHIPPING` | 5 | 5 |
| `F11` | Atomic PTS/QTS Sync, 24h Nonce Dedup & Offline SQLite WAL Outbox (#12 / #11) | `sync.*`, `outbox.*`, `messages.*` | `fabushi.shell.*`, `fabushi.stage.*` | `REAL SHIPPING` | 5 | 5 |
| `F12` | Virtualized Transcript, 900s Grouping & 11+ Canonical Cards (#13 / #12) | `messages.*` | `fabushi.stage.*` | `REAL SHIPPING` | 5 | 5 |
| `F13` | Message Edit, Delete Tombstone, Forward, PTS Reactions, Pins, TTL & Export (#14 / #13) | `messages.*`, `chats.*` | `fabushi.stage.*` | `REAL SHIPPING` | 5 | 5 |
| `F14` | IME-Safe Canonical Composer (80ms Fence), Drafts, Formatting & Voice Gesture (#15 / #14) | `messages.*` | `fabushi.composer.*` | `REAL SHIPPING` | 5 | 5 |
| `F15` | 49+ Hermes/Fabushi Slash Commands & `@` Context Badges (#16 / #15) | `fabushi.*` | `fabushi.composer.*` | `REAL SHIPPING` | 5 | 5 |
| `F16` | Muse Context Inspector (Profile, Members, 5-Tab Media, Bot/Computer/Coding) (#17 / #16) | `chats.*`, `fabushi.*` | `fabushi.inspector.*` | `REAL SHIPPING` | 5 | 5 |
| `F17` | Unified Hermes/Fabushi Agent Runtime, 29 Toolsets & 7-Tab Studio (#22 / #17) | `fabushi.*` | `fabushi.studio.*` | `REAL SHIPPING` | 5 | 5 |
| `F18` | Multi-Agent Protocol (Floor Control, 5-Hop Loop Budget, Workspace, `shouldReply`) (#23 / #18) | `multi_agent.*` | `fabushi.stage.*`, `fabushi.inspector.*` | `REAL SHIPPING` | 5 | 5 |
| `F19` | Pure-Rust Magpie AI Gateway, Routing, Circuit Breakers, PII Masking & Vault OAuth (#24 / #19) | `magpie.*`, `vault.*` | `fabushi.magpie.*`, `fabushi.settings.*` | `REAL SHIPPING` | 5 | 5 |
| `F20` | Third-Party Coding Agents (Claude Code, Codex, ACP) Mobile Client (#25 / #20) | `coding.*` | `fabushi.stage.*`, `fabushi.inspector.*`, `fabushi.studio.*` | `REAL SHIPPING` | 5 | 5 |
| `F21` | Official OAuth 2.1 Remote MCP Grants, PKCE/QR & 27-Tool Redacted Audit (#26 / #21) | `mcp.*` | `fabushi.studio.*`, `fabushi.settings.*` | `REAL SHIPPING` | 5 | 5 |
| `F22` | UU-Remote Computer Workbench, Multi-PTY, Tailnet & `<10ms` Takeover (#27 / #22) | `computers.*`, `remote.*`, `tailnet.*` | `fabushi.remote.*` | `REAL SHIPPING` | 5 | 5 |
| `F23` | Zero-Knowledge Split-Key V2 & X25519 Vault (24 MiB) + RFC 6238 TOTP + Remote Obscura (#21 / #23) | `vault.*` | `fabushi.settings.*`, `fabushi.auth.*` | `REAL SHIPPING` | 5 | 5 |
| `F24` | External Mini Apps, Live WebMCP, Floating PiP Restore & Global Dharma (#28 / #24) | `miniapp.*`, `webmcp.*` | `fabushi.miniapp.*` | `REAL SHIPPING` | 5 | 5 |
| `F25` | Chunked Media CAS, HTTP 206/416, 5-Bit 63B Waveform, Stickers, Stories & Instant View (#20 / #25) | `media.*`, `stories.*`, `stickers.*` | `fabushi.stage.*`, `fabushi.composer.*` | `REAL SHIPPING` | 5 | 5 |
| `F26` | 1:1 & Group Calls (`LiveKitWebRTC`/`CallKit`/`ReplayKit`) & Talk Mode `<60ms` VAD (#29 / #26) | `calls.*`, `talk.*` | `fabushi.call.*` | `PARTIAL / STUB` | 5 | 5 |
| `F27` | Unified Tantivy Full-Text + Hybrid Memory + Bots/MiniApps + Offline FTS5 Search (#18 / #27) | `search.*` | `fabushi.chatlist.*`, `fabushi.stage.*` | `REAL SHIPPING` | 5 | 5 |
| `F28` | E2EE Messaging (Double Ratchet 1:1, Sender Keys Group & 4-Emoji/QR Safety) (#19 / #28) | `e2ee.*` | `fabushi.inspector.*`, `fabushi.stage.*` | `PARTIAL / STUB` | 5 | 5 |
| `F29` | Economy, Stars Ledger, Gifts, Premium, Redeem & StoreKit 2 IAP/Restore (#31 / #29) | `wallet.*` | `fabushi.settings.*` | `PARTIAL / STUB` | 5 | 5 |
| `F30` | Action Bus, App Intents / Siri Shortcuts, `fabushi://` URLs & APNs Push Registration (#33 / #30) | `actions.*`, `push.*` | `fabushi.shell.*`, `fabushi.settings.*` | `PARTIAL / STUB` | 5 | 5 |
| `F31` | Apple HIG Design System (4 Themes, WCAG `>138` Guard, Liquid Glass, Dynamic Type, `zh-Hans`/`en`) (#8 / #31) | `settings.*` | `fabushi.settings.*`, `fabushi.shell.*` | `REAL SHIPPING` | 5 | 5 |
| `F32` | Full 26-Section Apple Settings Suite, Search & Real CAS/SQLite Storage Purge (#30 / #32) | `settings.*` | `fabushi.settings.*` | `PARTIAL / STUB` | 5 | 5 |
| `F33` | Network, 4-Tier Config Cascade, HTTP/SOCKS5/MTProto Proxy & Update Check (#32 / #33) | `proxy.*`, `settings.*` | `fabushi.settings.*`, `fabushi.auth.*` | `REAL SHIPPING` | 5 | 5 |
| `F34` | Fail-Closed Governance, Anti-Stub C-ABI & Honest Backlog Ledger (`OCRM-001`, `FBCP-001`, `TON`, `Passport`) (#1/#34 / #34) | `generation.*`, `settings.*` | `fabushi.shell.*`, `fabushi.settings.*` | `REQUIREMENT-ONLY / PLANNED` | 5 | 5 |

- **Tier Totals**:
  - **Rust FFI Tier 1 (Core Functional Happy Paths)**: 34 × 5 = **170** tests (`tier1_auth_shell_messaging.rs`: 80, `tier1_agents_platform_hig.rs`: 90)
  - **Rust FFI Tier 2 (Boundary, Corner-Case & Fail-Closed)**: 34 × 5 = **170** tests (`tier2_auth_shell_messaging.rs`: 80, `tier2_agents_platform_hig.rs`: 90)
  - **Rust FFI Tier 3 (Pairwise Cross-Feature Interactions)**: **36** tests (`tier3_pairwise_combos.rs`, every `F01..F34` covered in >= 2 pairs with body-level RPC verification)
  - **Rust FFI Tier 4 (Real-World Multi-Feature Lifecycles)**: **18** tests (`tier4_real_world_scenarios.rs`, >= 4 distinct declared features genuinely exercised in every scenario)
  - **Swift 6 V167 XCUITest Suite (`mobile/ios/FabushiUITests/V167/**`)**: **68** genuine opaque-box UI tests (`FabushiV167Tier1UITests`: **34** covering `F01..F34`, `FabushiV167Tier2UITests`: **16** Apple HIG/boundary/negative/`v1.6.8` polish, `FabushiV167Tier3UITests`: **10** adaptive iPhone/iPad cross-feature pairs, `FabushiV167Tier4UITests`: **8** full multi-screen user journeys)
  - **Total Executable Production E2E Tests**: **394 Rust + 68 Swift = 462** executable E2E tests + **15** zero-`unsafe` Rust CI self-validation & hardening regression tests (`self_validate_antistub.rs`) + **10** Python `simulator_loopback_server.py --self-test` checks.

### 3.1 Honest Upstream Blocker & Planned Capability Ledger (7 Tracked IDs)

Locked in `tests/e2e/rust_ffi/src/contracts.rs` (`HONEST_TRACKED_CAPABILITY_IDS`) and verified via `settings.capability_ledger` (`verify_honest_capability_disposition`):
1. `OCRM-001` (`upstream_planned`) — 141-adapter OpenClaw channel runtime
2. `FBCP-001` (`upstream_planned`) — Fabushi Control Plane
3. `TON_ONCHAIN_SIGNING` (`upstream_planned`) — TON on-chain signing
4. `TELEGRAM_PASSPORT` (`upstream_planned`) — Telegram Passport
5. `APNS_HTTP2_SERVER_DELIVERY` (`upstream_stub` / `server_blocked`) — Server `apns.rs` HTTP/2 delivery stub
6. `WEBRTC_NAT_SFU_TRANSPORT` (`upstream_loopback_stub` / `server_blocked`) — LiveKit/WebRTC NAT/SFU media transport
7. `STOREKIT_SERVER_BINDING` (`server_blocked`) — Apple App Store StoreKit 2 server binding

---

## 4. Standardized Opaque-Box `XCUIApplication` Launch Contract (Swift 6 / iOS 17+)

All `v1.6.8` XCUITests live under `mobile/ios/FabushiUITests/V167/**` (recursively discovered by XcodeGen 2.46.0 with zero `project.yml` edits), import ONLY `Foundation`, `UIKit`, and `XCTest` (**zero `@testable import Fabushi`**), declare top-level nonisolated `Codable & Sendable` loopback DTOs + `FabushiLoopbackClient`, and use `@MainActor` `override func setUp() async throws` / `tearDown() async throws` for zero-warning Swift 6.0 `-strict-concurrency=complete -warnings-as-errors` compliance.

### 4.1 Apple Native System Launch Arguments & Simulator Controls
- **Localization**:
  - `["-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN"]`
  - `["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]`
- **Dynamic Type**:
  - `["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityExtraExtraExtraLarge"]`
  - Paired with `xcrun simctl ui "$UDID" content_size accessibility-extra-extra-extra-large`
- **System Appearance & Contrast**:
  - `xcrun simctl ui "$UDID" appearance dark|light`
  - `xcrun simctl ui "$UDID" increase_contrast enabled|disabled`

### 4.2 Standardized Fabushi E2E Environment & CLI Arguments
`xcodebuild test-without-building` forwards only shell variables prefixed with `TEST_RUNNER_` into `FabushiUITests-Runner.app` (`ProcessInfo.processInfo.environment["<NAME>"]`). `FabushiE2EUITestCase` propagates the following into `XCUIApplication.launchEnvironment` and `launchArguments`:

| Key / Flag | Purpose |
|---|---|
| `FABUSHI_UI_TEST_MODE=1` | Enables deterministic opaque-box E2E launch (disables onboarding animation delays; NEVER enables mocks or synthetic sample chats). |
| `FABUSHI_FEATURE_HOST_TEST=0` | Explicitly disables legacy synthetic commerce / fake feature host. |
| `FABUSHI_E2E_SERVER_URL=http://127.0.0.1:<port>` | Ephemeral loopback HTTP server URL (`tests/e2e/simulator_loopback_server.py`); simultaneously forwarded as `GPUI_TELEGRAM_SERVER_URL` and CLI args `["--server-url", "<url>"]` matching `gpui_telegram_client` Config Tiers 1 & 2. |
| `FABUSHI_APP_DATA=<ephemeral-dir>` | Isolated per-test SQLite WAL / Tantivy / CAS directory (reused across `terminate()` + `launch()` when `resetState == false` for cold-restart durability). |
| `FABUSHI_E2E_RESET_STATE=1` | Resets ephemeral local state on clean-start tests (omitted on cold-restart durability tests). |
| `FABUSHI_UI_THEME=night\|tinted\|day\|classic` | Selects one of the 4 `v1.6.8` themes on launch (`F31`). |
| `FABUSHI_UI_ACCENT_HEX=#RRGGBB` | Injects custom accent hex to verify WCAG `>138` luminance contrast darkening (`F31`). |
| `FABUSHI_UI_REDUCE_MOTION=1` / `FABUSHI_UI_HIGH_CONTRAST=1` | Deterministic accessibility state toggles for Reduce Motion & High Contrast verification. |
| `TEST_RUNNER_*` | CI shell prefix used by `xcodebuild` to pass runner configuration (`TEST_RUNNER_FABUSHI_E2E_SERVER_URL`, `TEST_RUNNER_FABUSHI_E2E_TIER`, `TEST_RUNNER_FABUSHI_E2E_FORM_FACTOR`, `TEST_RUNNER_FABUSHI_E2E_SOURCE_SHA`). |

### 4.3 ATS / Localhost Loopback Networking Note for M1/M6
- Rust `reqwest` / WebSocket transport uses POSIX sockets (exempt from Apple ATS), so `http://127.0.0.1:<port>` operates seamlessly in the Rust host.
- Because `mobile/ios/project.yml:95-96` currently sets `NSAllowsArbitraryLoads: false` without `NSAllowsLocalNetworking: true`, **M1/M6 should add `NSAllowsLocalNetworking: true`** to `Info.plist` (`project.yml`) so native Swift `URLSession` / `WKWebView` / `AVPlayer` media/MiniApp loopback E2E runs on `http://127.0.0.1:<port>` succeed cleanly on iOS Simulator while preserving strict HTTPS enforcement for all remote hosts.

### 4.4 Canonical V167 Per-Tier XCUITest Classes & CI Targeting
Production simulator E2E targets ONLY the canonical per-tier Swift 6 XCUITest classes under `mobile/ios/FabushiUITests/V167/` (never unfiltered `-only-testing:FabushiUITests`):
- `Support/FabushiAccessibilityIDs.swift` — all 13 `fabushi.*` accessibility namespaces + `v1.6.8` two-layer toast, compose isolation, 3-way inspector, remote permission modal, and ReplayKit screen-share IDs
- `Support/FabushiE2EUITestCase.swift` — `@MainActor` base class (0 `test*` methods), nonisolated `FabushiLoopbackClient`, Apple HIG/A11y/touch-target/dead-control assertions, checkpoint screenshots, and `FABUSHI_E2E_STATE_BASE64=` evidence emitter
- `tier1` -> `-only-testing:FabushiUITests/FabushiV167Tier1UITests` (**34** tests across `Tier1/FabushiV167Tier1UITests.swift` + `Tier1/FabushiV167Tier1UITests+AgentsAndPlatform.swift`)
- `tier2` -> `-only-testing:FabushiUITests/FabushiV167Tier2UITests` (**16** Apple HIG / boundary / negative / `v1.6.8` tests)
- `tier3` -> `-only-testing:FabushiUITests/FabushiV167Tier3UITests` (**10** adaptive iPhone/iPad cross-feature pairwise tests)
- `tier4` -> `-only-testing:FabushiUITests/FabushiV167Tier4UITests` (**8** full multi-screen E2E user journeys)

Before executing `xcodebuild test-without-building`, CI enforces a fail-closed pre-flight guard verifying `mobile/ios/FabushiUITests/V167/` exists and defines each requested `FabushiV167Tier<1..4>UITests` class (refusing to fall back to legacy `FabushiUITests` or `GlobalDharmaJourneyUITests`), starts `tests/e2e/simulator_loopback_server.py --host 127.0.0.1 --port 0 --url-file ... --port-file ...`, verifies `/healthz`, and exports `TEST_RUNNER_FABUSHI_E2E_SERVER_URL`, `TEST_RUNNER_FABUSHI_E2E_TIER`, and `TEST_RUNNER_FABUSHI_E2E_FORM_FACTOR`.

---

## 5. GitHub Actions CI Runner & CLI Usage

Workflow: [`.github/workflows/ios-e2e-suite.yml`](.github/workflows/ios-e2e-suite.yml).

### 5.1 Modes & Caching
- **Exact Checkout (`ref`)**: All 5 `actions/checkout@v5` steps explicitly check out `ref: ${{ github.event.pull_request.head.sha || github.sha }}` so both `push` and `pull_request` events match `git rev-parse HEAD == EXPECTED_HEAD`.
- **Caching Strategy**: Combines `Swatinem/rust-cache@v2` (cross-run target dir caching for `tests/e2e/rust_ffi -> ../../../target/e2e`, `source/host -> ../../target/host`, and macOS `source/host -> ../../target`) with `mozilla-actions/sccache-action@v0.0.11` using local disk `SCCACHE_DIR=${RUNNER_TEMP}/sccache` (no GHA Cache v2 burst write throttling, 0 sccache write errors).
- **Safe `runner.py` Path & Target-Dir Defaults**: `tests/e2e/runner.py` resolves all `--host-lib`, `--mutant-lib`, `--baseline-lib`, and `CARGO_TARGET_DIR` paths to canonical absolute paths via `.resolve()`, defaults `CARGO_TARGET_DIR` to `<repo_root>/target/e2e` (never creating `tests/e2e/rust_ffi/target/`), preserves existing `RUSTFLAGS` while adding `-D warnings`, and rejects both `mutant_stub` and `mutant-stub` (including symlinks).
1. **`self-validate` Mode** (default on `feat/e2e-suite-v167` before M1 exports `fabushi_host_create`):
   - **`ubuntu-self-validate` (`ubuntu-latest`)**:
     - `python3 -m py_compile tests/e2e/runner.py tests/e2e/spec_coverage_check.py tests/e2e/simulator_loopback_server.py`
     - `PYTHONDONTWRITEBYTECODE=1 python3 tests/e2e/simulator_loopback_server.py --self-test --report "$EVIDENCE_DIR/loopback_self_test_report.json"`
     - `python3 tests/e2e/spec_coverage_check.py --strict --matrix tests/e2e/feature_matrix.json --report <path>` (validates all 394 Rust tests + 68 Swift 6 V167 XCUITests + 13 UI namespaces + 34 features + `v1.6.8` authority lock + preserved legacy files)
     - Builds baseline `8128951` `source/host` (`libmahayana_app_host.so`) whenever `run_baseline_negative_proof == 'true'` (default on all `push`/`pull_request` runs -> 0 skipped steps)
     - Runs `tests/e2e/runner.py --mode self-validate --baseline-lib <path> --report <path>` compiling all 394 Tier 1–4 tests + `mutant_stub` under `RUSTFLAGS="-D warnings"`, and running all **15** zero-`unsafe` `self_validate_antistub` tests
     - Verifies `SHA256SUMS.txt` (`sha256sum -c`) and uploads evidence via `actions/upload-artifact@v7`.
   - **`macos-swift6-uitest-validate` (`macos-15`)**:
     - Fail-closed selection of `/Applications/Xcode_16.4.app` (configurable via `xcode_app` input)
     - Runs runtime-matched simulator discovery verifying BOTH an available iPhone (`iPhone 16 Pro`/`iPhone 16`) AND iPad (`iPad Pro 13-inch (M4)`/`iPad Air 13-inch (M3)`)
     - Runs `PYTHONDONTWRITEBYTECODE=1 python3 tests/e2e/simulator_loopback_server.py --self-test --report "$EVIDENCE_DIR/loopback_self_test_report.json"` verifying Darwin loopback socket parity
     - Runs fail-closed Swift 6 strict-concurrency `-typecheck` in two passes: **Pass 1** cross-file check over ALL `mobile/ios/FabushiUITests/**/*.swift` (`-module-name FabushiUITests`) AND **Pass 2** strict fail-closed `-swift-version 6 -strict-concurrency=complete -warnings-as-errors` over `mobile/ios/FabushiUITests/V167/**/*.swift` (`-module-name FabushiUITestsV167`) against real `iphonesimulator` SDK
     - Verifies `SHA256SUMS.txt` (`shasum -a 256 -c`) and uploads evidence via `actions/upload-artifact@v7`.
2. **`production` Mode** (auto-enabled once `fabushi_host_create` exists in `source/host/include/mahayana_app_host.h`, or via `workflow_dispatch` with `tier: tier1|tier2|tier3|tier4|all`):
   - **`production-rust-ffi-e2e` (`ubuntu-latest`)**: runs `py_compile`, `simulator_loopback_server.py --self-test`, `spec_coverage_check.py --strict`, builds debug + release `libmahayana_app_host.so` (verifying `catch_unwind` in release too), and executes Tiers 1 -> 2 -> 3 -> 4 sequentially fail-fast via `tests/e2e/runner.py --mode production --tier <tier> --host-lib <path>`.
   - **`production-macos-simulator-e2e` (`macos-15`)**: discovers runtime-matched iPhone & iPad simulators BEFORE building, caches `source/host -> ../../target` via `Swatinem/rust-cache@v2`, builds `scripts/build-mahayana-ios.sh iphonesimulator arm64`, runs `xcodegen generate`, runs `xcodebuild build-for-testing -destination "platform=iOS Simulator,id=$IPHONE_UDID" ARCHS=arm64 ONLY_ACTIVE_ARCH=YES`, boots & installs `Fabushi.app` + `FabushiUITests-Runner.app` on BOTH iPhone and iPad simulators, launches `tests/e2e/simulator_loopback_server.py --host 127.0.0.1 --port 0 --url-file ... --port-file ...` with graceful `SIGTERM` cleanup trap and exports `TEST_RUNNER_FABUSHI_E2E_SERVER_URL`, executes per-tier `-only-testing:FabushiUITests/FabushiV167Tier<N>UITests` sequentially via `xcodebuild test-without-building -xctestrun`, and uploads verified `.xcresult` + `SHA256SUMS.txt`.

### 5.2 CLI Examples (Executed Inside GitHub Actions Only)
```bash
# Static coverage + anti-facade verification
python3 tests/e2e/spec_coverage_check.py --strict \
  --matrix tests/e2e/feature_matrix.json \
  --report "$EVIDENCE_DIR/coverage_report.json"

# Self-validate harness + 394-test zero-warning compile + mutant_stub + baseline 8128951 proof
CARGO_TARGET_DIR="$GITHUB_WORKSPACE/target/e2e" \
python3 tests/e2e/runner.py \
  --mode self-validate \
  --baseline-lib "$GITHUB_WORKSPACE/target/host/debug/libmahayana_app_host.so" \
  --report "$EVIDENCE_DIR/runner_self_validate_report.json"

# Self-validate harness post-M1 (explicitly skips baseline 8128951 negative proof)
CARGO_TARGET_DIR="$GITHUB_WORKSPACE/target/e2e" \
python3 tests/e2e/runner.py \
  --mode self-validate \
  --skip-baseline-proof \
  --report "$EVIDENCE_DIR/runner_self_validate_report.json"

# Production execution against M1-M6 real host library (fail-fast Tiers 1 -> 2 -> 3 -> 4)
CARGO_TARGET_DIR="$GITHUB_WORKSPACE/target/e2e" \
python3 tests/e2e/runner.py \
  --mode production \
  --tier all \
  --host-lib "$GITHUB_WORKSPACE/target/host/release/libmahayana_app_host.so" \
  --report "$EVIDENCE_DIR/runner_production_report.json"
```

---

## 6. Legacy File Deprecation & Preservation Notice

- **`tests/e2e/master_e2e_suite.swift` — DEPRECATED (PRESERVED UNTOUCHED PENDING M6)**:
  - Targets the superseded Nov 8 (`8128951`) legacy 37-feature list.
  - **MUST NOT be deleted or modified by E2E Track** because `mobile/ios/project.yml:146` directly compiles `../../tests/e2e/master_e2e_suite.swift` into `FabushiTests`, and `mobile/ios/FabushiTests/TelegramE2ETests.swift` calls `MasterE2ERunner.run(tier:)`. M6 removes `master_e2e_suite.swift` when cleaning `project.yml`.
- **`mobile/ios/FabushiUITests/FabushiUITests.swift` & `GlobalDharmaJourneyUITests.swift` — PRESERVED UNTOUCHED**:
  - Three existing workflows (`ios-protected-session.yml`, `ios-global-dharma-offline.yml`, `ios-protected-global-dharma.yml`) hard-depend on `GlobalDharmaJourneyUITests.swift` and its literal `'app.launchEnvironment["FABUSHI_FEATURE_HOST_TEST"] = "0"'`.
- **Deleted Unreferenced Legacy Files**: `tests/e2e/challenger_m6_core_adversarial.swift` and `tests/e2e/challenger_m6_iter2_adversarial.swift` (zero references across repo/workflows) have been removed cleanly.
