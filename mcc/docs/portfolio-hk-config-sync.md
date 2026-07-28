# Portfolio project: HK Config Sync

**Status:** MVP shipped in MCC — disk profiles + Git config sync panel (`hk-config` clone, `gh` auth, create private repo, pull/apply, push).  
**Parent product:** HK Operator MCC + Cyberpad (hybrid BLE HID / GATT).  
**Thesis:** configuration is data, not code — so the pad follows you across machines.

---

## Why this exists

Daily work moves between many environments:

- Arch (dev PC)
- Ubuntu servers (homelab / VPS)
- Raspberry Pi
- ESP32 development
- Different AI tools and project trees

If the macro pad depended on machine-specific code or hardcoded paths, every new workstation would be a special case. The current architecture already pushes intelligence into **configuration** and keeps the hardware close to **stateless** (HID chords or macro events; bindings live on the host).

That yields a portable workflow:

1. Pair the macro pad  
2. Launch MCC  
3. Import a profile (or auto-load)  
4. Same buttons, same behavior — regardless of machine  

Adding another machine should mean **clone config**, not **fork firmware**.

---

## Design principle

| Layer | Owns | Does not own |
| --- | --- | --- |
| **Firmware** | BLE, HID, profile/preset switching, button events, OTA | Personal macros, AI prompts, paths |
| **MCC** | UI, device management, config editor, execute, sync UX | “Your” workflow content as compiled code |
| **Config repo** | Profiles, actions, bindings, settings | Hardware form factor |

HK V2 / V3 / a different pad can all consume the **same** config schema.

Evolution path (intentional):

> weekend hot-glue pad → config-driven input platform that travels with you

---

## Proposed layout

Local runtime (example):

```text
~/.config/hk/
├── profiles/
│   ├── dev.json
│   ├── terminal.json
│   └── ai.json
├── settings.json
└── macros.json          # or folded into profiles — TBD
```

Versioned source of truth (Git):

```text
hk-config/
├── profiles/
├── settings.json
└── README.md
```

**New machine:**

```bash
git clone git@github.com:<you>/hk-config.git
hk-mcc --import ./hk-config
# Done.
```

MCC today already persists `~/.config/hk-operator/store.json`. This project generalizes that into **named profiles + sync**, without baking paths into firmware.

---

## Sync UX (future)

Version awareness in MCC, not only “git pull in a terminal”:

```text
⚠ Config update available
Current: a84c12d
Latest:  91fd3e7

[Update]
```

Diff summary:

```text
Changes:
+ Added AI preset button 6
~ Changed Terminal button 2
- Removed old Git macro
```

Implementation sketch (tomorrow+):

- Treat `hk-config` as a Git checkout (or submodule / sibling repo).  
- MCC reads HEAD, compares to last-applied commit stored in `settings.json`.  
- Update = `git pull` (or fetch + merge) → validate schema → import → optional “write slots to pad”.  
- Never auto-push secrets; export allow-list for command macros stays explicit.

---

## Separation of concerns (target)

```text
Firmware          MCC                 Config repo
────────          ───                 ───────────
BLE / HID         UI                  Personal workflow
Presets           Device mgmt         AI commands
Button events     Config editor       Terminal macros
OTA               Sync                Dev shortcuts
```

Hardware change ≠ workflow rewrite.

---

## Non-goals (for the first build)

- Building this feature in the current session  
- Cloud proprietary sync (Git-first is enough)  
- Auto-executing unreviewed remote command macros  
- Firmware storing full action payloads  

---

## Acceptance sketch (when we build)

1. Export current MCC store → `hk-config` shaped tree.  
2. Import on a clean `~/.config/hk/` (or MCC path) restores bindings + actions.  
3. Documented clone → import path for a second Linux box.  
4. Optional: show local vs remote commit + apply update.  
5. Portfolio write-up: problem → architecture diagram → demo GIF of import on second machine.

---

## Relation to this repo

- **Now:** Cyberpad + MCC prove hybrid HID/macro and host-side execution.  
- **Next portfolio slice:** portable, versioned **hk-config** + MCC import/sync.  
- Keep firmware dumb; keep workflow in Git.

When implementation starts, open a dedicated branch / checklist from this doc — do not expand firmware surface for sync.
