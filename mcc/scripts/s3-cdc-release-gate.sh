#!/usr/bin/env bash
# Build and run the exact-identity S3 TinyUSB CDC release verifier.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="$ROOT/target"
VERIFIER="$TARGET_DIR/release/cyberdeck-dongle-verify"
EXPECTED_VERSION='s3-dongle-validation 0.5.3 protocol-v0.3'

die() {
  echo "ERROR: $*" >&2
  exit 1
}

build_verifier() {
  command -v cargo >/dev/null 2>&1 || die "cargo is not installed"
  CARGO_TARGET_DIR="$TARGET_DIR" cargo build --offline --locked --release \
    --manifest-path "$ROOT/Cargo.toml" \
    -p cyberdeck-dongle --bin cyberdeck-dongle-verify
  [[ -x "$VERIFIER" ]] || die "CDC verifier was not built: $VERIFIER"
}

probe_once() {
  local output
  if output="$("$VERIFIER" version 2>&1)" && [[ "$output" == "$EXPECTED_VERSION" ]]; then
    printf '%s\n' "$output"
    return 0
  fi

  # OpenOCD already requires non-interactive sudo on these boards. Try the
  # exact same fixed verifier as root only when normal USB permissions prevent
  # its libusb CDC fallback from opening the S3.
  if output="$(sudo -n "$VERIFIER" version 2>&1)" && \
      [[ "$output" == "$EXPECTED_VERSION" ]]; then
    printf '%s\n' "$output"
    return 0
  fi
  printf '%s' "$output"
  return 1
}

wait_for_version() {
  local wait_seconds="$1"
  local deadline attempts=0 output='' last_error='no CDC probe attempted'

  [[ "$wait_seconds" =~ ^[[:digit:]]+$ ]] || \
    die "wait seconds must be an integer (got '$wait_seconds')"
  ((wait_seconds >= 1 && wait_seconds <= 120)) || \
    die "wait seconds must be in 1..120 (got '$wait_seconds')"
  [[ -x "$VERIFIER" ]] || die "missing prebuilt CDC verifier: $VERIFIER"

  deadline=$((SECONDS + wait_seconds))
  while ((SECONDS <= deadline)); do
    attempts=$((attempts + 1))
    if output="$(probe_once)"; then
      [[ "$output" == "$EXPECTED_VERSION" ]] || \
        die "verifier returned unexpected success output: '$output'"
      echo "==> exact S3 TinyUSB CDC version proven: $output"
      return 0
    fi
    last_error="$output"
    sleep 1
  done

  echo "ERROR: exact S3 TinyUSB CDC version was not proven after $attempts attempts" >&2
  echo "       expected: $EXPECTED_VERSION" >&2
  [[ -z "$last_error" ]] || echo "       last probe: $last_error" >&2
  return 1
}

prove_responsive() {
  local proof_seconds="$1"
  local deadline attempts=0 output=''

  [[ "$proof_seconds" =~ ^[[:digit:]]+$ ]] || \
    die "proof seconds must be an integer (got '$proof_seconds')"
  ((proof_seconds >= 1 && proof_seconds <= 120)) || \
    die "proof seconds must be in 1..120 (got '$proof_seconds')"
  [[ -x "$VERIFIER" ]] || die "missing prebuilt CDC verifier: $VERIFIER"

  deadline=$((SECONDS + proof_seconds))
  while :; do
    attempts=$((attempts + 1))
    if ! output="$(probe_once)"; then
      echo "ERROR: S3 CDC responsiveness proof failed on attempt $attempts" >&2
      echo "       expected: $EXPECTED_VERSION" >&2
      [[ -z "$output" ]] || echo "       probe: $output" >&2
      return 1
    fi
    [[ "$output" == "$EXPECTED_VERSION" ]] || \
      die "verifier returned unexpected success output: '$output'"
    ((SECONDS >= deadline)) && break
    sleep 1
  done

  echo "==> sustained S3 CDC proof passed for ${proof_seconds}s across $attempts exact replies"
}

case "${1:-}" in
  build)
    [[ "$#" -eq 1 ]] || die "usage: $0 build"
    build_verifier
    ;;
  wait)
    [[ "$#" -eq 2 ]] || die "usage: $0 wait <seconds>"
    wait_for_version "$2"
    ;;
  prove)
    [[ "$#" -eq 3 ]] || die "usage: $0 prove <wait-seconds> <proof-seconds>"
    wait_for_version "$2"
    prove_responsive "$3"
    ;;
  *)
    die "usage: $0 {build|wait <seconds>|prove <wait-seconds> <proof-seconds>}"
    ;;
esac
