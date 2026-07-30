# Cyberpad V2 — PCB planning (Phase 8)

> Status: **planning** (NeoPixel decision locked) · Companion to [`../hardware/cyberpad-v2/`](../hardware/cyberpad-v2/)  
> No flash · No UUID / BLE-name change · V1 not retired

Cyberpad = edge **input + HID/macro emitter**. MCC holds profile source of truth.

## V1 → V2

| V1 | V2 |
| --- | --- |
| DevKit fixed to perfboard | Female 2.54 mm headers — swappable DevKit |
| Point-to-point switches | Kailh MX **hotswap** sockets |
| Copper ground bar | GND **pour**; all DevKit GNDs + switch commons |
| Discrete R/G/B LEDs | **3× SK6812MINI-E** on 3V3 (see NeoPixels) |
| Hot-glue chassis | PCB + OpenSCAD case draft |

## Pin map (firmware today + V2 PCB)

| Function | GPIO | Net on PCB |
| --- | --- | --- |
| B1 | 2 | SW1 → GPIO2, other side GND |
| B2 | 3 | SW2 |
| B3 | 4 | SW3 |
| B4 | 6 | SW4 |
| B5 | 5 | SW5 |
| LED_RED (legacy discrete) | 21 | Optional DNP footprint only |
| LED_GREEN (legacy discrete) | 12 | Optional DNP |
| LED_BLUE (legacy discrete) | 15 | Optional DNP |
| NeoPixel DIN | **7** | `NEOPIX_DIN` → 330 Ω → LED1 DIN |

Button GPIOs stay fixed for first bring-up with existing hybrid firmware.
DIN uses **GPIO7** because current `v0.2.0` firmware never drives it — NeoPixels
stay quiet while discrete LED pins still blink during legacy bring-up.

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

## NeoPixels (locked)

| Decision | Choice | Why |
| --- | --- | --- |
| Count | **3** | Matches V1’s three status LEDs (preset / pair cues); small current budget |
| Part | **SK6812MINI-E** (3535 / “NeoPixel Mini”) | Addressable RGB; reliable at **3.3 V**; common keyboard footprint |
| Power | **3V3** from DevKit `3V3` header pin | No 5 V level shifter; 3 LEDs ≪ rail budget; simpler first fab |
| DIN GPIO | **GPIO7** | Free of button + legacy LED pins; unused by hybrid `v0.2.0` FW |
| Order | LED1 (nearest DIN) → LED2 → LED3 | Silk: `NP1` / `NP2` / `NP3` map to legacy R / G / B roles in future FW |

### Rejected for V2.0 fab

- **5 V WS2812B + level shifter** — works, but extra BOM and layout for three LEDs only  
- **5× under-key pixels** — nicer UX later; defer until case light pipes + FW budget exist  
- **Reusing GPIO21 as DIN** — current FW still toggles 21 as discrete LED → noisy bus on first boot  

### Passives / layout

```text
DevKit 3V3 ──┬── bulk 10 µF (near LED cluster)
             ├── NP1 VDD + 100 nF
             ├── NP2 VDD + 100 nF
             └── NP3 VDD + 100 nF

GPIO7 ── 330–470 Ω ── NP1 DIN
NP1 DOUT ──────────── NP2 DIN
NP2 DOUT ──────────── NP3 DIN
NP* GND → pour
```

- Keep DIN short; series R close to **MCU**/header side  
- Do **not** run NeoPixel VDD from GPIO  
- Optional: solder jumper to cut 3V3 to LED cluster for dark bring-up  

### Firmware note (later — not this change)

Existing sketch still drives discrete `LED_RED/GREEN/BLUE` on 21/12/15.
NeoPixel FW is a **separate** bump after electrical bring-up; keep BLE name /
UUID / `FW_INFO` until that deliberate release. First PCB spin: buttons + probe
with current hybrid image; pixels dark until NeoPixel FW.

## Artifacts

| File | Purpose |
| --- | --- |
| [`hardware/cyberpad-v2/pcb/board-concept.svg`](../hardware/cyberpad-v2/pcb/board-concept.svg) | Visual floorplan (3× NP + GPIO7 / 3V3) |
| KiCad / Gerbers | **Not yet** — no KiCad on build host; import concept next |

## Bring-up

1. Continuity GPIO↔socket, commons↔GND, NeoPixel chain open/short checks  
2. Fit DevKit in headers  
3. Flash/load **existing** hybrid firmware (operator machine — not CI)  
4. Buttons work; NeoPixels stay dark (GPIO7 idle)  
5. Probe: name `Cyberdeck Pad`, info `Cyberdeck Pad Hybrid v0.2.0` until deliberate bump  
6. Later: NeoPixel FW on GPIO7; leave discrete LED footprints DNP  

## Execute backlog

1. Confirm DevKit SKU measurements  
2. ~~NeoPixel power/count decision~~ **Done** (this doc)  
3. KiCad real footprints + DRC + Gerber  
4. Sync SCAD outline to final PCB  
5. Case print dry-fit  
