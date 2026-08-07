# Cyberdeck Pad BLE protocol — v0.3 DRAFT (banks)

> **STATUS: DRAFT — not implemented, not flashed.** The pad currently runs the
> validation build (`Cyberpad Val C6`) speaking v0.2. This document is the
> proposed change for review. `PROTOCOL.md` stays canonical until this lands.

Adds **banks** — an outer index above presets — so the pad addresses
`5 × 6 × 3 = 90` slots instead of 18, without changing the 18-slot page that
already works.

## What changes from v0.2

| Area | v0.2 | v0.3 |
|---|---|---|
| Capacity | 18 slots | 90 slots (5 banks × 18) |
| Slots char | 486 B, whole pad | 487 B, **one bank per transaction** |
| Bank select | — | new `BankSel` characteristic |
| MacroEvent | 2 B `[preset, action]` | 3 B `[bank, preset, action]` |
| Info string | `Cyberdeck Pad Hybrid v0.2.0` | `Cyberdeck Pad Hybrid v0.3.0` |
| NeoPixel | green solid/flash = BLE link | **colour = bank**, flashing blue = disconnected |
| B3 | toggle pad lights | **short = bank cycle**, long-hold = toggle lights |
| Battery | `0x180f` advertised, unimplemented | **real BAS**, `0x2a19` notify → Plasma tray |
| Preset LEDs | R/G/B encode preset 1–6 | unchanged |

Bank and preset are therefore readable **at the same time** — NeoPixel colour
for the bank, the three discrete LEDs for the preset within it.

## UUIDs

| Role | UUID |
|------|------|
| Service | `c0de0001-3d17-4a00-8000-00805f9b34fb` |
| Slots | `c0de0002-3d17-4a00-8000-00805f9b34fb` |
| MacroEvent | `c0de0003-3d17-4a00-8000-00805f9b34fb` |
| Info | `c0de0004-3d17-4a00-8000-00805f9b34fb` |
| **BankSel** | **`c0de0005-3d17-4a00-8000-00805f9b34fb`** *(new)* |

## Banks

`BANK_COUNT = 5`, zero-based `0..=4`.

| Bank | Name | NeoPixel | Scope |
|---|---|---|---|
| 0 | desktop | Green | window / tiling / workspaces / monitors |
| 1 | dev | Amber | editors, terminal, ssh, prompts |
| 2 | browser | Magenta | cockpit, dashboards, AI, CRM |
| 3 | misc | Red | capture, system, hardware, 3DL tools |
| 4 | overflow | White | spare |

**Blue is reserved** for the disconnected indication and must not be a bank
colour.

## BankSel (1 byte)

Read/write, `0..=BANK_COUNT-1`. Selects which bank the `Slots` characteristic
pages in or out. Also reflects the bank the operator selected with B3, and
notifies on change so the host UI can follow the pad.

Writing a value `>= BANK_COUNT` is rejected; the pad keeps its current bank.

## Slots (487 bytes, one bank per transaction)

```
byte 0        bank index (must equal current BankSel, else reject whole write)
bytes 1..486  18 × 27-byte records, row-major preset 0..=5, action 0..=2
```

Record layout is **unchanged from v0.2**:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 | `mode` — `0` HID, `1` Macro |
| 1 | 1 | `mod` — HID modifier bitmask |
| 2 | 1 | `key` — HID keycode (`0` = none) |
| 3 | 24 | `label` — UTF-8, NUL-padded |

487 B fits the negotiated MTU (firmware requests 517), so a bank is still a
single transaction — no chunking. The bank byte is echoed inside the payload
deliberately: it makes a write self-describing and catches a torn
select-then-write sequence instead of silently writing the wrong bank.

Full sync = 5 sequential select+write pairs.

## MacroEvent notify (3 bytes)

`[bank_idx u8, preset_idx u8, action_idx u8]` — all zero-based;
`bank 0..=4`, `preset 0..=5`, `action 0..=2`.

## Buttons

| Button | Short press | Long hold |
|---|---|---|
| B1 | cycle preset 1..6 | toggle BlueZ HID fallback *(unchanged)* |
| B2 / B4 / B5 | action 0 / 1 / 2 *(unchanged)* | held-action release *(unchanged)* |
| **B3** | **cycle bank 0..4** | **toggle pad lights** |

B3 reuses the short/long idiom B1 already implements
(`BLUEZ_FALLBACK_HOLD_MS` pattern), so no new interaction convention is
introduced. Lights-toggle moves from short to long press — the only behaviour
regression for muscle memory, and it is the lower-frequency action.

Cycling bank does **not** reset the preset.

## NeoPixel state machine

Precedence, highest first:

1. `lightsEnabled == false` → **off**
2. `connected == false` (or BlueZ fallback force-flash) → **flashing blue**
   at `CONN_NEO_FLASH_MS`
3. otherwise → **solid bank colour**

So solid-anything means "linked", and the colour tells you where you are.
This supersedes the green-only behaviour in `firmware/common/conn_neopixel.h`,
which currently hardcodes `rgbLedWrite(RGB_BUILTIN, 0, level, 0)` — the helper
needs an RGB triple rather than a single green level.

Bank colours must stay distinguishable at `CONN_NEO_BRIGHT 36`. If amber and
red prove too close on hardware, swap amber for cyan — a *solid* cyan is still
unambiguous against a *flashing* blue.

## Battery (standard BLE Battery Service)

Use the **standard** service, not a vendor characteristic:

| Role | UUID |
|------|------|
| Battery Service | `0000180f-0000-1000-8000-00805f9b34fb` |
| Battery Level | `00002a19-0000-1000-8000-00805f9b34fb` — `uint8`, 0..100 %, notify |

The pad already *advertises* `0x180f` but implements nothing behind it — there
is no ADC or battery code in the firmware today, and BlueZ exposes no
`org.bluez.Battery1` for the device. Implementing the real characteristic is
what turns it on.

**Why standard, not vendor:** BlueZ maps `0x180f`/`0x2a19` onto
`org.bluez.Battery1.Percentage`, UPower picks that up automatically, and the
Plasma battery applet then shows pad charge with **zero host-side code**. MCC
reads the same D-Bus property rather than parsing a custom payload. A vendor
characteristic would require writing all of that by hand.

### Sampling

- ADC1 on the C6 is GPIO0–GPIO6. Buttons occupy 2, 3, 4, 5, 6; LEDs 12/21/15;
  NeoPixel GPIO8. **GPIO0 and GPIO1 are the only free ADC-capable pins.**
- Read through a divider, oversample (≥16 reads, mean) to suppress ADC noise,
  and convert with a LiPo curve — **not** a linear voltage map, which badly
  misreports mid-discharge.
- Update `0x2a19` on change ≥1 % and at most every ~30 s. Notify subscribers.
- Apply hysteresis so a keypress current spike cannot bounce the reported level.

### Low-battery indication

Extends the NeoPixel precedence list. Low battery must **not** claim a colour —
red is already bank 3 and blue is reserved for disconnect. Instead the bank
colour **pulses** while charge is below the threshold, so bank identity is
preserved and the alert is still unmissable:

1. `lightsEnabled == false` → off
2. `connected == false` → flashing blue
3. `battery < BAT_LOW_PCT` (proposed 15) → **pulsing bank colour**
4. otherwise → solid bank colour

### Hardware (confirmed 2026-08-05)

| Item | Value |
|---|---|
| Cell | **LP603449**, 1S LiPo, 3.7 V nominal, ~1100 mAh |
| Charger | **TP4056 with protection** (DW01A + FS8205A) — over-discharge cutoff ≈ 2.4 V |
| Sense pin | **GPIO1** (ADC1_CH1), 12 dB attenuation, 12-bit |
| Divider | 100 k / 100 k + 100 nF to GND at the tap |
| Divider drain | 21 µA (~0.05 mAh/day — negligible against 1100 mAh) |
| Reads | 4.20 V → 2.10 V at pin · 3.00 V → 1.50 V at pin |

`BAT_FULL_MV 4200`, `BAT_EMPTY_MV 3400`, `BAT_DIVIDER 2.0`, `BAT_LOW_PCT 15`.

**Power path — unresolved.** The cell currently feeds the board's 5 V pin via
TP4056 `OUT+`, which supplies 3.0–4.2 V, not 5 V. That goes through the onboard
3.3 V LDO. If the board carries an AMS1117 (~1.1 V dropout) the rail is out of
regulation even at full charge; a low-dropout part (SGM2212-class, ~300 mV)
holds until roughly 3.6 V. Recommended fix that is correct either way: **boost
LiPo → 5 V into the 5 V pin**, with a Schottky preventing USB backfeed into the
boost. Battery percentage computed on top of a sagging rail is unreliable
regardless of curve quality, so settle this before tuning breakpoints.

### LiPo curve (1S, light load)

Linear interpolation between breakpoints. Trim against real measurements once
the power path is fixed — these are nominal, not measured on this pad.

| mV | % | | mV | % |
|---|---|---|---|---|
| 4200 | 100 | | 3820 | 50 |
| 4060 | 90 | | 3790 | 40 |
| 3980 | 80 | | 3770 | 30 |
| 3920 | 70 | | 3740 | 20 |
| 3870 | 60 | | 3680 | 10 |
| | | | 3400 | 0 |

0 % is set at 3400 mV — the practical floor for a regulated rail, well above the
DW01A cutoff, so the protection IC is a backstop and not the normal stop point.

## S3 dongle transport — must be flashed with the pad

The pad is not the only device speaking this protocol. The S3 dongle proxies
the Slots characteristic over USB CDC as base64, and its sizes are compile-time
constants:

| Location | v0.2 | v0.3 |
|---|---|---|
| `s3-dongle-validation.ino:41` | `#define HYBRID_SLOTS_BYTES 486` | **487** |
| `firmware/common/cpad_base64.h` | buffer for 486 B → ~648 chars | **487 B → ~650 chars** |
| CDC command help text | `slots write <base64-486>` | `<base64-487>` |
| Proxied characteristics | `c0de0002` only | **+ `c0de0005` (BankSel)** |
| MacroEvent forwarding | 2 B | **3 B** |

A v0.2 dongle rejects a v0.3 write outright — the decode length check at
`:373` fails with `ERR b64 decode got=487 want=486`. There is no graceful
degradation, so **pad and dongle must be flashed together**. Host-side, MCC
should read `Info` from the pad *and* the dongle build string and refuse to
sync on a version mismatch rather than emitting a write that will be rejected.

Adding a `bank` argument to the CDC verbs (`slots read <bank>` /
`slots write <bank> <b64>`) is the natural companion change, so the dongle can
page all five banks over serial the same way BLE does.

## Storage

5 × 486 = **2430 bytes** of slot data in NVS, up from 486. Well within the C6
budget (the hybrid build measured 52% flash / 7% RAM at v0.2).

## Host compatibility

The host reads `Info` first:

- `v0.3.0` or later → banks available; use `BankSel`, expect 3-byte MacroEvent
- `v0.2.x` → legacy; single bank, 486-byte Slots, 2-byte MacroEvent

`store.json` needs a matching migration: `padSlots` (flat 18) and `padBindings`
(`"preset-action"`) become bank-scoped. Proposed key form `"bank-preset-action"`,
with existing keys read as bank 0 so no binding is lost.

## Open questions

1. **Empty-bank skip.** Bank 4 (overflow) starts empty. Should B3 skip banks
   with no populated slots, or is a reachable empty bank preferable?
2. **Bank names on device.** Names live host-side today. Storing them on the
   pad would cost 5 × 24 B and let the pad report them, but nothing currently
   displays text.
3. **BankSel race.** Select-then-write assumes a single host. The echoed bank
   byte detects a mismatch but does not prevent it. Acceptable given the
   "do not open a second BLE link" rule?

## Unchanged from v0.2

- Device name `Cyberdeck Pad`, advertised as HID keyboard
- Preset LED table (R / G / B, single + dual-solid, preset 1–6)
- Host connection rule — do **not** open a second BLE link; use BlueZ against
  the already-bonded HID connection
