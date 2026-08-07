// Pure v0.3 bank/store migration helpers. DOM-free for Node tests.

export const STORE_SCHEMA_VERSION = 3;
export const BANK_COUNT = 5;
export const PRESET_COUNT = 6;
export const ACTION_COUNT = 3;
export const SLOT_COUNT = PRESET_COUNT * ACTION_COUNT;
export const SLOT_LABEL_MAX_BYTES = 23;

const labelEncoder = new TextEncoder();
const labelDecoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

export const BANKS = [
  { name: "daily", color: "green" },
  { name: "apps", color: "amber" },
  { name: "web", color: "magenta" },
  { name: "3dl ops", color: "red" },
  { name: "spare", color: "white" },
];

export function emptySlotBank() {
  return Array.from({ length: SLOT_COUNT }, () => ({
    mode: 0,
    mod: 0,
    key: 0,
    label: "",
  }));
}

export function emptyPadBanks() {
  return Array.from({ length: BANK_COUNT }, () => emptySlotBank());
}

export function truncatePadLabel(value) {
  const label = String(value ?? "");
  if (label.includes("\0")) {
    throw new TypeError("Pad label cannot contain NUL");
  }
  const bytes = labelEncoder.encode(label);
  if (bytes.length <= SLOT_LABEL_MAX_BYTES) {
    return labelDecoder.decode(bytes);
  }
  let end = SLOT_LABEL_MAX_BYTES;
  while (end > 0 && (bytes[end] & 0xc0) === 0x80) end--;
  return labelDecoder.decode(bytes.subarray(0, end));
}

function cloneSlot(slot) {
  let label;
  try {
    label = truncatePadLabel(slot?.label);
  } catch {
    return null;
  }
  return {
    mode: Number(slot?.mode) === 1 ? 1 : 0,
    mod: Number(slot?.mod) & 0xff,
    key: Number(slot?.key) & 0xff,
    label,
  };
}

function cloneBank(bank) {
  const slots = bank.map(cloneSlot);
  return slots.every(Boolean) ? slots : null;
}

export function normalizePadBanks(raw) {
  if (!Array.isArray(raw)) return null;
  if (raw.length === SLOT_COUNT && raw.every((slot) => !Array.isArray(slot))) {
    const bank = cloneBank(raw);
    if (!bank) return null;
    const banks = emptyPadBanks();
    banks[0] = bank;
    return banks;
  }
  if (
    raw.length === BANK_COUNT &&
    raw.every((bank) => Array.isArray(bank) && bank.length === SLOT_COUNT)
  ) {
    const banks = raw.map(cloneBank);
    return banks.every(Boolean) ? banks : null;
  }
  return null;
}

export function validPadBanks(raw) {
  return (
    Array.isArray(raw) &&
    raw.length === BANK_COUNT &&
    raw.every((bank) => Array.isArray(bank) && bank.length === SLOT_COUNT)
  );
}

export function storeReplacementBlocked(state) {
  return Boolean(state?.padIoBusy || state?.storeReplaceBusy);
}

export function buildPortablePadState(padBindings, padSlots) {
  if (padSlots != null && !validPadBanks(padSlots)) {
    throw new TypeError("Cannot export malformed padSlots");
  }
  const normalizedSlots = padSlots == null ? null : normalizePadBanks(padSlots);
  if (padSlots != null && !normalizedSlots) {
    throw new TypeError("Cannot export padSlots with an invalid label");
  }
  return {
    schemaVersion: STORE_SCHEMA_VERSION,
    padBindings: migratePadBindings(padBindings),
    padSlots: normalizedSlots,
  };
}

export function readPortablePadState(data) {
  if (!data || Array.isArray(data) || typeof data !== "object") {
    return {
      hasPadBindings: false,
      hasPadSlots: false,
      padBindings: null,
      padSlots: null,
    };
  }
  const schemaVersion = data.schemaVersion ?? 0;
  if (!Number.isInteger(schemaVersion) || schemaVersion < 0) {
    throw new TypeError("Imported schemaVersion must be a non-negative integer");
  }
  if (schemaVersion > STORE_SCHEMA_VERSION) {
    throw new TypeError(
      `Imported schema ${schemaVersion} is newer than supported schema ${STORE_SCHEMA_VERSION}`
    );
  }
  const hasPadBindings = Object.prototype.hasOwnProperty.call(data, "padBindings");
  const hasPadSlots = Object.prototype.hasOwnProperty.call(data, "padSlots");
  let padSlots = null;
  if (hasPadSlots && data.padSlots != null) {
    padSlots = normalizePadBanks(data.padSlots);
    if (!padSlots) {
      throw new TypeError("Imported padSlots must be 18 legacy slots or five banks of 18");
    }
  }
  return {
    hasPadBindings,
    hasPadSlots,
    padBindings: hasPadBindings ? migratePadBindings(data.padBindings) : null,
    padSlots,
  };
}

export function parseBindingKey(key) {
  const parts = String(key).split("-");
  let bank;
  let preset;
  let action;
  let legacy = false;
  if (parts.length === 2) {
    [preset, action] = parts.map(Number);
    bank = 0;
    legacy = true;
  } else if (parts.length === 3) {
    [bank, preset, action] = parts.map(Number);
  } else {
    return null;
  }
  if (
    !Number.isInteger(bank) ||
    !Number.isInteger(preset) ||
    !Number.isInteger(action) ||
    bank < 0 || bank >= BANK_COUNT ||
    preset < 0 || preset >= PRESET_COUNT ||
    action < 0 || action >= ACTION_COUNT
  ) {
    return null;
  }
  return { bank, preset, action, legacy };
}

export function bankBindingKey(bank, preset, action) {
  return `${bank}-${preset}-${action}`;
}

export function infoSupportsBanks(info) {
  const match = String(info ?? "").match(
    /^Cyberdeck Pad Hybrid v(\d+)\.(\d+)\.(\d+)$/
  );
  if (!match) return false;
  const major = Number(match[1]);
  const minor = Number(match[2]);
  return major === 0 && minor === 3;
}

export function migratePadBindings(raw) {
  const bindings = raw && typeof raw === "object" ? raw : {};
  const out = {};
  // Explicit v0.3 keys win over a colliding legacy bank-0 key.
  for (const [key, value] of Object.entries(bindings)) {
    const parsed = parseBindingKey(key);
    if (parsed && !parsed.legacy) out[key] = value;
  }
  for (const [key, value] of Object.entries(bindings)) {
    const parsed = parseBindingKey(key);
    if (!parsed) {
      if (!(key in out)) out[key] = value;
      continue;
    }
    const canonical = bankBindingKey(parsed.bank, parsed.preset, parsed.action);
    if (!(canonical in out)) out[canonical] = value;
  }
  return out;
}
