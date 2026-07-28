# HK Operator

Configuration-driven BLE macro pad + desktop Mission Control Center (MCC).

## Origin

Built in one day from spare mechanical switches, perfboard, point-to-point wiring,
and hot glue. After ~four months of daily use without meaningful firmware failures,
the pad earned muscle memory — then grew a Rust desktop companion (MCC) so
**behavior lives in configuration**, not hardcoded firmware payloads.

## Architecture

```text
Firmware (ESP32-C6)     MCC (Tauri / Rust / BlueZ)     Config
─────────────────       ──────────────────────────     ──────
BLE HID + GATT          Profiles, bindings, UI         Portable JSON
Presets / LEDs          Dispatch, paste, composers     Examples only in git
Button events           OS adapters (clipboard, …)     Runtime under ~/.config
```

| Layer | Path |
| --- | --- |
| Firmware sketch | [`firmware/`](./firmware/) |
| Desktop MCC | [`mcc/`](./mcc/) |
| Example profiles | [`config/examples/`](./config/examples/) |

Runtime config (not committed): `~/.config/hk-operator/` (or the MCC path used while developing from this tree).

## Quick start (Linux)

### Firmware
See [`firmware/README.md`](./firmware/README.md). Board: ESP32-C6. Flash only the Cyberdeck Pad serial device.

### MCC
```bash
cd mcc
npm install
cargo build -p mcc-desktop --no-default-features
# serve UI + run binary (see mcc/scripts and README)
```

### Profiles
Export/import portable JSON from the MCC UI, or copy [`config/examples/dev.json`](./config/examples/dev.json)
into your profiles directory.

## Slash composer
Bind an action of type `composer` with value `ai`. Each pad press pastes the next
token from the composer list (timeout resets to the first command).

## License
[MIT](./LICENSE). (Apache-2.0 was the patent-grant alternative; GPL-3.0 was rejected for portfolio reuse friction.)

## Status
Portfolio scaffold. Original working trees remain the recovery source until this
repo is verified end-to-end.
