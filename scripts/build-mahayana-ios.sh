#!/usr/bin/env bash
# scripts/build-mahayana-ios.sh
# Production iOS Rust cross-compilation pipeline for source/host (libmahayana_app_host.a).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Ensure Rust/Homebrew tools are discoverable even from minimal Xcode Run Script PATH
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

# Respect Xcode environment variables if passed from XcodeGen Run Script phase
PLATFORM="${1:-${PLATFORM_NAME:-iphonesimulator}}"
ARCH="${2:-${CURRENT_ARCH:-arm64}}"
CONFIG="${CONFIGURATION:-Release}"

# Fail closed: source/host/Cargo.toml is the single canonical standalone mobile bridge manifest
MANIFEST="${MANIFEST:-$ROOT/source/host/Cargo.toml}"
if [[ ! -f "$MANIFEST" ]]; then
  echo "[build-mahayana-ios] Error: Missing canonical iOS bridge manifest at $MANIFEST" >&2
  exit 2
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# Lean profile flags (keep debuginfo stripped without stripping exported C-ABI symbols!)
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_DEV_INCREMENTAL=false
export CARGO_PROFILE_DEV_STRIP="debuginfo"
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_TEST_INCREMENTAL=false
export CARGO_PROFILE_TEST_STRIP="debuginfo"
export CARGO_PROFILE_RELEASE_DEBUG=0
export CARGO_PROFILE_RELEASE_INCREMENTAL=false
export CARGO_PROFILE_RELEASE_STRIP="debuginfo"

# Explicit Apple iOS deployment targets & bitcode settings (matching CI ios-gpui-telegram-ci.yml)
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-17.0}"
export CFLAGS_aarch64_apple_ios="${CFLAGS_aarch64_apple_ios:--miphoneos-version-min=${IPHONEOS_DEPLOYMENT_TARGET}}"
export CFLAGS_aarch64_apple_ios_sim="${CFLAGS_aarch64_apple_ios_sim:--mios-simulator-version-min=${IPHONEOS_DEPLOYMENT_TARGET}}"
export CFLAGS_x86_64_apple_ios="${CFLAGS_x86_64_apple_ios:--mios-simulator-version-min=${IPHONEOS_DEPLOYMENT_TARGET}}"
export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS:--Cembed-bitcode=no}"

# Sanitize Xcode-injected host compiler/SDK variables that break Rust build.rs & proc-macros
unset SDKROOT PLATFORM_DISPLAY_NAME PLATFORM_NAME CURRENT_ARCH ARCHS SDK_NAME MACOSX_DEPLOYMENT_TARGET
unset CC CXX CFLAGS CXXFLAGS LDFLAGS AR RANLIB

# Determine profile directory name and cargo build flag
CONFIG_LOWER="$(echo "$CONFIG" | tr '[:upper:]' '[:lower:]')"
CARGO_PROFILE_FLAG="--release"
PROFILE_SUBDIR="release"
if [[ "$CONFIG_LOWER" == "debug" && "${FORCE_RELEASE:-1}" != "1" ]]; then
  CARGO_PROFILE_FLAG=""
  PROFILE_SUBDIR="debug"
fi

build_target() {
  local target="$1"
  local output_dir="$2"

  echo "[build-mahayana-ios] Target: $target | Platform: $output_dir | Config: $PROFILE_SUBDIR"

  if ! rustup target list --installed | grep -q "^${target}$"; then
    echo "[build-mahayana-ios] Target $target not found in rustup, installing..."
    rustup target add "$target"
  fi

  echo "[build-mahayana-ios] Executing cargo build --lib (manifest: $MANIFEST, target dir: $CARGO_TARGET_DIR)..."
  if [[ -n "$CARGO_PROFILE_FLAG" ]]; then
    cargo build \
      --manifest-path "$MANIFEST" \
      --lib \
      "$CARGO_PROFILE_FLAG" \
      --target "$target"
  else
    cargo build \
      --manifest-path "$MANIFEST" \
      --lib \
      --target "$target"
  fi

  local artifact="$CARGO_TARGET_DIR/$target/$PROFILE_SUBDIR/libmahayana_app_host.a"
  if [[ ! -s "$artifact" ]]; then
    echo "[build-mahayana-ios] Error: Staticlib artifact missing or empty at $artifact" >&2
    exit 1
  fi

  mkdir -p "$ROOT/build/rust/$output_dir"
  cp "$artifact" "$ROOT/build/rust/$output_dir/libmahayana_app_host.a"
  echo "[build-mahayana-ios] Copied staticlib to $ROOT/build/rust/$output_dir/libmahayana_app_host.a"
}

case "$PLATFORM" in
  iphoneos)
    build_target aarch64-apple-ios iphoneos
    ;;
  iphonesimulator)
    if [[ "$ARCH" == "x86_64" ]]; then
      build_target x86_64-apple-ios iphonesimulator
    else
      build_target aarch64-apple-ios-sim iphonesimulator
    fi
    ;;
  all)
    build_target aarch64-apple-ios iphoneos
    build_target aarch64-apple-ios-sim iphonesimulator
    ;;
  *)
    echo "[build-mahayana-ios] Unsupported Apple platform: $PLATFORM" >&2
    exit 2
    ;;
esac

if [[ "${SKIP_CLEAN_BUILD_GARBAGE:-0}" != "1" && -f "$ROOT/scripts/clean_build_garbage.py" ]]; then
  echo "[build-mahayana-ios] Running build hygiene garbage collection..."
  python3 "$ROOT/scripts/clean_build_garbage.py"
fi

echo "[build-mahayana-ios] Pipeline completed successfully."
