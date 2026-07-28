# Architecture

HK Operator is a configuration-driven input platform. Cyberpad is the physical
ESP32-C6 controller. The HK Operator Mission Control Center (MCC) is the desktop
application that resolves Cyberpad events against portable configuration.

## System overview

Three primary layers:

1. **Cyberpad firmware** — switch matrix, presets, LEDs, BLE HID, custom GATT
2. **HK Operator MCC** — BlueZ host, bindings, action dispatch, UI, sync
3. **Portable configuration** — profiles, actions, composers, allowlists

```text
┌─────────────┐     BLE HID + GATT      ┌─────────────────┐
│  Cyberpad   │◄───────────────────────►│  MCC (desktop)  │
│  firmware   │     (one BlueZ bond)    │  Tauri / Rust   │
└─────────────┘                         └────────┬────────┘
                                                 │
                                                 ▼
                                        ~/.config/hk-operator/
                                        (+ optional hk-config git)
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

- Discover and communicate with Cyberpad through BlueZ
- Resolve button events against configuration
- Dispatch local actions
- Manage profiles and bindings
- Perform clipboard, keyboard, process-launch, URL, path, and composer actions
- Gate shell execution through explicit permission
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
HID: send keyboard report directly
        OR
Macro: emit GATT MacroEvent
        ↓
BlueZ connection
        ↓
Rust listener
        ↓
Binding resolver
        ↓
Action dispatcher
        ↓
Clipboard / keyboard / URL / process / composer adapter
```

## Hybrid HID and GATT design

- **HID** preserves independent, low-friction keyboard behavior when MCC is closed
- **GATT** enables richer desktop-side actions (shell, composers, URLs, paths)
- The same bonded BlueZ connection is reused — MCC does not open a second BLE link
- Failure of MCC does not remove all useful device behavior (HID slots still work)

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
- Configuration is validated for shape (types, categories, URL schemes)
- Shell commands require explicit **Allow shell** (allowlist in store)
- Arbitrary shell execution must not occur silently
- Private configuration should stay out of public Git; examples are sanitized

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

- [Cyberpad V1 hardware](./hardware-v1.md)
- [Cyberpad BLE protocol](../mcc/protocol/PROTOCOL.md)
- [MCC](../mcc/README.md)
- [V1 hardware, summarized](./ESP-AND-HOT-GLUE.md)
