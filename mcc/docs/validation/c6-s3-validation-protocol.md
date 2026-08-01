# C6 ↔ S3 validation protocol — version 0

**Status:** experimental · protocol version **0** · allowed to be replaced before V2  
**Not** the final production dongle contract.

## Transport

- BLE GATT notify from C6 (peripheral) → S3 (central)
- Service / characteristic UUIDs: see [`protocol/validation_uuids.md`](../../protocol/validation_uuids.md)
- Payload: fixed **12-byte** little-endian packed packet (no strings)

## Packet layout (LE)

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 | `version` — must be `0` |
| 1 | 1 | `msg_type` |
| 2 | 2 | `seq` — `uint16` LE, monotonically increasing on C6 |
| 4 | 1 | `modifiers` — USB HID modifier bitmap |
| 5 | 6 | `keys[6]` — USB HID usage IDs (0 = empty slot) |
| 11 | 1 | `checksum` — XOR of bytes `[0..10]` |

Total size: **12**. Reject any other length.

## Message types

| Value | Name | Notes |
|-------|------|-------|
| `0x01` | `HELLO` | C6 → S3 after subscribe / on demand; keys ignored |
| `0x02` | `KEYBOARD_REPORT` | Full keyboard state (modifiers + up to 6 keys) |
| `0x03` | `RELEASE_ALL` | Equivalent to empty keyboard state; S3 must release USB |
| `0x04` | `HEARTBEAT` | Keepalive; keys ignored |
| `0x05` | `LIGHTS` | Pad indicator toggle; `modifiers` 1=on / 0=off (NeoPixels follow) |

Unknown `version` or `msg_type` → reject (never forward to USB).  
Bad checksum → reject.  
Malformed length → reject.

## Keyboard state model

`KEYBOARD_REPORT` is **complete state**, not edge events.

Examples:

- Press A: `modifiers=0`, `keys=[0x04,0,0,0,0,0]`
- Release: `KEYBOARD_REPORT` with all keys 0, **or** `RELEASE_ALL`

## Safety (S3 receiver)

Send empty USB keyboard report when:

- BLE disconnect
- validation failure that could leave keys held
- session reset / queue clear
- heartbeat timeout
- error state
- `hid release-all` diagnostic command

## Host testability

Encode/decode mirrored in:

- `firmware/common/validation_protocol.h` (device)
- `test/validation_proto.test.mjs` (host)
