#!/usr/bin/env bash
# Flash C6 experimental validation firmware via OpenOCD USB-JTAG.
# Selects the Cyberpad by USB serial 20:6E:F1:11:5F:34 (not the S3).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
C6_SERIAL="${C6_SERIAL:-20:6E:F1:11:5F:34}"
FQBN='esp32:esp32:esp32c6'
SKETCH="$ROOT/firmware/c6-s3-dongle-validation"
OCD_HOME="${OPENOCD_ESP32:-$HOME/.arduino15/packages/esp32/tools/openocd-esp32/v0.12.0-esp32-20251215}"
BUILD_DIR="${BUILD_DIR:-/tmp/c6-s3-dongle-validation-exp}"
BOOT_APP0="${BOOT_APP0:-$HOME/.arduino15/packages/esp32/hardware/esp32/3.3.8/tools/partitions/boot_app0.bin}"

echo "==> compile experimental"
arduino-cli compile --fqbn "$FQBN" \
  --build-property 'compiler.cpp.extra_flags=-DCYBERPAD_EXPERIMENTAL_S3_DONGLE=1' \
  --libraries "$ROOT/firmware" \
  --libraries /run/media/stitch/data3/Operating/pi-iot/libraries \
  --build-path "$BUILD_DIR" \
  "$SKETCH"

BIN="$BUILD_DIR/c6-s3-dongle-validation.ino.bin"
BOOT="$BUILD_DIR/c6-s3-dongle-validation.ino.bootloader.bin"
PART="$BUILD_DIR/c6-s3-dongle-validation.ino.partitions.bin"
test -f "$BIN" && test -f "$BOOT" && test -f "$PART" && test -f "$BOOT_APP0"

echo "==> flash C6 serial=$C6_SERIAL"
# Soft reauth helps when JTAG PIPE errors after pyusb/CDC claims.
DEVPATH="$(python3 - <<PY
import os
ser=os.environ.get("C6_SERIAL","$C6_SERIAL")
for name in os.listdir("/sys/bus/usb/devices"):
    base=f"/sys/bus/usb/devices/{name}"
    try:
        if open(base+"/idVendor").read().strip()!="303a":
            continue
        if ser in open(base+"/serial").read():
            print(name); break
    except Exception:
        pass
PY
)"
if [[ -n "${DEVPATH:-}" ]]; then
  echo "reauth $DEVPATH"
  echo 0 | sudo tee "/sys/bus/usb/devices/$DEVPATH/authorized" >/dev/null
  sleep 0.5
  echo 1 | sudo tee "/sys/bus/usb/devices/$DEVPATH/authorized" >/dev/null
  sleep 1
fi

sudo "$OCD_HOME/bin/openocd" -s "$OCD_HOME/share/openocd/scripts" \
  -f board/esp32c6-builtin.cfg \
  -c "adapter serial $C6_SERIAL" \
  -c "adapter speed 5000" \
  -c "program_esp $BOOT 0x0 verify" \
  -c "program_esp $PART 0x8000 verify" \
  -c "program_esp $BOOT_APP0 0xe000 verify" \
  -c "program_esp $BIN 0x10000 verify reset exit"

echo "==> done. Press RESET on the Cyberpad (not BOOT), then look for BLE name: Cyberpad Val C6"
