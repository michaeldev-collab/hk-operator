# Cyberpad V2 — hardware draft (Phase 8)

> **Draft / undecided until PCB fab + bring-up.**  
> No flash · No UUID / BLE advertise-name change · V1 remains valid daily-driver evidence.

Cyberpad stays an **edge input node**: buttons + indicators in, BLE HID / MacroEvent out.
**MCC / host config** remains the source of truth for profiles and bindings. On-device
NVS slots are a cache for offline HID — same hybrid model as V1.

## Bundle

| Piece | Path | Role |
| --- | --- | --- |
| Total hardware plan | this README | Stack: PCB + case + BOM + validation order |
| PCB plan | [`../../docs/hardware-v2-pcb.md`](../../docs/hardware-v2-pcb.md) | Headers, hotswap, GND pour, NeoPixels TBD |
| Board concept (top) | [`pcb/board-concept.svg`](./pcb/board-concept.svg) | Floorplan / silk intent |
| Case (OpenSCAD) | [`case/cyberpad-v2-case.scad`](./case/cyberpad-v2-case.scad) | Parametric shell + plate |
| Case STL (assembly preview) | [`case/cyberpad-v2-case.stl`](./case/cyberpad-v2-case.stl) | Bottom + plate stacked |
| Case STL (bottom only) | [`case/cyberpad-v2-bottom.stl`](./case/cyberpad-v2-bottom.stl) | Printable bottom shell |
| Case STL (plate only) | [`case/cyberpad-v2-plate.stl`](./case/cyberpad-v2-plate.stl) | Switch plate |

## Design intent (from V1)

V1 proved the layout with perfboard, point-to-point wiring, a copper ground bus, and
hot glue ([photos](../../media/hardware/cyberpad-v1/)). V2 keeps **muscle memory**
(B1 cycle, B2/B4/B5 actions, B3 secondary) while making parts **serviceable**:

- ESP32-C6 DevKit on **female 2.54 mm headers** (board is a socket, not a soldered MCU)
- Switches in **MX hotswap sockets** (not DuPont headers — those are wrong for MX)
- **GND copper pour** tying switch commons + all DevKit GND pins (better V1 ground-bus)
- Path to **onboard NeoPixels + SMD resistors** (power/count TBD next)

## Nominal dimensions (draft — will move)

| Parameter | mm | Notes |
| --- | --- | --- |
| PCB outline | 95 × 78 | Fits case cavity with ~0.4 mm clearance |
| Switch pitch | 19.05 | Standard 1u MX |
| DevKit keepout | ~52 × 28 | Confirm against your exact C6 DevKit SKU |
| Case outer | 102 × 85 × 22 | Bottom shell + switch plate |
| USB cutout | bottom edge | DevKit USB-C faces out |

Edit numbers in the SCAD `/* params */` block and re-export STL when PCB dims lock.

## Firmware pin map (keep for first bring-up)

| Function | GPIO |
| --- | --- |
| B1 | 2 |
| B2 | 3 |
| B3 | 4 |
| B4 | 6 |
| B5 | 5 |
| LED_R / G / B (legacy discrete) | 21 / 12 / 15 |

First PCB spin should boot **existing** `Cyberdeck Pad Hybrid v0.2.0` firmware before
any NeoPixel GPIO reassignment.

## Grounding (short answer)

Do **not** star every return to a single ESP GND pin on skinny traces.

Do: **GND pour** on the PCB; connect **every** DevKit GND header pad into the pour;
tie all switch commons into the pour. Multiple ESP GND pins = multiple vias into the
same plane (the DevKit already bonds them on-module).

## Validation order (undecided → locked)

1. Confirm DevKit SKU + measure header span / USB overhang  
2. Finalize NeoPixel count + 3V3 vs 5V  
3. KiCad footprints + DRC (concept SVG is not Gerbers)  
4. Adjust SCAD to match final PCB outline  
5. Print case draft · dry-fit DevKit + switches  
6. Fab PCB · continuity · plug firmware · probe (no CI flash)  
7. Enclosure v2 (screw bosses, strain relief) after electrical OK  

## Related

- [`docs/hardware-v1.md`](../../docs/hardware-v1.md)
- [`docs/hardware-v2-pcb.md`](../../docs/hardware-v2-pcb.md)
- [`firmware/README.md`](../../firmware/README.md)
- [`mcc/protocol/PROTOCOL.md`](../../mcc/protocol/PROTOCOL.md)
