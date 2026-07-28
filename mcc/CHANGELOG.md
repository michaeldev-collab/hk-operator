# Changelog

All notable changes to 3DL Macro Command Center.

## 0.2.0 — Hybrid BLE desktop
- Re-anchored on Cyberdeck Pad hybrid firmware (`ble-hid-hotkey-ble-config`).
- Added `cyberdeck-ble` + `cyberdeck-probe` (BlueZ GATT on bonded HID link).
- Added Tauri 2 desktop shell: pad 3×3 grid, HID vs macro editors, sync slots,
  MacroEvent listener, action execution with command allow-list.
- Persistence moved to `~/.config/3dl-macro-command-center/store.json` in desktop
  mode (browser mode still uses localStorage).
- Seeded Cyberdeck launcher macros (sysmon / task-app / vscode scripts).
- Docs rewritten pad-first; WiFi portal is optional firmware fallback.
- **Flash not performed in this release cycle** — compile-only until the correct
  ESP32-C6 is confirmed on serial.

## 0.1.1
- Import confirm + dedupe, load normalization, openUrl scheme guard, a11y, tests 21→31.

## 0.1.0
- Initial static HTML/CSS/JS localStorage dashboard.
