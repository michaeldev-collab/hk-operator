#!/usr/bin/env bash
# Flash S3 validation firmware via OpenOCD USB-JTAG (no /dev/ttyACM required).
# Selects the dongle by USB serial A0:F2:62:F3:D5:CC (not the C6).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
S3_SERIAL="${S3_SERIAL:-A0:F2:62:F3:D5:CC}"
FQBN='esp32:esp32:esp32s3:USBMode=default,CDCOnBoot=cdc,FlashSize=4M,PSRAM=disabled,UploadMode=cdc'
SKETCH="$ROOT/firmware/s3-dongle-validation"
OCD_HOME="${OPENOCD_ESP32:-$HOME/.arduino15/packages/esp32/tools/openocd-esp32/v0.12.0-esp32-20251215}"
BUILD_DIR="${BUILD_DIR:-/tmp/s3-dongle-validation-build}"
BOOT_APP0="${BOOT_APP0:-$HOME/.arduino15/packages/esp32/hardware/esp32/3.3.8/tools/partitions/boot_app0.bin}"

echo "==> compile"
arduino-cli compile --fqbn "$FQBN" \
  --libraries "$ROOT/firmware" \
  --libraries /run/media/stitch/data3/Operating/pi-iot/libraries \
  --build-path "$BUILD_DIR" \
  "$SKETCH"

BIN="$BUILD_DIR/s3-dongle-validation.ino.bin"
BOOT="$BUILD_DIR/s3-dongle-validation.ino.bootloader.bin"
PART="$BUILD_DIR/s3-dongle-validation.ino.partitions.bin"
test -f "$BIN"
test -f "$BOOT"
test -f "$PART"
test -f "$BOOT_APP0"

echo "==> flash via OpenOCD esp_usb_jtag serial=$S3_SERIAL (needs sudo for libusb)"
echo "    Arduino flash_args offsets: 0x0 / 0x8000 / 0xe000 / 0x10000"
# Segmented program_esp is more reliable than one huge merged image on this link.
sudo "$OCD_HOME/bin/openocd" -s "$OCD_HOME/share/openocd/scripts" \
  -f board/esp32s3-builtin.cfg \
  -c "adapter serial $S3_SERIAL" \
  -c "adapter speed 10000" \
  -c "program_esp $BOOT 0x0 verify" \
  -c "program_esp $PART 0x8000 verify" \
  -c "program_esp $BOOT_APP0 0xe000 verify" \
  -c "program_esp $BIN 0x10000 verify reset exit"

echo "==> done"
echo "If USB stays on 303a:1001 and BLE name S3ValDongle never appears, press the"
echo "board RESET button (BOOT not held) — OpenOCD soft-reset may leave ROM USB-JTAG idle."
