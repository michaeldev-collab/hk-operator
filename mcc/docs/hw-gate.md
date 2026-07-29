# Hardware gate — compile / probe only (no flash)

Per operator: **do not upload** while another ESP may be on serial. Never assume
the connected ACM device is Cyberpad without confirming identity.

## Verified without flashing (template)

Record results locally; **do not commit device MAC addresses** or private host
paths to the public tree.

| Check | Result |
|-------|--------|
| Hybrid firmware **compile** (`esp32:esp32:esp32c6`) | Record OK / fail + binary size |
| Bonded Cyberpad visible to BlueZ | Record name (`Cyberdeck Pad` compat) + connected/paired — **omit MAC in public docs** |
| `cyberdeck-probe status` | Record OK / fail |
| `cyberdeck-probe info` / GATT | Expect info string `Cyberdeck Pad Hybrid v0.2.0` after hybrid flash |
| Serial port | Confirm board identity before any upload |

## When you want the full gate

1. Unplug other ESPs; plug the Cyberpad ESP32-C6.
2. Confirm port (`arduino-cli board list`).
3. Explicitly approve upload, then:
   ```bash
   arduino-cli upload -p <PORT> --fqbn esp32:esp32:esp32c6 \
     ../firmware
   ```
4. Re-pair if needed → `cyberdeck-probe info` should print `Cyberdeck Pad Hybrid v0.2.0`.
5. Flip a slot to macro → `cyberdeck-probe listen` / MCC **Listen for macros**.
6. `Sync to pad` → reboot pad → `read-slots` still matches.

See also: [`docs/portfolio-engineering-plan.md`](../../docs/portfolio-engineering-plan.md).
