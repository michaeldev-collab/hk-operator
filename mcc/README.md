# HK Operator — Mission Control Center (MCC)

Pad-first companion for the **Cyberdeck Pad** (ESP32 BLE HID hotkeys) plus a
local action catalog (prompts, URLs, commands, paths).

Hybrid model:
- **HID slots** — pad types keystrokes itself (works with this app closed).
- **Macro slots** — pad notifies this desktop app over custom GATT on the
  **same** BlueZ link as the keyboard; the app runs the real action.

Firmware: [`../firmware/`](../firmware/)  
Protocol: [`protocol/PROTOCOL.md`](protocol/PROTOCOL.md)

## Stack
| Layer | Choice |
|-------|--------|
| Desktop | Tauri 2 + Rust (`bluer` / BlueZ) |
| UI | Existing vanilla HTML/CSS/JS in `src/` |
| Probe CLI | `cyberdeck-probe` (`cargo run -p cyberdeck-probe`) |
| Browser fallback | `python3 -m http.server` — catalog only, no BLE |

## Desktop app
```bash
cd ~/hk-operator/mcc
npm install
npm run dev          # Tauri + UI
```

Persistence: `~/.config/hk-operator/store.json`
(actions + `padBindings` + `composers` + `allowedCommands`).

**Commands never run silently** — click **Allow shell** (or confirm on first Run).

## Probe CLI (no flash)
```bash
cargo run -p cyberdeck-probe -- status
cargo run -p cyberdeck-probe -- info          # needs hybrid firmware
cargo run -p cyberdeck-probe -- read-slots
cargo run -p cyberdeck-probe -- listen
```

Pair **Cyberdeck Pad** as a Bluetooth keyboard first. The probe/app talk GATT on
that existing connection — they do not open a second BLE link.

## Firmware (compile only until you approve flash)
```bash
arduino-cli compile --fqbn esp32:esp32:esp32c6 \
  --libraries ~/Arduino/libraries \
  ../firmware
```

Do **not** `upload` until the correct ESP32-C6 is on the serial port.

WiFi config portal is optional (`-DENABLE_WIFI_FALLBACK=1`). Default hybrid build
is BLE-only.

## Browser-only UI
```bash
cd src && python3 -m http.server 8000
```

## License
MIT — see [`../LICENSE`](../LICENSE).
