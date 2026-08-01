# Dongle slots CDC proxy

When the S3 validation dongle is BLE-linked to the Cyberpad, MCC Refresh/Sync
goes through USB CDC on the dongle instead of BlueZ.

## Flow

```
MCC  --CDC-->  S3 dongle  --BLE slots GATT c0de0002-->  Cyberpad C6
MCC  <--CDC--  S3 dongle  <--BLE val notify c0de1002--  Cyberpad C6
Host <--USB HID-- S3 dongle
```

BlueZ must stay blocked/disconnected from the pad while the dongle owns the link.

## CDC commands (line-oriented, 115200)

| Command | Response |
|---------|----------|
| `status` | `state=… ble_connected=0\|1 … slots_ready=0\|1 transport=dongle` |
| `slots read` | `SLOTS <base64 of 486 packed bytes>` |
| `slots write <base64>` | `OK` or `ERR …` |
| `pad info` | `INFO <fw string>` |

Pack layout matches hybrid firmware / `cyberdeck-ble`: 18 × 27-byte slots
(`mode`, `mod`, `key`, `label[24]`).

## Versions

- C6 experimental: `Cyberpad C6 S3-Dongle Validation 0.4.0`
- S3 dongle: `s3-dongle-validation 0.4.0`
- MCC: `cyberdeck-dongle` crate (serialport or libusb CDC)

## Ops

1. Flash C6 experimental + S3 0.4.0; RST both.
2. Wait for solid NeoPixels (auto-reconnect).
3. Keep `20:6E:F1:11:5F:36` blocked in BlueZ.
4. MCC Refresh should show `via S3 dongle`; Sync writes slots through the proxy.
