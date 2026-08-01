# S3 Dongle Validation PoC — Implementation Plan

Branch: `experiment/s3-dongle-validation`  
Status: validation only — not Cyberpad V2 / not production dongle.

## Phase 0 findings (repo + firmware)

| Item | Finding |
|------|---------|
| MCC repo | Tauri 2 + `cyberdeck-ble` (BlueZ) + vanilla UI — no in-repo firmware tree before this PoC |
| C6 firmware | External: `/run/media/stitch/data3/Operating/pi-iot/esp32/ble-hid-hotkeys/ble-hid-hotkey-ble-config/` |
| Framework | Arduino-ESP32 3.3.8 + `HijelHID_BLEKeyboard` + `NimBLE-Arduino` |
| Build | `arduino-cli` FQBN `esp32:esp32:esp32c6` |
| BLE HID | `keyboard.tap(key, mod)` on press edge (no hold/release path today) |
| Switch path | `INPUT_PULLUP`, 40 ms debounce, edge HIGH→LOW → `handleButtonPress` |
| Preset/macro | 6 presets × 3 actions; HID tap or GATT MacroEvent notify |
| Docs / probe | `protocol/PROTOCOL.md`, `docs/hw-gate.md`, `cyberdeck-probe` |

## Isolation strategy

1. **Do not modify** the external production sketch path as the sole source of truth for this PoC.
2. Vendor a validation C6 sketch under `firmware/c6-s3-dongle-validation/` with
   `CYBERPAD_EXPERIMENTAL_S3_DONGLE` (default `0` = hybrid behavior preserved).
3. New S3 project under `firmware/s3-dongle-validation/`.
4. Shared packed protocol header: `firmware/common/validation_protocol.h`.
5. Host encode/decode tests: `test/validation_proto.test.mjs`.
6. Portfolio / PCB / CAD files untouched. Unrelated WIP stashed off this branch.

When experimental mode is **enabled** on C6: direct BLE HID is temporarily disabled;
only the validation GATT peripheral runs. Documented limitation.

## Files to add

| Path | Role |
|------|------|
| `docs/validation/s3-dongle-poc-plan.md` | This plan |
| `docs/validation/s3-mini-hardware-identification.md` | Phase 1 hardware ID |
| `docs/validation/c6-s3-validation-protocol.md` | Protocol v0 |
| `docs/validation/s3-dongle-poc-results.md` | Test log |
| `firmware/common/validation_protocol.h` | Packed encode/decode (C) |
| `firmware/s3-dongle-validation/` | S3 USB HID + BLE central bridge |
| `firmware/c6-s3-dongle-validation/` | C6 hybrid + experimental transport |
| `protocol/validation_uuids.md` | Stable experimental UUIDs |
| `test/validation_proto.test.mjs` | Host encode/decode tests |
| `scripts/flash-s3-dongle-validation.sh` | OpenOCD flash helper (no ttyACM) |

## Files changed

- None of the existing MCC desktop/portfolio sources for this PoC (unless a tiny README pointer is added later).

## Verification plan

1. Compile S3 + both C6 flag modes.
2. Flash S3 via OpenOCD (`esp_usb_jtag`) — host kernel currently lacks matching `cdc_acm`.
3. Test A: local `hid test a` → host key event.
4. Flash C6 experimental when C6 USB is available; Tests B–D.
5. Compile/regression Test E with flag off.

## Known blockers at plan time

- Host running kernel `7.1.5-arch1-1` vs modules `7.1.5-arch1-2` → `cdc_acm` unloadable (reboot needed for `/dev/ttyACM*`).
- C6 Cyberpad is BLE-connected (`20:6E:F1:11:5F:36`) but **not** on USB — flash requires plug-in.
- S3 identified via OpenOCD over native USB JTAG (`303a:1001`).
