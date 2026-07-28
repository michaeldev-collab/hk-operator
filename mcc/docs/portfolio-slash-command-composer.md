# Portfolio project: Slash-command composer (pad button)

**Status:** design only — do not implement yet.  
**Source:** ChatGPT working notes (user) — interaction model for AI / Matrix slash commands.  
**Parent product:** Cyberdeck Pad + MCC (Rust desktop).  
**Thesis:** one physical button becomes a **command composer** (rotating token stack), not one key per slash command.

---

## Reading this as

Press AI button → paste next `/command` in a cycle → type Space (or other separator) between presses → chain a stack → submit. Hardware stays simple; MCC owns the state machine.

---

## Interaction model

```text
Press button → inserts /help
Press space
Press button again → rotates → inserts next command
Keep chaining until the prompt has the stack you want
Submit (Enter)
```

Example sequence:

| Press | Inserted token |
| --- | --- |
| 1 | `/help` |
| (space) | separator |
| 2 | `/review` |
| (space) | separator |
| 3 | `/another-command` |

Unlimited orchestration combinations from **one** switch, without dedicating a pad key per slash command.

---

## Internal state machine (host-side)

Firmware should only emit “AI button fired” (HID chord or MacroEvent). **MCC** holds:

```text
commands[]          # ordered list of slash tokens, configurable
current_index       # 0 .. len-1
timeout_ms          # e.g. 3000–5000
separator           # " " | "\n" | custom
```

On press:

```text
AI button pressed:
    paste/type commands[current_index]   # clipboard + auto-paste, or type
    current_index = (current_index + 1) % command_count
    start/reset idle timeout
```

### Reset cycle when (any)

| Trigger | Why |
| --- | --- |
| **Enter** pressed | prompt submitted — next session starts at index 0 |
| **Preset** changes on pad | context switched — don’t resume mid-list |
| **Timeout** expires (3–5s default) | abandoned mid-compose |
| **MCC** explicitly clears sequence | UI / sync / profile load |

Without predictable reset, the button unexpectedly starts halfway through the list later.

---

## MCC settings to expose (three knobs)

1. **Cycle mode** — each press selects the next command (list editable in UI / profile JSON).  
2. **Reset mode** — Enter, timeout, and/or preset change (checkboxes).  
3. **Separator** — space, newline, or custom text (user types Space today; software can inject separator automatically later).

That software layer is what lets a small junk-drawer pad behave like a larger control surface.

---

## Relation to HID vs macro path

| Approach | Pros | Cons |
| --- | --- | --- |
| **MacroEvent → MCC** | Full state machine, auto-paste, no KDE shortcut hacks | Needs reliable GATT notify / listen |
| **HID chord → host shortcut → fire API** | Works today on this Arch/KDE box | Shortcut/env fragility; harder to watch Enter for reset |

Target architecture: composer lives in **MCC** (Rust), driven by macro (or fire) events. HID-only cycle would require embedding the list on-device — fight the “config is data” rule.

---

## Schema sketch (config / profile — not built)

```json
{
  "composers": {
    "ai": {
      "commands": [
        "/help",
        "/review"
      ],
      "separator": " ",
      "timeoutMs": 4000,
      "resetOn": ["enter", "presetChange", "timeout", "explicitClear"]
    }
  }
}
```

Fits the portable `hk-config` story: same composer list on every workstation after import.

---

## Non-goals (first slice)

- Implementing cycle/reset in firmware  
- Building this in the current session  
- Auto-submitting the chat (Enter can reset index; submit stays human unless opted in)  

---

## Acceptance sketch (when we build)

1. Bind one pad slot to composer `ai`.  
2. Three presses with spaces between produce three distinct tokens in order.  
3. After Enter (or timeout / preset change), next press starts at `commands[0]` again.  
4. List + separator + timeout editable in MCC and round-trip through profile export/import.  

---

## Portfolio narrative (adjacent)

MCC is a **Rust** desktop app (Tauri + BlueZ). Coming from C++, the syntax differs; structure (structs, enums, traits ≈ interfaces, static typing, resource ownership) transfers. Ownership/borrowing is the main mental shift.

The load-bearing skill isn’t loops — it’s architecture:

- How presets are represented  
- How commands are stored  
- How device and app communicate  
- How profile switching works  
- How state propagates to the device  

Broader growth pattern across projects: not just languages, but **execution environments** (ESP32, APIs, web, Rust desktop, Linux ops) with shared concepts (state, APIs, serialization, events, IPC).

Daily use of MCC + pad = continuous feedback on those design choices.

---

## See also

- [portfolio-hk-config-sync.md](./portfolio-hk-config-sync.md) — Git-backed portable profiles  
- [../ARCHITECTURE.md](../ARCHITECTURE.md) — current MCC / pad stack  
