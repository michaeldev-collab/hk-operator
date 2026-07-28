# Cybercat firmware — Hybrid BLE config

Firmware for **Cybercat**, the physical controller for HK Operator. Drop-in
evolution of `ble-hid-hotkey.ino` / `ble-hid-hotkey-wifi`. Adds a custom GATT
service on the **same BLE link** as the HID keyboard so HK Operator MCC (desktop)
can sync slots and receive macro fire events — no second connection, no WiFi AP
for daily use.

**Firmware id (compatibility):** `Cyberdeck Pad Hybrid v0.2.0` — **6 presets**
(18 slots). BLE advertised name remains **`Cyberdeck Pad`**. Do not rename these
without coordinating all bonded hosts and MCC/probe matchers.

Hardware story: [`../docs/hardware-v1.md`](../docs/hardware-v1.md)

## Modes per slot
| Mode | Value | Behavior |
|------|-------|----------|
| HID | `0` | Cybercat types `mod`+`key` (works with desktop app closed) |
| Macro | `1` | Cybercat notifies `MacroEvent` `{presetIdx, actionIdx}`; desktop runs the action |

## Preset LEDs
| Preset | Indicator |
|--------|-----------|
| 1 | Red |
| 2 | Green |
| 3 | Blue |
| 4 | Red + Green |
| 5 | Green + Blue |
| 6 | Red + Blue |

B1 cycles 1→6→1. Dual-LED presets use two solids (no blink) so they stay readable at a glance.

## GATT UUIDs
| Role | UUID |
|------|------|
| Service | `c0de0001-3d17-4a00-8000-00805f9b34fb` |
| Slots (R/W, **486** bytes) | `c0de0002-3d17-4a00-8000-00805f9b34fb` |
| MacroEvent (notify, 2 bytes) | `c0de0003-3d17-4a00-8000-00805f9b34fb` |
| Info (R) | `c0de0004-3d17-4a00-8000-00805f9b34fb` |

### Slots binary layout
18 × `Hotkey` packed row-major (preset 0..5, action 0..2 = B2/B4/B5):

```
uint8 mode;      // 0=hid, 1=macro
uint8 mod;       // KEY_MOD_* bitmask
uint8 key;       // KEY_* (0 = none)
char  label[24]; // NUL-padded
```

Total **486 bytes**. Prefer MTU ≥ 517 on the host. NVS namespace: `hotkeys3`.

### MacroEvent payload
`[presetIdx u8, actionIdx u8]` — zero-based. If notify fails / no clear subscriber,
blue LED double-blinks then restores the preset LED pattern.

## Flash
```bash
arduino-cli compile --fqbn esp32:esp32:esp32c6 \
  --libraries ~/Arduino/libraries \
  .

# Pick the Cybercat serial device — do NOT flash the wrong board.
arduino-cli upload -p /dev/ttyACM1 --fqbn esp32:esp32:esp32c6 \
  .
```

(Adjust port. Default partition is usually fine — no WiFi stack.)

## WiFi rescue (optional)
Compile with `-DENABLE_WIFI_FALLBACK=1` to restore long-press B3 AP portal.
Default build has WiFi **off**; short-press B3 only toggles LEDs.

## Desktop
See [`../mcc/`](../mcc/) — `probe/` CLI and Tauri app use these UUIDs via
BlueZ on the already-bonded HID connection. MCC expects **18** slots after this
firmware is flashed.
