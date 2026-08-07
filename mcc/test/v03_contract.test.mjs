import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const root = new URL("../", import.meta.url);
const read = (path) => readFile(new URL(path, root), "utf8");

const [
  c6,
  s3,
  commonNeo,
  libraryNeo,
  commonB64,
  libraryB64,
  ble,
  dongle,
  tauri,
  app,
  padBanks,
  flashC6,
  flashS3,
  cdcGate,
  dongleVerifier,
  padGuard,
] =
  await Promise.all([
    read("firmware/c6-s3-dongle-validation/c6-s3-dongle-validation.ino"),
    read("firmware/s3-dongle-validation/s3-dongle-validation.ino"),
    read("firmware/common/conn_neopixel.h"),
    read("firmware/CyberpadValidationProto/src/conn_neopixel.h"),
    read("firmware/common/cpad_base64.h"),
    read("firmware/CyberpadValidationProto/src/cpad_base64.h"),
    read("crates/cyberdeck-ble/src/lib.rs"),
    read("crates/cyberdeck-dongle/src/lib.rs"),
    read("src-tauri/src/main.rs"),
    read("src/app.js"),
    read("src/pad_banks.js"),
    read("scripts/flash-c6-validation.sh"),
    read("scripts/flash-s3-dongle-validation.sh"),
    read("scripts/s3-cdc-release-gate.sh"),
    read("crates/cyberdeck-dongle/src/bin/cyberdeck-dongle-verify.rs"),
    read("firmware/c6-s3-dongle-validation/pad_led_guard.h"),
  ]);

test("duplicated firmware headers stay byte-identical", () => {
  assert.equal(commonNeo, libraryNeo);
  assert.equal(commonB64, libraryB64);
});

test("C6 exposes the exact v0.3 bank wire contract", () => {
  assert.match(c6, /CYBERDECK_BANK_SEL_UUID\s*=\s*"c0de0005-3d17-4a00-8000-00805f9b34fb"/);
  assert.match(c6, /FW_INFO\s*=\s*"Cyberdeck Pad Hybrid v0\.3\.1"/);
  assert.match(c6, /BANK_COUNT\s*=\s*5/);
  assert.match(c6, /static_assert\(SLOT_BYTES == 27/);
  assert.match(c6, /static_assert\(BANK_SLOTS_BYTES == 486/);
  assert.match(c6, /static_assert\(SLOTS_PAGE_BYTES == 487/);
  assert.match(c6, /uint8_t payload\[3\] = \{bank, presetIdx, actionIdx\}/);
  assert.match(c6, /const uint8_t selectedBank = gCurrentBank/);
  assert.match(c6, /saveBankData\(selectedBank, &candidate\[0\]\[0\]\)/);
  assert.match(c6, /NVS_SCHEMA_VERSION\s*=\s*3/);
});

test("C6 battery constants and interpolation points match the draft", () => {
  assert.match(c6, /BATTERY_SERVICE_UUID\s*=\s*"180f"/);
  assert.match(c6, /BATTERY_LEVEL_UUID\s*=\s*"2a19"/);
  assert.match(c6, /BAT_FULL_MV\s*=\s*4200/);
  assert.match(c6, /BAT_EMPTY_MV\s*=\s*3400/);
  assert.match(c6, /BAT_DIVIDER\s*=\s*2\.0f/);
  assert.match(c6, /BAT_LOW_PCT\s*=\s*15/);
  assert.match(c6, /BAT_OVERSAMPLE_READS\s*=\s*16/);
  assert.match(c6, /BAT_SAMPLE_INTERVAL_MS\s*=\s*30000/);
  assert.match(c6, /gValSubscriptionPending\s*=\s*true/);
  assert.match(c6, /void processValidationSubscription\(\)/);
  assert.match(c6, /pBatteryChar->notify\(\);[\s\S]{0,400}delay\(30\);/);
  const curveBody = c6.match(/static const Point curve\[\] = \{([\s\S]*?)\n\s*\};/)?.[1];
  assert.ok(curveBody, "battery curve table not found");
  const points = [...curveBody.matchAll(/\{(\d+),\s*(\d+)\}/g)].map((m) => [
    Number(m[1]),
    Number(m[2]),
  ]);
  assert.deepEqual(points, [
    [3400, 0],
    [3680, 10],
    [3740, 20],
    [3770, 30],
    [3790, 40],
    [3820, 50],
    [3870, 60],
    [3920, 70],
    [3980, 80],
    [4060, 90],
    [4200, 100],
  ]);
});

test("C6 USB diagnostic mode preserves native USB and exposes reset stages", () => {
  assert.match(c6, /#define CYBERPAD_USB_DIAGNOSTIC 0/);
  assert.match(
    c6,
    /CYBERPAD_USB_DIAGNOSTIC && !ARDUINO_USB_CDC_ON_BOOT[\s\S]{0,160}#error/,
  );
  assert.match(c6, /\[diag\] boot reset_reason=/);
  assert.match(c6, /\[diag\] stage=before-gatt/);
  assert.match(c6, /\[diag\] alive ms=/);
});

test("GPIO12 arbitration is fail-safe toward USB", () => {
  // The guard is keyed on the USB pin numbers, not on LED_GREEN, so moving the
  // LED to GPIO13 or adding another indicator on a USB pin stays covered.
  assert.match(padGuard, /CPAD_USB_DM_PIN\s*=\s*12/);
  assert.match(padGuard, /CPAD_USB_DP_PIN\s*=\s*13/);
  assert.match(
    padGuard,
    /padLedPinIsUsbReserved[\s\S]{0,200}CPAD_USB_DM_PIN[\s\S]{0,40}CPAD_USB_DP_PIN/,
  );

  // Default must be "USB owns the pin". A 900 ms blocking sample in setup()
  // previously concluded "no host" while plugged in -- USB re-enumeration after
  // a reset takes 1-2 s -- and killed USB on every reset. Arbitration must
  // therefore run over a grace window and never claim on one negative sample.
  assert.match(padGuard, /static bool usbOwnsGreen = true;/);
  assert.match(padGuard, /padLedArbitrateTick\(uint32_t graceMs = \d{4,}\)/);
  assert.match(padGuard, /if \(HWCDC::isPlugged\(\)\)[\s\S]{0,160}decided = true;/);
  assert.match(padGuard, /if \(millis\(\) >= graceMs\)/);
  assert.doesNotMatch(padGuard, /padLedArbitrateUsb/); // blocking version is gone

  // Every pad LED write in BOTH firmware branches must route through the guard.
  assert.doesNotMatch(c6, /(?:pinMode|digitalWrite|analogWrite)\s*\(\s*LED_GREEN/);
  assert.match(c6, /padLedArbitrateTick\(\)/);
});

test("S3 validates a canonical page before selecting its bank", () => {
  assert.match(s3, /FW_VERSION "s3-dongle-validation 0\.5\.3 protocol-v0\.3"/);
  assert.match(s3, /HYBRID_SLOTS_BYTES 487/);
  assert.match(s3, /HYBRID_SLOTS_B64_BYTES 652/);
  assert.match(s3, /NimBLEDevice::setMTU\(517\)/);
  assert.match(s3, /if \(len != 3 \|\| data\[0\] >= BANK_COUNT/);
  assert.match(s3, /static const char prefix\[\] = "Cyberdeck Pad Hybrid v"/);
  const writeStart = s3.indexOf("static void cmdSlotsWrite");
  const validate = s3.indexOf("b64.length() != HYBRID_SLOTS_B64_BYTES", writeStart);
  const select = s3.indexOf("if (!selectBank(bank))", writeStart);
  assert.ok(writeStart >= 0 && validate > writeStart && select > validate);
});

test("host layers agree on five banks and fail-safe batch restoration", () => {
  for (const source of [ble, padBanks]) {
    assert.match(source, /BANK_COUNT(?:: usize)?\s*=\s*5|BANK_COUNT\s*=\s*5/);
  }
  assert.match(app, /\n\s*BANK_COUNT,\n/);
  assert.match(tauri, /ACTION_COUNT, BANK_COUNT,/);
  assert.match(ble, /SLOTS_PAGE_BYTES: usize = 1 \+ SLOTS_BYTES/);
  assert.match(ble, /chunk\.len\(\) != expected_len/);
  assert.match(dongle, /SLOTS_B64_BYTES: usize = 652/);
  assert.match(dongle, /set_auto_detach_kernel_driver\(true\)/);
  assert.match(dongle, /DongleError::Unavailable/);
  assert.match(dongle, /self\.clear_input\(\)\?/);
  assert.match(tauri, /async fn pad_write_banks/);
  assert.match(tauri, /async fn pad_read_banks/);
  assert.match(tauri, /async fn pad_restore_bank/);
  assert.match(app, /tauriInvoke\("pad_write_banks"/);
  assert.match(app, /tauriInvoke\("pad_read_banks"/);
  assert.match(app, /let saveTail = Promise\.resolve\(\)/);
  assert.match(app, /\.\.\.buildPortablePadState\(state\.padBindings, state\.padSlots\)/);
  assert.match(app, /const portable = readPortablePadState\(data\)/);
  assert.match(app, /await persistSaveSnapshot\(captureSaveSnapshot\(\)\)/);
  assert.match(app, /if \(state\.storeReplaceBusy\)/);
  assert.match(app, /setStoreReplaceBusy\(true\)/);
  assert.match(app, /async function importProfileDisk\([\s\S]*?setPadIoBusy\(true\)/);
  assert.match(app, /async function gitPullApply\([\s\S]*?setPadIoBusy\(true\)/);
  assert.match(tauri, /approved_command_values/);
  assert.match(tauri, /validate_action_ids/);
});

test("S3 application proof is exact and gates every C6 mutation", () => {
  const exactVersion = "s3-dongle-validation 0.5.3 protocol-v0.3";
  assert.match(dongle, new RegExp(`EXPECTED_DONGLE_VERSION[^\\n]+${exactVersion.replaceAll(".", "\\.")}`));
  assert.match(dongle, /pub fn version\(&mut self\)/);
  assert.match(dongle, /self\.version\(\)\?;[\s\S]{0,200}self\.status_line\(\)\?/);
  assert.match(dongleVerifier, /DonglePad::open\(\)/);
  assert.match(dongleVerifier, /dongle\.version\(\)/);
  assert.match(dongleVerifier, /"banks"\s*=> verify_banks/);
  assert.match(dongleVerifier, /for bank in 0\.\.BANK_COUNT/);
  assert.match(dongleVerifier, /\.read_slots\(bank as u8\)/);
  assert.match(dongleVerifier, /restore_bank\(dongle, initial_bank\)/);
  assert.match(dongleVerifier, /BANK_PAGES_OK banks=/);
  assert.match(cdcGate, /cargo build --offline --locked --release/);
  assert.match(cdcGate, /prove_responsive\(\)/);
  assert.match(flashS3, /cyberpad-v03-final-s3-051/);
  assert.match(flashS3, /CDC_PROOF_SECONDS=30/);
  assert.match(flashC6, /CDC_PROOF_SECONDS=30/);
  assert.match(s3, /#define BLE_CONNECT_TIMEOUT_MS 500/);
  assert.match(s3, /setConnectTimeout\(BLE_CONNECT_TIMEOUT_MS\)/);
  assert.match(s3, /setConnectRetries\(0\)/);
  assert.doesNotMatch(flashC6, /SKIP_S3|BYPASS_S3/);

  const c6Proof = flashC6.indexOf('bash "$CDC_GATE" prove');
  const c6Mutation = flashC6.indexOf('-c "program_esp ');
  assert.ok(c6Proof >= 0 && c6Mutation > c6Proof, "C6 flash must follow live S3 proof");

  const s3Programs = [...flashS3.matchAll(/-c "program_esp /g)];
  assert.equal(s3Programs.length, 4, "S3 script must contain one four-segment program sequence");
  const s3Proof = flashS3.indexOf('bash "$CDC_GATE" prove', s3Programs.at(-1).index);
  const recoveryReset = flashS3.indexOf('-c "reset run"', s3Proof);
  assert.ok(s3Proof > s3Programs.at(-1).index, "S3 CDC proof must follow programming");
  assert.ok(recoveryReset > s3Proof, "only reset recovery may follow a failed proof");
  assert.equal(flashS3.indexOf('-c "program_esp ', recoveryReset), -1);
});
