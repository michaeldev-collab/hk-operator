# HK Operator — portfolio repo prep (DO NOT START YET)

**Status:** queued for tomorrow — design / checklist only.  
**Do not** create `hk-operator/`, copy trees, sanitize, CI, or push until this session is explicitly started.

**Sources to leave untouched until then:**
- MCC: `/home/stitch/3dl-macro-command-center` (recovery copy — never delete)
- Firmware: under `/run/media/stitch/data3/Operating/pi-iot/esp32/ble-hid-hotkeys/` (hybrid GATT sketch)

Related design docs (also not implement until scheduled):
- [PORTFOLIO.md](./PORTFOLIO.md)
- [portfolio-hk-config-sync.md](./portfolio-hk-config-sync.md)
- [portfolio-slash-command-composer.md](./portfolio-slash-command-composer.md)

---

## Project name
`hk-operator` — public GitHub portfolio repository.

## Origin story (for README later)
One-day ESP32 BLE macro pad (spare switches, perfboard, point-to-point, solid copper common ground, hot glue) → ~4 months daily use, muscle memory, no meaningful firmware bugs → Rust **HK Mission Control Center (MCC)** → configuration-driven input platform (not a hardcoded pad).

## Architecture to preserve
1. **Firmware** — buttons, BLE, hardware state, device events  
2. **MCC** — profiles, config editor, dispatch, paste, shortcuts, launch, import/export  
3. **OS adapter** — ydotool, clipboard, process launch, platform specifics  
4. **Configuration** — profiles, mappings, commands, paths, timing, cycling  

## Target tree
```text
hk-operator/
├── firmware/
├── mcc/
├── config/examples/ + schema/
├── docs/
├── hardware/v1/ + v2/
├── media/
├── scripts/
├── .github/workflows/
├── .gitignore, LICENSE, CHANGELOG.md, CONTRIBUTING.md, README.md
```

## Phases (execute in order tomorrow)

| Phase | Work | Gate |
| --- | --- | --- |
| **1 Audit** | Build systems, deps, config paths, hardcoded paths, personal commands, secrets, generated/temp, dead code. **No logic changes.** Produce audit summary first. |
| **2 Safe create** | New `hk-operator` structure; copy firmware → `firmware/`, MCC → `mcc/`; preserve history if practical; **do not delete original**. |
| **3 Privacy** | Strip slash commands, private paths, usernames, secrets; examples for Dev/Terminal/AI; schema; runtime config → `~/.config/hk-operator/`; block private config from git. |
| **4 Build verify** | Firmware build unchanged pins/BLE; MCC `cargo check` / test / release; clear errors on missing deps. |
| **5 Docs** | README + architecture, hardware-v1, configuration, protocol, roadmap. |
| **6 GitHub quality** | Actions (fmt, check, test, firmware), issue/PR templates, CONTRIBUTING, CHANGELOG; **license tradeoffs presented before choosing**. |

## Preferred commits
1. `chore: initialize HK Operator repository`  
2. `feat(firmware): add validated BLE hotkey firmware`  
3. `feat(mcc): add Rust mission control center`  
4. `refactor(config): separate runtime configuration from source`  
5. `docs: document architecture and V1 hardware`  
6. `ci: add firmware and Rust verification workflows`  

## Constraints (non-negotiable)
- No unnecessary firmware rewrite / MCC redesign / unrelated features  
- No private 3DL commands, credentials, or internal paths in the public tree  
- Do not break the working device  
- Original project = recovery source until new repo verified  
- Small reviewable changes; stop before destructive uncertainty  
- License: present MIT vs Apache-2.0 vs GPL tradeoffs — **do not pick without user OK**

## Final verification checklist (tomorrow EOD)
- [ ] Final tree shown  
- [ ] Removed/replaced private values listed  
- [ ] Firmware builds  
- [ ] MCC checks/builds  
- [ ] No secrets tracked  
- [ ] Example configs work  
- [ ] Original project unchanged  
- [ ] Exact git init/push commands provided  

## Tomorrow kickoff prompt (paste)

```text
Execute docs/hk-operator-prep.md for PROJECT HK Operator.
Start at PHASE 1 — AUDIT only; show audit summary; then proceed through phases 2–6
with constraints as written. Original 3dl-macro-command-center and firmware paths
stay untouched as recovery. Present license tradeoffs before writing LICENSE.
```
