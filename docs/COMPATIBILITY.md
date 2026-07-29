# Compatibility matrix

Canonical packaging and upgrade/rollback guidance:
[`docs/release-packaging.md`](./release-packaging.md).

## Supported baseline (public)

| MCC | Firmware `FW_INFO` | Protocol | BLE advertise name |
| --- | --- | --- | --- |
| `0.2.x` | `Cyberdeck Pad Hybrid v0.2.0` | [`PROTOCOL.md`](../mcc/protocol/PROTOCOL.md) — 18 slots / 486 B | `Cyberdeck Pad` |

Before upgrading, run probe-only checks (`cyberdeck-probe status` / `info`).
Do not flash from CI. Do not commit device MACs.
