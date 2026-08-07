import assert from "node:assert/strict";
import { test } from "node:test";
import {
  BANK_COUNT,
  SLOT_LABEL_MAX_BYTES,
  SLOT_COUNT,
  bankBindingKey,
  buildPortablePadState,
  emptyPadBanks,
  infoSupportsBanks,
  migratePadBindings,
  normalizePadBanks,
  parseBindingKey,
  readPortablePadState,
  storeReplacementBlocked,
  truncatePadLabel,
  validPadBanks,
} from "../src/pad_banks.js";

test("legacy flat slots migrate to bank 0 without aliasing empty banks", () => {
  const flat = Array.from({ length: SLOT_COUNT }, (_, i) => ({
    mode: 0,
    mod: 0,
    key: i,
    label: `slot-${i}`,
  }));
  const banks = normalizePadBanks(flat);
  assert.ok(validPadBanks(banks));
  assert.equal(banks.length, BANK_COUNT);
  assert.equal(banks[0][3].label, "slot-3");
  assert.equal(banks[1][3].label, "");
  banks[1][0].label = "bank-one";
  assert.equal(banks[2][0].label, "");
});

test("banked slot shape round-trips and malformed shapes are rejected", () => {
  const banks = emptyPadBanks();
  banks[4][17].label = "last";
  const normalized = normalizePadBanks(banks);
  assert.equal(normalized[4][17].label, "last");
  assert.equal(normalizePadBanks(banks.slice(0, 4)), null);
  assert.equal(normalizePadBanks([[]]), null);
});

test("pad labels use at most 23 UTF-8 bytes without splitting code points", () => {
  const exact = `${"a".repeat(19)}💡`;
  assert.equal(new TextEncoder().encode(exact).length, SLOT_LABEL_MAX_BYTES);
  assert.equal(truncatePadLabel(exact), exact);

  const truncated = truncatePadLabel(`${"a".repeat(20)}💡`);
  assert.equal(truncated, "a".repeat(20));
  assert.ok(new TextEncoder().encode(truncated).length <= SLOT_LABEL_MAX_BYTES);
  assert.equal(truncated.includes("�"), false);
  assert.equal(truncatePadLabel("\ufeffbank"), "\ufeffbank");
});

test("pad labels reject embedded NUL instead of hiding trailing content", () => {
  assert.throws(() => truncatePadLabel("before\0after"), /cannot contain NUL/);

  const banks = emptyPadBanks();
  banks[2][4].label = "before\0after";
  assert.equal(normalizePadBanks(banks), null);
});

test("legacy bindings migrate to bank 0 and explicit v0.3 keys win", () => {
  const migrated = migratePadBindings({
    "2-0": "legacy",
    "0-2-0": "explicit",
    "4-5-2": "overflow",
  });
  assert.equal(migrated["0-2-0"], "explicit");
  assert.equal(migrated["4-5-2"], "overflow");
  assert.equal(migrated["2-0"], undefined);
});

test("portable pad state round-trips all five banks at schema 3", () => {
  const banks = emptyPadBanks();
  banks[0][0].label = "desktop";
  banks[4][17].label = "overflow";
  const exported = buildPortablePadState({ "4-5-2": "last" }, banks);
  assert.equal(exported.schemaVersion, 3);

  const imported = readPortablePadState(JSON.parse(JSON.stringify(exported)));
  assert.equal(imported.hasPadBindings, true);
  assert.equal(imported.hasPadSlots, true);
  assert.equal(imported.padBindings["4-5-2"], "last");
  assert.equal(imported.padSlots[0][0].label, "desktop");
  assert.equal(imported.padSlots[4][17].label, "overflow");
});

test("portable pad import migrates legacy data and rejects future/malformed data", () => {
  const flat = emptyPadBanks()[0];
  flat[3].label = "legacy";
  const imported = readPortablePadState({
    padBindings: { "2-0": "legacy-action" },
    padSlots: flat,
  });
  assert.equal(imported.padBindings["0-2-0"], "legacy-action");
  assert.equal(imported.padSlots[0][3].label, "legacy");
  assert.equal(imported.padSlots[1][3].label, "");

  assert.throws(
    () => readPortablePadState({ schemaVersion: 4, padSlots: flat }),
    /newer than supported/
  );
  assert.throws(() => readPortablePadState({ padSlots: [[]] }), /five banks/);
  flat[0].label = "bad\0label";
  assert.throws(() => readPortablePadState({ padSlots: flat }), /five banks/);
});

test("store replacement rejects pad I/O and replacement re-entry", () => {
  assert.equal(storeReplacementBlocked({ padIoBusy: false, storeReplaceBusy: false }), false);
  assert.equal(storeReplacementBlocked({ padIoBusy: true, storeReplaceBusy: false }), true);
  assert.equal(storeReplacementBlocked({ padIoBusy: false, storeReplaceBusy: true }), true);
});

test("binding keys enforce bank/preset/action bounds", () => {
  assert.deepEqual(parseBindingKey("2-1"), {
    bank: 0,
    preset: 2,
    action: 1,
    legacy: true,
  });
  assert.equal(bankBindingKey(4, 5, 2), "4-5-2");
  assert.equal(parseBindingKey("5-0-0"), null);
  assert.equal(parseBindingKey("0-6-0"), null);
  assert.equal(parseBindingKey("0-0-3"), null);
});

test("Info version detection keeps v0.2 on the legacy single-bank path", () => {
  assert.equal(infoSupportsBanks("Cyberdeck Pad Hybrid v0.3.0"), true);
  assert.equal(infoSupportsBanks("Cyberdeck Pad Hybrid v0.3.99"), true);
  assert.equal(infoSupportsBanks("Cyberdeck Pad Hybrid v0.4.0"), false);
  assert.equal(infoSupportsBanks("Cyberdeck Pad Hybrid v1.0.0"), false);
  assert.equal(infoSupportsBanks("Cyberdeck Pad Hybrid v0.2.9"), false);
  assert.equal(infoSupportsBanks("0.3.0"), false);
  assert.equal(infoSupportsBanks("Cyberdeck Pad Hybrid v0.3"), false);
  assert.equal(infoSupportsBanks("Cyberdeck Pad Hybrid v0.3.0 trailing"), false);
  assert.equal(infoSupportsBanks("garbage"), false);
});
