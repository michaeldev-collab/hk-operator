# S3 Dongle PoC — Validation Results

Branch: `experiment/s3-dongle-validation` (merged into `main` for MCC integration)  
Host OS: Arch Linux (kernel `7.1.5-arch1-1`, modules pkg `7.1.5-arch1-2`)  
Date: 2026-07-30 · MCC dongle-slot integration recorded 2026-07-30 ~22:49

Status labels: `NOT RUN` · `BLOCKED` · `COMPILED` · `FLASHED` · `PHYSICALLY VERIFIED` · `FAILED`

| Test | Build | Flash | Physical | Result | Evidence | Remaining issue |
|------|-------|-------|----------|--------|----------|-----------------|
| A — S3 USB HID smoke | COMPILED | FLASHED | PHYSICALLY VERIFIED | PASS | After physical RESET: USB enum `Espressif Systems ESP32S3_DEV` HID keyboard (`usbhid`); userspace CDC sent `hid test a`; host `/dev/input/event30` saw KEY_A press (`val=1`) then release (`val=0`); diag RX included `[hid] test a complete` / `state=USB_READY` | Kernel `cdc_acm` still missing (no `/dev/ttyACM*`); used pyusb CDC instead |
| B — BLE transport (serial-triggered) | COMPILED | FLASHED (C6/S3 0.3.0) | PHYSICALLY VERIFIED | PASS | Host BlueZ blocked on pad; S3 CDC `state=CONNECTED ble_connected=1`; heartbeats + `KEYBOARD_REPORT` from BLE | Keep BlueZ blocked/disconnected during runs |
| C — Real Cyberpad switch | COMPILED | FLASHED | PHYSICALLY VERIFIED | PASS | User confirmed end-to-end while both Neos solid; S3 CDC showed B2 → `keys=04` then release-all | — |
| D — Disconnect release-all | COMPILED | FLASHED | NOT RUN | NOT RUN | Release-all seen on empty BLE reports; intentional disconnect test not separately logged | **Release gate** (below) |
| E — Direct BLE HID regression (flag=0) | COMPILED | NOT RUN | NOT RUN | COMPILED | Flag=0 binary strings include `Cyberdeck Pad Hybrid v0.2.0` | **Release gate** (below) |
| F — MCC dongle-slot integration | COMPILED | FLASHED (0.4.0) | PHYSICALLY VERIFIED | PASS | MCC prefers CDC when linked; Refresh shows `via S3 dongle`; Sync writes slots through proxy. See [`dongle-slots-proxy.md`](dongle-slots-proxy.md). | Formal reconnect-after-MCU-reset still a release gate |

## Environment notes

| Item | Result |
|------|--------|
| S3 identity | ESP32-S3 dual tap `0x120034e5`, flash **4096 KB**, USB `303a:1001` serial `A0:F2:62:F3:D5:CC` |
| `/dev/ttyACM*` | Absent during initial runs — `cdc_acm` vermagic mismatch (`arch1-1` running vs `arch1-2` modules); CDC via userspace/libusb |
| Flash path | OpenOCD `esp_usb_jtag` via `scripts/flash-s3-dongle-validation.sh` |
| Flash image | Verified in flash: bootloader@0, partitions@0x8000, boot_app0@0xe000, app@0x10000 |
| C6 | BLE `20:6E:F1:11:5F:36` **Cyberpad Val C6**; host BlueZ blocked during bridge; NeoPixel solid=subscribed |
| NeoPixel | S3 GPIO48 / C6 GPIO8 — flashing=disconnected, solid blue=linked |
| Host proto tests | `npm run test:proto` — 6/6 pass |

## Stop conditions / follow-ups

1. ~~S3 app boot after OpenOCD soft-reset~~ — **resolved by physical RESET** (2026-07-30 ~21:54). TinyUSB HID + CDC came up as `ESP32S3_DEV`.
2. **Host cannot load `cdc_acm`** until reboot onto matching kernel/modules — CDC worked via userspace/libusb for Test A / MCC.
3. ~~C6 USB / experimental flash~~ — flashed via OpenOCD; bridge validated 2026-07-30 ~22:25.
4. ~~MCC integration~~ — **done** (dongle-slot proxy in MCC; see Test F).

## Limitations (by design)

- Experimental C6 dongle mode disables direct BLE HID while that firmware mode is active
- Protocol v0 is disposable for the PoC bridge
- External production firmware path unchanged
- Host BlueZ must stay blocked on the pad while the dongle owns the central slot

## Concept status

**Dongle-proxied MCC slots (0.4.0):** C6 keeps hybrid `c0de0001` slots + validation notify; S3 CDC `slots read/write`; MCC prefers dongle transport when linked. See [`dongle-slots-proxy.md`](dongle-slots-proxy.md). **Flashed 2026-07-30 ~22:49** via OpenOCD; physical RST required to leave ROM/JTAG idle, then verify MCC Refresh shows `via S3 dongle`.

## Release gates (remaining physical validation)

These were called out as gaps (Test D / E / reconnect) and must pass before treating the dongle path as release-ready:

- [ ] Disconnect while keys are held always produces release-all
- [ ] S3 reset while keys are held produces release-all
- [ ] C6 reset while keys are held produces release-all
- [ ] Direct BLE mode remains physically functional
- [ ] MCC reconnects after either MCU restarts
