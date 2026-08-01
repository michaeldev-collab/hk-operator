# DORHEA ESP32-S3 Mini — Hardware Identification

Date: 2026-07-30  
Host: Arch Linux `7.1.5-arch1-1` (installed modules package: `7.1.5-arch1-2`)  
Method: USB descriptor inspection + OpenOCD `esp_usb_jtag` (sudo)

## Summary

One Espressif USB device was present. OpenOCD with `board/esp32s3-builtin.cfg`
successfully halted dual Tensilica taps and auto-detected **4096 KB** flash.
This matches an **ESP32-S3** with native **USB Serial/JTAG**, not an external
USB-UART bridge.

No second Espressif USB device was present. The Cyberdeck Pad C6 appears only
as a bonded BLE keyboard (`20:6E:F1:11:5F:36`).

## Findings

| Item | Value | Confidence |
|------|-------|------------|
| Expected board | DORHEA ESP32-S3 Mini | inferred (operator intent) |
| Detected SoC | ESP32-S3 (dual JTAG tap `0x120034e5`) | **confirmed** |
| Chip revision | not read from eFuse in this pass | **unknown** |
| Flash size | 4096 KB (OpenOCD auto-detect) | **confirmed** |
| PSRAM | not probed; Arduino default left disabled | **unknown** |
| USB VID:PID (stock / ROM USB-JTAG) | `303a:1001` | **confirmed** |
| USB product string | `USB JTAG/serial debug unit` | **confirmed** |
| USB manufacturer | `Espressif` | **confirmed** |
| USB serial string | `A0:F2:62:F3:D5:CC` | **confirmed** |
| USB-C exposes native ESP32-S3 USB | yes (USB Serial/JTAG composite) | **confirmed** |
| USB Serial/JTAG present | Interface 0 CDC Comm + 1 CDC Data + 2 vendor JTAG | **confirmed** |
| External USB-UART bridge | none observed on this connector | **confirmed** |
| `/dev/ttyACM*` node | absent — `cdc_acm` not loadable (kernel/modules mismatch) | **confirmed** |
| OpenOCD access | works with sudo via libusb on iface 2 | **confirmed** |
| Onboard LED pin | not confirmed on this board silk/docs in-session | **unknown** |
| BOOT button pin | typical S3 Mini BOOT=GPIO0 — **not confirmed** on this unit | **unknown** / inferred |
| Reset procedure | USB re-plug or OpenOCD `reset` | **confirmed** (OpenOCD path) |
| Bootloader / recovery | hold BOOT + tap RESET to re-enter USB download / JTAG; OpenOCD halt works even with invalid app image | **inferred** + partial **confirmed** |
| App image on flash | OpenOCD warned “Application image is invalid” | **confirmed** |

## Serial ports

| Port | Status |
|------|--------|
| `/dev/ttyACM*` | **not present** — do not guess |
| OpenOCD / libusb | usable flash/debug path without tty |

**Blocker:** running kernel `7.1.5-arch1-1` cannot load modules from
`7.1.5-arch1-2` (`Exec format error`). Host reboot onto the installed kernel
is required before CDC ACM serial monitor works.

## C6 presence (same session)

| Item | Value | Confidence |
|------|-------|------------|
| BLE name | `Cyberdeck Pad` | **confirmed** |
| BLE address | `20:6E:F1:11:5F:36` | **confirmed** |
| Bonded / connected to host BlueZ | yes / yes | **confirmed** |
| Hybrid GATT UUID advertised | `c0de0001-3d17-4a00-8000-00805f9b34fb` | **confirmed** |
| C6 USB serial | not connected | **confirmed** |

## Recovery notes (S3)

1. Prefer OpenOCD flash while `303a:1001` is visible.
2. After TinyUSB HID firmware, USB may re-enumerate away from `303a:1001`.
3. To recover download/JTAG: hold BOOT, tap RESET, release BOOT (standard S3 Mini
   procedure — **inferred** until physically verified on this DORHEA unit).
4. Do not rely on an onboard LED for core validation.
