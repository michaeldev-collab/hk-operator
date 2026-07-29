# Verification harness

Hardware-independent coverage for proving the HK Operator event path where
practical:

```text
physical press → firmware → BLE/GATT → BlueZ → Rust → binding → dispatch → host effect
```

Phase 1 defined the inventory. Phase 2 filled HW-independent regressions for
codec, dispatch gates, and composer FSM. HITL paths remain listed, not green.

## Layers

| Layer | Runner | Hardware | Status |
| --- | --- | --- | --- |
| JS logic smoke + Phase 2 schema | `npm run test:js` | No | Active |
| BLE slot codec + desktop pure modules | `npm run test:rust` | No | Active |
| Harness inventory | `npm run test:harness` | No | Active |
| HITL probe checklist | Manual / optional | Yes | Template only |
| Public evidence capture | Phase 6 | Capture / mockups | [`docs/portfolio-evidence/`](../portfolio-evidence/) shipped (illustrative + hardware photos) |

## Capability matrix

| ID | Capability | Automated now | HITL later | Public evidence |
| --- | --- | --- | --- | --- |
| V-DISCOVER | Device discovery by compat name | — | Y | redacted |
| V-BOND | Reuse existing HID bond (no 2nd link) | — | Y | note |
| V-INFO | Read firmware info characteristic | — | Y | redacted string only |
| V-SLOTS-R | Read / pack 486-byte slots | `slots_codec` | Y write | — |
| V-SLOTS-W | Write slots + round-trip | pack unit | Y | — |
| V-MACRO | MacroEvent bytes | `from_bytes` unit | Y notify | GIF later |
| V-IDX | Preset/action index bounds | `PadSlots::get` | — | — |
| V-HID-FALLBACK | HID works with MCC closed | — | Y | note |
| V-GATT-DOWN | MCC degrades when GATT missing | — | Y | note |
| V-PROFILE-IO | Profile shape / example hygiene | `config_profile.mjs` | import UX | example JSON |
| V-GIT-SYNC | Pull / pull-apply messaging | — | manual | sanitized |
| V-COMPOSER | Rotate / timeout / stack | `composer` module + JS normalize | paste HITL | GIF later |
| V-DISPATCH | URL / allowlist / unknown type | `dispatch` module + JS schema | shell HITL | screenshot later |
| V-FAIL | Failure reporting | gate strings | UI | — |

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

## Running

```bash
cd mcc
npm test
```

## Non-goals

- No firmware flash
- No UUID / BLE name / `FW_INFO` changes
- No claiming HITL paths are verified
- No CI workflow yet (Phase 5)
