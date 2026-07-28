# Cyberdeck Pad BLE protocol (host side)

Canonical UUIDs and binary layout for HK Operator MCC / probe CLI.

Firmware: [`../../firmware/`](../../firmware/)  
Info string (v0.2+): `Cyberdeck Pad Hybrid v0.2.0`

## UUIDs
| Role | UUID |
|------|------|
| Service | `c0de0001-3d17-4a00-8000-00805f9b34fb` |
| Slots | `c0de0002-3d17-4a00-8000-00805f9b34fb` |
| MacroEvent | `c0de0003-3d17-4a00-8000-00805f9b34fb` |
| Info | `c0de0004-3d17-4a00-8000-00805f9b34fb` |

## Device name
BLE advertised name: **`Cyberdeck Pad`**

## Presets & LEDs
B1 cycles presets **1..6**. Three LEDs encode the active preset:

| Preset | LED |
|--------|-----|
| 1 | Red |
| 2 | Green |
| 3 | Blue |
| 4 | Red + Green (dual solid) |
| 5 | Green + Blue (dual solid) |
| 6 | Red + Blue (dual solid) |

## Slots (486 bytes)
18 × 27-byte records, row-major `preset 0..=5`, `action 0..=2` (B2, B4, B5):

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 | `mode` — `0` HID, `1` Macro |
| 1 | 1 | `mod` — HID modifier bitmask |
| 2 | 1 | `key` — HID keycode (`0` = none) |
| 3 | 24 | `label` — UTF-8, NUL-padded |

Prefer MTU ≥ 517 on both ends (firmware requests 517).

## MacroEvent notify
2 bytes: `[preset_idx u8, action_idx u8]` (zero-based, preset may be `0..=5`).

## Host connection rule
Do **not** open a second BLE link. Use BlueZ against the device already bonded/connected as HID keyboard.
