# Architecture

## Goal
Cyberdeck Pad companion: configure hybrid HID/macro slots over BLE GATT, run
rich macros on the Linux host, and keep a searchable action catalog.

## Stack decision
**Chosen: Tauri 2 + Rust (`bluer`) + vanilla `src/` UI.**

Why not browser-only forever:
- BlueZ GATT against an already-bonded HID device requires a native host.
- Macro execution (shell / open path) must not live in a random web page.

Why keep vanilla JS:
- Same UI works as a static catalog when Tauri is not running.
- No frontend bundler for day-to-day edits.

Why not a second BLE connection:
- ESP32 peripheral ≈ one Central. The host OS already owns the HID link.
- Desktop uses BlueZ on that bonded device (`cyberdeck-ble`).

## Layers
```
src/*.js/html/css  ──invoke──▶  src-tauri (Rust)
                                    │
                                    ├── cyberdeck-ble (BlueZ GATT)
                                    ├── ~/.config/.../store.json
                                    └── shell / clipboard / open

Firmware (ESP32-C6)
  HijelHID_BLEKeyboard + custom GATT
    Slots R/W · MacroEvent notify · Info R
```

## Data model
**Action** (catalog): same fields as v0.1 (`id`, `name`, `category`, `type`,
`value`, `tags`, `favorite`, `lastUsed`, `createdAt`).

**Store** (desktop):
```json
{
  "actions": [ /* Action */ ],
  "padBindings": { "2-0": "a_…", "2-1": "a_…", "2-2": "a_…" },
  "allowedCommands": ["a_…"]
}
```

**Device slot** (NVS / GATT, 27 bytes × **18**):
`mode (hid|macro) + mod + key + label[24]` — presets `0..=5`, actions `0..=2`.

Preset LEDs: P1 R · P2 G · P3 B · P4 R+G · P5 G+B · P6 R+B.

Macro bindings live on the **desktop**. Device only stores `mode=macro` (+ label).

## Security
- Action values rendered as text (`textContent` / `<pre>`), never HTML.
- URL open restricted to `http(s):`.
- Command macros require explicit allow-list entry before `bash -lc`.
- GATT writes only after device is found via BlueZ (paired keyboard expected).

## Upgrade path
- Cross-platform BLE backends behind a trait (Windows/macOS later).
- Optional SQLite if the catalog grows large.
- WiFi portal remains compile-flag rescue on firmware.
- **Portfolio (not built yet):** see [docs/PORTFOLIO.md](docs/PORTFOLIO.md)
  — Git-backed profiles ([portfolio-hk-config-sync.md](docs/portfolio-hk-config-sync.md))
  and slash-command composer
  ([portfolio-slash-command-composer.md](docs/portfolio-slash-command-composer.md)).
