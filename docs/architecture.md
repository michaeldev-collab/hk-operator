# Architecture

HK Operator is a configuration-driven input platform. Cyberpad is the physical
ESP32-C6 controller. The HK Operator Mission Control Center (MCC) is the desktop
application that resolves Cyberpad events against portable configuration.

## System overview

Four primary pieces:

1. **Cyberpad firmware (ESP32-C6)** — switch matrix, presets, LEDs, BLE HID, custom GATT
2. **Optional S3 dongle** — BLE central to the pad; USB HID keyboard + CDC control to the host
3. **HK Operator MCC** — bindings, action dispatch, UI, sync (dongle CDC preferred; BlueZ fallback)
4. **Portable configuration** — profiles, actions, composers, allowlists

```text
Preferred (validated dongle path)
─────────────────────────────────
┌─────────────┐   BLE    ┌─────────────┐  USB HID/CDC  ┌─────────────────┐
│  Cyberpad   │─────────►│  S3 dongle  │──────────────►│  MCC (desktop)  │
│  C6         │          │  bridge     │               │  Tauri / Rust   │
└─────────────┘          └─────────────┘               └────────┬────────┘
   (BlueZ blocked on pad while dongle owns the link)            │
                                                                ▼
                                                       ~/.config/hk-operator/

Fallback (direct BLE)
─────────────────────
┌─────────────┐   BLE HID + GATT   ┌─────────────────┐
│  Cyberpad   │◄──────────────────►│  MCC / BlueZ    │
│  C6         │  (one host bond)   │  Tauri / Rust   │
└─────────────┘                    └─────────────────┘
```

## Responsibilities

### Cyberpad firmware

- Scan physical switches
- Track hardware preset state
- Drive preset LEDs
- Emit standard BLE HID input
- Emit custom GATT macro events
- Continue supporting basic HID behavior without MCC running

### MCC

- Prefer the S3 dongle USB CDC path for slot read/write when linked; fall back to BlueZ GATT
- Resolve button / fire-API events against configuration
- Dispatch local actions
- Manage profiles and bindings
- Perform clipboard, keyboard, process-launch, URL, path, and composer actions
- Gate shell execution through explicit permission
- Keep ydotoold on an owner-only socket (never world-writable `/tmp`)
- Import, export, and synchronize portable configuration

### Configuration

- Define profiles
- Define button bindings
- Define action catalogs
- Define composers
- Define allowed commands
- Remain outside the public repository when private
- Support moving the same workflow between machines

## Event flow

```text
Physical switch press
        ↓
Cyberpad firmware
        ↓
Is binding HID or Macro?
        ↓
Preferred: HID report → S3 dongle → host USB HID
            Macro / slots → S3 CDC → MCC
        OR (fallback)
HID: BLE keyboard report to BlueZ
Macro: GATT MacroEvent → BlueZ → MCC listener
        ↓
Binding resolver / fire API
        ↓
Action dispatcher
        ↓
Clipboard / keyboard / URL / process / composer adapter
```

## Hybrid HID, GATT, and dongle design

- **USB HID via S3** is the preferred daily path when the validation dongle is linked
- **Direct BLE HID** remains the offline / no-dongle fallback
- **GATT / CDC slots** enable richer desktop-side actions (shell, composers, URLs, paths, slot sync)
- On the direct path, the same bonded BlueZ connection is reused — MCC does not open a second BLE link
- Failure of MCC does not remove all useful device behavior (HID slots still type)

## State and persistence

| Kind of state | Source of truth | Notes |
| --- | --- | --- |
| Preset index / LEDs | Cyberpad firmware (runtime + NVS slots) | Hardware-local |
| Slot HID/macro payloads | Cyberpad NVS (synced from MCC) | Written via GATT Slots characteristic |
| Actions, bindings, composers | MCC `~/.config/hk-operator/store.json` | Host-local live store |
| Named profiles | `~/.config/hk-operator/profiles/` and/or `hk-config` git clone | Portable |
| Public examples | [`config/examples/`](../config/examples/) | Sanitized demos only |
| Private workflow content | User's private config repo / local store | Not in this public tree |

## Trust boundaries

- Physical input originates on Cyberpad and crosses BLE to the host
- MCC treats GATT events as local device input, then resolves against config
- UI form validation covers action shape / URL schemes; **Rust profile apply is serde-only** for actions (see threat model)
- Shell commands require an allowlisted action id **and** matching value fingerprint (`Allow shell` / `allowedCommands` map)
- Profile / git apply does **not** import `allowedCommands` — shell approvals stay machine-local
- Editing a command’s text invalidates prior approval until re-approved
- Localhost fire API requires `X-HK-Fire-Token` (token file under config dir); `GET /` remains a health probe
- Private configuration should stay out of public Git; examples are sanitized
- Full review: [`security-threat-model.md`](./security-threat-model.md)

## Failure modes

| Failure | Expected behavior | Recovery |
| --- | --- | --- |
| Bluetooth disabled | Probe/MCC cannot find Cyberpad | Enable adapter; `bluetoothctl power on` |
| Cyberpad not paired | Status: not found | Pair as a Bluetooth keyboard first |
| MCC closed | HID slots still type; macro slots do nothing useful | Launch MCC |
| GATT unavailable, HID connected | Keyboard works; sync/listen fails | Reconnect / check firmware hybrid build |
| Missing OS dependency (ydotool, etc.) | Paste/undo may degrade to clipboard-only | Install deps; check `YDOTOOL_SOCKET` |
| Invalid config JSON | Import/apply rejected | Fix JSON; restore last known-good profile |
| Unknown action id on binding | Fire reports missing action | Rebind slot in MCC |
| Wrong firmware / protocol version | Slot size / info string mismatch | Flash matching hybrid firmware |
| Wrong serial device while flashing | Brick risk on the *other* ESP | Confirm ACM port before upload |

## Portability

The platform is portable because behavior is **configuration-defined** rather than
compiled into Cyberpad firmware. Clone or sync profiles, pair Cyberpad, launch MCC.

## Extension points

- Additional OS adapters (beyond clipboard / ydotool / open / shell)
- More physical Cyberpad revisions (layout preserved)
- OTA firmware updates
- Additional action types
- Git configuration synchronization (MCC panel)
- Custom PCB and enclosure
- Alternate hardware layouts

## Related docs

- [Sanitized threat model](./security-threat-model.md)
- [Cyberpad V1 hardware](./hardware-v1.md)
- [Cyberpad BLE protocol](../mcc/protocol/PROTOCOL.md)
- [MCC](../mcc/README.md)
- [V1 hardware, summarized](./ESP-AND-HOT-GLUE.md)
