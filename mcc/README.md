# HK Operator — Mission Control Center (MCC)

Desktop companion for **Cyberpad** (ESP32-C6) plus a local action catalog
(prompts, URLs, commands, paths).

Validated host transports:
- **Preferred** — Cyberpad C6 → S3 dongle → USB HID/CDC → MCC (BlueZ blocked on the pad).
- **Fallback** — Cyberpad C6 → direct BLE HID/GATT → BlueZ/MCC.

Hybrid behavior:
- **HID slots** — keystrokes reach the host (via S3 USB HID or direct BLE HID); works with MCC closed for typing.
- **Macro / slots control** — MCC runs richer actions; slot sync prefers dongle CDC when linked, else BlueZ GATT.

Firmware: [`../firmware/`](../firmware/)  
Protocol: [`protocol/PROTOCOL.md`](protocol/PROTOCOL.md)  
Architecture: [`../docs/architecture.md`](../docs/architecture.md)  
Hardware: [`../docs/hardware-v1.md`](../docs/hardware-v1.md)

## Stack
| Layer | Choice |
|-------|--------|
| Desktop | Tauri 2 + Rust (`bluer` / BlueZ) |
| UI | Existing vanilla HTML/CSS/JS in `src/` |
| Probe CLI | `cyberdeck-probe` (`cargo run -p cyberdeck-probe`) *(crate name kept for compatibility)* |
| Browser fallback | `python3 -m http.server` — catalog only, no BLE |

## Desktop app
```bash
cd ~/hk-operator/mcc
npm install
npm run dev          # Tauri + UI
```

Persistence: `~/.config/hk-operator/store.json`
(actions + `padBindings` + `composers` + `allowedCommands`).

**Slash composer:** bind type `composer` (value `ai`) to a pad key. **Double-tap**
starts or rotates the live slash; **Space** commits; double-tap again stacks the
next command. New loop = UI **Reset cycle** or a `composer-reset` action (no idle
timeout, Esc left alone for Cursor). The writer captures the focused window at
session start and aborts (no Ctrl+A/Delete) if focus moves away mid-compose.

**Commands never run silently** — click **Allow shell** (or confirm on first Run).

## Probe CLI (no flash)
```bash
cargo run -p cyberdeck-probe -- status
cargo run -p cyberdeck-probe -- info          # needs hybrid firmware
cargo run -p cyberdeck-probe -- read-slots
cargo run -p cyberdeck-probe -- listen
```

**Dongle mode:** keep the pad blocked/disconnected in BlueZ so the S3 bridge owns
the BLE central slot. MCC Refresh should show `via S3 dongle` when linked.

**Direct BLE mode:** pair Cyberpad as a Bluetooth keyboard first. The OS may show
the legacy advertised name **`Cyberdeck Pad`**. Probe/MCC talk GATT on that
existing bond — they do not open a second BLE link.

## Firmware (compile only until you approve flash)
```bash
arduino-cli compile --fqbn esp32:esp32:esp32c6 \
  --libraries ~/Arduino/libraries \
  ../firmware
```

Do **not** `upload` until the correct ESP32-C6 (Cyberpad) is on the serial port.

WiFi config portal is optional (`-DENABLE_WIFI_FALLBACK=1`). Default hybrid build
is BLE-only.

## Browser-only UI
```bash
cd src && python3 -m http.server 8000
```

## License
MIT — see [`../LICENSE`](../LICENSE).
