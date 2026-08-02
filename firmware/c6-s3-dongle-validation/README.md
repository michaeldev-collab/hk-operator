# C6 Cyberpad — S3 dongle validation sketch

Vendored validation copy of the hybrid Cyberpad firmware.

**Production / daily driver remains:**
`/run/media/stitch/data3/Operating/pi-iot/esp32/ble-hid-hotkeys/ble-hid-hotkey-ble-config/`

## Build flags

| Flag | Default | Behavior |
|------|---------|----------|
| `CYBERPAD_EXPERIMENTAL_S3_DONGLE` | `0` | Hybrid BLE HID + Cyberdeck GATT (normal) |
| `CYBERPAD_EXPERIMENTAL_S3_DONGLE=1` | off | Validation GATT only; **direct BLE HID disabled** |

### Normal (regression)

```bash
arduino-cli compile --fqbn esp32:esp32:esp32c6 \
  --libraries /home/stitch/3dl-macro-command-center/firmware \
  --libraries /run/media/stitch/data3/Operating/pi-iot/libraries \
  firmware/c6-s3-dongle-validation
```

### Experimental dongle transport

```bash
arduino-cli compile --fqbn esp32:esp32:esp32c6 \
  --build-property 'compiler.cpp.extra_flags=-DCYBERPAD_EXPERIMENTAL_S3_DONGLE=1' \
  --libraries /home/stitch/3dl-macro-command-center/firmware \
  --libraries /run/media/stitch/data3/Operating/pi-iot/libraries \
  firmware/c6-s3-dongle-validation
```

## Experimental behavior

- Advertises validation `c0de1001-…` as **Cyberpad Val C6** plus hybrid Cyberdeck
  slots GATT `c0de0001-…` (same layout as MCC / production)
- B2/B4/B5 → configured slot actions through the subscribed S3 dongle, or
  automatically through direct BLE HID when BlueZ owns the link
- Serial: `val test a`, `val release`, `hello`, `status`
- Heartbeat every 2s while subscribed
- Onboard NeoPixel (ESP32-C6 Dev Module **GPIO8** / `RGB_BUILTIN`):
  **solid green** when either S3 or BlueZ is linked, **slow green flash** when
  alone, **fast green flash** whenever explicit BlueZ fallback is engaged
- Transport is told apart by subscription, not by `keyboard.isConnected()` —
  that is true for any central on the shared NimBLE server, the S3 included.
  The S3 subscribes to validation; only a bonded BlueZ host encrypts (`isPaired()`)
- Default path is S3 dongle; when the dongle is absent and BlueZ connects,
  `keyboard.tap()` is selected automatically. Long-press **B1** ~1.2s toggles
  explicit BlueZ fallback.
- MCC Refresh/Sync goes through the S3 dongle CDC proxy (not BlueZ) while linked

## Flash

Only when the **Cyberpad C6** USB port is identified (do not guess):

```bash
arduino-cli upload -p <C6_PORT> --fqbn esp32:esp32:esp32c6 \
  firmware/c6-s3-dongle-validation
```
