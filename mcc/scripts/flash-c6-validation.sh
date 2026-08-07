#!/usr/bin/env bash
# Flash C6 experimental validation firmware via OpenOCD USB-JTAG.
# Selects the Cyberpad by USB serial 20:6E:F1:11:5F:34 (not the S3).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
C6_SERIAL="${C6_SERIAL:-20:6E:F1:11:5F:34}"
S3_SERIAL='A0:F2:62:F3:D5:CC'
FQBN='esp32:esp32:esp32c6'
SKETCH="$ROOT/firmware/c6-s3-dongle-validation"
CDC_GATE="$ROOT/scripts/s3-cdc-release-gate.sh"
CDC_WAIT_SECONDS=12
CDC_PROOF_SECONDS=30
OCD_HOME="${OPENOCD_ESP32:-$HOME/.arduino15/packages/esp32/tools/openocd-esp32/v0.12.0-esp32-20260424}"
BUILD_DIR="${BUILD_DIR:-/tmp/cyberpad-v03-final-c6}"
BOOT_APP0="${BOOT_APP0:-$HOME/.arduino15/packages/esp32/hardware/esp32/3.3.11/tools/partitions/boot_app0.bin}"
HASH_MANIFEST="${HASH_MANIFEST:-$BUILD_DIR/flash-artifacts.sha256}"
SKIP_COMPILE="${SKIP_COMPILE:-1}"
DRY_RUN="${DRY_RUN:-0}"
GENERATE_HASH_MANIFEST="${GENERATE_HASH_MANIFEST:-0}"

die() {
  echo "ERROR: $*" >&2
  exit 1
}

for flag_name in SKIP_COMPILE DRY_RUN GENERATE_HASH_MANIFEST; do
  flag_value="${!flag_name}"
  [[ "$flag_value" == "0" || "$flag_value" == "1" ]] || \
    die "$flag_name must be 0 or 1 (got '$flag_value')"
done

if [[ "$SKIP_COMPILE" != "1" ]]; then
  [[ ! -e "$HASH_MANIFEST" ]] || \
    die "refusing to rebuild hash-frozen artifacts; use a fresh BUILD_DIR"
  echo "==> compile experimental"
  arduino-cli compile --fqbn "$FQBN" \
    --build-property 'compiler.cpp.extra_flags=-DCYBERPAD_EXPERIMENTAL_S3_DONGLE=1' \
    --libraries "$ROOT/firmware" \
    --libraries /run/media/stitch/data3/Operating/pi-iot/libraries \
    --build-path "$BUILD_DIR" \
    "$SKETCH"
fi

BIN="$BUILD_DIR/c6-s3-dongle-validation.ino.bin"
BOOT="$BUILD_DIR/c6-s3-dongle-validation.ino.bootloader.bin"
PART="$BUILD_DIR/c6-s3-dongle-validation.ino.partitions.bin"
test -x "$OCD_HOME/bin/openocd"
test -f "$OCD_HOME/share/openocd/scripts/board/esp32c6-builtin.cfg"
test -r "$CDC_GATE"
test -f "$BIN" && test -f "$BOOT" && test -f "$PART" && test -f "$BOOT_APP0"
test -f "$BUILD_DIR/build.options.json" && test -f "$BUILD_DIR/flash_args"
grep -Fq '"fqbn": "esp32:esp32:esp32c6"' "$BUILD_DIR/build.options.json"
grep -Fq 'compiler.cpp.extra_flags=-DCYBERPAD_EXPERIMENTAL_S3_DONGLE=1' "$BUILD_DIR/build.options.json"
grep -Fq '/esp32/3.3.11' "$BUILD_DIR/build.options.json"

mapfile -t FLASH_ARGS < "$BUILD_DIR/flash_args"
EXPECTED_FLASH_ARGS=(
  '--flash-mode dio --flash-freq 80m --flash-size 4MB'
  '0x0 c6-s3-dongle-validation.ino.bootloader.bin'
  '0x8000 c6-s3-dongle-validation.ino.partitions.bin'
  '0xe000 boot_app0.bin'
  '0x10000 c6-s3-dongle-validation.ino.bin'
)
[[ "${#FLASH_ARGS[@]}" -eq "${#EXPECTED_FLASH_ARGS[@]}" ]] || \
  die "unexpected flash_args line count in $BUILD_DIR/flash_args"
for i in "${!EXPECTED_FLASH_ARGS[@]}"; do
  [[ "${FLASH_ARGS[$i]}" == "${EXPECTED_FLASH_ARGS[$i]}" ]] || \
    die "unexpected flash_args line $((i + 1)): '${FLASH_ARGS[$i]}'"
done

artifact_sha256() {
  local digest
  read -r digest _ < <(sha256sum "$1")
  printf '%s' "$digest"
}

generate_hash_manifest() {
  local manifest_dir manifest_tmp
  manifest_dir="$(dirname "$HASH_MANIFEST")"
  [[ -d "$manifest_dir" ]] || die "hash manifest directory does not exist: $manifest_dir"
  [[ ! -e "$HASH_MANIFEST" ]] || die "refusing to overwrite hash manifest: $HASH_MANIFEST"
  manifest_tmp="$(mktemp "$manifest_dir/.flash-artifacts.sha256.XXXXXX")"
  {
    printf 'bootloader=%s\n' "$(artifact_sha256 "$BOOT")"
    printf 'partitions=%s\n' "$(artifact_sha256 "$PART")"
    printf 'boot_app0=%s\n' "$(artifact_sha256 "$BOOT_APP0")"
    printf 'application=%s\n' "$(artifact_sha256 "$BIN")"
  } > "$manifest_tmp"
  chmod 0444 "$manifest_tmp"
  mv "$manifest_tmp" "$HASH_MANIFEST"
  echo "==> created read-only hash manifest: $HASH_MANIFEST"
}

if [[ "$GENERATE_HASH_MANIFEST" == "1" ]]; then
  [[ "$DRY_RUN" == "1" ]] || \
    die "GENERATE_HASH_MANIFEST=1 is allowed only with DRY_RUN=1"
  generate_hash_manifest
fi

[[ -r "$HASH_MANIFEST" ]] || die "missing hash manifest: $HASH_MANIFEST"
mapfile -t HASH_LINES < "$HASH_MANIFEST"
HASH_KEYS=(bootloader partitions boot_app0 application)
HASH_FILES=("$BOOT" "$PART" "$BOOT_APP0" "$BIN")
[[ "${#HASH_LINES[@]}" -eq "${#HASH_KEYS[@]}" ]] || \
  die "hash manifest must contain exactly four ordered entries"
for i in "${!HASH_KEYS[@]}"; do
  if [[ "${HASH_LINES[$i]}" =~ ^${HASH_KEYS[$i]}=([[:xdigit:]]{64})$ ]]; then
    expected_hash="${BASH_REMATCH[1],,}"
  else
    die "malformed hash manifest entry $((i + 1)): '${HASH_LINES[$i]}'"
  fi
  actual_hash="$(artifact_sha256 "${HASH_FILES[$i]}")"
  [[ "$actual_hash" == "$expected_hash" ]] || \
    die "${HASH_KEYS[$i]} hash mismatch: got $actual_hash, expected $expected_hash"
done
echo "==> immutable artifact hash check passed: $HASH_MANIFEST"

USB_DEVICE=""
USB_IDENTITY_MATCHES=0
USB_AUTHORIZED_MATCHES=0
for serial_file in /sys/bus/usb/devices/*/serial; do
  [[ -r "$serial_file" ]] || continue
  read -r found_serial < "$serial_file"
  usb_dir="${serial_file%/serial}"
  [[ -r "$usb_dir/idVendor" && -r "$usb_dir/idProduct" ]] || continue
  read -r found_vid < "$usb_dir/idVendor"
  read -r found_pid < "$usb_dir/idProduct"
  if [[ "$found_serial" == "$C6_SERIAL" && "$found_vid" == "303a" && "$found_pid" == "1001" ]]; then
    USB_IDENTITY_MATCHES=$((USB_IDENTITY_MATCHES + 1))
    if [[ -r "$usb_dir/authorized" ]]; then
      read -r found_authorized < "$usb_dir/authorized"
      if [[ "$found_authorized" == "1" ]]; then
        USB_AUTHORIZED_MATCHES=$((USB_AUTHORIZED_MATCHES + 1))
        USB_DEVICE="$usb_dir"
      fi
    fi
  fi
done
[[ "$USB_IDENTITY_MATCHES" -eq 1 ]] || \
  die "expected exactly one C6 303a:1001 serial $C6_SERIAL in sysfs; found $USB_IDENTITY_MATCHES"
[[ "$USB_AUTHORIZED_MATCHES" -eq 1 && -n "$USB_DEVICE" ]] || \
  die "the exact C6 USB device is not uniquely present with authorized=1"
echo "==> exact C6 identity: $USB_DEVICE serial=$C6_SERIAL"

if [[ "$DRY_RUN" == "1" ]]; then
  echo "==> dry run complete; device was not touched"
  exit 0
fi

command -v sudo >/dev/null 2>&1 || die "sudo is not installed"
sudo -n true || die "non-interactive sudo is unavailable; refusing unattended flash"
bash "$CDC_GATE" build

echo "==> require live S3 CDC release gate before any C6 mutation (serial=$S3_SERIAL)"
bash "$CDC_GATE" prove "$CDC_WAIT_SECONDS" "$CDC_PROOF_SECONDS" || \
  die "sustained exact S3 CDC responsiveness was not proven; C6 was not touched"

echo "==> flash C6 serial=$C6_SERIAL"
sudo -n "$OCD_HOME/bin/openocd" -s "$OCD_HOME/share/openocd/scripts" \
  -f board/esp32c6-builtin.cfg \
  -c "adapter serial $C6_SERIAL" \
  -c "adapter speed 5000" \
  -c "program_esp $BOOT 0x0 verify" \
  -c "program_esp $PART 0x8000 verify" \
  -c "program_esp $BOOT_APP0 0xe000 verify" \
  -c "program_esp $BIN 0x10000 verify reset exit"

echo "==> programmed, verified, and issued OpenOCD reset-run"
echo "    Expected app identity: Cyberpad Val C6 / Cyberdeck Pad Hybrid v0.3.1"
