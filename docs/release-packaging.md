# Phase 7 — Release packaging plan

> Status: **draft** (docs only). No firmware flash. No UUID / BLE advertise-name changes.
> Last updated: 2026-07-29 · Repo: [hk-operator](https://github.com/michaeldev-collab/hk-operator)

This plan turns “it builds on my machine” into a **reproducible public release** for
HK Operator MCC + Cyberpad hybrid firmware, without claiming CI flash or HITL
bond verification.

## 1. Goals

| Goal | Done when |
| --- | --- |
| Align product versions | One release tag maps cleanly to MCC app + workspace crates + FW_INFO |
| Ship MCC Linux artifact | Tagged GitHub Release includes a built `.deb` (and optionally AppImage) |
| Firmware integrity | Release notes list source path + `sha256` of the `.ino` (and binary if built offline) |
| Compatibility matrix | Operators know which MCC ↔ firmware ↔ protocol combo is supported |
| Rollback | Documented restore for MCC package + config + firmware (flash is operator-local) |
| Probe-only diagnostics | Pre/post upgrade checks without flash in CI |

## 2. Current baseline (Known)

| Surface | Current value | Notes |
| --- | --- | --- |
| MCC UI / npm / Tauri `version` | `0.2.0` | `mcc/package.json`, `tauri.conf.json` |
| Cargo workspace `version` | `0.2.0` | Aligned with npm/Tauri/`FW_INFO` (Phase 7 step 1) |
| Bundle target | `deb` only | `tauri.conf.json` → `bundle.targets` |
| Firmware info string | `Cyberdeck Pad Hybrid v0.2.0` | `FW_INFO` in sketch; do not rename casually |
| BLE advertise name | `Cyberdeck Pad` | Compatibility identifier |
| Protocol | 6×3 slots, 486-byte Slots blob | `mcc/protocol/PROTOCOL.md` |
| Config dir | `~/.config/hk-operator/` | Store, profiles, fire token, settings |
| CI | check/test/JS/firmware **source** gate | No flash; no release artifact job yet |

## 3. Versioning scheme

Propose **semver for the product release**, with locked companions:

```
HK Operator release  vX.Y.Z
├── MCC app / Tauri / npm     X.Y.Z
├── Cargo workspace crates    X.Y.Z   (cyberdeck-ble, mcc-desktop, cyberdeck-probe)
└── Firmware info string      Cyberdeck Pad Hybrid vX.Y.Z
```

Rules:

1. **First public package tag:** prefer `v0.2.0` (MCC app + crates + FW_INFO already on `0.2.0`).
2. Protocol-breaking changes (slot layout, UUID, preset count) → **major** or explicit
   `PROTOCOL.md` bump + matrix row; never silent.
3. BLE name / UUID changes remain a **CEO/protocol gate** — out of scope for routine releases.

### Pre-tag checklist (version)

- [x] `mcc/Cargo.toml` workspace.package.version == Tauri/npm version (`0.2.0`)
- [ ] `FW_INFO` matches release notes firmware line (or matrix documents intentional lag)
- [ ] `mcc/CHANGELOG.md` section for the tag
- [ ] Git tag `vX.Y.Z` on `main` after CI green

## 4. MCC Linux artifact

### 4.1 Primary: Debian package (already configured)

```bash
cd mcc
npm ci
npm run build   # tauri build → .deb
```

Expected output (path may vary by Tauri 2):

```
mcc/src-tauri/target/release/bundle/deb/hk-operator_*.deb
# or productName-derived: HK Operator_*.deb
```

Release asset naming:

```
hk-operator-mcc_X.Y.Z_amd64.deb
```

Install / uninstall (operator machine):

```bash
sudo dpkg -i hk-operator-mcc_X.Y.Z_amd64.deb
# rollback previous:
sudo dpkg -i hk-operator-mcc_PREV_amd64.deb
```

### 4.2 Secondary (optional later): AppImage

Not configured today. Add only if `.deb` proves awkward for portfolio demos.
Keep as follow-on; do not block first release.

### 4.3 CI release job (Phase 7 execute)

**Done:** [`.github/workflows/release.yml`](../.github/workflows/release.yml)

- Trigger: `push` tags `v*` (also `workflow_dispatch` → artifact only, no Release)
- Builds `.deb` on `ubuntu-22.04` with Tauri Linux deps
- Renames to `hk-operator-mcc_<ver>_amd64.deb` + sha256
- Attaches `firmware-checksums.md` (source sha256; **no flash**)
- Uploads to the GitHub Release for the tag

Acceptance: push `vX.Y.Z` → Release page has downloadable `.deb`.

## 5. Firmware artifact + checksum

**CI never flashes.** Packaging = integrity of what operators flash locally.

### 5.1 Source checksum (required every release)

```bash
sha256sum firmware/ble-hid-hotkey-ble-config.ino > dist/firmware-vX.Y.Z.sha256
```

Publish in Release notes:

| File | SHA-256 |
| --- | --- |
| `ble-hid-hotkey-ble-config.ino` | `<hex>` |

### 5.2 Compiled binary checksum (optional, operator-local)

When an operator compiles with Arduino CLI / IDE for `esp32:esp32:esp32c6`:

```bash
# after local compile — path depends on Arduino build cache
sha256sum <firmware.bin> >> private-or-release-notes
```

Public releases **may** attach a prebuilt `.bin` only if:

- Board FQBN pinned in docs
- Library / core versions pinned
- Build reproduced twice with matching hash

Until that is proven, ship **source + sha256** only.

### 5.3 Probe-only preflight (no flash)

```bash
cd mcc
cargo run -p cyberdeck-probe -- status   # address redacted in logs (P3-08)
cargo run -p cyberdeck-probe -- info     # expect Cyberdeck Pad Hybrid vX.Y.Z
```

Record results in private notes; do not commit MACs.

## 6. Compatibility matrix

| MCC release | Firmware `FW_INFO` | Protocol | BLE name | Slots | Notes |
| --- | --- | --- | --- | --- | --- |
| `0.2.x` | `Cyberdeck Pad Hybrid v0.2.0` | PROTOCOL.md (486 B / 6×3) | `Cyberdeck Pad` | 18 | Current public baseline |
| older / unknown | other info string | unknown | any | — | Treat as unsupported; probe `info` before upgrade |

Support policy:

- **Supported:** same major.minor family on MCC + FW_INFO as matrix row.
- **Best-effort:** MCC newer than firmware within `0.2.x` if protocol unchanged.
- **Unsupported:** UUID / slot-size / preset-count mismatch — do not “fix forward” silently.

Upgrade order (safe):

1. Backup config (below)
2. Install MCC package for target version
3. Probe `info` / `status`
4. Flash firmware **only if** matrix says FW bump is required (operator machine; CEO gate if production fleet)
5. Re-probe; re-pair only if bond breaks

## 7. Config backup, migrate, rollback

### 7.1 Backup (before upgrade)

```bash
ts=$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p ~/hk-operator-backup-$ts
cp -a ~/.config/hk-operator ~/hk-operator-backup-$ts/config
# optional: export active profile via MCC UI to profiles/
```

Do **not** commit backups to the public repo.

### 7.2 What survives package rollback

| Data | Location | Survives `.deb` downgrade? |
| --- | --- | --- |
| Store / bindings | `~/.config/hk-operator/store.json` | Yes |
| Profiles | `.../profiles/`, `.../hk-config/` | Yes |
| Fire token | `.../fire_token` | Yes (same-user secret) |
| Settings | `.../settings.json` | Yes |

### 7.3 Rollback steps

1. Install previous `.deb`
2. Restore config tree from backup if the new version migrated schema incompatibly
3. Confirm probe `info` still matches matrix (firmware usually unchanged on MCC-only rollback)
4. If firmware was flashed forward and MCC rolled back: either re-flash previous FW
   (operator-local) or keep MCC at the newer version — matrix decides

### 7.4 Schema migrate (when needed)

Document any `store.json` field changes in CHANGELOG. Prefer backward-compatible
serde defaults (already used for composers / allowlist). Breaking store changes
require a one-shot migrate note in the release.

## 8. Toolchain pins (document in release notes)

| Tool | Pin / record |
| --- | --- |
| Rust | stable (CI uses `dtolnay/rust-toolchain@stable` — record `rustc -V` on build host) |
| Node | 22.x (CI) |
| Tauri CLI | from `mcc/package-lock.json` |
| Linux | amd64 Ubuntu 22.04+ / Debian bookworm-class for `.deb` consumers |
| ESP32 core (local FW build) | Record `esp32` board package version when publishing a `.bin` |

## 9. Release notes template

```markdown
## HK Operator vX.Y.Z

### Artifacts
- hk-operator-mcc_X.Y.Z_amd64.deb
- firmware/ble-hid-hotkey-ble-config.ino (sha256: …)

### Compatibility
- MCC X.Y.Z ↔ Cyberdeck Pad Hybrid vX.Y.Z ↔ PROTOCOL (486 B, 6×3)
- BLE name: Cyberdeck Pad (unchanged)

### Upgrade
1. Backup ~/.config/hk-operator
2. Install .deb
3. Probe status/info
4. Flash firmware only if matrix requires (not via CI)

### Rollback
- Reinstall previous .deb; restore config backup if needed

### Security
- Link docs/security-threat-model.md
- No flash in CI; no UUID/BLE rename in this release (unless explicitly listed)
```

## 10. Phase 7 execution backlog (WIP=1)

| Order | Task | Acceptance |
| --- | --- | --- |
| 1 | Align Cargo workspace version to `0.2.0` | **Done** — workspace `0.2.0` |
| 2 | Add `docs/COMPATIBILITY.md` generated/filled from §6 | **Done** |
| 3 | Script `scripts/release-checksums.sh` for `.ino` sha256 | **Done** |
| 4 | Tag-triggered GH Action: build + upload `.deb` | **Done** — `release.yml` |
| 5 | Cut `v0.2.0` (or next) with notes + checksums | Public release page complete |
| 6 | Operator dry-run: install → probe → rollback `.deb` | Written result in private notes |

## 11. Non-goals

- CI or Actions **flash**
- Changing BLE advertise name, UUIDs, or `FW_INFO` without a dedicated gate
- Publishing private daily-driver profiles or fire tokens
- Claiming HITL bond verification from CI alone
- Phase 8 hardware redesign

## Related

- [Portfolio engineering plan](./portfolio-engineering-plan.md)
- [Protocol](../mcc/protocol/PROTOCOL.md)
- [Hardware gate (compile/probe)](../mcc/docs/hw-gate.md)
- [Threat model](./security-threat-model.md)
- [MCC README](../mcc/README.md)
