# Cyberpad V2 — PCB planning (Phase 8)

> Status: **planning draft** · Companion to [`../hardware/cyberpad-v2/`](../hardware/cyberpad-v2/)  
> No flash · No UUID / BLE-name change · V1 not retired

Cyberpad = edge **input + HID/macro emitter**. MCC holds profile source of truth.

## V1 → V2

| V1 | V2 |
| --- | --- |
| DevKit fixed to perfboard | Female 2.54 mm headers — swappable DevKit |
| Point-to-point switches | Kailh MX **hotswap** sockets |
| Copper ground bar | GND **pour**; all DevKit GNDs + switch commons |
| Discrete R/G/B LEDs | Path to NeoPixels + SMD R/C (TBD) |
| Hot-glue chassis | PCB + OpenSCAD case draft |

## Pin map (firmware today)

| Function | GPIO | Net on PCB |
| --- | --- | --- |
| B1 | 2 | SW1 → GPIO2, other side GND |
| B2 | 3 | SW2 |
| B3 | 4 | SW3 |
| B4 | 6 | SW4 |
| B5 | 5 | SW5 |
| LED_RED | 21 | Optional DNP discrete or unused |
| LED_GREEN | 12 | Optional DNP |
| LED_BLUE | 15 | Optional DNP |
| NeoPixel DIN | TBD free GPIO | Reserve silk; do not steal button pins |

## ESP socket

- Two rows **female** 2.54 mm headers matching ESP32-C6 DevKitC-class male pins  
- Full socket (unused pins still present)  
- USB-C toward board edge for cable clearance  

## Switches

Use **MX hotswap sockets**, not DuPont female headers (wrong mechanic for MX stems/pins).

```text
GPIOx ── hotswap A
GND   ── hotswap B
```

`INPUT_PULLUP` + active-low unchanged.

## Grounding

1. Bottom (and preferably top) **GND pour**  
2. Every DevKit **GND** pad → pour (vias)  
3. Every switch common → pour  
4. Avoid single-pin star returns  

V1’s one ground bar was electrically one node; the pour is the same idea done properly.
Connecting multiple ESP GND pins into the pour is **good** — not “pick one ground only.”

## NeoPixels (placeholder)

Reserve footprints: LED + 330–470 Ω DIN series + 100 nF at VCC each.  
Count / 3V3 vs 5V / exact GPIO — **next decision before fab**.

## Artifacts

| File | Purpose |
| --- | --- |
| [`hardware/cyberpad-v2/pcb/board-concept.svg`](../hardware/cyberpad-v2/pcb/board-concept.svg) | Visual floorplan |
| KiCad / Gerbers | **Not yet** — no KiCad on build host; import concept next |

## Bring-up

1. Continuity GPIO↔socket, commons↔GND  
2. Fit DevKit in headers  
3. Flash/load **existing** hybrid firmware (operator machine — not CI)  
4. Buttons work before NeoPixel firmware  
5. Probe: name `Cyberdeck Pad`, info `Cyberdeck Pad Hybrid v0.2.0` until deliberate bump  

## Execute backlog

1. Confirm DevKit SKU measurements  
2. NeoPixel power/count decision  
3. KiCad real footprints + DRC + Gerber  
4. Sync SCAD outline to final PCB  
5. Case print dry-fit  
