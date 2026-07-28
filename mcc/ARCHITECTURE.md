# MCC architecture (pointer)

Canonical platform architecture lives at
[`../docs/architecture.md`](../docs/architecture.md).

**Cyberpad** is the physical ESP32-C6 controller. This MCC tree is the Rust/Tauri
desktop companion: hybrid HID/macro slots over BLE GATT, local action catalog,
profiles, and git config sync.

BLE advertise name / firmware info strings remain compatibility identifiers
(`Cyberdeck Pad`, `Cyberdeck Pad Hybrid v0.2.x`) — see
[`protocol/PROTOCOL.md`](./protocol/PROTOCOL.md).
