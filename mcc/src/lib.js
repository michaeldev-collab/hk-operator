// lib.js — pure, DOM-free logic. Importable in the browser AND in Node tests.
// No localStorage, no document here on purpose, so it is unit-testable.

export const CATEGORIES = [
  "URLs",
  "Terminal Commands",
  "Project Paths",
  "Cursor Prompts",
  "Prompts",
  "Client/Admin Tools",
];

export const ACTION_TYPES = ["url", "command", "prompt", "path", "note", "composer"];

export function defaultComposers() {
  return {
    ai: {
      // Public portfolio defaults — no private board slash names.
      commands: ["/help", "/review", "/plan"],
      separator: " ",
      timeoutMs: 4000,
      resetOn: ["timeout", "explicitClear"],
    },
  };
}

/**
 * Stable FNV-1a 64-bit fingerprint of a command action value (P3-03).
 * Must match Rust `command_value_fingerprint` in dispatch.rs.
 */
export function commandValueFingerprint(value) {
  let hash = 0xcbf29ce484222325n;
  const bytes = new TextEncoder().encode(String(value ?? ""));
  const prime = 0x100000001b3n;
  const mask = 0xffffffffffffffffn;
  for (const b of bytes) {
    hash ^= BigInt(b);
    hash = (hash * prime) & mask;
  }
  return hash.toString(16).padStart(16, "0");
}

/** Normalize store allowlist: id → fingerprint object (legacy arrays dropped). */
export function normalizeAllowedCommands(raw) {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  const out = {};
  for (const [id, fp] of Object.entries(raw)) {
    if (typeof fp === "string" && fp) out[id] = fp;
  }
  return out;
}

export function isCommandAllowed(action, allowedCommands = {}) {
  if (!action || action.type !== "command") return false;
  const fp = allowedCommands[action.id];
  if (!fp) return false;
  return fp === commandValueFingerprint(action.value);
}

/**
 * Redact a Bluetooth MAC for UI display (P3-08).
 * Must match Rust `redact_ble_address` in cyberdeck-ble (last octet kept).
 */
export function redactBleAddress(addr) {
  const trimmed = String(addr ?? "").trim();
  if (!trimmed) return "(no address)";
  const parts = trimmed.split(/[:\-]/).filter(Boolean);
  if (
    parts.length === 6 &&
    parts.every((p) => /^[0-9A-Fa-f]{2}$/.test(p))
  ) {
    return `**:**:**:**:**:${parts[5].toUpperCase()}`;
  }
  return "**:**:**:**:**:**";
}

export function normalizeComposers(raw) {
  const base = defaultComposers();
  if (!raw || typeof raw !== "object") return base;
  const out = { ...base };
  for (const [id, cfg] of Object.entries(raw)) {
    if (!cfg || typeof cfg !== "object") continue;
    const commands = Array.isArray(cfg.commands)
      ? cfg.commands.map((c) => String(c).trim()).filter(Boolean)
      : base.ai.commands;
    out[id] = {
      commands: commands.length ? commands : ["/help"],
      separator: String(cfg.separator ?? " "),
      timeoutMs: Math.max(500, Number(cfg.timeoutMs) || 4000),
      resetOn: Array.isArray(cfg.resetOn) ? cfg.resetOn.map(String) : ["timeout", "explicitClear"],
    };
  }
  return out;
}

// Tiny, dependency-free unique id.
export function uid() {
  return "a_" + Date.now().toString(36) + "_" + Math.random().toString(36).slice(2, 8);
}

// Validate a raw action. Returns { ok, errors: [string] }.
export function validateAction(a) {
  const errors = [];
  if (!a || typeof a !== "object") return { ok: false, errors: ["action is not an object"] };
  if (!a.name || !String(a.name).trim()) errors.push("name is required");
  if (!CATEGORIES.includes(a.category)) errors.push(`category must be one of: ${CATEGORIES.join(", ")}`);
  if (!ACTION_TYPES.includes(a.type)) errors.push(`type must be one of: ${ACTION_TYPES.join(", ")}`);
  if (!a.value || !String(a.value).trim()) errors.push("content/value is required");
  if (a.type === "url" && a.value && !/^https?:\/\//i.test(String(a.value).trim())) {
    errors.push("url actions must start with http:// or https://");
  }
  return { ok: errors.length === 0, errors };
}

// Take user input and produce a complete, stored-shape action.
// Preserves id/createdAt/lastUsed/favorite when editing an existing one.
export function normalizeAction(input, existing = null) {
  const now = new Date().toISOString();
  const tags = Array.isArray(input.tags)
    ? input.tags
    : String(input.tags || "")
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean);
  return {
    id: existing?.id || input.id || uid(),
    name: String(input.name || "").trim(),
    category: input.category,
    description: String(input.description || "").trim(),
    type: input.type,
    value: String(input.value || "").trim(),
    tags,
    favorite: Boolean(input.favorite ?? existing?.favorite ?? false),
    lastUsed: input.lastUsed ?? existing?.lastUsed ?? null,
    createdAt: existing?.createdAt || input.createdAt || now,
  };
}

// Filter + sort actions for display.
// opts: { query, category, type, favoritesOnly }
export function filterActions(actions, opts = {}) {
  const q = String(opts.query || "").trim().toLowerCase();
  let out = actions.filter((a) => {
    if (opts.category && opts.category !== "all" && a.category !== opts.category) return false;
    if (opts.type && opts.type !== "all" && a.type !== opts.type) return false;
    if (opts.favoritesOnly && !a.favorite) return false;
    if (!q) return true;
    const hay = [a.name, a.description, a.value, a.category, a.type, (a.tags || []).join(" ")]
      .join(" ")
      .toLowerCase();
    return hay.includes(q);
  });
  // Favorites first, then most-recently-used, then name.
  out.sort((x, y) => {
    if (x.favorite !== y.favorite) return x.favorite ? -1 : 1;
    const lx = x.lastUsed || "";
    const ly = y.lastUsed || "";
    if (lx !== ly) return lx > ly ? -1 : 1;
    return (x.name || "").localeCompare(y.name || "");
  });
  return out;
}
