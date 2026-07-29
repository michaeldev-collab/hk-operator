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
| Other local processes / malware | Loopback fire API; readable config; ydotool socket if world-mode |
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
| Shell requires allowlisted **action id** | `dispatch::command_gate` + `Store.allowed_commands` |
| UI confirm before first allow | `app.js` `runAction` → `allow_command` |
| Unknown action types rejected | `execute_action` → `unknown_type_err` |
| Profile name path sanitization | `git_sync::sanitize_profile_name` (`/`, `\`, NUL → `_`) |
| Composer id / non-empty commands | `composer::composer_precheck` |
| Slot blob length checks | `PadSlots::pack` / `unpack`; `pad_write_slots` expects 18 |
| MacroEvent needs ≥2 bytes | `MacroEvent::from_bytes` |
| Git pull ff-only | `git_sync::pull` |
| `gh` login gate for repo create | `create_github_repo` |
| Declared Tauri capabilities | `capabilities/default.json` (`core:default`, `shell:allow-open`) |

## 6. Explicit non-claims (not in code)

These are **absent** — do not document them as shipped controls:

- No TLS on the fire API (plain HTTP loopback)
- No sandbox (seccomp / Landlock / bubblewrap) for `bash -lc`
- No content hash or re-approval when an allowlisted action’s `value` changes
- No remote URL scheme/host allowlist for `git_sync::set_remote`
- No signature / integrity check on profile JSON or git payloads
- No Rust-side semantic validation of imported actions (serde shape only)
- No encryption of store/profiles at rest
- No rate limit / replay protection on fire or MacroEvent
- No application-layer BLE auth beyond BlueZ pairing
- No redaction of BLE MAC in UI / probe output
- Fire token may still be readable by the same local user via `fire_token` file or regenerated `.desktop` Exec lines (not a cross-user secret store)

## 7. Attack scenarios (status from code)

| Scenario | Status | Basis |
| --- | --- | --- |
| Remote internet client hits fire API | **Mitigated** | Bind `127.0.0.1` only |
| Local process fires bindings via loopback without token | **Mitigated** | Token required on `POST /fire/*`; GET fire → 405 |
| Local process with token file / desktop Exec access | **Partial** | Same-user readable secret; not cross-UID by default |
| `ftp:` / `javascript:` URL via Rust dispatch | **Mitigated** | `url_gate` prefix check |
| Shell with cold (non-allowlisted) id | **Mitigated** | `command_gate` |
| Shell after allowlist + mutated `value` | **Partial** | Gate is id-only; `bash -lc` runs current value |
| Profile / git pull pre-seeds `allowed_commands` | **Mitigated** | Apply ignores profile allowlist; retains live ∩ action ids |
| Profile name `../` traversal | **Mitigated** | `sanitize_profile_name` |
| Arbitrary git remote URL | **Unmitigated** | `set_remote` trim + non-empty only |
| BLE MacroEvent without bond | **Partial** | Relies on BlueZ; no app auth on payload |
| HID typing with MCC closed | **By design** | Firmware HID mode |
| Cross-user keystroke via ydotool socket | **Unmitigated** | `ydotoold -P 0666` when started by MCC |
| Import arbitrary filesystem path | **Unmitigated** | `import_profile(path)` reads caller path |

## 8. Findings backlog (Phase 4)

| ID | Severity | Evidence | Finding | Residual risk if unfixed / notes |
| --- | --- | --- | --- | --- |
| **P3-01** | High → **Remediated** | `fire_api` + `spawn_localhost_fire_api` | Token required for `POST /fire/*`; GET fire rejected; health `/` open | Residual: same-user token file / `.desktop` Exec readability |
| **P3-02** | High → **Remediated** | `profile_apply` + `apply_profile_file` | Profile/git apply ignores `allowedCommands`; keeps live allowlist ∩ action ids | Residual: operator must re-approve shell after intentional allowlist migration |
| **P3-03** | High | `execute_action` `"command"` → `bash -lc` | No sandbox; allowlist is action-id only, not value hash | Approved id + edited value = arbitrary shell |
| **P3-04** | Medium | `ensure_ydotoold` `-P 0666` | World-accessible uinput control socket | Other local users/processes can inject keystrokes |
| **P3-05** | Medium | `git_sync::set_remote` | No scheme/host allowlist on remote URL | Operator can be pointed at attacker-controlled remote |
| **P3-06** | Medium | `import_profile` | Arbitrary path read into live store | Confused-deputy overwrite of operator config |
| **P3-07** | Medium | `save_store` accepts full `Store` | Webview/`save_store` can expand allowlist without `allow_command` UX | XSS / compromised webview → shell approvals |
| **P3-08** | Low | `PadStatus.address`; UI status; probe logs | BLE MAC displayed / printed without redaction | Shoulder-surf / log leakage of device address |
| **P3-09** | Low | `url_gate` case-sensitive vs JS regex; MacroEvent no index bounds | Edge inconsistencies | Low direct impact |

### Severity notes

- **High** findings assume a hostile *local* process or hostile *profile/git* content — the intended threat for a desktop operator tool that executes host actions.
- This is **not** a claim of remote internet RCE. Loopback bind is real mitigation for WAN clients.

## 9. Residual risk summary

HK Operator MCC is a **high-privilege local automation surface** by design: pad presses and the fire API can open URLs, paste text, and (after approval) run shell. Localhost binding, fire-token gate, URL/allowlist gates, and non-import of profile allowlists are real.

Open residual risk centers on **P3-03+** (id-only shell approval, ydotool mode, git remote validation, etc.) and same-user readability of the fire token.

**Next gate:** continue Phase 4 with P3-03 (command value binding / sandbox) or P3-04 (ydotool socket mode), separate commits each.

## 10. Verification of this document

| Check | Result |
| --- | --- |
| Controls cited exist in tree | Verified against `main.rs`, `dispatch.rs`, `composer.rs`, `git_sync.rs`, `lib.js` |
| Non-claims listed | Explicit §6 |
| Remediations shipped | **P3-01, P3-02** (this Phase 4 start) |
| Firmware UUID / BLE name / flash | Untouched |
| Runtime exploit exercise | **Not performed** |

## Related

- Portfolio plan: [`portfolio-engineering-plan.md`](./portfolio-engineering-plan.md)
- Verification harness: [`verification/harness.md`](./verification/harness.md)
- Architecture: [`architecture.md`](./architecture.md)
- Dispatch gates (code): `mcc/src-tauri/src/dispatch.rs`
- Composer FSM (code): `mcc/src-tauri/src/composer.rs`
