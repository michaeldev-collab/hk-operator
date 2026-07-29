# Verification harness (Phase 1 stubs)

Hardware-independent scaffolding for proving the HK Operator event path where
practical:

```text
physical press → firmware → BLE/GATT → BlueZ → Rust → binding → dispatch → host effect
```

Full regression depth is Phase 2. This document and the stub runners define
**what** will be verified and **how evidence is classified** — without claiming
unrun paths are green.

## Layers

| Layer | Runner | Hardware | Status |
| --- | --- | --- | --- |
| JS logic smoke | `npm run test:js` | No | Existing |
| BLE slot codec unit | `npm run test:rust` | No | Existing |
| Harness inventory stub | `npm run test:harness` | No | **Stub (this phase)** |
| Protocol/dispatch/composer regressions | Phase 2 commits | No | Not started |
| HITL probe checklist | Manual / optional | Yes | Template only |
| Public evidence capture | Phase 6 | Capture only | Not started |

## Capability matrix (stubs)

| ID | Capability | Automated now | Harness stub | HITL later | Public evidence |
| --- | --- | --- | --- | --- | --- |
| V-DISCOVER | Device discovery by compat name | — | listed | Y | redacted |
| V-BOND | Reuse existing HID bond (no 2nd link) | — | listed | Y | note |
| V-INFO | Read firmware info characteristic | — | listed | Y | redacted string only |
| V-SLOTS-R | Read 486-byte slots | pack unit only | listed | Y | — |
| V-SLOTS-W | Write slots + round-trip | pack unit only | listed | Y | — |
| V-MACRO | MacroEvent notify receipt | — | listed | Y | GIF later |
| V-IDX | Preset/action index bounds | — | listed | Y | — |
| V-HID-FALLBACK | HID works with MCC closed | — | listed | Y | note |
| V-GATT-DOWN | MCC degrades when GATT missing | — | listed | Y | note |
| V-PROFILE-IO | Profile import/export shape | smoke partial | listed | — | example JSON |
| V-GIT-SYNC | Pull / pull-apply messaging | — | listed | manual | sanitized |
| V-COMPOSER | Rotate / timeout / stack | smoke partial | listed | — | GIF later |
| V-DISPATCH | Action types + allowlist | smoke partial | listed | — | screenshot later |
| V-FAIL | Failure reporting | — | listed | Y | — |

## Redaction rules (publishable evidence)

Never commit or publish:

- Bluetooth device addresses (MAC)
- Private absolute paths (`/home/…`, host-specific mounts)
- Private slash routers / personal prompts
- Client URLs, credentials, private git remotes
- Live profile JSON from a daily-driver machine

Prefer: generic fixtures under `config/examples/`, redacted transcripts,
screenshots with private panels cropped.

## Local HITL (optional)

Use [`../mcc/docs/hw-gate.md`](../mcc/docs/hw-gate.md) as the compile/probe gate.
Record MAC and host paths **only in private notes**, not in this repository.

## Running stubs

```bash
cd mcc
npm run test:js
npm run test:rust
npm run test:harness
```

`test:harness` must exit 0 while stubs are incomplete — it inventories
capabilities and fails only if the stub file itself is broken.

## Non-goals (Phase 1 stubs)

- No firmware flash
- No UUID / BLE name / `FW_INFO` changes
- No full Phase 2 regression suites
- No CI workflow yet (Phase 5)
