# TEST_READY.md — Fabushi iOS `v1.6.8` Opaque-Box 4-Tier E2E Suite Certification

**Status**: **E2E SUITE READY FOR FINAL MILESTONE PHASE 1 — SELF-VALIDATE CI CERTIFIED** *(100% self-validate PASS on GitHub Actions; M1–M6 production execution intentionally scheduled for Final Milestone Phase 1 — ZERO production execution claims prior to M1–M6 merge)*  
**Publication Date**: `2026-11-23`  
**Target Repository & Release Pin**: `fabushiios/fabushi-ios` — `MARKETING_VERSION: 1.6.8`, `CURRENT_PROJECT_VERSION: 35`  
**E2E Track Branch**: `feat/e2e-suite-v167`  
**Locked Upstream Authority (`v1.6.8`)**: `bhrumom/gpui-telegram` branch `main`, release **`Fabushi v1.6.8`** (tag `v1.6.8`, published `2026-11-23T06:19:36Z`), exact commit `424aa33cf67a11a778da2dd3b8cca7ea35072b07`, root tree `181a98ba298a49e64ae0ce5b6ac0160616134708` (`11,351` tracked blobs: `3,063` `crates/**` + `7,713` `inherited/**`, `0` gitlinks)  
**Observed Live Upstream Advancement (`2026-11-23T09:33:44Z`)**: `bhrumom/gpui-telegram@main` advanced `+1` commit to `57a62ef92be2fa816704d7337aca3254dc8f943e` (root tree `733c8dcd0e65be3d1829517bba5ecae7b622cb70`, tag `v1.6.9` — macOS TCC stale-csreq recovery, Gemini HUD binding, popup spam, and desktop hardening; `11,353` tracked blobs: `3,064` `crates/**` + `7,713` `inherited/**`, `0` gitlinks; GitHub `/releases/latest` published `Fabushi v1.6.9` at `2026-11-23T09:51:25Z`). Per Project Orchestrator approval and `AGENTS.md` exact-authority governance, this certified E2E suite on `feat/e2e-suite-v167` stays cleanly locked to `Fabushi v1.6.8` (`424aa33cf67a11a778da2dd3b8cca7ea35072b07`, tree `181a98ba298a49e64ae0ce5b6ac0160616134708`) in exact 1:1 lockstep with `TEST_INFRA.md`, `tests/e2e/feature_matrix.json`, and `tests/e2e/spec_coverage_check.py`, while cross-track final live-`main` rebaseline runs in M1c / M6 / Final Milestone.  
**Certified Executable Suite-Code Baseline**: commit `4416b6deeb739f58620a3321efb41a87280940b7`, root tree `c69787083ed7bd5c6e17d1e717c36d72f15eac51` (pre-M1 `main` merge-base: `8128951a71503a6f109ce0cb37618d022fe8e79a`)  
**Verified GitHub Actions CI Provenance**: `GITHUB_RUN_ID=45145111639` — https://github.com/fabushiios/fabushi-ios/actions/runs/45145111639 (workflow `.github/workflows/ios-e2e-suite.yml`, ID `427406955`)  
**Provenance Lineage Note**: This commit updates `TEST_READY.md` only on top of certified suite-code baseline `4416b6deeb739f58620a3321efb41a87280940b7` (zero executable code, matrix, or workflow changes), triggers its own exact-HEAD GitHub Actions verification run on `feat/e2e-suite-v167`, and will be updated with the final merged `main` HEAD SHA (`IOS_EXPECTED_HEAD`) and Final Milestone Phase 1 production Tier 1–4 `GITHUB_RUN_ID` + Actions URL + production artifact SHA-256 digests after M1–M6 integration.

**Governing Repo-Relative Documents**:
- `AGENTS.md` (Mandatory active upstream authority & non-negotiable GitHub Actions CI-only execution policy)
- `PROJECT.md` (Single canonical iOS architecture, 34-feature inventory, Interface Contracts #1–#4)
- `docs/specs/gpui-telegram-main-ios-parity.md` (Canonical iOS parity specification, `IOS-MAIN-LEDGER-01..08`, `IOS-MAIN-GATE-01..10`, `IOS-MAIN-AC-01..21`)
- `TEST_INFRA.md` (Opaque-box harness architecture, 6 C-ABI symbols, 3-layer fail-closed anti-stub gate, Swift 6 XCUITest launch contract)
- `tests/e2e/feature_matrix.json` (Machine-readable `v1.6.8` 34-feature + 462-test bijection lock)
- `.github/workflows/ios-e2e-suite.yml` (Authoritative GitHub Actions CI pipeline for `self-validate` and `production` modes)

---

## 1. Executive Summary & Strict Anti-Fabrication Boundary

### 1.1 What IS Certified Ready & Empirically Proven on `feat/e2e-suite-v167` (CI-Only Proof)

All **462 executable opaque-box production E2E tests** (`394` Rust FFI + `68` Swift 6 V167 XCUITests; exceeding the `>= 391` project requirement), plus **15** Rust anti-stub/hardening self-validation tests and **11** cross-platform Python loopback server self-tests, are authored, matrix-locked, zero-warning compiled/typechecked, and independently certified via GitHub Actions (`GITHUB_RUN_ID=45145111639`, `0` skipped active steps):

- **394 Standalone Rust Edition 2024 C-ABI / JSON-RPC Production E2E Tests** (`tests/e2e/rust_ffi/tests/tier{1,2,3,4}_*.rs`):
  - Zero Cargo path dependencies on `source/host` or `source/desktop_core`; dynamically `dlopen`s `libmahayana_app_host.{so,dylib}` via `libloading` (`FABUSHI_HOST_LIB`).
  - Every single test connects exclusively through `FabushiE2EHarness::connect_production()`, enforcing a **3-layer fail-closed anti-stub gate** (Layer 1 canonicalized path refusal of `mutant_stub`/`mutant-stub`, Layer 2 `dlsym` refusal of `fabushi_e2e_is_mutant_stub`, Layer 3 startup synthetic `unknown_command` behavioral probe) with **zero mock fallback**.
  - Compiled cleanly on `ubuntu-latest` with `RUSTFLAGS="-D warnings"` (`0` warnings, `0` errors), covering `34/34` features (`F01..F34`), `33/33` RPC namespaces, and `6/6` required C-ABI symbols.
- **68 Swift 6.0 Strict-Concurrency Opaque-Box XCUITests** (`mobile/ios/FabushiUITests/V167/**`):
  - Imports ONLY `Foundation`, `UIKit`, and `XCTest` (**zero `@testable import Fabushi`**, **zero `XCTSkip`**, **zero `#if`/`#elseif` guards**), backed by `tests/e2e/simulator_loopback_server.py`.
  - Typechecked cleanly on `macos-15` (`Xcode 16.4 (16F6)` / `Swift 6.1.2` / `iPhoneSimulator18.5.sdk`) with `-swift-version 6 -strict-concurrency=complete -warnings-as-errors` (`0` warnings, `0` errors across all `7` V167 files + full `9`-file cross-file `FabushiUITests` check), covering `34/34` features (`F01..F34`) and `13/13` Tier-body `fabushi.*` UI accessibility namespaces.
- **Strict Static Coverage, Bijection & Anti-Facade Gate (`tests/e2e/spec_coverage_check.py --strict`)**:
  - `PASS` verifying 1:1 bijection of all `394` Rust `fn tc_*` and `68` Swift `func test*` identifiers against `tests/e2e/feature_matrix.json`, `34/34` features, `33/33` code-only RPC namespaces, `13/13` Tier-body UI namespaces, `6/6` C-ABI symbols + sentinel, zero tautological/vacuous assertions, zero forbidden local paths, and zero `__pycache__`/`*.pyc` artifacts.
- **Empirical Anti-Stub & Baseline Negative Proof (`tests/e2e/rust_ffi/tests/self_validate_antistub.rs`)**:
  - **`15 passed; 0 failed; 0 ignored`** on `ubuntu-latest` (zero `unsafe` blocks), empirically proving `connect_production()` rejects missing/stub/symlinked/renamed libraries, `mutant_stub` trips all 8 stub/facade defect detectors, real compiled baseline `8128951` `libmahayana_app_host.so` is caught and rejected (missing `fabushi_host_*` symbols, fail-open `source/host/src/dispatch.rs:373-380` (line `376`) `"status": "acknowledged"` wildcard arm, and `debug.trigger_panic` `catch_unwind` non-compliant error envelope at `source/host/src/dispatch.rs:57-59`), and `loopback_server.rs` passes all RFC 9112 chunked/206/416, RFC 6455 WebSocket, UTF-8 prefix, RFC 3986 percent-decode, unseeded credential, and media Bearer revocation checks.
- **Cross-Platform Stateful Loopback Server (`tests/e2e/simulator_loopback_server.py --self-test`)**:
  - **`11/11` checks PASS on `ubuntu-latest` AND `11/11` checks PASS on `macos-15` (Darwin)**, including atomic `--url-file`/`--port-file`, clean `SIGTERM`, Split-Key V2 vs `legacy_v1`, chunked CAS + HTTP 206/416, RFC 6455 `/api/v1/ws`, `v1.6.8` remote nodes, `/__e2e__/*` control plane, fail-closed `404`, and thread safety.
- **Dual iOS Simulator Provisioning Probe (`macos-15`)**:
  - Empirically verified both runtime-matched `iOS 18.5` simulators available on GitHub Actions `macos-15`: **`iPhone 16 Pro`** and **`iPad Pro 13-inch (M4)`**.

### 1.2 What Runs in Final Milestone Phase 1 Post-M1–M6 Merge (Explicitly NOT Claimed Passed Here)

- On branch `feat/e2e-suite-v167`, `source/host/` and `mobile/ios/Fabushi/` remain at pre-M1 baseline `8128951a71503a6f109ce0cb37618d022fe8e79a`, which does not yet export `fabushi_host_create`..`fabushi_host_free_string` or M2–M5 Apple HIG SwiftUI views.
- Accordingly, `.github/workflows/ios-e2e-suite.yml` deterministically resolves `mode=self-validate` on `feat/e2e-suite-v167`, running all 3 validation jobs with `0` skipped active steps while gating the 2 production execution jobs (`Production Ubuntu Rust FFI E2E (Tiers 1-4)` and `Production macOS iPhone & iPad Simulator XCUITests (Tiers 1-4)`) as `skipped`.
- **Zero Premature Production Claims**: This document **does NOT claim** that the `394` Rust FFI production E2E tests or the `68` Swift 6 V167 XCUITests have executed against baseline `8128951` or M1–M6 binaries.
- **Automatic Activation After M1–M6 Merge**: Once M1 exports `fabushi_host_create` in `source/host/include/mahayana_app_host.h` and M1–M6 merge into `main`, `.github/workflows/ios-e2e-suite.yml` automatically switches `detect-mode` to `mode=production, tier=all` (and supports per-tier `workflow_dispatch`):
  1. `production-rust-ffi-e2e` (`ubuntu-latest`): compiles BOTH debug AND release `libmahayana_app_host.so` (verifying `std::panic::catch_unwind` in release as well as debug) and executes all **`394`** Rust E2E tests sequentially fail-fast (`tier1 -> tier2 -> tier3 -> tier4`).
  2. `production-macos-simulator-e2e` (`macos-15`): builds `build/rust/iphonesimulator/libmahayana_app_host.a`, runs `xcodegen generate`, runs `xcodebuild build-for-testing` once, boots and installs `Fabushi.app` + `FabushiUITests-Runner.app` onto BOTH `iPhone` and `iPad` simulators, starts `tests/e2e/simulator_loopback_server.py` (`127.0.0.1:0`), and executes all **`68`** Swift 6 V167 XCUITests sequentially across `FabushiV167Tier1..4UITests` on BOTH form factors (`136` total simulator test executions per full `tier=all` run, with `.xcresult` bundles and `SHA256SUMS.txt`).

### 1.3 Honestly Tracked Upstream & External Blockers (`Blocker != Complete`)

All **7** canonical upstream-planned / server-blocked capability IDs (`OCRM-001`, `FBCP-001`, `TON_ONCHAIN_SIGNING`, `TELEGRAM_PASSPORT`, `APNS_HTTP2_SERVER_DELIVERY`, `WEBRTC_NAT_SFU_TRANSPORT`, `STOREKIT_SERVER_BINDING`) plus physical-device/App Store release gates are explicitly tracked and fail-closed verified in Section 5 — never fabricated as shipping or passed.

---

## 2. GitHub Actions CI Runner Commands & Execution Semantics

> **Policy Mandate (`AGENTS.md`)**: Every build, test, compiler, linter, validator, and simulator run executes **exclusively in GitHub Actions (`ubuntu-latest` / `macos-15`)** — NEVER locally on developer workstations or assistant containers.

### 2.1 Triggering `.github/workflows/ios-e2e-suite.yml` (Workflow ID `427406955`)

- **Pre-Merge (`feat/e2e-suite-v167`, before `.github/workflows/ios-e2e-suite.yml` exists on default branch `main`)**:
  - GitHub Actions enables `workflow_dispatch` (`gh workflow run`) only once the workflow file is present on the repository default branch (`main`).
  - While on `feat/e2e-suite-v167`, pushing to the branch triggers `.github/workflows/ios-e2e-suite.yml` automatically in `self-validate` mode (`on.push.branches: [feat/e2e-suite-v167, main]`, no `paths:` filter):

```bash
# Pre-merge: trigger self-validate CI automatically on feat/e2e-suite-v167
git push origin feat/e2e-suite-v167

# Monitor and inspect exact-HEAD CI runs (read-only gh CLI)
gh run list -R fabushiios/fabushi-ios --workflow ios-e2e-suite.yml --limit 10 \
  --json databaseId,headBranch,headSha,event,status,conclusion,createdAt
gh run watch <run-id> -R fabushiios/fabushi-ios --exit-status
gh run view <run-id> -R fabushiios/fabushi-ios \
  --json databaseId,headSha,event,status,conclusion,url,jobs
```

- **Post-Merge (`main` / Final Milestone Phase 1, once `.github/workflows/ios-e2e-suite.yml` is merged to `main`)**:
  - Copy-pasteable `gh workflow run` invocations covering every `workflow_dispatch` input (`tier`, `run_baseline_negative_proof`, `xcode_app`):

```bash
# 1. Production Tier 1 only (170 Rust FFI debug+release + 34 Swift 6 XCUITests on iPhone & iPad)
gh workflow run ios-e2e-suite.yml -R fabushiios/fabushi-ios --ref main -f tier=tier1

# 2. Production Tier 2 only (170 Rust FFI debug+release + 16 Swift 6 XCUITests on iPhone & iPad)
gh workflow run ios-e2e-suite.yml -R fabushiios/fabushi-ios --ref main -f tier=tier2

# 3. Production Tier 3 only (36 Rust FFI debug+release + 10 Swift 6 XCUITests on iPhone & iPad)
gh workflow run ios-e2e-suite.yml -R fabushiios/fabushi-ios --ref main -f tier=tier3

# 4. Production Tier 4 only (18 Rust FFI debug+release + 8 Swift 6 XCUITests on iPhone & iPad)
gh workflow run ios-e2e-suite.yml -R fabushiios/fabushi-ios --ref main -f tier=tier4

# 5. Full Production 4-Tier Gate (fail-fast tier1 -> tier2 -> tier3 -> tier4: 394 Rust + 68 Swift x 2 form factors)
gh workflow run ios-e2e-suite.yml -R fabushiios/fabushi-ios --ref main -f tier=all

# 6. Post-M1 Harness Self-Validation (skips pre-M1 8128951 baseline negative proof)
gh workflow run ios-e2e-suite.yml -R fabushiios/fabushi-ios --ref main \
  -f tier=self-validate \
  -f run_baseline_negative_proof=false

# 7. Optional iOS 26.2 SDK / Liquid Glass Verification on macos-15 (Xcode 26.3)
gh workflow run ios-e2e-suite.yml -R fabushiios/fabushi-ios --ref main \
  -f tier=all \
  -f xcode_app=/Applications/Xcode_26.3.app
```

### 2.2 In-CI Ubuntu Runner Commands — **GitHub Actions CI Only (`AGENTS.md`)**

Executed strictly inside `ubuntu-latest` jobs (`ubuntu-self-validate` and `production-rust-ffi-e2e`):

```bash
# 1. Cross-platform simulator loopback server self-test (11 deterministic checks)
PYTHONDONTWRITEBYTECODE=1 python3 tests/e2e/simulator_loopback_server.py \
  --self-test \
  --report "$EVIDENCE_DIR/loopback_self_test_report.json"

# 2. Strict static coverage, 462-test bijection & anti-facade gate
python3 tests/e2e/spec_coverage_check.py \
  --strict \
  --matrix tests/e2e/feature_matrix.json \
  --report "$EVIDENCE_DIR/coverage_report.json"

# 3a. Harness self-validate WITH pre-M1 baseline 8128951 negative proof
CARGO_TARGET_DIR="$GITHUB_WORKSPACE/target/e2e" \
python3 tests/e2e/runner.py \
  --mode self-validate \
  --baseline-lib "$GITHUB_WORKSPACE/target/host/debug/libmahayana_app_host.so" \
  --report "$EVIDENCE_DIR/runner_self_validate_report.json"

# 3b. Harness self-validate POST-M1 (explicitly skips pre-M1 baseline negative proof)
CARGO_TARGET_DIR="$GITHUB_WORKSPACE/target/e2e" \
python3 tests/e2e/runner.py \
  --mode self-validate \
  --skip-baseline-proof \
  --report "$EVIDENCE_DIR/runner_self_validate_report.json"

# 4. Production Rust FFI E2E against M1-M6 debug & release libmahayana_app_host.so
#    (--tier accepts: tier1 | tier2 | tier3 | tier4 | all)
CARGO_TARGET_DIR="$GITHUB_WORKSPACE/target/e2e" \
python3 tests/e2e/runner.py \
  --mode production \
  --tier "$TIER" \
  --host-lib "$GITHUB_WORKSPACE/target/host/debug/libmahayana_app_host.so" \
  --report "$EVIDENCE_DIR/runner_prod_debug_report.json"

CARGO_TARGET_DIR="$GITHUB_WORKSPACE/target/e2e" \
python3 tests/e2e/runner.py \
  --mode production \
  --tier "$TIER" \
  --host-lib "$GITHUB_WORKSPACE/target/host/release/libmahayana_app_host.so" \
  --report "$EVIDENCE_DIR/runner_prod_release_report.json"
```

### 2.3 In-CI macOS-15 Runner Commands — **GitHub Actions CI Only (`AGENTS.md`)**

Executed strictly inside `macos-15` jobs (`macos-swift6-uitest-validate` and `production-macos-simulator-e2e`):

```bash
# 1. Darwin simulator loopback server self-test (11 deterministic checks incl. RFC 6455 /api/v1/ws)
PYTHONDONTWRITEBYTECODE=1 python3 tests/e2e/simulator_loopback_server.py \
  --self-test \
  --report "$EVIDENCE_DIR/loopback_self_test_report.json"

# 2. Self-validate Swift 6.0 strict-concurrency fail-closed typecheck (Xcode 16.4 / iPhoneSimulator18.5.sdk)
xcrun --sdk iphonesimulator swiftc \
  -typecheck -target arm64-apple-ios17.0-simulator \
  -sdk "$SIM_SDK" \
  -F "$SIM_PLATFORM/Developer/Library/Frameworks" \
  -I "$SIM_PLATFORM/Developer/usr/lib" -L "$SIM_PLATFORM/Developer/usr/lib" \
  -swift-version 6 -strict-concurrency=complete -warnings-as-errors \
  -parse-as-library -module-name FabushiUITestsV167 \
  "${V167_FILES[@]}"

# 3. Production simulator build + dual-form-factor install + per-tier XCUITest execution (iPhone + iPad)
bash scripts/build-mahayana-ios.sh iphonesimulator arm64
(cd mobile/ios && xcodegen generate)
xcodebuild build-for-testing \
  -project mobile/ios/Fabushi.xcodeproj \
  -scheme Fabushi -configuration Debug \
  -destination "platform=iOS Simulator,id=$IPHONE_SIM_UDID" \
  -derivedDataPath "$DERIVED_DATA" \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO

# Install both Fabushi.app and FabushiUITests-Runner.app onto BOTH iPhone & iPad simulators
xcrun simctl install "$UDID" "$FABUSHI_APP"
xcrun simctl install "$UDID" "$UITEST_RUNNER_APP"

# Launch ephemeral loopback server (127.0.0.1:0) and export TEST_RUNNER_FABUSHI_E2E_SERVER_URL
PYTHONDONTWRITEBYTECODE=1 python3 -u tests/e2e/simulator_loopback_server.py \
  --host 127.0.0.1 --port 0 \
  --url-file "$LOOPBACK_URL_FILE" --port-file "$LOOPBACK_PORT_FILE"

# Execute canonical V167 per-tier classes sequentially on BOTH iPhone and iPad simulators:
#   FabushiV167Tier1UITests | FabushiV167Tier2UITests | FabushiV167Tier3UITests | FabushiV167Tier4UITests
xcodebuild test-without-building \
  -xctestrun "$XCTESTRUN" \
  -destination "platform=iOS Simulator,id=$UDID" \
  -only-testing:"FabushiUITests/${TEST_CLASS}" \
  -resultBundlePath "$EVIDENCE_DIR/${T}-${FORM}.xcresult"
```

### 2.4 Pass/Fail Semantics & Fail-Closed Gates

- **Exit Code**: Must be `0` across all active jobs with **0 skipped active steps**.
- **Anti-Stub Enforcement**: `runner.py --mode production` fails closed if `--host-lib` is absent or resolves (including symlinks) to `mutant_stub`/`mutant-stub`, enforces `RUSTFLAGS="-D warnings"`, and runs tiers sequentially fail-fast (`tier1 -> tier2 -> tier3 -> tier4`).
- **Pre-Flight XCUITest Targeting**: `production-macos-simulator-e2e` refuses to fall back to unfiltered `FabushiUITests` or legacy test classes; every requested `FabushiV167Tier<1..4>UITests` class must exist under `mobile/ios/FabushiUITests/V167/`.
- **Evidence Integrity**: Both Ubuntu and macOS jobs compute and verify `SHA256SUMS.txt` (`sha256sum -c` / `shasum -a 256 -c`) prior to `actions/upload-artifact@v7` (`if-no-files-found: error`).

---

## 3. Exact Verified Tier 1–4 & Harness Self-Validation Test Counts

Every count below is cross-checked 1:1 across source code (`tests/e2e/rust_ffi/tests/*.rs`, `mobile/ios/FabushiUITests/V167/**/*.swift`, `tests/e2e/simulator_loopback_server.py`), `tests/e2e/feature_matrix.json`, `tests/e2e/spec_coverage_check.py --strict`, and `TEST_INFRA.md` (`0` discrepancies).

| Suite / Layer | Tier / Category | Authoritative Source Files | Verified Count | `feat/e2e-suite-v167` CI Proof (`self-validate`) | Final Milestone Phase 1 (`production`) |
|---|---|---|---:|---|---|
| **Rust FFI E2E** | **Tier 1** — Core Functional Happy Paths (`34 × 5`) | `tier1_auth_shell_messaging.rs` (`80`), `tier1_agents_platform_hig.rs` (`90`) | **`170`** | Compiled (`-D warnings`, `0` warnings) + `--strict` PASS | Runs against M1–M6 debug & release `libmahayana_app_host.so` |
| **Rust FFI E2E** | **Tier 2** — Boundary, Corner-Case & Fail-Closed (`34 × 5`) | `tier2_auth_shell_messaging.rs` (`80`), `tier2_agents_platform_hig.rs` (`90`) | **`170`** | Compiled (`-D warnings`, `0` warnings) + `--strict` PASS | Runs against M1–M6 debug & release `libmahayana_app_host.so` |
| **Rust FFI E2E** | **Tier 3** — Pairwise Cross-Feature Interactions (`34/34` features, `2..3` pairs each) | `tier3_pairwise_combos.rs` (`36`) | **`36`** | Compiled (`-D warnings`, `0` warnings) + `--strict` PASS | Runs against M1–M6 debug & release `libmahayana_app_host.so` |
| **Rust FFI E2E** | **Tier 4** — Real-World Multi-Feature Lifecycles (`>= 4` features/scenario) | `tier4_real_world_scenarios.rs` (`18`) | **`18`** | Compiled (`-D warnings`, `0` warnings) + `--strict` PASS | Runs against M1–M6 debug & release `libmahayana_app_host.so` |
| **Rust FFI Subtotal** | **Tiers 1–4 (`34/34` Features, `33/33` RPC NS, `6` C-ABI Symbols + Sentinel)** | **6 Tier files** (`tests/e2e/rust_ffi/tests/tier*.rs`) | **`394`** | **PASS (`cargo test --no-run -D warnings` + `--strict`)** | **SCHEDULED FOR FINAL MILESTONE PHASE 1** |
| **Swift 6 V167 XCUITest** | **Tier 1** — Per-Feature Apple HIG Happy Paths (`F01..F34`) | `Tier1/FabushiV167Tier1UITests.swift` (`16`), `Tier1/FabushiV167Tier1UITests+AgentsAndPlatform.swift` (`18`) | **`34`** | Typechecked (`-swift-version 6 -strict-concurrency=complete -warnings-as-errors`, `0` warnings) | Runs on BOTH iPhone & iPad Simulators (`34 × 2 = 68` runs) |
| **Swift 6 V167 XCUITest** | **Tier 2** — Apple HIG, AX5, VoiceOver, 4 Themes, WCAG, Negative & `v1.6.8` Polish | `Tier2/FabushiV167Tier2UITests.swift` (`16`) | **`16`** | Typechecked (`-warnings-as-errors`, `0` warnings) + `--strict` PASS | Runs on BOTH iPhone & iPad Simulators (`16 × 2 = 32` runs) |
| **Swift 6 V167 XCUITest** | **Tier 3** — Adaptive iPhone/iPad & Cross-Feature UI Pairs | `Tier3/FabushiV167Tier3UITests.swift` (`10`) | **`10`** | Typechecked (`-warnings-as-errors`, `0` warnings) + `--strict` PASS | Runs on BOTH iPhone & iPad Simulators (`10 × 2 = 20` runs) |
| **Swift 6 V167 XCUITest** | **Tier 4** — Full Multi-Screen Real-World User Journeys | `Tier4/FabushiV167Tier4UITests.swift` (`8`) | **`8`** | Typechecked (`-warnings-as-errors`, `0` warnings) + `--strict` PASS | Runs on BOTH iPhone & iPad Simulators (`8 × 2 = 16` runs) |
| **Swift 6 V167 Subtotal** | **Tiers 1–4 (`34/34` Features, `13/13` Tier-Body `fabushi.*` UI NS, `0` `@testable`, `0` `XCTSkip`)** | **7 Swift files** (`2` Support + `5` Tier files) | **`68`** | **PASS (`Xcode 16.4` / `Swift 6.1.2` / `iPhoneSimulator18.5.sdk`)** | **SCHEDULED FOR FINAL MILESTONE PHASE 1 (`136` dual-form-factor runs)** |
| **Combined Production E2E** | **Rust FFI (`394`) + Swift 6 V167 XCUITest (`68`)** *(Requirement: `>= 391`)* | **6 Rust Tier + 7 Swift V167 files** | **`462`** | **CERTIFIED READY (Compile / Typecheck / Bijection 100% PASS)** | **EXECUTES IN FINAL MILESTONE PHASE 1** |
| **Rust CI Self-Validate** | Zero-`unsafe` 3-layer anti-stub gate, `mutant_stub`, baseline `8128951` negative proof, loopback & oracle tests | `tests/e2e/rust_ffi/tests/self_validate_antistub.rs` | **`15`** | **EXECUTED: `15 passed; 0 failed; 0 ignored` (`ubuntu-latest`)** | Re-runnable post-M1 via `--skip-baseline-proof` |
| **Python Loopback Self-Test** | Stateful HTTP/1.1, Split-Key V2, CAS 206/416, RFC 6455 `/api/v1/ws`, `/__e2e__/*`, thread safety | `tests/e2e/simulator_loopback_server.py --self-test` | **`11`** | **EXECUTED: `11/11 PASS` (`ubuntu-latest`) + `11/11 PASS` (`macos-15`)** | Executes pre-flight in `production-rust-ffi-e2e` |

### 3.1 Verified Contract Surface & Self-Validation Test Roster

- **6 Required C-ABI Symbols**: `fabushi_host_create`, `fabushi_host_dispatch`, `fabushi_host_poll_events`, `fabushi_host_cancel`, `fabushi_host_destroy`, `fabushi_host_free_string`.
- **1 Forbidden Production Sentinel Symbol**: `fabushi_e2e_is_mutant_stub` (exported only by `tests/e2e/rust_ffi/mutant_stub/`, refused at `dlopen`/`dlsym` by `connect_production()`).
- **33 Required RPC Namespaces (`33/33` covered)**: `auth`, `vault`, `session`, `chats`, `folders`, `topics`, `contacts`, `messages`, `sync`, `outbox`, `e2ee`, `search`, `media`, `stories`, `stickers`, `fabushi`, `multi_agent`, `magpie`, `coding`, `mcp`, `computers`, `remote`, `tailnet`, `miniapp`, `webmcp`, `calls`, `talk`, `wallet`, `settings`, `proxy`, `push`, `actions`, `generation`.
- **13 Required `fabushi.*` UI Accessibility Namespaces (`13/13` covered in Tier bodies)**: `fabushi.auth`, `fabushi.shell`, `fabushi.chatlist`, `fabushi.compose`, `fabushi.stage`, `fabushi.composer`, `fabushi.inspector`, `fabushi.studio`, `fabushi.magpie`, `fabushi.remote`, `fabushi.miniapp`, `fabushi.call`, `fabushi.settings`.
- **15 Executed Rust `self_validate_antistub.rs` Tests (`15/15 PASS`, zero `unsafe`)**:
  1. `self_validate_loopback_http_206_416_and_fault_injection`
  2. `self_validate_loopback_rfc6455_websocket_handshake_and_frames`
  3. `self_validate_loopback_chunked_multipacket_media_upload_206_and_stateful_routes`
  4. `self_validate_loopback_ws_unmasked_1002_fragmented_extended_and_split_read`
  5. `self_validate_domain_and_boundary_oracles`
  6. `self_validate_connect_production_fails_closed_without_real_host`
  7. `self_validate_mutant_stub_catches_all_eight_defect_modes`
  8. `self_validate_baseline_8128951_host_negative_proof`
  9. `self_validate_loopback_blocking1_canonical_routes_and_fail_closed_404`
  10. `self_validate_loopback_blocking2_split_key_v2_login_and_password_rotation`
  11. `self_validate_loopback_blocking2_alipay_ws_revocation_and_chat_idempotency`
  12. `self_validate_loopback_hardening_non_ascii_utf8_prefix_safety`
  13. `self_validate_loopback_hardening_percent_decoded_query_params`
  14. `self_validate_loopback_hardening_reject_empty_whitespace_unseeded_auth`
  15. `self_validate_loopback_hardening_media_routes_enforce_token_revocation`
- **11 Executed Python `simulator_loopback_server.py --self-test` Checks (`11/11 PASS` on both `ubuntu-latest` and `macos-15`)**:
  1. `cli_atomic_files_healthz_and_sigterm`
  2. `auth_split_key_v2_rotation_and_logout`
  3. `chats_messages_idempotency_pts_sync_and_cjk_search`
  4. `media_cas_chunked_206_416_and_token_revocation`
  5. `vault_subpath_percent_decode_and_x25519_device_sync`
  6. `computer_lease_emergency_takeover_and_v168_nodes`
  7. `passkeys_alipay_instant_view_agent_mcp_miniapps`
  8. `e2e_control_seed_state_fault_500_offline_and_reset`
  9. `fail_closed_404_unknown_and_sibling_routes`
  10. `thread_safety_concurrent_requests`
  11. `rfc6455_websocket_handshake_echo_ping_revocation_and_404`

---

## 4. Complete 34-Feature Coverage Checklist (`F01..F34`)

Verified 1:1 against `tests/e2e/feature_matrix.json`, `PROJECT.md`, and `TEST_INFRA.md`. Honest upstream distribution: **`28` `REAL SHIPPING`**, **`5` `PARTIAL / STUB`** (`F26`, `F28`, `F29`, `F30`, `F32`), and **`1` `REQUIREMENT-ONLY / PLANNED`** (`F34`). Every feature is covered across Rust Tiers 1–4 and Swift 6 V167 XCUITests; production runtime verification executes in Final Milestone Phase 1 post-M1–M6 merge.

| ID | Feature Name | `PROJECT.md` / `survey_2` | M# | RPC Namespaces | `fabushi.*` UI Namespaces | Rust T1 + T2 | Rust T3 Pairwise | Rust T4 Scenarios | Swift 6 V167 T1–T4 | Honest Upstream Status |
|---|---|---|---|---|---|---:|---|---|---|---|
| `F01` | Password Login, Registration & Split-Key V2 Bootstrap | `#2` / `survey_2 #1` | `M2` | `auth`, `vault`, `session` | `fabushi.auth` | `5 + 5` | `PAIR-01, 02` | `SCEN-01, 17` | `T1-F01`; `T2-10`; `T4-01` | `REAL SHIPPING` |
| `F02` | Email OTP Registration & Phone SMS Verification | `#3` / `survey_2 #2` | `M2` | `auth` | `fabushi.auth` | `5 + 5` | `PAIR-03, 04` | `SCEN-01` | `T1-F02` | `REAL SHIPPING` |
| `F03` | Alipay QR Login & Mobile OAuth Callback | `#4` / `survey_2 #3` | `M2` | `auth` | `fabushi.auth` | `5 + 5` | `PAIR-05, 06` | `SCEN-14, 16` | `T1-F03` | `REAL SHIPPING` |
| `F04` | Passkeys (WebAuthn/FIDO2) & QR Device Pairing | `#5` / `survey_2 #4` | `M2` | `auth` | `fabushi.auth`, `fabushi.settings` | `5 + 5` | `PAIR-07, 08` | `SCEN-02` | `T1-F04` | `REAL SHIPPING` |
| `F05` | Cloud 2FA Password & Active Device Sessions | `#6` / `survey_2 #5` | `M2` | `auth`, `session` | `fabushi.auth`, `fabushi.settings` | `5 + 5` | `PAIR-03, 07` | `SCEN-02` | `T1-F05` | `REAL SHIPPING` |
| `F06` | Multi-Account Switcher, Generation Fencing & Local Passcode/Biometric Lock | `#7` / `survey_2 #6` | `M2` | `session`, `generation` | `fabushi.shell`, `fabushi.auth`, `fabushi.settings` | `5 + 5` | `PAIR-09, 10` | `SCEN-02, 03, 18` | `T1-F06`; `T2-11`; `T3-03`; `T4-01` | `REAL SHIPPING` |
| `F07` | Unified Adaptive Apple Shell (iPad 3-Col / iPhone TabView + NavigationStack + `.sheet`) | `#8` / `survey_2 #7` | `M2` | `settings`, `session`, `actions` | `fabushi.shell` | `5 + 5` | `PAIR-11, 12` | `SCEN-01, 15` | `T1-F07`; `T2-01, 07, 08`; `T3-01, 02`; `T4-08` | `REAL SHIPPING` |
| `F08` | Grok Bot Multi-Select Compose (`To:` Picker & Atomic DM/Group Creation) | `#9` / `survey_2 #8` | `M3` | `chats` | `fabushi.compose` | `5 + 5` | `PAIR-13, 14` | `SCEN-04` | `T1-F08`; `T2-09`; `T3-04`; `T4-02` | `REAL SHIPPING` |
| `F09` | Unified Chat List, Custom Folders, Archive, Pin, Mute & Forum Topics | `#10` / `survey_2 #9` | `M3` | `chats`, `folders`, `topics` | `fabushi.chatlist` | `5 + 5` | `PAIR-15, 16` | `SCEN-03, 13` | `T1-F09`; `T2-16` | `REAL SHIPPING` |
| `F10` | Contacts, Peer Resolution, Blocklist, Send-As & Prestart Bot | `#11` / `survey_2 #10` | `M3` | `contacts`, `chats` | `fabushi.chatlist`, `fabushi.shell`, `fabushi.composer`, `fabushi.stage` | `5 + 5` | `PAIR-04, 14` | `SCEN-04` | `T1-F10` | `REAL SHIPPING` |
| `F11` | Atomic PTS/QTS Sync Engine, 24h `client_nonce` Dedup & Offline SQLite WAL Outbox | `#12` / `survey_2 #11` | `M1/M3` | `sync`, `outbox`, `messages` | `fabushi.shell`, `fabushi.stage` | `5 + 5` | `PAIR-09, 15, 35` | `SCEN-01, 03, 05, 12, 17` | `T1-F11`; `T2-06, 07`; `T3-03`; `T4-01` | `REAL SHIPPING` |
| `F12` | Virtualized Transcript, 900s Bubble Grouping & 11+ Canonical Message Cards | `#13` / `survey_2 #12` | `M3` | `messages` | `fabushi.stage` | `5 + 5` | `PAIR-17, 18` | `SCEN-04, 05, 06, 08, 12, 13` | `T1-F12`; `T4-06` | `REAL SHIPPING` |
| `F13` | Message Edit, Soft-Delete Tombstone, Forward, PTS-Fenced Reactions, Pins, TTL & Export | `#14` / `survey_2 #13` | `M3` | `messages`, `chats` | `fabushi.stage` | `5 + 5` | `PAIR-17, 19` | `SCEN-05` | `T1-F13`; `T3-10` | `REAL SHIPPING` |
| `F14` | IME-Safe Canonical Composer (80ms CJK Commit Fence), Drafts, Formatting & Voice Gesture | `#15` / `survey_2 #14` | `M3` | `messages` | `fabushi.composer` | `5 + 5` | `PAIR-20, 21` | `SCEN-05` | `T1-F14`; `T2-01, 09, 10` | `REAL SHIPPING` |
| `F15` | 49+ Hermes/Fabushi Slash Commands & `@` Context Reference Badges | `#16` / `survey_2 #15` | `M3` | `fabushi` | `fabushi.composer` | `5 + 5` | `PAIR-20, 22` | `SCEN-04, 06` | `T1-F15`; `T3-05`; `T4-02` | `REAL SHIPPING` |
| `F16` | Muse Context Inspector (Profile, Members, 5-Tab Shared Media, Bot/Computer/Coding Config) | `#17` / `survey_2 #16` | `M3` | `chats`, `fabushi` | `fabushi.inspector` | `5 + 5` | `PAIR-23, 24` | `SCEN-06, 08, 10, 13` | `T1-F16`; `T2-12`; `T3-01, 02`; `T4-04` | `REAL SHIPPING` |
| `F17` | Unified Hermes/Fabushi Agent Runtime, 29 Toolsets & 7-Tab Agent Studio | `#22` / `survey_2 #17` | `M4` | `fabushi` | `fabushi.studio` | `5 + 5` | `PAIR-22, 26, 31` | `SCEN-06, 07, 09, 11, 16` | `T1-F17`; `T2-01`; `T3-05`; `T4-02, 03, 05` | `REAL SHIPPING` |
| `F18` | Multi-Agent Protocol (Floor Control, 5-Hop Loop Budget, Shared Workspace, Grok `shouldReply`) | `#23` / `survey_2 #18` | `M4` | `multi_agent` | `fabushi.stage`, `fabushi.inspector` | `5 + 5` | `PAIR-13, 27` | `SCEN-04, 08` | `T1-F18`; `T3-04`; `T4-02` | `REAL SHIPPING` |
| `F19` | Pure-Rust Magpie AI Gateway, Routing Groups, Circuit Breakers, PII Masking & Vault OAuth Sync | `#24` / `survey_2 #19` | `M4` | `magpie`, `vault` | `fabushi.magpie`, `fabushi.settings` | `5 + 5` | `PAIR-25, 28` | `SCEN-07` | `T1-F19`; `T2-07`; `T3-09`; `T4-03` | `REAL SHIPPING` *(iOS zero-knowledge vault OAuth sync; no local desktop home-directory OAuth credential scan)* |
| `F20` | Third-Party Coding Agents (Claude Code, Codex, ACP) Mobile Client | `#25` / `survey_2 #20` | `M4` | `coding` | `fabushi.stage`, `fabushi.inspector`, `fabushi.studio` | `5 + 5` | `PAIR-23, 27` | `SCEN-08` | `T1-F20`; `T4-03` | `REAL SHIPPING` *(remote coding host client; local iOS subprocess strictly forbidden)* |
| `F21` | Official OAuth 2.1 Remote MCP Server Grants, PKCE/QR Approval & 27-Tool Redacted Audit | `#26` / `survey_2 #21` | `M4` | `mcp` | `fabushi.studio`, `fabushi.settings` | `5 + 5` | `PAIR-26, 29` | `SCEN-09` | `T1-F21`; `T3-07`; `T4-03` | `REAL SHIPPING` |
| `F22` | UU-Remote Computer Workbench, Multi-Session PTY Terminal, Tailnet & `<10ms` Takeover | `#27` / `survey_2 #22` | `M5` | `computers`, `remote`, `tailnet` | `fabushi.remote` | `5 + 5` | `PAIR-24, 29, 30` | `SCEN-09, 10` | `T1-F22`; `T2-14`; `T3-07`; `T4-04` | `REAL SHIPPING` *(iOS controller + PTY; unattended host daemon desktop-only)* |
| `F23` | Zero-Knowledge Split-Key V2 & X25519 Credential Vault (24 MiB) + RFC 6238 TOTP + Remote Obscura | `#21` / `survey_2 #23` | `M1/M2` | `vault` | `fabushi.settings`, `fabushi.auth` | `5 + 5` | `PAIR-01, 08, 28` | `SCEN-01, 02, 07` | `T1-F23`; `T2-11`; `T3-03, 09` | `REAL SHIPPING` *(remote Obscura host control; local iOS sidecar unsupported)* |
| `F24` | Telegram-Parity External Mini Apps, WebMCP Live Control, Floating PiP & Global Dharma | `#28` / `survey_2 #24` | `M5` | `miniapp`, `webmcp` | `fabushi.miniapp` | `5 + 5` | `PAIR-31, 32` | `SCEN-11` | `T1-F24`; `T2-13`; `T3-06`; `T4-05` | `REAL SHIPPING` |
| `F25` | Chunked Resumable Media CAS, HTTP 206/416 Streaming, 5-Bit 63B Waveform, Stickers, Stories & Instant View | `#20` / `survey_2 #25` | `M1/M3` | `media`, `stories`, `stickers` | `fabushi.stage`, `fabushi.composer` | `5 + 5` | `PAIR-18, 21` | `SCEN-05, 13, 15` | `T1-F25` | `REAL SHIPPING` |
| `F26` | 1:1 & Group Calls (`LiveKitWebRTC`/`CallKit`/`ReplayKit`) & Full-Duplex AI Talk Mode (`<60ms` VAD) | `#29` / `survey_2 #26` | `M5` | `calls`, `talk` | `fabushi.call` | `5 + 5` | `PAIR-33, 34` | `SCEN-12, 18` | `T1-F26`; `T2-15`; `T3-08`; `T4-06` | `PARTIAL / STUB` *(tracks `WEBRTC_NAT_SFU_TRANSPORT`)* |
| `F27` | Unified Tantivy Full-Text + Hybrid Memory + Bots/MiniApps Search & Offline FTS5 Fallback | `#18` / `survey_2 #27` | `M1/M3` | `search` | `fabushi.chatlist`, `fabushi.stage` | `5 + 5` | `PAIR-16, 19` | `SCEN-05, 13` | `T1-F27`; `T3-10` | `REAL SHIPPING` |
| `F28` | E2EE Messaging (Double Ratchet 1:1, Sender Keys Group & 4-Emoji/QR Safety Numbers) | `#19` / `survey_2 #28` | `M1/M3` | `e2ee` | `fabushi.inspector`, `fabushi.stage` | `5 + 5` | `PAIR-33, 35` | `SCEN-12` | `T1-F28`; `T3-08, 10`; `T4-01, 06` | `PARTIAL / STUB` *(upstream PreKey/crypto partial; full client wiring tracked)* |
| `F29` | Economy, Stars Ledger, Star Gifts, Premium Tiers, Redeem Codes & StoreKit 2 IAP/Restore | `#31` / `survey_2 #29` | `M5` | `wallet` | `fabushi.settings` | `5 + 5` | `PAIR-06, 32` | `SCEN-11, 14, 18` | `T1-F29`; `T4-05, 07` | `PARTIAL / STUB` *(tracks `STOREKIT_SERVER_BINDING`, `TON_ONCHAIN_SIGNING`)* |
| `F30` | Action Bus, iOS App Intents / Siri Shortcuts, `fabushi://` URLs & Push Token Registration | `#33` / `survey_2 #30` | `M5` | `actions`, `push` | `fabushi.shell`, `fabushi.settings` | `5 + 5` | `PAIR-05, 30` | `SCEN-10, 16, 18` | `T1-F30`; `T3-06`; `T4-04` | `PARTIAL / STUB` *(tracks `APNS_HTTP2_SERVER_DELIVERY`)* |
| `F31` | Apple HIG Design System (4 `v1.6.8` Themes, WCAG `>138` Guard, Liquid Glass, Dynamic Type, `zh-Hans`/`en`) | `#8` / `survey_2 #31` | `M2` | `settings` | `fabushi.settings`, `fabushi.shell` | `5 + 5` | `PAIR-11, 36` | `SCEN-03, 15` | `T1-F31`; `T2-01..05`; `T3-01, 02`; `T4-08` | `REAL SHIPPING` |
| `F32` | Full 26-Section Apple Settings Suite (Search, Real CAS/SQLite Storage Metrics & Purge) | `#30` / `survey_2 #32` | `M5` | `settings` | `fabushi.settings` | `5 + 5` | `PAIR-12, 36` | `SCEN-14, 15, 17` | `T1-F32`; `T2-01, 02`; `T4-07, 08` | `PARTIAL / STUB` *(22 real shipping; Business/Passport/TON partial)* |
| `F33` | Network, 4-Tier Config Cascade, Server URL, HTTP/SOCKS5/MTProto Proxy Probe & Update Check | `#32` / `survey_2 #33` | `M5` | `proxy`, `settings` | `fabushi.settings`, `fabushi.auth` | `5 + 5` | `PAIR-02, 25` | `SCEN-07, 10, 17` | `T1-F33`; `T2-06, 10`; `T4-07` | `REAL SHIPPING` *(App Store Guideline 2.5.2: `binary_self_update_allowed=false`)* |
| `F34` | Fail-Closed Governance, Anti-Stub C-ABI Invariants & Honest Upstream Backlog Disposition | `#1/#34` / `survey_2 #34` | `M1/M6` | `generation`, `settings` | `fabushi.shell`, `fabushi.settings` | `5 + 5` | `PAIR-10, 34` | `SCEN-03, 09, 11, 14, 16, 18` | `T1-F34`; `T2-16`; `T4-07` | `REQUIREMENT-ONLY / PLANNED` *(honest ledger for all 7 capability IDs)* |

---

## 5. Honestly Tracked Upstream & External Blockers (Zero Fabrication)

Locked in `tests/e2e/rust_ffi/src/contracts.rs` (`HONEST_TRACKED_CAPABILITY_IDS` / `verify_honest_capability_disposition`, permitting ONLY `"upstream_planned" | "server_blocked" | "desktop_only_sandbox"`) and asserted by `TC-T1-F34-04`, `TC-T2-F34-05`, `TC-T2-F26-05`, `TC-T2-F29-04`, `TC-T2-F29-05`, `TC-T1-F30-05`, and Swift `TC-UI-T1-F34` / `TC-UI-T2-15` / `TC-UI-T2-16`:

| # | Capability ID | Canonical Disposition | Feature(s) | Exact Honesty / Never-Fabricated Boundary |
|---|---|---|---|---|
| 1 | `OCRM-001` | `upstream_planned` | `F34` | 141 unwired OpenClaw channel adapters remain requirement-only upstream; RPC MUST fail closed, never return synthetic `ok: true`. |
| 2 | `FBCP-001` | `upstream_planned` | `F34` | Fabushi Control Plane D1 remains upstream requirement-only; reported honestly in `settings.capability_ledger`. |
| 3 | `TON_ONCHAIN_SIGNING` | `upstream_planned` | `F29`, `F32`, `F34` | TON wallet on-chain signing is not implemented upstream; fails closed as `upstream_planned`. |
| 4 | `TELEGRAM_PASSPORT` | `upstream_planned` | `F32`, `F34` | Telegram Passport remains upstream requirement-only; fails closed in settings and capability ledger. |
| 5 | `APNS_HTTP2_SERVER_DELIVERY` | `server_blocked` (`upstream_stub`) | `F30`, `F34` | iOS registers/unregisters real APNs device tokens, while upstream server `apns.rs` HTTP/2 delivery dispatcher is a stub. |
| 6 | `WEBRTC_NAT_SFU_TRANSPORT` | `server_blocked` (`upstream_loopback_stub`) | `F26`, `F34` | iOS preserves real `LiveKitWebRTC`/`CallKit`/`ReplayKit` owners, while upstream `gpui_telegram_webrtc` uses loopback SDP/ICE vs production NAT/SFU media. |
| 7 | `STOREKIT_SERVER_BINDING` | `server_blocked` | `F29`, `F34` | iOS implements App Store Guideline 3.1.1 StoreKit 2 IAP/Restore (`FabushiPayStoreKit`), while server-side Apple App Store JWS/receipt entitlement binding requires external App Store Connect & server provisioning. |

### External Hardware & App Store Release Gates (Non-Fabricable)
- Physical iOS device install & launch (`physical-device-install-launch`), installed-app upgrade data migration (`installed-app-upgrade-data-migration`), two-device physical live call media verification, Cloudflare `.fabapp` signer keys, and signed TestFlight / `FabushiBroadcastUpload` extension provisioning (`testflight-install-and-app-store-validation`) are external hardware/service gates tracked separately under `IOS-MAIN-AC-01..21` and NEVER fabricated by Simulator or loopback E2E runs.

---

## 6. Exact-HEAD GitHub Actions CI Evidence & Artifact SHA-256 Digests

### 6.1 Certified Suite-Code Baseline Run (E2E-2 — Zero-Diff Executable Code Proof)

- **Workflow**: `.github/workflows/ios-e2e-suite.yml` (`iOS v1.6.8 Opaque-Box E2E Suite`, Workflow ID `427406955`)
- **Run Provenance**: `GITHUB_RUN_ID=45145111639` — https://github.com/fabushiios/fabushi-ios/actions/runs/45145111639
- **Exact `headSha`**: `4416b6deeb739f58620a3321efb41a87280940b7` (root tree `c69787083ed7bd5c6e17d1e717c36d72f15eac51`)
- **Overall Result**: `status: completed`, `conclusion: success` (`0` skipped active steps across all 3 active jobs)

| Job Name | Job ID | Runner | Conclusion | Steps (Skipped) | Key Verified Outputs |
|---|---|---|---|---|---|
| `Detect E2E Execution Mode` | `137592359883` | `ubuntu-latest` | `success` | `5/5` (`0`) | `mode=self-validate`, `tier=self-validate`, `run_baseline_negative_proof=true`, `xcode_app=/Applications/Xcode_16.4.app` |
| `Ubuntu Rust FFI & Anti-Stub Self-Validate` | `137592449422` | `ubuntu-latest` | `success` | `17/17` (`0`) | `11/11` loopback self-test; `spec_coverage_check --strict` PASS (`394 + 68 = 462`); `cargo test --no-run -D warnings` `0` warnings; `self_validate_antistub` `15 passed; 0 failed; 0 ignored`; `SHA256SUMS.txt` `7/7 OK` |
| `macOS Swift 6 UITests Typecheck & Dual-Simulator Probe` | `137592449197` | `macos-15` | `success` | `10/10` (`0`) | `Xcode 16.4 (16F6)` / `Swift 6.1.2` / `iPhoneSimulator18.5.sdk`; dual simulators `iPhone 16 Pro` & `iPad Pro 13-inch (M4)` (`iOS 18.5`); Darwin loopback `11/11` PASS; `V167/** -warnings-as-errors` `7` files `0` warnings + `FabushiUITests` `9` files `0` errors; `SHA256SUMS.txt` `7/7 OK` |
| `Production Ubuntu Rust FFI E2E (Tiers 1-4)` | `137592452111` | `ubuntu-latest` | `skipped` | `0/0` (`0`) | Mode-gated `mode == 'production'` (activates automatically post-M1–M6 merge) |
| `Production macOS iPhone & iPad Simulator XCUITests (Tiers 1-4)` | `137592453050` | `macos-15` | `skipped` | `0/0` (`0`) | Mode-gated `mode == 'production'` (activates automatically post-M1–M6 merge) |

- **Uploaded GitHub Actions Artifacts & Verified SHA-256 Digests (`actions/upload-artifact@v7`)**:
  - `fabushi-ios-e2e-ubuntu-4416b6deeb739f58620a3321efb41a87280940b7-45145111639-1` — Artifact ID `14803932453` (`5,372` bytes): `sha256:bdc251357bc5a20dcaced3308991a86b7d0cfd918e6264671a6dcaaaf45da166`
  - `fabushi-ios-e2e-macos-4416b6deeb739f58620a3321efb41a87280940b7-45145111639-1` — Artifact ID `14802464479` (`4,048` bytes): `sha256:1544f491f874a93af3971071104e791481d175ac4471527d6d7ca896f4f56ee9`

### 6.2 Prior E2E-1 Certification Lineage Record

- `GITHUB_RUN_ID=45124582395` — https://github.com/fabushiios/fabushi-ios/actions/runs/45124582395 (`headSha`: `0b835141f80c0bbd76cab9b88863b0c6436c4dff`, `conclusion: success`):
  - Ubuntu Artifact ID `14794892911` (`3,076` bytes): `sha256:a6ff9a16efb564a2f587e36c0eec6864b144a3b29f3e0ff24722b846ef80004b`
  - macOS Artifact ID `14794988500` (`1,610` bytes): `sha256:02bfa40b07dcbdc1bedcd09ebf9f83f59c22460439d11b0630c359480a1ad7b5`

### 6.3 Exact-HEAD Binding Note for This Commit & Final Milestone Phase 1

- Because a Git blob cannot embed its own commit SHA, this file anchors the certified executable suite-code commit (`4416b6deeb739f58620a3321efb41a87280940b7`) whose executable test suite, matrix, loopback server, and workflow trees are byte-for-byte identical to this commit. Pushing this `TEST_READY.md`-only commit to `feat/e2e-suite-v167` triggers a fresh exact-HEAD `.github/workflows/ios-e2e-suite.yml` run recorded in the E2E-3 verification evidence.
- After M1–M6 integration into `main` and green Final Milestone Phase 1 production Tier 1–4 execution, `TEST_READY.md` will be updated with the merged `main` `IOS_EXPECTED_HEAD` SHA, production `GITHUB_RUN_ID` + Actions URL, and production Rust/`.xcresult` artifact SHA-256 digests (`scripts/check-desktop-main-ios-architecture.py --complete`).

---

## 7. Build Hygiene, Preserved Legacy Files & `IOS-MAIN-LEDGER-07` Attestation

- **CI Target & Bytecode Hygiene**:
  - `CARGO_TARGET_DIR` is isolated to `$GITHUB_WORKSPACE/target/e2e` and `$GITHUB_WORKSPACE/target/host` (zero `tests/e2e/rust_ffi/target/`).
  - Python CI steps enforce `PYTHONDONTWRITEBYTECODE=1` and out-of-tree `${RUNNER_TEMP}/pycompile_cache` with explicit `test ! -d tests/e2e/__pycache__` verification.
  - Zero `.agents/`, zero `*.db`/`*.db-wal`/`*.db-shm`, zero `.gitignore`-trapped generator scripts, and zero local workstation paths exist anywhere in the E2E worktree or this certification file.
- **Preserved Legacy Files (Byte-for-Byte Identical to `main` `8128951a71503a6f109ce0cb37618d022fe8e79a`)**:
  - `tests/e2e/master_e2e_suite.swift` (blob `c0f3ffcdc20ec773d1e70900b1f2017a34f0e540`) — deprecated legacy Nov 8 suite preserved untouched pending M6 `mobile/ios/project.yml:146` and `FabushiTests/TelegramE2ETests.swift` cleanup.
  - `mobile/ios/FabushiUITests/FabushiUITests.swift` (blob `69ada028ab1ff3e44bb8ed0c7c83189d2ea5a326`) & `mobile/ios/FabushiUITests/GlobalDharmaJourneyUITests.swift` (blob `7bea8e1974cbee479eaebb0292f02e4a90bdbba9`) — preserved untouched for existing protected-session and Global Dharma offline workflows (`ios-protected-session.yml`, `ios-global-dharma-offline.yml`, `ios-protected-global-dharma.yml`).
- **`IOS-MAIN-LEDGER-07` Static Integrity Attestation**:
  - Contains zero forbidden local host paths, zero banned legacy/typo markers, zero all-zero placeholder SHAs, and valid anchored GitHub Actions run provenance (`GITHUB_RUN_ID=45145111639`, https://github.com/fabushiios/fabushi-ios/actions/runs/45145111639) bound to non-zero 40-hex commit SHA `4416b6deeb739f58620a3321efb41a87280940b7`.

---

## 8. Formal Certification Gate Sign-Off & Final Milestone Phase 1 Handoff

- [x] **Gate 1 — Complete 462-Test Opaque-Box Suite & 34-Feature Bijection**: `394` Rust FFI (`170 + 170 + 36 + 18`) + `68` Swift 6 V167 XCUITests (`34 + 16 + 10 + 8`) = **`462`** executable tests (`>= 391`), `34/34` features (`F01..F34`), `33/33` RPC namespaces, `13/13` Tier-body `fabushi.*` UI namespaces, `6/6` C-ABI symbols + sentinel, `0` `@testable import Fabushi`, `0` `XCTSkip`, `0` tautologies.
- [x] **Gate 2 — Zero-Warning Multi-Platform Compilation & Strict Concurrency**: `ubuntu-latest` Rust Edition 2024 `RUSTFLAGS="-D warnings"` (`0` warnings) + `macos-15` `Xcode 16.4` / `Swift 6.1.2` `-swift-version 6 -strict-concurrency=complete -warnings-as-errors` (`0` warnings, `0` errors).
- [x] **Gate 3 — Empirical 3-Layer Anti-Stub Gate & Baseline Negative Proof**: `15/15` zero-`unsafe` `self_validate_antistub` PASS on `ubuntu-latest`, catching `mutant_stub` (all 8 defect modes + symlink/renamed `dlsym` bypass) and real compiled baseline `8128951` `libmahayana_app_host.so`.
- [x] **Gate 4 — Cross-Platform Loopback Server & Dual iOS Simulator Readiness**: `11/11` `simulator_loopback_server.py --self-test` PASS on BOTH `ubuntu-latest` and `macos-15` (including RFC 6455 `/api/v1/ws`), plus runtime-matched `iPhone 16 Pro` & `iPad Pro 13-inch (M4)` (`iOS 18.5`) verified on `macos-15`.
- [x] **Gate 5 — Honest Governance, Blocker Ledger & `IOS-MAIN-LEDGER-07` Compliance**: All `7/7` tracked upstream/server blockers enforced fail-closed, transparent `v1.6.9` live upstream advancement note recorded, zero local paths, zero fabricated production results.
- [ ] **Gate 6 — Final Milestone Phase 1 Production Execution (Post-M1–M6 Merge)**: Intentionally open until M1–M6 merge activates `mode=production, tier=all` executing all `394` Rust FFI tests (debug + release `libmahayana_app_host.so`) and all `68` Swift 6 V167 XCUITests across BOTH iPhone and iPad Simulators (`136` dual-form-factor runs) with exact-HEAD production proof.

**Final E2E Track Verdict**: **E2E SUITE READY FOR FINAL MILESTONE PHASE 1 — SELF-VALIDATE CI CERTIFIED (`GITHUB_RUN_ID=45145111639`, `4416b6deeb739f58620a3321efb41a87280940b7`)**.
