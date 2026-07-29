#!/usr/bin/env bash
# Print SHA-256 for release notes (firmware source). No flash.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INO="$ROOT/firmware/ble-hid-hotkey-ble-config.ino"
if [[ ! -f "$INO" ]]; then
  echo "missing: $INO" >&2
  exit 1
fi
echo "## Firmware checksum"
echo
echo "| File | SHA-256 |"
echo "| --- | --- |"
sum=$(sha256sum "$INO" | awk '{print $1}')
echo "| \`firmware/ble-hid-hotkey-ble-config.ino\` | \`$sum\` |"
echo
echo "FW_INFO (from sketch; verify manually if bumped):"
grep -E 'FW_INFO\s*=' "$INO" || true
