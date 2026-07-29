# HK Operator — Portfolio Engineering Plan

> Status: **Phase 0–3 complete · Phase 4 started** (P3-01/P3-02 remediated).  
> Repo: public `michaeldev-collab/hk-operator`  
> Date: 2026-07-29  
> Constraint: do not rewrite Git history; each future commit must be a real engineering pass.

## A. Executive summary

HK Operator already proves its portfolio thesis: a one-day **Cyberpad** ESP32-C6
prototype (perfboard, switches, copper/solder ground bus, hot glue) was used
daily for roughly four months, became muscle memory, then grew a host platform
around it — hybrid BLE HID + custom GATT on one BlueZ bond, a Rust/Tauri Mission
Control Center (MCC), and configuration-driven portable profiles.

What it does **not** yet prove for a complete case study: a verified baseline
harness, deep regression coverage, a sanitized threat model with a separate
remediation pass, continuous integration, scrubbed public hygiene (in progress
in Phase 0), packaged releases, and end-to-end demo evidence that stays free of
private workflow content.

**Differentiation from Family Hub:** Family Hub demonstrates product
architecture, web/API/database/embedded integration, security review, CI, and
CAD/physical deployment. HK Operator must demonstrate observing a real personal
workflow, validating a custom physical interface, BLE HID/GATT and BlueZ
engineering, native Linux/Tauri development, config-driven architecture,
portable Git-backed profiles, constrained local execution, AI command
composition, long-term hardware validation, then verify → review → remediate →
automate → demonstrate → package.

**IoT / edge rule:** Cyberpad is an edge input/display node. Bindings,
composers, allowlists, and profiles live on the host. Rich execution must not
migrate into firmware.

## B. Current-state inventory (as of Phase 0)

### Architecture

```text
Cyberpad firmware  -- BLE HID + GATT -->  BlueZ
                                            |
                                      cyberdeck-ble
                                            |
                         MCC (Tauri) -- store / profiles / hk-config git
                                            |
                              localhost fire API :17321
```

### Compatibility identifiers (do not change without bonded-host evaluation)

| Item | Value |
| --- | --- |
| BLE advertised name | `Cyberdeck Pad` |
| Firmware info string | `Cyberdeck Pad Hybrid v0.2.0` |
| Presets × actions | 6 × 3 = 18 slots |
| Slot record | 27 bytes; total payload 486 bytes |
| GATT UUIDs | `c0de0001`–`c0de0004-3d17-4a00-8000-00805f9b34fb` |

### Components

| Component | State |
| --- | --- |
| Hybrid firmware source | Implemented; flash not claimed verified in-tree without HITL |
| `cyberdeck-ble` + probe CLI | Implemented |
| MCC Tauri desktop | Implemented |
| JS frontend | Implemented |
| Composer | Implemented (Enter-key global reset still open) |
| Profile import/export | Implemented |
| Git config sync | Implemented |
| Public examples | Sanitized `config/examples/` |
| Hardware photos | `media/hardware/cyberpad-v1/` |
| Automated tests | JS smoke (~36) + BLE pack unit tests (2) |
| CI | Missing (empty `.github/` at Phase 0 start) |
| Packaging | Partial (`deb` target; Cargo 0.1 / app 0.2 version drift) |

### Security controls present (not a full review)

- Explicit per-action shell allowlist before `command` execution
- URL scheme checks for `http(s)` in validation paths
- Profiles/live config intended outside the public git tree
- Public seeds avoid private Matrix slash routers

### Known limitations

- `TEST_PLAN.md` historically claimed browser-only “never executes” — Tauri does execute allowed commands
- HITL log may lag firmware info string (v0.1 vs v0.2)
- Pull-and-apply must surface pull failures clearly (improved in daily driver; keep tested)
- No AppImage; no GitHub Actions yet
- OTA and Cyberpad V2 PCB are future work

## C. Gap matrix

| ID | Area | Current | Target | Sev | Portfolio | Eng | HW | Phase |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| G01 | CI | Empty Actions | fmt/check/test/clippy/JS/firmware compile/hygiene | critical | high | high | no | 5 |
| G02 | Tests | Thin | Protocol/dispatch/composer/config harness | high | high | high | no* | 1–2 |
| G03 | Threat model | None | Sanitized review artifact | high | high | high | no | 3 |
| G04 | Remediation | Ad hoc | Finding→fix→test→docs | high | high | high | no | 4 |
| G05 | Public safety | MAC/paths/stale docs | Clean public tree | critical | high | med | no | **0** |
| G06 | Doc drift | Stale claims | Aligned docs | medium | med | low | no | 0–6 |
| G07 | Demo evidence | Photos only | Sanitized E2E sequence | high | high | med | capture | 6 |
| G08 | Packaging | deb; ver drift | Release + rollback | medium | med | med | no | 7 |
| G09 | HITL | Stale gate notes | Redacted checklist + results | high | high | high | yes | 1 |
| G10 | Enter reset | Roadmap | Optional UX | low | low | low | no | later |
| G11 | V2 hardware | Notes | Separate V2 plan; V1 preserved | low | med | med | design | 8 |
| G12 | Flash proof | Compile-focused | Probe/HITL; never CI flash | medium | high | high | optional | 1 |

\* Hardware-in-the-loop optional for some paths; maximize hardware-independent coverage.

## D. Phase plan

### Phase 0 — Baseline and repository hygiene (this document)

- **Objective:** Trustworthy inventory + remove public leaks before more evidence.
- **Non-goals:** Feature work, flash, BLE rename, CI workflows yet.
- **Deliverables:** This plan; scrubbed hygiene (separate commit).
- **Acceptance:** Plan committed; public tree free of device MAC and private home paths; smoke + BLE unit tests still green.
- **Must not change:** UUIDs, BLE advertise name, `FW_INFO`, HID/macro semantics.

### Phase 1 — End-to-end verification harness

Prove practical paths: press → FW → BLE/GATT → BlueZ → Rust → binding → dispatch → visible host effect. Separate automated HW-independent tests, HITL, manual acceptance, and publishable redacted evidence.

**Stubs landed:** [`docs/verification/harness.md`](./verification/harness.md), `mcc/test/verification/harness.mjs`, `cyberdeck-ble` verification stub tests. Run `cd mcc && npm run test:harness`.

### Phase 2 — Protocol and dispatcher regression tests

Deterministic coverage for codec/malformed payloads, profile shape, URL schemes, allowlist, composer rotate/timeout/stack, unknown action types. Split commits by subsystem.

**Landed:** `slots_codec` + `mcc-desktop` `dispatch`/`composer` pure modules + JS `dispatch`/`composer`/`config_profile` runners. Run `cd mcc && npm test`.

### Phase 3 — Threat model and sanitized security review

Docs-only commit: assets, actors, trust boundaries, attack scenarios, STRIDE-equivalent, controls, findings, residual risks. Do not claim controls that are not in code.

**Landed:** [`docs/security-threat-model.md`](./security-threat-model.md) — findings P3-01…P3-09 are Phase 4 backlog (no remediations in Phase 3).

### Phase 4 — Security remediation

Separate commit(s) from Phase 3. Each finding: reference, code change, regression test, docs, residual risk. Anticipate (only if confirmed): structured process args, stronger profile validation, atomic config + last-known-good, git remote validation, redacted logs, protocol pre-checks.

**Started:** P3-01…P3-08 remediated (fire token, profile allowlist scrub, value-fingerprint shell allowlist, ydotool `0600`, GitHub-only git remotes, import path confinement, save_store allowlist gate, BLE MAC redaction). Remaining: P3-09.

### Phase 5 — Continuous integration

GitHub Actions: Rust fmt/check/test/clippy, JS smoke, firmware **compile** (never flash), public hygiene scans, example JSON + doc link checks. Pin/document board core and libraries.

### Phase 6 — Portfolio evidence and demonstration

Sanitized screenshots/GIF/short video: Cyberpad press → MCC event → binding → composer/action → desktop result. No private prompts, client URLs, home paths, MACs, or personal remotes.

### Phase 7 — Reproducible packaging and release

Align versions; firmware artifact + checksums; MCC Linux package; toolchain pins; compatibility matrix; upgrade/rollback; config backup/migrate; probe-only diagnostics; release notes.

### Phase 8 — Future Cyberpad hardware revision

Custom PCB / mount / enclosure / strain relief / serviceability while **preserving muscle-memory layout**. V1 remains the validated daily-driver chapter — not “defective because crude.”

## E. Proposed commit roadmap

1. `docs: baseline portfolio engineering inventory and verification map` — Phase 0 (this file)
2. `chore(public): scrub device MAC, private paths, and doc hygiene` — Phase 0
3. `test: add hardware-independent end-to-end verification harness stubs` — Phase 1
4. `test(protocol): add slot codec and malformed payload regression coverage` — Phase 2
5. `test(dispatch): cover URL, shell allowlist, and unknown action failures` — Phase 2
6. `test(composer): cover rotation, timeout lock, and stacking edge cases` — Phase 2
7. `test(config): validate profiles, import failure, and example hygiene` — Phase 2
8. `docs(security): add sanitized threat model and review findings` — Phase 3
9. `fix(security): remediate review findings with regression coverage` — Phase 4
10. `ci: verify Rust, JS smoke, firmware compile, and public-repo hygiene` — Phase 5
11. `docs(portfolio): add sanitized Cyberpad-to-MCC workflow evidence` — Phase 6
12. `release: package MCC and firmware with compatibility and rollback guide` — Phase 7
13. `docs(hardware): specify Cyberpad V2 without retiring V1 validation` — Phase 8

Do **not** combine security review and remediation into one commit.

## F. Verification matrix (target)

| Capability | Unit | Integration | HITL | Manual | Public evidence | CI |
| --- | --- | --- | --- | --- | --- | --- |
| Slot codec | Y | — | — | — | snippet | Y |
| Malformed GATT | Y | — | opt | — | — | Y |
| Discovery / bond | — | mock | Y | Y | redacted | — |
| Slot R/W round-trip | Y | mock | Y | Y | redacted | partial |
| Macro notify | — | mock | Y | Y | GIF/video | — |
| HID fallback | — | — | Y | Y | note | — |
| Profile I/O | Y | Y | — | Y | example | Y |
| Git pull/apply | — | Y | — | Y | sanitized | partial |
| Composer | Y | — | — | Y | GIF | Y |
| Dispatch / allowlist | Y | Y | — | Y | screenshot | Y |
| Firmware compile | — | — | — | Y | log | Y |
| Flash | — | — | CEO gate | gated | never CI | N |

## G. Risk register

| Risk | Mitigation |
| --- | --- |
| Daily-driver breakage | No BLE ID changes without evaluation; preserve HID fallback |
| Firmware compatibility | Compat table; probe before any flash; no CI flash |
| BLE pairing disruption | Reuse bonded HID link; no second BLE connection |
| Config loss | Last-known-good backup before apply (Phase 4) |
| Shell execution | Keep explicit allowlist; prefer structured argv after review |
| Git-sync corruption | Surface pull failures; validate profile before apply |
| Private-data leakage | Phase 0 scrub; CI path/MAC scans |
| Packaging drift | Pin versions; release checklist |
| Hardware availability | Maximize hardware-independent tests |
| Documentation drift | This inventory + CI link checks |

## H. First execution after plan approval

**Phase 0** is the first execution phase: publish this inventory, then scrub public hygiene in a second commit. Do not proceed to Phase 1 until smoke + BLE unit tests remain green and public `rg` no longer finds device MACs or private absolute home paths in tracked docs.

## Critical constraints (standing)

- Do not rewrite, squash, reorder, or manufacture Git history.
- Do not expose private configuration, internal prompts, credentials, device addresses, or private remotes.
- Keep live user profiles outside this repository.
- Do not flash firmware without explicit approval; do not assume the connected serial device is Cyberpad.
- Do not remove compatibility identifiers without bonded-host / protocol evaluation.
- Preserve HID fallback when MCC is unavailable.
- Keep rich actions on the host; Cyberpad stays an edge node.
- V1 hardware is validation evidence, not a defect narrative.

## Related docs

- [Architecture](./architecture.md)
- [Cyberpad V1 hardware](./hardware-v1.md)
- [Roadmap](./ROADMAP.md)
- [BLE protocol](../mcc/protocol/PROTOCOL.md)
- [MCC test plan](../mcc/TEST_PLAN.md)
