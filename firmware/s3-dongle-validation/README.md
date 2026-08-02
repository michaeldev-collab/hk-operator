# Cyberpad S3 Validation Dongle (PoC)

Bench firmware for the DORHEA ESP32-S3 Mini:

1. Enumerate as USB HID keyboard (+ CDC diagnostics when available)
2. Act as BLE central for the experimental C6 validation service
3. Bridge validated keyboard-state packets to USB HID

## Build

```bash
cd /home/stitch/3dl-macro-command-center

arduino-cli compile --fqbn \
  'esp32:esp32:esp32s3:USBMode=default,CDCOnBoot=cdc,FlashSize=4M,PSRAM=disabled,UploadMode=cdc' \
  --libraries firmware \
  --libraries /run/media/stitch/data3/Operating/pi-iot/libraries \
  firmware/s3-dongle-validation
```

## Flash (OpenOCD / USB JTAG)

When `/dev/ttyACM*` is unavailable (current host kernel/modules mismatch), use:

```bash
./scripts/flash-s3-dongle-validation.sh
```

Requires the board to enumerate as `303a:1001` (USB Serial/JTAG). After TinyUSB
firmware runs, recovery is BOOT+RESET to get JTAG/download back.

## Monitor

Prefer CDC after reboot restores `cdc_acm`:

```bash
arduino-cli monitor -p /dev/ttyACM0 -c baudrate=115200
```

## Connection NeoPixel

Onboard WS2812 (ESP32-S3 Dev Module **GPIO48** / `RGB_BUILTIN`):

- **Solid green** — BLE linked to Cyberpad
- **Flashing green** — disconnected / scanning

## Auto-reconnect

On boot the dongle targets the PoC peer (`20:6E:F1:11:5F:36`) and retries every
~2.5s after drops / failed connects. `disconnect` or `reconnect off` stops that;
`connect known` / `reconnect on` turns it back on.

## Slots proxy (MCC)

When linked, CDC commands talk to the pad hybrid slots GATT:

- `slots read` → `SLOTS <base64-486>`
- `slots write <base64-486>` → `OK` / `ERR …`
- `pad info` → `INFO <fw string>`
- `status` includes `slots_ready=0|1` and `transport=dongle`

## Safety

- No keystrokes on boot
- `hid test a` = one press + release only
- Disconnect / bad packet / heartbeat timeout → USB release-all
