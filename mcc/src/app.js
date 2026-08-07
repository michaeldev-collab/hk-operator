// app.js — UI + storage glue. Browser uses localStorage; Tauri uses ~/.config store + BLE.
import {
  CATEGORIES,
  ACTION_TYPES,
  validateAction,
  normalizeAction,
  filterActions,
  uid,
  defaultComposers,
  normalizeComposers,
} from "./lib.js";
import { SEED_ACTIONS, SEED_PAD_BINDINGS } from "./seed.js";
import {
  STORE_SCHEMA_VERSION,
  BANK_COUNT,
  PRESET_COUNT,
  ACTION_COUNT,
  SLOT_COUNT,
  BANKS,
  emptySlotBank,
  emptyPadBanks,
  normalizePadBanks,
  validPadBanks,
  bankBindingKey,
  buildPortablePadState,
  infoSupportsBanks,
  migratePadBindings,
  readPortablePadState,
  storeReplacementBlocked,
  truncatePadLabel,
} from "./pad_banks.js";

const STORAGE_KEY = "3dl.macro.actions.v1";
const BTN_NAMES = ["B2", "B4", "B5"];

/**
 * Preset 3 (index 2) is a host bridge: the pad always types Ctrl+Alt+1/2/3
 * (KDE → MCC fire API). The UI only edits which MCC action that chord runs.
 * Literal HID editing stays on other presets.
 */
const HOST_BRIDGE_PRESET = 2;
const BRIDGE_MOD = 0x05; // Ctrl+Alt
const BRIDGE_KEYS = [0x1e, 0x1f, 0x20]; // HID 1 / 2 / 3
const PRESET_NAME_MAX = 32;

/** LED legend for UI (matches firmware). */
const PRESET_LED = [
  "● Red",
  "● Green",
  "● Blue",
  "●● Red+Green",
  "●● Green+Blue",
  "●● Red+Blue",
];

function defaultPresetNames() {
  return Array.from({ length: PRESET_COUNT }, (_, i) => `Preset ${i + 1}`);
}

function normalizePresetNames(names) {
  const out = defaultPresetNames();
  if (!Array.isArray(names)) return out;
  for (let i = 0; i < PRESET_COUNT; i++) {
    const n = String(names[i] ?? "").trim();
    if (n) out[i] = n.slice(0, PRESET_NAME_MAX);
  }
  return out;
}

function presetDisplayName(preset) {
  const p = Number(preset);
  return state.padPresetNames?.[p] || `Preset ${p + 1}`;
}

function isHostBridgePreset(preset, bank = state.padBank) {
  return Number(bank) === 0 && Number(preset) === HOST_BRIDGE_PRESET;
}

function bridgeChordForAction(actionIdx) {
  return {
    mode: 0,
    mod: BRIDGE_MOD,
    key: BRIDGE_KEYS[actionIdx] ?? BRIDGE_KEYS[0],
  };
}

/** Keep the historical KDE bridge in bank 0 only; other banks remain literal/macro. */
function ensureHostBridgeSlots(banks) {
  if (!validPadBanks(banks)) return banks;
  const slots = banks[0];
  for (let a = 0; a < ACTION_COUNT; a++) {
    const i = HOST_BRIDGE_PRESET * ACTION_COUNT + a;
    const key = bankBindingKey(0, HOST_BRIDGE_PRESET, a);
    const bound = state.actions.find((x) => x.id === state.padBindings[key]);
    const chord = bridgeChordForAction(a);
    const prev = slots[i] || {};
    slots[i] = {
      ...chord,
      label: truncatePadLabel(
        prev.label || (bound ? bound.name : `Ctrl+Alt+${a + 1}`)
      ),
    };
  }
  return banks;
}

function selectedBankSlots(create = false) {
  if (!validPadBanks(state.padSlots)) {
    if (!create) return emptySlotBank();
    state.padSlots = emptyPadBanks();
  }
  return state.padSlots[state.padBank];
}

function selectedBindingKey(preset, action, bank = state.padBank) {
  return bankBindingKey(bank, preset, action);
}

const isTauri = () =>
  typeof window !== "undefined" && !!(window.__TAURI_INTERNALS__ || window.__TAURI__);

function tauriInvoke(cmd, args) {
  const core = window.__TAURI__?.core;
  if (!core?.invoke) throw new Error("Tauri invoke unavailable");
  return core.invoke(cmd, args);
}

function tauriListen(event, handler) {
  const eventApi = window.__TAURI__?.event;
  if (!eventApi?.listen) return Promise.resolve(() => {});
  return eventApi.listen(event, (e) => handler(e.payload));
}

const state = {
  actions: [],
  padBindings: {}, // "bank-preset-action" -> actionId
  padPresetNames: defaultPresetNames(),
  composers: defaultComposers(),
  allowedCommands: new Set(),
  query: "",
  category: "all",
  type: "all",
  favoritesOnly: false,
  editingId: null,
  // pad
  padSlots: null, // five arrays of 18 {mode, mod, key, label}
  padBank: 0,
  padSupportsBanks: null,
  padProtocolCompatible: null,
  padIoBusy: false,
  storeReplaceBusy: false,
  padAddress: null,
  padTransport: null, // "dongle" | "bluez" | null
  bluezBlocked: null, // true = BlueZ parked (dongle-friendly)
  padListening: false,
  editingSlot: null, // {bank, preset, action}
};

function dedupeById(actions) {
  const map = new Map();
  for (let a of actions) {
    if (map.has(a.id)) a = { ...a, id: uid() };
    map.set(a.id, a);
  }
  return [...map.values()];
}

function browserLoad() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        return {
          actions: dedupeById(parsed.map((a) => normalizeAction(a))),
          padBindings: {},
          padPresetNames: defaultPresetNames(),
          composers: defaultComposers(),
          allowedCommands: new Set(),
          padSlots: null,
        };
      }
      if (parsed && Array.isArray(parsed.actions)) {
        return {
          actions: dedupeById(parsed.actions.map((a) => normalizeAction(a))),
          padBindings: migratePadBindings(parsed.padBindings),
          padPresetNames: normalizePresetNames(parsed.padPresetNames),
          composers: normalizeComposers(parsed.composers),
          allowedCommands: new Set(parsed.allowedCommands || []),
          padSlots: normalizePadBanks(parsed.padSlots),
        };
      }
    }
  } catch (e) {
    console.warn("load failed, reseeding", e);
  }
  const seeded = SEED_ACTIONS.map((a) => normalizeAction(a));
  const padBindings = migratePadBindings(SEED_PAD_BINDINGS);
  // Remap seed binding ids to actual seeded action ids by stable name.
  const byName = Object.fromEntries(seeded.map((a) => [a.name, a.id]));
  const remapped = {};
  for (const [k, nameOrId] of Object.entries(padBindings)) {
    remapped[k] = byName[nameOrId] || nameOrId;
  }
  const padPresetNames = defaultPresetNames();
  const composers = defaultComposers();
  const store = {
    schemaVersion: STORE_SCHEMA_VERSION,
    actions: seeded,
    padBindings: remapped,
    padPresetNames,
    composers,
    allowedCommands: [],
    padSlots: null,
  };
  localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
  return {
    actions: seeded,
    padBindings: remapped,
    padPresetNames,
    composers,
    allowedCommands: new Set(),
    padSlots: null,
  };
}

function browserSave(snapshot) {
  localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify({
      actions: snapshot.actions,
      schemaVersion: STORE_SCHEMA_VERSION,
      padBindings: snapshot.padBindings,
      padPresetNames: snapshot.padPresetNames,
      composers: snapshot.composers,
      allowedCommands: [...snapshot.allowedCommands],
      padSlots: snapshot.padSlots,
    })
  );
}

async function desktopLoad() {
  const store = await tauriInvoke("get_store");
  let actions = Array.isArray(store.actions) ? store.actions.map(fromRustAction) : [];
  let padBindings = migratePadBindings(store.padBindings);
  let padPresetNames = normalizePresetNames(store.padPresetNames);
  let composers = normalizeComposers(store.composers);
  let allowedCommands = new Set(store.allowedCommands || []);
  let padSlots = normalizePadBanks(store.padSlots);

  if (actions.length === 0) {
    actions = SEED_ACTIONS.map((a) => normalizeAction(a));
    const byName = Object.fromEntries(actions.map((a) => [a.name, a.id]));
    padBindings = {};
    for (const [k, nameOrId] of Object.entries(migratePadBindings(SEED_PAD_BINDINGS))) {
      padBindings[k] = byName[nameOrId] || nameOrId;
    }
    padPresetNames = defaultPresetNames();
    composers = defaultComposers();
    await desktopSave(
      actions,
      padBindings,
      allowedCommands,
      padPresetNames,
      composers,
      padSlots
    );
  }
  return { actions, padBindings, padPresetNames, composers, allowedCommands, padSlots };
}

function fromRustAction(a) {
  return normalizeAction({
    id: a.id,
    name: a.name,
    category: a.category,
    description: a.description,
    type: a.type,
    value: a.value,
    tags: a.tags || [],
    favorite: a.favorite,
    lastUsed: a.lastUsed,
    createdAt: a.createdAt,
  });
}

function toRustAction(a) {
  return {
    id: a.id,
    name: a.name,
    category: a.category,
    description: a.description,
    type: a.type,
    value: a.value,
    tags: a.tags || [],
    favorite: !!a.favorite,
    lastUsed: a.lastUsed,
    createdAt: a.createdAt,
  };
}

async function desktopSave(
  actions = state.actions,
  padBindings = state.padBindings,
  allowed = state.allowedCommands,
  padPresetNames = state.padPresetNames,
  composers = state.composers,
  padSlots = state.padSlots
) {
  await tauriInvoke("save_store", {
    store: {
      schemaVersion: STORE_SCHEMA_VERSION,
      actions: actions.map(toRustAction),
      padBindings: migratePadBindings(padBindings),
      padPresetNames: normalizePresetNames(padPresetNames),
      composers: normalizeComposers(composers),
      allowedCommands: [...allowed],
      padSlots: validPadBanks(padSlots) ? padSlots : null,
    },
  });
}

function captureSaveSnapshot() {
  return {
    actions: state.actions.map((action) => ({
      ...action,
      tags: [...(action.tags || [])],
    })),
    padBindings: { ...migratePadBindings(state.padBindings) },
    allowedCommands: new Set(state.allowedCommands),
    padPresetNames: [...normalizePresetNames(state.padPresetNames)],
    composers: structuredClone(normalizeComposers(state.composers)),
    padSlots: validPadBanks(state.padSlots)
      ? state.padSlots.map((bank) => bank.map((slot) => ({ ...slot })))
      : null,
  };
}

let saveTail = Promise.resolve();

function persistSaveSnapshot(snapshot) {
  return isTauri()
    ? desktopSave(
        snapshot.actions,
        snapshot.padBindings,
        snapshot.allowedCommands,
        snapshot.padPresetNames,
        snapshot.composers,
        snapshot.padSlots
      )
    : Promise.resolve(browserSave(snapshot));
}

function applySaveSnapshot(snapshot) {
  state.actions = snapshot.actions.map((action) => ({
    ...action,
    tags: [...(action.tags || [])],
  }));
  state.padBindings = { ...snapshot.padBindings };
  state.allowedCommands = new Set(snapshot.allowedCommands);
  state.padPresetNames = [...snapshot.padPresetNames];
  state.composers = structuredClone(snapshot.composers);
  state.padSlots = snapshot.padSlots
    ? snapshot.padSlots.map((bank) => bank.map((slot) => ({ ...slot })))
    : null;
}

function save() {
  if (state.storeReplaceBusy) {
    // Replacement operations reload the authoritative backend store before
    // releasing the UI. Dropping incidental saves here prevents an old UI
    // snapshot from overwriting the imported profile mid-transaction.
    console.warn("store save skipped while profile replacement is in progress");
    return Promise.resolve();
  }
  // Capture at the mutation boundary, then serialize whole-store writes in the
  // same order as UI events. Backend locking alone cannot prevent an older
  // fire-and-forget snapshot from landing after a newer edit.
  const snapshot = captureSaveSnapshot();
  const result = saveTail.then(() => persistSaveSnapshot(snapshot));
  saveTail = result.catch((error) => {
    console.error("store save failed", error);
  });
  return result;
}

const $ = (sel) => document.querySelector(sel);
const el = (tag, props = {}, ...kids) => {
  const node = document.createElement(tag);
  Object.assign(node, props);
  for (const k of kids) node.append(k);
  return node;
};

function toast(msg) {
  const t = $("#toast");
  t.textContent = msg;
  t.classList.add("show");
  clearTimeout(toast._t);
  toast._t = setTimeout(() => t.classList.remove("show"), 1800);
}

function markUsed(action) {
  action.lastUsed = new Date().toISOString();
  save();
}

async function copyValue(action) {
  try {
    await navigator.clipboard.writeText(action.value);
    markUsed(action);
    toast(`Copied: ${action.name}`);
  } catch (e) {
    const ta = el("textarea", { value: action.value });
    document.body.append(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
    markUsed(action);
    toast(`Copied: ${action.name}`);
  }
}

function openUrl(action) {
  if (!/^https?:\/\//i.test(action.value)) {
    toast("Blocked: not an http(s) URL");
    return;
  }
  markUsed(action);
  window.open(action.value, "_blank", "noopener");
}

async function runAction(action) {
  if (!isTauri()) {
    if (action.type === "url") return openUrl(action);
    return copyValue(action);
  }
  if (action.type === "command" && !state.allowedCommands.has(action.id)) {
    if (
      !confirm(
        `Allow shell execution for "${action.name}"?\n\n${action.value}\n\nThis is stored until you clear allow-list data.`
      )
    ) {
      toast("Command not allowed");
      return;
    }
    state.allowedCommands.add(action.id);
    await tauriInvoke("allow_command", { actionId: action.id });
    await save();
  }
  try {
    const msg = await tauriInvoke("execute_action_id", { actionId: action.id });
    markUsed(action);
    toast(msg);
  } catch (e) {
    toast(String(e));
  }
}

function valuePlaceholder(type) {
  return (
    {
      url: "https://…",
      command: "git status && git log --oneline -5",
      prompt: "Write a concise summary of…",
      path: "~/projects/…",
      note: "free text…",
      composer: "ai",
      "composer-commit": "ai",
      "composer-reset": "ai",
    }[type] || ""
  );
}

function render() {
  renderFilters();
  renderCards();
  renderCount();
  renderPadGrid();
}

function renderCount() {
  const total = state.actions.length;
  const shown = currentList().length;
  $("#count").textContent = `${shown} shown / ${total} total`;
}

function currentList() {
  return filterActions(state.actions, {
    query: state.query,
    category: state.category,
    type: state.type,
    favoritesOnly: state.favoritesOnly,
  });
}

function renderFilters() {
  const sel = $("#categoryFilter");
  if (sel.dataset.built) return;
  sel.append(el("option", { value: "all", textContent: "All categories" }));
  for (const c of CATEGORIES) sel.append(el("option", { value: c, textContent: c }));
  const tsel = $("#typeFilter");
  tsel.append(el("option", { value: "all", textContent: "All types" }));
  for (const t of ACTION_TYPES) tsel.append(el("option", { value: t, textContent: t }));
  sel.dataset.built = "1";
}

function badge(text, cls = "") {
  return el("span", { className: `badge ${cls}`, textContent: text });
}

function renderCards() {
  const grid = $("#grid");
  grid.replaceChildren();
  const list = currentList();
  if (list.length === 0) {
    grid.append(
      el("p", {
        className: "empty",
        textContent: "No actions match. Try clearing filters or add one.",
      })
    );
    return;
  }
  for (const a of list) {
    const card = el("div", { className: "card" });
    const head = el("div", { className: "card-head" });
    head.append(el("h3", { className: "card-title", textContent: a.name }));
    const fav = el("button", {
      className: "fav" + (a.favorite ? " on" : ""),
      title: "Toggle favorite",
      textContent: a.favorite ? "★" : "☆",
      onclick: () => {
        a.favorite = !a.favorite;
        save();
        render();
      },
    });
    fav.setAttribute("aria-pressed", String(a.favorite));
    head.append(fav);
    card.append(head);

    const meta = el("div", { className: "meta" });
    meta.append(badge(a.category, "cat"));
    meta.append(badge(a.type, "type type-" + a.type));
    card.append(meta);
    if (a.description) card.append(el("p", { className: "desc", textContent: a.description }));
    card.append(el("pre", { className: "value", textContent: a.value }));
    if (a.tags?.length) {
      const tagWrap = el("div", { className: "tags" });
      for (const t of a.tags) tagWrap.append(badge("#" + t, "tag"));
      card.append(tagWrap);
    }

    const foot = el("div", { className: "card-foot" });
    const primary =
      a.type === "url"
        ? el("button", { className: "btn primary", textContent: "Open ↗", onclick: () => openUrl(a) })
        : isTauri() && (a.type === "command" || a.type === "path")
          ? el("button", { className: "btn primary", textContent: "Run", onclick: () => runAction(a) })
          : el("button", { className: "btn primary", textContent: "Copy", onclick: () => copyValue(a) });
    foot.append(primary);
    if (a.type === "url") {
      foot.append(el("button", { className: "btn", textContent: "Copy", onclick: () => copyValue(a) }));
    } else if (isTauri() && (a.type === "prompt" || a.type === "note")) {
      foot.append(el("button", { className: "btn", textContent: "Copy", onclick: () => copyValue(a) }));
    }
    if (isTauri() && a.type === "command") {
      const allowed = state.allowedCommands.has(a.id);
      foot.append(
        el("button", {
          className: "btn" + (allowed ? " on" : ""),
          textContent: allowed ? "Allowed ✓" : "Allow shell",
          onclick: async () => {
            if (allowed) return;
            if (!confirm(`Allow shell for "${a.name}"?\n\n${a.value}`)) return;
            state.allowedCommands.add(a.id);
            await tauriInvoke("allow_command", { actionId: a.id });
            await save();
            render();
            toast("Command allowed");
          },
        })
      );
    }
    foot.append(el("button", { className: "btn", textContent: "Edit", onclick: () => openForm(a.id) }));
    foot.append(
      el("button", {
        className: "btn danger",
        textContent: "Delete",
        onclick: () => {
          if (confirm(`Delete "${a.name}"?`)) {
            state.actions = state.actions.filter((x) => x.id !== a.id);
            save();
            render();
            toast("Deleted");
          }
        },
      })
    );
    foot.append(
      el("span", {
        className: "used",
        textContent: a.lastUsed ? "used " + new Date(a.lastUsed).toLocaleString() : "never used",
      })
    );
    card.append(foot);
    grid.append(card);
  }
}

/** Build 6 preset columns in JS so desktop cannot stick on a cached 3-column HTML shell. */
function ensurePadGridStructure() {
  let mount = $("#padGridMount");
  const panel = $("#padPanel");
  if (!mount && panel) {
    mount = el("div", { id: "padGridMount" });
    const hint = $("#padHint");
    if (hint) panel.insertBefore(mount, hint);
    else panel.append(mount);
  }
  if (!mount) return;

  // Drop any leftover static columns from an older HTML shell.
  if (panel) {
    for (const node of [...panel.querySelectorAll(".pad-row-title, .pad-grid, .pad-preset")]) {
      if (!mount.contains(node)) node.remove();
    }
  }

  const title = panel?.querySelector(".pad-head h2");
  if (title) {
    const bank = BANKS[state.padBank] || BANKS[0];
    title.textContent = `Cyberdeck Pad · bank ${state.padBank + 1}/${BANK_COUNT} · ${bank.name}`;
  }
  const countLine = panel?.querySelector(".pad-preset-count");
  if (countLine) {
    countLine.textContent =
      "Name presets below — click a title to rename · P4–P6 use dual LEDs";
  }

  const existing = mount.querySelectorAll(".pad-preset[data-preset]");
  if (existing.length === PRESET_COUNT) {
    for (const col of existing) {
      const p = Number(col.getAttribute("data-preset"));
      const input = col.querySelector("input.preset-name");
      if (input && document.activeElement !== input) {
        input.value = presetDisplayName(p);
      }
      const ledLine = col.querySelector(".preset-led");
      if (ledLine) {
        const bridgeNote = isHostBridgePreset(p, state.padBank) ? " · MCC bridge" : "";
        ledLine.textContent = `P${p + 1} · ${PRESET_LED[p] || ""}${bridgeNote}`;
      }
    }
    return;
  }

  mount.replaceChildren();

  const rows = [
    { title: "Presets 1–3 · single LED", start: 0, end: 3 },
    { title: "Presets 4–6 · dual LED", start: 3, end: PRESET_COUNT },
  ];
  for (const row of rows) {
    if (row.start >= PRESET_COUNT) break;
    mount.append(
      el("h3", { className: "pad-row-title", textContent: row.title })
    );
    const grid = el("div", { className: "pad-grid" });
    grid.setAttribute("aria-label", row.title);
    for (let p = row.start; p < Math.min(row.end, PRESET_COUNT); p++) {
      const col = el("div", { className: "pad-preset" });
      col.setAttribute("data-preset", String(p));
      const led = PRESET_LED[p] || "";
      const bridgeNote = isHostBridgePreset(p, state.padBank) ? " · MCC bridge" : "";
      const nameInput = el("input", {
        className: "preset-name",
        type: "text",
        maxLength: PRESET_NAME_MAX,
        value: presetDisplayName(p),
        title: "Rename this preset",
        spellcheck: false,
      });
      nameInput.setAttribute("aria-label", `Name for hardware preset ${p + 1}`);
      nameInput.addEventListener("change", () => commitPresetName(p, nameInput.value));
      nameInput.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          nameInput.blur();
        }
      });
      col.append(nameInput);
      col.append(
        el("p", {
          className: "preset-led",
          textContent: `P${p + 1} · ${led}${bridgeNote}`,
        })
      );
      const slotList = el("div", { className: "pad-slot-list" });
      slotList.setAttribute("data-slots", "");
      col.append(slotList);
      grid.append(col);
    }
    mount.append(grid);
  }
}

function commitPresetName(preset, raw) {
  const p = Number(preset);
  if (Number.isNaN(p) || p < 0 || p >= PRESET_COUNT) return;
  const names = normalizePresetNames(state.padPresetNames);
  const trimmed = String(raw || "").trim().slice(0, PRESET_NAME_MAX);
  names[p] = trimmed || `Preset ${p + 1}`;
  state.padPresetNames = names;
  save();
  toast(`Preset ${p + 1} → ${names[p]}`);
}

function renderBankSelector() {
  const select = $("#padBankSelect");
  if (!select) return;
  if (select.options.length !== BANK_COUNT) {
    select.replaceChildren(
      ...BANKS.map((bank, i) =>
        el("option", {
          value: String(i),
          textContent: `${i + 1} · ${bank.name} · ${bank.color}`,
        })
      )
    );
  }
  for (let i = 0; i < select.options.length; i++) {
    select.options[i].disabled = state.padSupportsBanks === false && i > 0;
  }
  select.value = String(state.padBank);
}

async function selectPadBank(event) {
  const bank = Number(event?.target?.value);
  if (!Number.isInteger(bank) || bank < 0 || bank >= BANK_COUNT) return;
  if (state.padIoBusy) {
    renderBankSelector();
    toast("Another pad operation is already in progress");
    return;
  }
  state.padBank = bank;
  renderBankSelector();
  renderPadGrid();
  if (!isTauri()) return;
  setPadIoBusy(true);
  try {
    const slots = await tauriInvoke("pad_read_slots", {
      address: state.padAddress,
      bank,
    });
    if (!validPadBanks(state.padSlots)) state.padSlots = emptyPadBanks();
    state.padSlots[bank] = slots;
    ensureHostBridgeSlots(state.padSlots);
    await save();
    renderPadGrid();
  } catch (e) {
    toast(`Bank ${bank + 1} selected in UI; device read failed: ${e}`);
  } finally {
    setPadIoBusy(false);
  }
}

function renderPadGrid() {
  renderBankSelector();
  ensurePadGridStructure();
  const slots = selectedBankSlots();
  const presets = document.querySelectorAll(".pad-preset[data-preset]");
  for (const col of presets) {
    const p = Number(col.getAttribute("data-preset"));
    const list = col.querySelector("[data-slots]");
    if (!list || Number.isNaN(p)) continue;
    list.replaceChildren();
    for (let a = 0; a < ACTION_COUNT; a++) {
      const slot = slots[p * ACTION_COUNT + a] || { mode: 0, mod: 0, key: 0, label: "" };
      const key = selectedBindingKey(p, a);
      const bound = state.actions.find((x) => x.id === state.padBindings[key]);
      const bridge = isHostBridgePreset(p, state.padBank);
      const modeMacro = !bridge && Number(slot.mode) === 1;
      const btn = el("button", {
        className:
          "pad-slot" + (bridge ? " mode-bridge" : modeMacro ? " mode-macro" : ""),
        type: "button",
        onclick: () => openSlotEditor(p, a),
      });
      btn.append(el("span", { className: "slot-btn", textContent: BTN_NAMES[a] }));
      btn.append(
        el("span", {
          className: "slot-label",
          textContent: bound
            ? bound.name
            : slot.label || (bridge ? "(pick action)" : "(empty)"),
        })
      );
      let modeText;
      if (bridge) {
        modeText = bound
          ? `mcc → ${bound.name}`
          : `mcc unbound (Ctrl+Alt+${a + 1})`;
      } else if (modeMacro) {
        modeText = `macro → ${bound ? bound.name : "unbound"}`;
      } else {
        modeText = `hid 0x${Number(slot.mod).toString(16)}+0x${Number(slot.key).toString(16)}`;
      }
      btn.append(el("span", { className: "slot-mode", textContent: modeText }));
      list.append(btn);
    }
  }
}

function openSlotEditor(preset, action) {
  const bank = state.padBank;
  state.editingSlot = { bank, preset, action };
  const slots = selectedBankSlots();
  const slot = slots[preset * ACTION_COUNT + action] || {
    mode: 0,
    mod: 0,
    key: 0,
    label: "",
  };
  const bridge = isHostBridgePreset(preset, bank);
  $("#slotTitle").textContent = `${BANKS[bank].name} · ${presetDisplayName(preset)} · ${BTN_NAMES[action]}`;
  $("#s_label").value = slot.label || "";
  $("#s_mode").value = bridge ? "0" : String(slot.mode ?? 0);
  $("#s_mod_ctrl").checked = !!(slot.mod & 1);
  $("#s_mod_shift").checked = !!(slot.mod & 2);
  $("#s_mod_alt").checked = !!(slot.mod & 4);
  $("#s_mod_gui").checked = !!(slot.mod & 8);
  $("#s_key").value = slot.key ? "0x" + Number(slot.key).toString(16) : "0";
  const sel = $("#s_action");
  sel.replaceChildren(el("option", { value: "", textContent: "(none)" }));
  for (const a of state.actions) {
    sel.append(el("option", { value: a.id, textContent: `${a.name} [${a.type}]` }));
  }
  const key = selectedBindingKey(preset, action, bank);
  sel.value = state.padBindings[key] || "";
  const modeRow = $("#s_modeRow");
  const bridgeNote = $("#s_bridgeNote");
  if (modeRow) modeRow.hidden = bridge;
  if (bridgeNote) {
    bridgeNote.hidden = !bridge;
    bridgeNote.textContent = `Host bridge: pad always types Ctrl+Alt+${
      action + 1
    }. Pick the MCC action below — Sync keeps the chord fixed.`;
  }
  syncSlotModeFields();
  $("#slotDialog").showModal();
}

function syncSlotModeFields() {
  const { bank, preset } = state.editingSlot || {};
  const bridge = isHostBridgePreset(preset, bank);
  if (bridge) {
    $("#s_hidFields").hidden = true;
    $("#s_macroFields").hidden = false;
    return;
  }
  const macro = $("#s_mode").value === "1";
  $("#s_hidFields").hidden = macro;
  $("#s_macroFields").hidden = !macro;
}

function parseU8(s) {
  const t = String(s || "0").trim();
  if (/^0x/i.test(t)) return parseInt(t.slice(2), 16) || 0;
  return parseInt(t, 10) || 0;
}

function applySlotEditor(ev) {
  ev.preventDefault();
  const { bank, preset, action } = state.editingSlot || {};
  if (preset == null) return;
  if (!validPadBanks(state.padSlots)) state.padSlots = emptyPadBanks();
  const slots = state.padSlots[bank];
  const key = selectedBindingKey(preset, action, bank);
  const actionId = $("#s_action").value;
  let labelIn;
  try {
    labelIn = truncatePadLabel($("#s_label").value);
  } catch (error) {
    toast(String(error));
    return;
  }

  if (isHostBridgePreset(preset, bank)) {
    const bound = state.actions.find((x) => x.id === actionId);
    const chord = bridgeChordForAction(action);
    let label;
    try {
      label = truncatePadLabel(
        labelIn || (bound ? bound.name : `Ctrl+Alt+${action + 1}`)
      );
    } catch (error) {
      toast(String(error));
      return;
    }
    slots[preset * ACTION_COUNT + action] = {
      ...chord,
      label,
    };
    if (actionId) state.padBindings[key] = actionId;
    else delete state.padBindings[key];
  } else {
    let mod = 0;
    if ($("#s_mod_ctrl").checked) mod |= 1;
    if ($("#s_mod_shift").checked) mod |= 2;
    if ($("#s_mod_alt").checked) mod |= 4;
    if ($("#s_mod_gui").checked) mod |= 8;
    const mode = Number($("#s_mode").value) === 1 ? 1 : 0;
    slots[preset * ACTION_COUNT + action] = {
      mode,
      mod,
      key: parseU8($("#s_key").value),
      label: labelIn,
    };
    if (mode === 1 && actionId) state.padBindings[key] = actionId;
    else if (mode === 1 && !actionId) delete state.padBindings[key];
    else if (mode === 0) {
      // Literal HID — binding is on-device; clear host binding if any.
      delete state.padBindings[key];
    }
  }
  save();
  $("#slotDialog").close();
  renderPadGrid();
  toast(
    isHostBridgePreset(preset, bank)
      ? "MCC action bound (Sync optional — chord already on pad)"
      : "Slot updated (Sync to pad to write device)"
  );
}

function updateBluezButton() {
  const btn = $("#padBluezBtn");
  if (!btn) return;
  if (state.bluezBlocked === true) {
    btn.textContent = "BlueZ: off";
    btn.classList.add("held");
    btn.classList.remove("on");
    btn.title =
      "BlueZ is parked (blocked). Click to unblock and let BlueZ take the pad.";
  } else if (state.bluezBlocked === false) {
    btn.textContent = "BlueZ: on";
    btn.classList.add("on");
    btn.classList.remove("held");
    btn.title =
      "BlueZ can claim the pad. Click to park it so the S3 dongle owns the link.";
  } else {
    btn.textContent = "BlueZ: …";
    btn.classList.remove("on", "held");
    btn.title = "BlueZ status unknown — Refresh first";
  }
}

function updatePadProtocolControls() {
  const incompatible = state.padProtocolCompatible === false;
  const busy = state.padIoBusy === true;
  const refresh = $("#padRefreshBtn");
  const sync = $("#padSyncBtn");
  const listen = $("#padListenBtn");
  const bank = $("#padBankSelect");
  const bluez = $("#padBluezBtn");
  if (refresh) refresh.disabled = busy;
  if (bank) bank.disabled = busy;
  if (bluez) bluez.disabled = busy;
  if (sync) {
    sync.disabled = busy || incompatible;
    sync.title = incompatible
      ? "S3 dongle/pad protocol mismatch — flash the matched pair"
      : busy
        ? "Another pad operation is in progress"
        : "";
  }
  if (listen) {
    listen.disabled = busy || incompatible;
    listen.title = incompatible
      ? "Macro forwarding disabled by protocol mismatch"
      : busy
        ? "Another pad operation is in progress"
        : "";
  }
}

function setPadIoBusy(busy) {
  state.padIoBusy = !!busy;
  updatePadProtocolControls();
}

function setStoreReplaceBusy(busy) {
  state.storeReplaceBusy = !!busy;
  document.body.inert = !!busy;
  document.body.setAttribute("aria-busy", String(!!busy));
}

async function refreshPad() {
  if (!isTauri()) return;
  if (state.padIoBusy) {
    toast("Another pad operation is already in progress");
    return;
  }
  let st = null;
  let line = "";
  let restoreBank = state.padBank;
  let successMessage = "";
  setPadIoBusy(true);
  try {
    st = await tauriInvoke("pad_status", { address: state.padAddress });
    if (st.address && st.address !== "via-s3-dongle") {
      state.padAddress = st.address;
    }
    state.padTransport = st.transport || null;
    const protocolCompatible = st.protocolCompatible ?? st.protocol_compatible;
    state.padProtocolCompatible = protocolCompatible !== false;
    state.padSupportsBanks = st.info ? infoSupportsBanks(st.info) : null;
    const selectedBank = st.selectedBank ?? st.selected_bank;
    if (
      state.padSupportsBanks === true &&
      Number.isInteger(selectedBank) &&
      selectedBank >= 0 && selectedBank < BANK_COUNT
    ) {
      state.padBank = selectedBank;
    } else if (state.padSupportsBanks === false) {
      state.padBank = 0;
    }
    restoreBank = state.padBank;
    const bluezBlocked = st.bluezBlocked ?? st.bluez_blocked;
    state.bluezBlocked = typeof bluezBlocked === "boolean" ? bluezBlocked : null;
    updateBluezButton();
    updatePadProtocolControls();
    const via =
      st.transport === "dongle"
        ? "via S3 dongle"
        : st.transport === "bluez"
          ? "via BlueZ"
          : "";
    const bz =
      state.bluezBlocked === true
        ? " · BlueZ parked"
        : state.bluezBlocked === false
          ? " · BlueZ ready"
          : "";
    line = `${st.name || "Cyberdeck Pad"} · ${st.address} · ${
      st.connected ? "connected" : "disconnected"
    }${st.paired ? " · paired" : ""}${via ? " · " + via : ""}${bz}${
      st.info ? " · " + st.info : ""
    }`;
    $("#padStatusLine").textContent = line;
    if (state.padProtocolCompatible === false) {
      $("#padStatusLine").textContent = `${line} · PROTOCOL MISMATCH — sync/listen disabled`;
      renderPadGrid();
      toast("S3 dongle protocol mismatch; refusing BlueZ fallback");
      return;
    }
    const read = await tauriInvoke("pad_read_banks", {
      address: state.padAddress,
    });
    const bankCount = Number(read?.bankCount ?? read?.bank_count);
    const readBanks = Array.isArray(read?.banks) ? read.banks : [];
    if (bankCount !== 1 && bankCount !== BANK_COUNT) {
      throw new Error(`invalid device bank count ${bankCount}`);
    }
    if (readBanks.length !== bankCount) {
      throw new Error(`read ${readBanks.length}/${bankCount} banks`);
    }
    const banks = validPadBanks(state.padSlots)
      ? state.padSlots.map((slots) => slots.map((slot) => ({ ...slot })))
      : emptyPadBanks();
    if (bankCount === BANK_COUNT) {
      if (!validPadBanks(readBanks)) throw new Error("invalid five-bank response");
      for (let bank = 0; bank < BANK_COUNT; bank++) banks[bank] = readBanks[bank];
    } else {
      if (!Array.isArray(readBanks[0]) || readBanks[0].length !== SLOT_COUNT) {
        throw new Error("invalid legacy bank-0 response");
      }
      banks[0] = readBanks[0];
    }
    state.padSupportsBanks = bankCount === BANK_COUNT;
    const restoredBank = Number(read?.restoredBank ?? read?.restored_bank);
    restoreBank =
      Number.isInteger(restoredBank) && restoredBank >= 0 && restoredBank < bankCount
        ? restoredBank
        : 0;
    state.padBank = restoreBank;
    state.padSlots = banks;
    ensureHostBridgeSlots(state.padSlots);
    await save();
    const readTransport = read?.transport || (st.transport === "dongle" ? "S3 dongle" : "pad");
    successMessage = `Read ${bankCount} bank${bankCount === 1 ? "" : "s"} via ${readTransport}`;
  } catch (e) {
    if (!validPadBanks(state.padSlots)) state.padSlots = emptyPadBanks();
    $("#padStatusLine").textContent = line
      ? line +
        (st?.transport === "dongle"
          ? " · slots proxy unavailable (dongle not linked?)"
          : " · GATT unavailable (flash hybrid firmware?)")
      : "Pad not found — connect S3 dongle (or pair Cyberdeck Pad over BlueZ)";
    try {
      const after = await tauriInvoke("pad_status", { address: state.padAddress });
      const selected = Number(after?.selectedBank ?? after?.selected_bank);
      if (Number.isInteger(selected) && selected >= 0 && selected < BANK_COUNT) {
        restoreBank = selected;
      }
    } catch {
      // The transactional backend already attempted restoration; keep the last
      // known selection when transport status is unavailable.
    }
    toast(`Pad refresh stopped: ${e}`);
  } finally {
    state.padBank = restoreBank;
    setPadIoBusy(false);
    renderPadGrid();
    if (successMessage) {
      toast(successMessage);
    }
  }
}

async function toggleBluez() {
  if (!isTauri()) return;
  if (state.padIoBusy) {
    toast("Another pad operation is already in progress");
    return;
  }
  // Parked → release. Active/unknown → park (safer for dongle).
  const wantEnabled = state.bluezBlocked === true;
  let refreshAfter = false;
  setPadIoBusy(true);
  try {
    const st = await tauriInvoke("pad_set_bluez_enabled", {
      enabled: wantEnabled,
      address: state.padAddress,
    });
    if (st.address && st.address !== "via-s3-dongle") {
      state.padAddress = st.address;
    }
    const bluezBlocked = st.bluezBlocked ?? st.bluez_blocked;
    state.bluezBlocked = typeof bluezBlocked === "boolean" ? bluezBlocked : !wantEnabled;
    updateBluezButton();
    toast(
      wantEnabled
        ? "BlueZ released — host may claim the pad"
        : "BlueZ parked — safe for S3 dongle"
    );
    refreshAfter = true;
  } catch (e) {
    toast(String(e));
  } finally {
    setPadIoBusy(false);
  }
  if (refreshAfter) await refreshPad();
}

async function syncPad() {
  if (!isTauri()) return;
  if (state.padIoBusy) {
    toast("Another pad operation is already in progress");
    return;
  }
  if (state.padProtocolCompatible === false) {
    toast("Protocol mismatch — flash the matched C6/S3 v0.3 pair first");
    return;
  }
  if (!validPadBanks(state.padSlots)) {
    toast("No five-bank slot set to sync — Refresh first");
    return;
  }
  const restoreBank = state.padBank;
  setPadIoBusy(true);
  try {
    ensureHostBridgeSlots(state.padSlots);
    const result = await tauriInvoke("pad_write_banks", {
      address: state.padAddress,
      banks: state.padSlots,
    });
    await save();
    toast(`Pad sync complete: ${result}`);
  } catch (e) {
    toast(String(e));
  } finally {
    setPadIoBusy(false);
    state.padBank = restoreBank;
    renderBankSelector();
    renderPadGrid();
  }
}

async function toggleListen() {
  if (!isTauri()) return;
  if (state.padIoBusy) {
    toast("Another pad operation is already in progress");
    return;
  }
  if (state.padProtocolCompatible === false) {
    toast("Protocol mismatch — macro listener disabled");
    return;
  }
  setPadIoBusy(true);
  try {
    if (state.padListening) {
      await tauriInvoke("stop_macro_listen");
      state.padListening = false;
      $("#padListenBtn").textContent = "Listen for macros";
      toast("Stopped listening");
    } else {
      await tauriInvoke("start_macro_listen", { address: state.padAddress });
      state.padListening = true;
      $("#padListenBtn").textContent = "Stop listening";
      toast("Listening for MacroEvent…");
    }
  } catch (e) {
    toast(String(e));
  } finally {
    setPadIoBusy(false);
  }
}

function openForm(id = null) {
  state.editingId = id;
  const a = id ? state.actions.find((x) => x.id === id) : null;
  $("#formTitle").textContent = id ? "Edit action" : "New action";
  $("#f_name").value = a?.name || "";
  $("#f_description").value = a?.description || "";
  $("#f_value").value = a?.value || "";
  $("#f_tags").value = (a?.tags || []).join(", ");
  $("#f_favorite").checked = a?.favorite || false;

  const catSel = $("#f_category");
  catSel.replaceChildren(...CATEGORIES.map((c) => el("option", { value: c, textContent: c })));
  catSel.value = a?.category || CATEGORIES[0];

  const typeSel = $("#f_type");
  typeSel.replaceChildren(...ACTION_TYPES.map((t) => el("option", { value: t, textContent: t })));
  typeSel.value = a?.type || "command";
  $("#f_value").placeholder = valuePlaceholder(typeSel.value);

  $("#formError").textContent = "";
  $("#dialog").showModal();
  $("#f_name").focus();
}

async function submitForm(ev) {
  ev.preventDefault();
  const input = {
    name: $("#f_name").value,
    category: $("#f_category").value,
    type: $("#f_type").value,
    description: $("#f_description").value,
    value: $("#f_value").value,
    tags: $("#f_tags").value,
    favorite: $("#f_favorite").checked,
  };
  const v = validateAction(input);
  if (!v.ok) {
    $("#formError").textContent = v.errors.join(" · ");
    return;
  }
  const existing = state.editingId ? state.actions.find((x) => x.id === state.editingId) : null;
  const action = normalizeAction(input, existing);
  if (
    existing &&
    state.allowedCommands.has(existing.id) &&
    (existing.type !== action.type || existing.value !== action.value)
  ) {
    state.allowedCommands.delete(existing.id);
  }
  if (existing) {
    state.actions = state.actions.map((x) => (x.id === existing.id ? action : x));
  } else {
    state.actions.push(action);
  }
  try {
    await save();
  } catch (e) {
    toast(`Save failed: ${e}`);
    return;
  }
  $("#dialog").close();
  render();
  toast(existing ? "Saved" : "Added");
}

function exportJson() {
  try {
    const payload = {
      actions: state.actions,
      ...buildPortablePadState(state.padBindings, state.padSlots),
      padPresetNames: normalizePresetNames(state.padPresetNames),
      composers: normalizeComposers(state.composers),
      allowedCommands: [...state.allowedCommands],
    };
    const blob = new Blob([JSON.stringify(payload, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = el("a", { href: url, download: "macro-actions.json" });
    document.body.append(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  } catch (error) {
    toast(`Export failed: ${error}`);
  }
}

function importJson(file) {
  if (storeReplacementBlocked(state)) {
    toast("Another pad/store operation is already in progress");
    return;
  }
  const reader = new FileReader();
  reader.onload = async () => {
    let replacing = false;
    let previous = null;
    try {
      const data = JSON.parse(reader.result);
      let list = Array.isArray(data) ? data : data.actions;
      if (!Array.isArray(list)) throw new Error("file is not an array / store");
      const portable = readPortablePadState(data);
      const cleaned = dedupeById(
        list.map((raw) => normalizeAction(raw)).filter((a) => validateAction(a).ok)
      );
      if (
        state.actions.length > 0 &&
        !confirm(
          `Replace all ${state.actions.length} current actions with ${cleaned.length} from this file?\n` +
            `This cannot be undone — Export first if you want a backup.`
        )
      ) {
        toast("Import cancelled");
        return;
      }
      if (storeReplacementBlocked(state)) {
        toast("Import cancelled because another pad/store operation started");
        return;
      }

      // Drain the previous save queue, closing the replacement gate in this
      // same JavaScript task so no stale snapshot can enter behind it.
      const pendingSave = save();
      setPadIoBusy(true);
      setStoreReplaceBusy(true);
      replacing = true;
      await pendingSave;
      previous = captureSaveSnapshot();

      state.actions = cleaned;
      // Portable JSON is data, never an authority grant. Imported command
      // actions must be approved locally against their exact text.
      state.allowedCommands = new Set();
      if (portable.hasPadBindings) state.padBindings = portable.padBindings;
      if (portable.hasPadSlots) state.padSlots = portable.padSlots;
      if (!Array.isArray(data) && Object.prototype.hasOwnProperty.call(data, "padPresetNames")) {
        state.padPresetNames = normalizePresetNames(data.padPresetNames);
      }
      if (!Array.isArray(data) && Object.prototype.hasOwnProperty.call(data, "composers")) {
        state.composers = normalizeComposers(data.composers);
      }
      await persistSaveSnapshot(captureSaveSnapshot());
      // Persistence is authoritative from this point; a later rendering error
      // must not roll the UI back to a store that is no longer on disk.
      previous = null;
      render();
      renderComposerPanel();
      renderPadGrid();
      const bankNote = portable.hasPadSlots ? " and pad banks" : "";
      toast(`Imported ${cleaned.length} actions${bankNote}`);
    } catch (e) {
      if (previous) {
        applySaveSnapshot(previous);
        render();
        renderComposerPanel();
        renderPadGrid();
      }
      toast("Import failed: " + e.message);
    } finally {
      if (replacing) {
        setStoreReplaceBusy(false);
        setPadIoBusy(false);
      }
    }
  };
  reader.onerror = () => toast("Import failed: file could not be read");
  reader.readAsText(file);
}

function renderComposerPanel() {
  const cfg = normalizeComposers(state.composers).ai || defaultComposers().ai;
  const ta = $("#composerCommands");
  const sep = $("#composerSeparator");
  if (!ta || !sep) return;
  if (document.activeElement !== ta) ta.value = (cfg.commands || []).join("\n");
  if (document.activeElement !== sep) sep.value = cfg.separator ?? " ";
}

function applyComposerPanel() {
  const commands = String($("#composerCommands").value || "")
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  const prev = normalizeComposers(state.composers).ai || defaultComposers().ai;
  const separator = String($("#composerSeparator").value ?? " ");
  state.composers = normalizeComposers({
    ...state.composers,
    ai: {
      commands,
      timeoutMs: prev.timeoutMs || 60000,
      separator,
      resetOn: ["space", "explicitClear"],
    },
  });
  save();
  toast("Composer saved");
}

async function resetComposerCycle() {
  if (isTauri()) {
    try {
      await tauriInvoke("reset_composer", { composerId: "ai" });
    } catch (e) {
      toast(String(e));
      return;
    }
  }
  toast("Composer cycle reset");
}

async function exportProfileDisk() {
  if (!isTauri()) {
    toast("Profile export needs the desktop app");
    return;
  }
  const name = prompt("Profile name", "dev");
  if (!name) return;
  try {
    await save();
    const path = await tauriInvoke("export_profile", { name });
    toast(`Exported → ${path}`);
  } catch (e) {
    toast(String(e));
  }
}

async function importProfileDisk() {
  if (!isTauri()) {
    toast("Profile import needs the desktop app");
    return;
  }
  const path = prompt(
    "Path to profile JSON",
    `${(window.__MCC_HOME_HINT || "~")}/.config/3dl-macro-command-center/profiles/dev.json`
  );
  if (!path) return;
  if (state.padIoBusy) {
    toast("Another pad operation is already in progress");
    return;
  }
  if (
    state.actions.length > 0 &&
    !confirm(`Replace current MCC store with profile from:\n${path}?`)
  ) {
    toast("Import cancelled");
    return;
  }
  let pendingSave;
  try {
    pendingSave = save();
  } catch (e) {
    toast(`Import blocked because current state could not be saved: ${e}`);
    return;
  }
  setPadIoBusy(true);
  setStoreReplaceBusy(true);
  try {
    try {
      await pendingSave;
    } catch (e) {
      toast(`Import blocked because current state could not be saved: ${e}`);
      return;
    }
    const result = await tauriInvoke("import_profile", { path });
    const store = result?.store ?? result;
    applyStoreFromRust(store);
    toast(result?.padWrite ? `Profile imported · ${result.padWrite}` : "Profile imported");
  } catch (e) {
    toast(String(e));
  } finally {
    try {
      applyStoreFromRust(await tauriInvoke("get_store"));
    } catch (e) {
      console.error("failed to reload authoritative store after profile import", e);
    }
    await reconcilePadSelectionFromStatus();
    setStoreReplaceBusy(false);
    setPadIoBusy(false);
  }
}

function applyStoreFromRust(store) {
  state.actions = (store.actions || []).map(fromRustAction);
  state.padBindings = migratePadBindings(store.padBindings);
  state.padPresetNames = normalizePresetNames(store.padPresetNames);
  state.composers = normalizeComposers(store.composers);
  state.allowedCommands = new Set(store.allowedCommands || []);
  const banks = normalizePadBanks(store.padSlots);
  if (banks) {
    state.padSlots = banks;
    ensureHostBridgeSlots(state.padSlots);
  }
  render();
  renderPadGrid();
  renderComposerPanel();
}

async function reconcilePadSelectionFromStatus() {
  if (!isTauri()) return;
  try {
    const status = await tauriInvoke("pad_status", { address: state.padAddress });
    if (status.address && status.address !== "via-s3-dongle") {
      state.padAddress = status.address;
    }
    state.padTransport = status.transport || state.padTransport;
    const selected = Number(status.selectedBank ?? status.selected_bank);
    if (Number.isInteger(selected) && selected >= 0 && selected < BANK_COUNT) {
      state.padBank = selected;
    }
    if (status.info) state.padSupportsBanks = infoSupportsBanks(status.info);
    renderBankSelector();
    renderPadGrid();
  } catch (error) {
    console.warn("pad selection reconciliation failed", error);
  }
}

function fillGitProfileSelect(profiles, active) {
  const sel = $("#gitProfileSelect");
  if (!sel) return;
  const cur = sel.value;
  sel.replaceChildren();
  const empty = el("option", {
    value: "",
    textContent: profiles?.length ? "Select…" : "No profiles yet",
  });
  sel.append(empty);
  for (const name of profiles || []) {
    sel.append(el("option", { value: name, textContent: name }));
  }
  if (active && profiles?.includes(active)) sel.value = active;
  else if (cur && profiles?.includes(cur)) sel.value = cur;
}

async function refreshGitSyncStatus() {
  const line = $("#gitSyncStatusLine");
  if (!isTauri()) {
    if (line) line.textContent = "Git sync needs the desktop app";
    return null;
  }
  try {
    const st = await tauriInvoke("git_sync_status");
    const auth = st.auth?.loggedIn
      ? `GitHub: ${st.auth.user || "logged in"}`
      : "GitHub: not logged in";
    const remote = st.remote || "(no remote)";
    const dirty = st.dirty ? " · dirty" : "";
    if (line) {
      line.textContent = `${auth} · ${remote} · branch ${st.branch || "—"} · ${
        st.profiles?.length || 0
      } profile(s)${dirty}`;
    }
    const input = $("#gitRemoteInput");
    if (input && st.remote && !input.value) input.value = st.remote;
    fillGitProfileSelect(st.profiles, st.settings?.activeProfile);
    return st;
  } catch (e) {
    if (line) line.textContent = String(e);
    return null;
  }
}

async function gitSaveRemote() {
  if (!isTauri()) return toast("Desktop only");
  const remote = $("#gitRemoteInput")?.value?.trim();
  if (!remote) return toast("Enter a remote URL");
  try {
    await tauriInvoke("git_sync_set_remote", { remote });
    toast("Remote saved");
    await refreshGitSyncStatus();
  } catch (e) {
    toast(String(e));
  }
}

async function gitEnsureRepo() {
  if (!isTauri()) return toast("Desktop only");
  try {
    await tauriInvoke("git_sync_ensure_repo");
    toast("Local hk-config ready");
    await refreshGitSyncStatus();
  } catch (e) {
    toast(String(e));
  }
}

async function gitPull() {
  if (!isTauri()) return toast("Desktop only");
  try {
    const msg = await tauriInvoke("git_sync_pull");
    toast(String(msg).slice(0, 120));
    await refreshGitSyncStatus();
  } catch (e) {
    toast(String(e));
  }
}

async function gitPush() {
  if (!isTauri()) return toast("Desktop only");
  try {
    const msg = await tauriInvoke("git_sync_push");
    toast(String(msg).slice(0, 120));
    await refreshGitSyncStatus();
  } catch (e) {
    toast(String(e));
  }
}

async function gitPullApply() {
  if (!isTauri()) return toast("Desktop only");
  const name = $("#gitProfileSelect")?.value;
  if (!name) return toast("Select a profile first");
  if (state.padIoBusy) return toast("Another pad operation is already in progress");
  if (
    state.actions.length > 0 &&
    !confirm(`Pull & replace live MCC store with profile "${name}"?`)
  ) {
    return;
  }
  let pendingSave;
  try {
    pendingSave = save();
  } catch (e) {
    toast(`Git apply blocked because current state could not be saved: ${e}`);
    return;
  }
  setPadIoBusy(true);
  setStoreReplaceBusy(true);
  try {
    try {
      await pendingSave;
    } catch (e) {
      toast(`Git apply blocked because current state could not be saved: ${e}`);
      return;
    }
    const result = await tauriInvoke("git_sync_pull_apply", { name });
    const store = result?.store ?? result;
    applyStoreFromRust(store);
    const pullMsg = result?.pullMessage || result?.pull_message || "";
    const count =
      result?.actionCount ??
      result?.action_count ??
      store?.actions?.length ??
      state.actions.length;
    const unchanged = !!result?.unchanged;
    const profile = result?.profile || name;
    const slotCount =
      result?.padSlotCount ?? result?.pad_slot_count ?? 0;
    const padWrite = result?.padWrite || result?.pad_write || "";
    const slotNote =
      slotCount === BANK_COUNT * SLOT_COUNT
        ? ` · ${BANK_COUNT * SLOT_COUNT} pad slots${padWrite ? ` (${padWrite})` : ""}`
        : slotCount === SLOT_COUNT
          ? ` · legacy bank 1 / ${SLOT_COUNT} pad slots${padWrite ? ` (${padWrite})` : ""}`
        : " · no pad slots in profile";
    if (unchanged) {
      toast(
        `Applied "${profile}" (${count} actions)${slotNote} — already matched live store. ${pullMsg}`.trim()
      );
    } else {
      toast(`Applied "${profile}" (${count} actions)${slotNote}. ${pullMsg}`.trim());
    }
    await refreshGitSyncStatus();
  } catch (e) {
    toast(String(e));
  } finally {
    try {
      applyStoreFromRust(await tauriInvoke("get_store"));
    } catch (e) {
      console.error("failed to reload authoritative store after Git apply", e);
    }
    await reconcilePadSelectionFromStatus();
    setStoreReplaceBusy(false);
    setPadIoBusy(false);
  }
}

async function gitPushProfile() {
  if (!isTauri()) return toast("Desktop only");
  const suggested = $("#gitProfileSelect")?.value || "dev";
  const name = prompt("Profile name to push", suggested);
  if (!name) return;
  try {
    // Persist current padSlots (from Refresh/edits) into the store before export.
    await save();
    const msg = await tauriInvoke("git_sync_push_profile", { name });
    toast(String(msg).slice(0, 160));
    await refreshGitSyncStatus();
    const sel = $("#gitProfileSelect");
    if (sel) sel.value = name.trim();
  } catch (e) {
    toast(String(e));
  }
}

async function ghLogin() {
  if (!isTauri()) return toast("Desktop only");
  try {
    const msg = await tauriInvoke("gh_auth_login");
    toast(String(msg));
  } catch (e) {
    toast(String(e));
  }
}

async function gitCreateRepo() {
  if (!isTauri()) return toast("Desktop only");
  const name = prompt("New private GitHub repo name", "hk-config");
  if (!name) return;
  try {
    const msg = await tauriInvoke("git_sync_create_repo", {
      name,
      privateRepo: true,
    });
    toast(String(msg).slice(0, 160));
    await refreshGitSyncStatus();
  } catch (e) {
    toast(String(e));
  }
}

async function init() {
  // Always show the pad config panel (6 presets live in HTML).
  const padPanel = $("#padPanel");
  if (padPanel) padPanel.hidden = false;
  renderPadGrid();

  if (isTauri()) {
    const loaded = await desktopLoad();
    state.actions = loaded.actions;
    state.padBindings = loaded.padBindings;
    state.padPresetNames = normalizePresetNames(loaded.padPresetNames);
    state.composers = normalizeComposers(loaded.composers);
    state.allowedCommands = loaded.allowedCommands;
    if (validPadBanks(loaded.padSlots)) {
      state.padSlots = loaded.padSlots;
    }
    renderPadGrid();
    renderComposerPanel();
    await tauriListen("macro-fired", async (p) => {
      if (state.storeReplaceBusy) return;
      const msg = String(p.result || "");
      const bank = BANKS[p.bank ?? 0]?.name || `bank ${Number(p.bank ?? 0) + 1}`;
      toast(`Macro ${bank}/${presetDisplayName(p.preset ?? 0)}/${BTN_NAMES[p.action] || p.action}: ${msg}`);
      // If a command was blocked, offer Allow + retry once.
      if (msg.includes("not allowed yet") && p.actionId) {
        const act = state.actions.find((a) => a.id === p.actionId);
        if (
          act &&
          confirm(
            `Allow shell for "${act.name}" so pad macros can run it?\n\n${act.value}`
          )
        ) {
          state.allowedCommands.add(act.id);
          await tauriInvoke("allow_command", { actionId: act.id });
          await save();
          try {
            const again = await tauriInvoke("execute_action_id", { actionId: act.id });
            toast(again);
          } catch (e) {
            toast(String(e));
          }
          render();
        }
      } else {
        render();
      }
    });
    await tauriListen("pad-error", (msg) => toast(String(msg)));
    await tauriListen("pad-listening", (on) => {
      state.padListening = !!on;
      $("#padListenBtn").textContent = on ? "Stop listening" : "Listen for macros";
    });
    await tauriListen("pad-bank-changed", (value) => {
      if (state.padIoBusy) return;
      const bank = Number(value);
      if (!Number.isInteger(bank) || bank < 0 || bank >= BANK_COUNT) return;
      state.padSupportsBanks = true;
      state.padBank = bank;
      renderBankSelector();
      renderPadGrid();
    });
    $("#padRefreshBtn").addEventListener("click", refreshPad);
    $("#padSyncBtn").addEventListener("click", syncPad);
    $("#padBankSelect").addEventListener("change", selectPadBank);
    $("#padBluezBtn").addEventListener("click", toggleBluez);
    $("#padListenBtn").addEventListener("click", toggleListen);
    $("#s_mode").addEventListener("change", syncSlotModeFields);
    $("#slotForm").addEventListener("submit", applySlotEditor);
    $("#slotCancelBtn").addEventListener("click", () => $("#slotDialog").close());
    await refreshPad();
    const n = document.querySelectorAll(".pad-preset[data-preset]").length;
    toast(`Pad UI: ${n} preset columns (need ${PRESET_COUNT})`);
    // Auto-listen so rebound macros work without an extra click.
    try {
      await tauriInvoke("start_macro_listen", { address: state.padAddress });
      state.padListening = true;
      $("#padListenBtn").textContent = "Stop listening";
    } catch (e) {
      console.warn("auto-listen failed", e);
      toast("Auto-listen failed — click Listen for macros");
    }
  } else {
    const loaded = browserLoad();
    state.actions = loaded.actions;
    state.padBindings = loaded.padBindings;
    state.padPresetNames = normalizePresetNames(loaded.padPresetNames);
    state.composers = normalizeComposers(loaded.composers);
    state.allowedCommands = loaded.allowedCommands;
    if (validPadBanks(loaded.padSlots)) {
      state.padSlots = loaded.padSlots;
    }
    renderPadGrid();
    renderComposerPanel();
    $("#padBankSelect")?.addEventListener("change", selectPadBank);
  }

  $("#search").addEventListener("input", (e) => {
    state.query = e.target.value;
    render();
  });
  $("#search").addEventListener("keydown", (e) => {
    if (e.key !== "Enter") return;
    const top = currentList()[0];
    if (!top) return;
    top.type === "url" ? openUrl(top) : isTauri() ? runAction(top) : copyValue(top);
  });
  $("#categoryFilter").addEventListener("change", (e) => {
    state.category = e.target.value;
    render();
  });
  $("#typeFilter").addEventListener("change", (e) => {
    state.type = e.target.value;
    render();
  });
  $("#favToggle").addEventListener("click", (e) => {
    state.favoritesOnly = !state.favoritesOnly;
    e.target.classList.toggle("on", state.favoritesOnly);
    e.target.setAttribute("aria-pressed", String(state.favoritesOnly));
    render();
  });
  $("#f_type").addEventListener("change", (e) => {
    $("#f_value").placeholder = valuePlaceholder(e.target.value);
  });
  $("#addBtn").addEventListener("click", () => openForm(null));
  $("#form").addEventListener("submit", submitForm);
  $("#cancelBtn").addEventListener("click", () => $("#dialog").close());
  $("#exportBtn").addEventListener("click", exportJson);
  $("#exportProfileBtn")?.addEventListener("click", exportProfileDisk);
  $("#importProfileBtn")?.addEventListener("click", importProfileDisk);
  $("#composerSaveBtn")?.addEventListener("click", applyComposerPanel);
  $("#composerResetBtn")?.addEventListener("click", resetComposerCycle);
  $("#gitSyncRefreshBtn")?.addEventListener("click", refreshGitSyncStatus);
  $("#gitRemoteSaveBtn")?.addEventListener("click", gitSaveRemote);
  $("#gitEnsureRepoBtn")?.addEventListener("click", gitEnsureRepo);
  $("#gitPullBtn")?.addEventListener("click", gitPull);
  $("#gitPushBtn")?.addEventListener("click", gitPush);
  $("#gitPullApplyBtn")?.addEventListener("click", gitPullApply);
  $("#gitPushProfileBtn")?.addEventListener("click", gitPushProfile);
  $("#ghAuthLoginBtn")?.addEventListener("click", ghLogin);
  $("#gitCreateRepoBtn")?.addEventListener("click", gitCreateRepo);
  refreshGitSyncStatus();
  $("#importInput").addEventListener("change", (e) => {
    if (e.target.files[0]) importJson(e.target.files[0]);
    e.target.value = "";
  });

  render();
}

init();
