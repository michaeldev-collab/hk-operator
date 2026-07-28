# Cyberpad V1 Hardware

## Origin

Cyberpad V1 was built in one day from parts already available rather than designed
around a custom PCB or enclosure. It is the physical controller for the
[HK Operator](../README.md) platform.

## Construction

Cyberpad V1 is literally an ESP32 development board and mechanical switches
mounted to perfboard, wired point-to-point, and held together with hot glue.

Explicit inventory:

- ESP32-C6 development board
- Generic perfboard
- Mechanical keyboard switches mounted directly to the perfboard
- Point-to-point wiring
- Solid copper common-ground bus
- Hot glue used for structural mounting
- Hot glue also used as an ergonomic hand-rest feature
- Three status LEDs
- No production enclosure
- No custom PCB

## Why it matters

Despite the crude construction:

- It was used daily for approximately four months
- It had no meaningful firmware failures during that validation period
- The physical layout became muscle memory
- The stable hardware interface justified building the HK Operator Mission
  Control Center (MCC)
- V1 acted as a real-world validation platform rather than a bench-only demo

## Ground bus

A solid copper bar serves as the common ground for the switch matrix. On
perfboard it reduces star-ground spaghetti, keeps returns short, and makes
point-to-point wiring easier to reason about when revising a one-day build.

## Planned media

Images are not in the repository yet. Planned paths:

| Shot | Path |
| --- | --- |
| Top view | `media/hardware/cyberpad-v1/top.jpg` *(planned)* |
| Underside wiring | `media/hardware/cyberpad-v1/underside.jpg` *(planned)* |
| Copper ground bus | `media/hardware/cyberpad-v1/ground-bus.jpg` *(planned)* |
| Hot-glue ergonomic support | `media/hardware/cyberpad-v1/hot-glue-rest.jpg` *(planned)* |
| Hand resting position | `media/hardware/cyberpad-v1/hand-rest.jpg` *(planned)* |
| MCC controlling Cyberpad | `media/hardware/cyberpad-v1/mcc-with-device.jpg` *(planned)* |

## Future hardware

Planned improvements (V1 is not being retired):

- Custom PCB
- Proper switch mounting
- Enclosure
- Strain relief
- Serviceability
- Preserving the validated physical layout that already has muscle memory

## Related

- [Architecture](./architecture.md)
- [Firmware](../firmware/README.md)
- [V1 hardware, summarized](./ESP-AND-HOT-GLUE.md)
