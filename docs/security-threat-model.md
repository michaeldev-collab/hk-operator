# HK Operator — sanitized threat model & security review

> **Phase 3** review · **Phase 4** remediations started 2026-07-29  
> Repo: public `michaeldev-collab/hk-operator`  
> Method: grey-box code review of MCC Tauri host, `cyberdeck-ble`, frontend
> glue, and git sync.  
> Confidence: high on cited paths; no runtime penetration test performed.

Do **not** claim controls that are not in code. Findings table marks remediated
items; remaining backlog is still Phase 4.

## 1. Scope

| In scope | Out of scope |
| --- | --- |
| MCC desktop (`mcc/src-tauri`, `mcc/src`) | Production cloud deploy |
| Localhost fire API | Client-facing SaaS |
| Profile import / git sync | Secrets rotation ops |
| Dispatch / composer / BLE host codec | Firmware flash / UUID changes |
| Public portfolio hygiene expectations | Daily-driver private profiles |

Threat framing is STRIDE-equivalent (spoofing, tampering, repudiation,
information disclosure, denial of service, elevation of privilege) mapped to
local desktop + BLE operator tooling.

## 2. Assets

| Asset | Why it matters |
| --- | --- |
| Live `Store` (`~/.config/hk-operator/store.json`) | Actions, bindings, composers |
| `allowed_commands` set | Capability to run host shell |
| Portable profiles (+ git `hk-config`) | Can replace live store including allowlist |
| Git remote + `gh` session | Operator GitHub identity |
| Focus / clipboard / `ydotool` injection | Types into whatever window is focused |
| BLE bond + pad as trusted input | Physical or bonded device can fire macros |
| Fire API (`127.0.0.1:17321`) | Same dispatcher as pad presses |

## 3. Actors

| Actor | Reach |
| --- | --- |
| Local operator | Full UI + Tauri IPC (intentional) |
| Other local processes / malware | Loopback fire API (needs token); readable config; ydotool only if same-user socket access |
| Physical / BLE proximity | Bonded Cyberpad HID + GATT; HID works with MCC closed |
| Malicious profile JSON | Import or pull-apply into live store |
| Malicious git remote | After `set_remote` + pull, attacker-controlled profile content |

## 4. Trust boundaries

```text
Cyberpad FW ──BLE HID+GATT──► BlueZ ──► MCC (Tauri)
                                      │
                                      ├─► Store / profiles / optional git
                                      ├─► localhost :17321 fire API
                                      └─► open / bash / clipboard / ydotool
```

| Boundary | Crossing | Evidence |
| --- | --- | --- |
| Firmware ↔ BLE | HID + GATT slots / MacroEvent / Info | `cyberdeck_ble` UUIDs; `firmware/` |
| BlueZ ↔ MCC | Discover, subscribe, fire binding | `CyberdeckPad`, `start_macro_listen_inner` |
| Webview ↔ Rust | Tauri `invoke` commands | `main.rs` `invoke_handler` |
| Local processes ↔ MCC | HTTP on loopback | `spawn_localhost_fire_api` |
| Disk / git ↔ Store | Profile JSON apply | `apply_profile_file`, `git_sync` |
| MCC ↔ host OS | Shell, open, paste | `execute_action`, `paste_text` |

## 5. Controls that exist in code today

| Control | Where |
| --- | --- |
| Fire API binds **localhost only** | `TcpListener::bind("127.0.0.1:17321")` in `spawn_localhost_fire_api` |
| Fire `/fire/*` requires shared token; GET fire rejected; `/` health open | `fire_api` + `X-HK-Fire-Token` / Bearer; token file `fire_token` mode 0600 |
| Profile / git apply **does not** install `allowedCommands` | `profile_apply::merge_allowlist_after_profile` via `apply_profile_file` |
| URL actions require `http://` or `https://` prefix | `dispatch::url_gate` → `execute_action` `"url"` |
| UI URL validation (case-insensitive) | `lib.js` `validateAction`; `app.js` `openUrl` |
| Shell requires allowlisted **action id + value fingerprint** | `dispatch::command_gate` + `command_value_fingerprint` (P3-03) |
| UI confirm before first allow | `app.js` `runAction` → `allow_command` |
| Unknown action types rejected | `execute_action` → `unknown_type_err` |
| Profile name path sanitization | `git_sync::sanitize_profile_name` (`/`, `\`, NUL → `_`) |
| Composer id / non-empty commands | `composer::composer_precheck` |
| ydotool socket owner-only when MCC starts daemon | `ydotoold -P 0600` via `ydotool_sock` (P3-04) |
| Slot blob length checks | `PadSlots::pack` / `unpack`; `pad_write_slots` expects 18 |
| MacroEvent needs ≥2 bytes + in-range indices | `MacroEvent::from_bytes` (`preset < 6`, `action < 3`) |
| Git pull ff-only | `git_sync::pull` |
| Git remote scheme/host allowlist | `validate_git_remote_url` — github.com HTTPS/SSH only (P3-05) |
| `gh` login gate for repo create | `create_github_repo` |
| Declared Tauri capabilities | `capabilities/default.json` (`core:default`, `shell:allow-open`) |

## 6. Explicit non-claims (not in code)

These are **absent** — do not document them as shipped controls:

- No TLS on the fire API (plain HTTP loopback)
- No sandbox (seccomp / Landlock / bubblewrap) for `bash -lc`
- No remote URL scheme/host allowlist beyond GitHub — other forges intentionally rejected
- No signature / integrity check on profile JSON or git payloads
- No Rust-side semantic validation of imported actions (serde shape only)
- No encryption of store/profiles at rest
- No rate limit / replay protection on fire or MacroEvent
- No application-layer BLE auth beyond BlueZ pairing
- Fire token may still be readable by the same local user via `fire_token` file or regenerated `.desktop` Exec lines (not a cross-user secret store)

## 7. Attack scenarios (status from code)

| Scenario | Status | Basis |
| --- | --- | --- |
| Remote internet client hits fire API | **Mitigated** | Bind `127.0.0.1` only |
| Local process fires bindings via loopback without token | **Mitigated** | Token required on `POST /fire/*`; GET fire → 405 |
| Local process with token file / desktop Exec access | **Partial** | Same-user readable secret; not cross-UID by default |
| `ftp:` / `javascript:` URL via Rust dispatch | **Mitigated** | `url_gate` ASCII-case-insensitive `http(s)://` prefix (trim) |
| Out-of-range MacroEvent indices | **Mitigated** | `MacroEvent::from_bytes` drops `preset≥6` / `action≥3` |
| Shell with cold (non-allowlisted) id | **Mitigated** | `command_gate` |
| Shell after allowlist + mutated `value` | **Mitigated** | P3-03 value fingerprint gate; re-approve required |
| Webview `save_store` expands allowlist | **Mitigated** | `retain_allowlist_for_save` ignores incoming allowlist; expand only via `allow_command` |
| Profile / git pull pre-seeds `allowed_commands` | **Mitigated** | Apply ignores profile allowlist; retains live ∩ action ids |
| Profile name `../` traversal | **Mitigated** | `sanitize_profile_name` |
| Arbitrary git remote URL | **Mitigated** | `validate_git_remote_url` allows github.com HTTPS/SSH only |
| BLE MacroEvent without bond | **Partial** | Relies on BlueZ; no app auth on payload |
| HID typing with MCC closed | **By design** | Firmware HID mode |
| Cross-user keystroke via ydotool socket | **Mitigated** | MCC starts `ydotoold -P 0600`; refuses non-owner sockets; default path under `$XDG_RUNTIME_DIR` |
| BLE MAC in UI / probe logs | **Mitigated** | `redact_ble_address` / `redactBleAddress` — last octet only in status UI and probe prints |

## 8. Findings backlog (Phase 4)

| ID | Severity | Evidence | Finding | Residual risk if unfixed / notes |
| --- | --- | --- | --- | --- |
| **P3-01** | High → **Remediated** | `fire_api` + `spawn_localhost_fire_api` | Token required for `POST /fire/*`; GET fire rejected; health `/` open | Residual: same-user token file / `.desktop` Exec readability |
| **P3-02** | High → **Remediated** | `profile_apply` + `apply_profile_file` | Profile/git apply ignores `allowedCommands`; keeps live allowlist ∩ action ids | Residual: operator must re-approve shell after intentional allowlist migration |
| **P3-03** | High → **Remediated** | `dispatch::command_gate` + fingerprint | Allowlist stores id→value fingerprint; edited values need re-approval | Residual: still `bash -lc` without OS sandbox (intentional non-goal this pass) |
| **P3-04** | Medium → **Remediated** | `ydotool_sock` + `ensure_ydotoold` | Socket mode `0600`; recreate if group/other bits set; avoid `/tmp` default | Residual: pre-existing foreign `YDOTOOL_SOCKET` still honored if already owner-only; same-user injection remains by design |
| **P3-05** | Medium → **Remediated** | `git_sync::validate_git_remote_url` | Only `github.com` HTTPS/SSH remotes; reject http/file/other hosts | Residual: GitHub itself can still host malicious profile content after pull (P3-02 still applies) |
| **P3-06** | Medium → **Remediated** | `profile_path` + `import_profile` | Import confined to config `profiles/` and `hk-config/profiles/`; rejects traversal / non-JSON / outside roots | Residual: hostile JSON *inside* allowed dirs still applies actions (P3-02 allowlist scrub still applies) |
| **P3-07** | Medium → **Remediated** | `retain_allowlist_for_save` + `save_store` | Incoming allowlist ignored; live entries retained ∩ action ids; expand only via `allow_command` | Residual: UI state can claim approvals until reload; re-create same id+value reuses prior approval |
| **P3-08** | Low → **Remediated** | `redact_ble_address` + UI/probe | Status line and probe logs show `**:**:**:**:**:XX`; full MAC kept in-process for GATT | Residual: full MAC still in live memory / `--address` CLI arg / BlueZ tools |
| **P3-09** | Low → **Remediated** | `url_gate` + `MacroEvent::from_bytes` | Case-insensitive http(s) aligned with JS; MacroEvent indices bounded to pad grid | Residual: hostile in-range MacroEvent still fires bound actions (by design / BlueZ trust) |

### Severity notes

- **High** findings assume a hostile *local* process or hostile *profile/git* content — the intended threat for a desktop operator tool that executes host actions.
- This is **not** a claim of remote internet RCE. Loopback bind is real mitigation for WAN clients.

## 9. Residual risk summary

HK Operator MCC is a **high-privilege local automation surface** by design: pad presses and the fire API can open URLs, paste text, and (after approval) run shell. Localhost binding, fire-token gate, URL/allowlist gates, and non-import of profile allowlists are real.

Open residual risk centers on same-user fire-token readability, lack of OS sandbox around `bash -lc`, and BlueZ-trust MacroEvent delivery. Phase 4 finding backlog **P3-01…P3-09** is remediated.

**Next gate:** Phase 5 — GitHub Actions (`cargo check` / `cargo test` + firmware compile CI).

## 10. Verification of this document

| Check | Result |
| --- | --- |
| Controls cited exist in tree | Verified against `main.rs`, `dispatch.rs`, `composer.rs`, `git_sync.rs`, `lib.js` |
| Non-claims listed | Explicit §6 |
| Remediations shipped | **P3-01 … P3-09** (Phase 4 complete) |
| Firmware UUID / BLE name / flash | Untouched |
| Runtime exploit exercise | **Not performed** |

## Related

- Portfolio plan: [`portfolio-engineering-plan.md`](./portfolio-engineering-plan.md)
- Verification harness: [`verification/harness.md`](./verification/harness.md)
- Architecture: [`architecture.md`](./architecture.md)
- Dispatch gates (code): `mcc/src-tauri/src/dispatch.rs`
- Composer FSM (code): `mcc/src-tauri/src/composer.rs`
