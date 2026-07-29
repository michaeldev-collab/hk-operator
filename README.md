# HK Operator

HK Operator is a configuration-driven input platform composed of:

- **Cyberpad**, a custom ESP32-C6 BLE physical controller
- The **HK Operator Mission Control Center** (MCC), a Rust/Tauri desktop application
- Portable, configuration-defined profiles and actions

HK Operator is the software and input platform. Cyberpad is the original physical
controller built to operate it.

Cyberpad began as a one-day prototype built from perfboard, spare mechanical
switches, point-to-point wiring, a solid copper ground bus, and hot glue. After
approximately four months of daily use without meaningful firmware failures, the
physical interface had become invisible enough to earn muscle memory. HK Operator
grew around that validated device rather than replacing it.

## Terminology

- **HK Operator** — the complete platform
- **Cyberpad** — the physical ESP32-C6 controller
- **MCC** — the Rust/Tauri desktop control and configuration application
  (HK Operator Mission Control Center)

## Architecture

```text
Cyberpad (ESP32-C6)     MCC (Tauri / Rust / BlueZ)     Config
─────────────────       ──────────────────────────     ──────
BLE HID + GATT          Profiles, bindings, UI         Portable JSON
Presets / LEDs          Dispatch, paste, composers     Examples only in git
Button events           OS adapters (clipboard, …)     Runtime under ~/.config
```

| Layer | Path |
| --- | --- |
| Cyberpad firmware | [`firmware/`](./firmware/) |
| Desktop MCC | [`mcc/`](./mcc/) |
| Example profiles | [`config/examples/`](./config/examples/) |

Runtime config (not committed): `~/.config/hk-operator/`.

## Quick start (Linux)

### Cyberpad firmware
See [`firmware/README.md`](./firmware/README.md). Board: ESP32-C6. Flash only the
Cyberpad serial device (do not flash another ESP on the same machine by mistake).

### MCC
```bash
cd mcc
npm install
cargo build -p mcc-desktop --no-default-features
# serve UI + run binary (see mcc/scripts and README)
```

### Profiles
Export/import portable JSON from the MCC UI, or copy
[`config/examples/dev.json`](./config/examples/dev.json) into your profiles directory.

## Slash composer
Bind an action of type `composer` with value `ai`. Rapid-press live-rotates the
slash preview; pause ≥ timeout locks it in; press again to pick the next token to stack.

## Git config sync
From the MCC **Git config sync** panel: init `~/.config/hk-operator/hk-config/`,
set a remote (or create a private repo via `gh`), then **Push current as…** /
**Pull & apply** named profiles.

## Documentation

- [Architecture](./docs/architecture.md)
- [Cyberpad V1 hardware](./docs/hardware-v1.md)
- [Cyberpad BLE protocol](./mcc/protocol/PROTOCOL.md)
- [MCC](./mcc/README.md)
- [Configuration examples](./config/examples/README.md)
- [Portfolio engineering plan](./docs/portfolio-engineering-plan.md)
- [V1 hardware, summarized](./docs/ESP-AND-HOT-GLUE.md)

## License
[MIT](./LICENSE). (Apache-2.0 was the patent-grant alternative; GPL-3.0 was rejected for portfolio reuse friction.)

## Status
Public portfolio cut. A private daily-driver tree remains the recovery / private-config source.
