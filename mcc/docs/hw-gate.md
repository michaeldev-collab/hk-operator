# Hardware gate (2026-07-27) — no flash

Per operator: **do not upload** while a different ESP is on serial.

## Verified without flashing
| Check | Result |
|-------|--------|
| Hybrid firmware **compile** (`esp32:esp32:esp32c6`) | OK — 52% flash, 7% RAM |
| Bonded device visible to BlueZ | `20:6E:F1:11:5F:36` **Cyberdeck Pad** connected+paired |
| `cyberdeck-probe status` | OK |
| `cyberdeck-probe info` / GATT service | **Missing** — current pad firmware is pre-hybrid (expected) |
| Serial port present | `/dev/ttyACM0` — **assumed other ESP; not used** |

## When you want the full gate
1. Unplug the other ESP; plug the Cyberdeck Pad ESP32-C6.
2. Confirm port (`arduino-cli board list`).
3. Explicitly approve upload, then:
   ```bash
   arduino-cli upload -p <PORT> --fqbn esp32:esp32:esp32c6 \
     /run/media/stitch/data3/Operating/pi-iot/esp32/ble-hid-hotkeys/ble-hid-hotkey-ble-config
   ```
4. Re-pair if needed → `cyberdeck-probe info` should print `Cyberdeck Pad Hybrid v0.1.0`.
5. Flip a slot to macro → `cyberdeck-probe listen` / MCC **Listen for macros**.
6. `Sync to pad` → reboot pad → `read-slots` still matches.
