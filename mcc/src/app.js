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
  commandValueFingerprint,
  normalizeAllowedCommands,
  isCommandAllowed,
} from "./lib.js";
import { SEED_ACTIONS, SEED_PAD_BINDINGS } from "./seed.js";

const STORAGE_KEY = "hk.operator.actions.v1";
const BTN_NAMES = ["B2", "B4", "B5"];

/**
 * Preset 3 (index 2) is a host bridge: the pad always types Ctrl+Alt+1/2/3
 * (KDE → MCC fire API). The UI only edits which MCC action that chord runs.
 * Literal HID editing stays on other presets.
 */
const PRESET_COUNT = 6;
const ACTION_COUNT = 3;
const SLOT_COUNT = PRESET_COUNT * ACTION_COUNT; // 18
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

function isHostBridgePreset(preset) {
  return Number(preset) === HOST_BRIDGE_PRESET;
}

function bridgeChordForAction(actionIdx) {
  return {
    mode: 0,
    mod: BRIDGE_MOD,
    key: BRIDGE_KEYS[actionIdx] ?? BRIDGE_KEYS[0],
  };
}

/** Force Preset 3 device slots to the Ctrl+Alt bridge chords before sync. */
function ensureHostBridgeSlots(slots) {
  if (!slots || slots.length !== SLOT_COUNT) return slots;
  for (let a = 0; a < ACTION_COUNT; a++) {
    const i = HOST_BRIDGE_PRESET * ACTION_COUNT + a;
    const key = `${HOST_BRIDGE_PRESET}-${a}`;
    const bound = state.actions.find((x) => x.id === state.padBindings[key]);
    const chord = bridgeChordForAction(a);
    const prev = slots[i] || {};
    slots[i] = {
      ...chord,
      label: (prev.label || (bound ? bound.name : `Ctrl+Alt+${a + 1}`)).slice(0, 23),
    };
  }
  return slots;
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
  padBindings: {}, // "p-a" -> actionId
  padPresetNames: defaultPresetNames(),
  composers: defaultComposers(),
  allowedCommands: {},
  query: "",
  category: "all",
  type: "all",
  favoritesOnly: false,
  editingId: null,
  // pad
  padSlots: null, // array of 18 {mode, mod, key, label}
  padAddress: null,
  padListening: false,
  editingSlot: null, // {preset, action}
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
          allowedCommands: {},
        };
      }
      if (parsed && Array.isArray(parsed.actions)) {
        return {
          actions: dedupeById(parsed.actions.map((a) => normalizeAction(a))),
          padBindings: parsed.padBindings || {},
          padPresetNames: normalizePresetNames(parsed.padPresetNames),
          composers: normalizeComposers(parsed.composers),
          allowedCommands: normalizeAllowedCommands(parsed.allowedCommands),
        };
      }
    }
  } catch (e) {
    console.warn("load failed, reseeding", e);
  }
  const seeded = SEED_ACTIONS.map((a) => normalizeAction(a));
  const padBindings = { ...SEED_PAD_BINDINGS };
  // Remap seed binding ids to actual seeded action ids by stable name.
  const byName = Object.fromEntries(seeded.map((a) => [a.name, a.id]));
  const remapped = {};
  for (const [k, nameOrId] of Object.entries(padBindings)) {
    remapped[k] = byName[nameOrId] || nameOrId;
  }
  const padPresetNames = defaultPresetNames();
  const composers = defaultComposers();
  const store = {
    actions: seeded,
    padBindings: remapped,
    padPresetNames,
    composers,
    allowedCommands: {},
  };
  localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
  return {
    actions: seeded,
    padBindings: remapped,
    padPresetNames,
    composers,
    allowedCommands: {},
  };
}

function browserSave() {
  localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify({
      actions: state.actions,
      padBindings: state.padBindings,
      padPresetNames: normalizePresetNames(state.padPresetNames),
      composers: normalizeComposers(state.composers),
      allowedCommands: normalizeAllowedCommands(state.allowedCommands),
    })
  );
}

async function desktopLoad() {
  const store = await tauriInvoke("get_store");
  let actions = Array.isArray(store.actions) ? store.actions.map(fromRustAction) : [];
  let padBindings = store.padBindings || {};
  let padPresetNames = normalizePresetNames(store.padPresetNames);
  let composers = normalizeComposers(store.composers);
  let allowedCommands = normalizeAllowedCommands(store.allowedCommands);

  if (actions.length === 0) {
    actions = SEED_ACTIONS.map((a) => normalizeAction(a));
    const byName = Object.fromEntries(actions.map((a) => [a.name, a.id]));
    padBindings = {};
    for (const [k, nameOrId] of Object.entries(SEED_PAD_BINDINGS)) {
      padBindings[k] = byName[nameOrId] || nameOrId;
    }
    padPresetNames = defaultPresetNames();
    composers = defaultComposers();
    await desktopSave(actions, padBindings, allowedCommands, padPresetNames, composers);
  }
  return { actions, padBindings, padPresetNames, composers, allowedCommands };
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
  composers = state.composers
) {
  await tauriInvoke("save_store", {
    store: {
      actions: actions.map(toRustAction),
      padBindings,
      padPresetNames: normalizePresetNames(padPresetNames),
      composers: normalizeComposers(composers),
      allowedCommands: normalizeAllowedCommands(allowed),
    },
  });
}

async function save() {
  if (isTauri()) await desktopSave();
  else browserSave();
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
  if (action.type === "command" && !isCommandAllowed(action, state.allowedCommands)) {
    if (
      !confirm(
        `Allow shell execution for "${action.name}"?\n\n${action.value}\n\nRe-approval is required if this command text changes.`
      )
    ) {
      toast("Command not allowed");
      return;
    }
    state.allowedCommands = {
      ...state.allowedCommands,
      [action.id]: commandValueFingerprint(action.value),
    };
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
      const allowed = isCommandAllowed(a, state.allowedCommands);
      foot.append(
        el("button", {
          className: "btn" + (allowed ? " on" : ""),
          textContent: allowed ? "Allowed ✓" : "Allow shell",
          onclick: async () => {
            if (allowed) return;
            if (!confirm(`Allow shell for "${a.name}"?\n\n${a.value}`)) return;
            state.allowedCommands = {
              ...state.allowedCommands,
              [a.id]: commandValueFingerprint(a.value),
            };
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

function emptySlots() {
  return Array.from({ length: SLOT_COUNT }, () => ({ mode: 0, mod: 0, key: 0, label: "" }));
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
  if (title) title.textContent = `Cyberpad · ${PRESET_COUNT} presets`;
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
      const bridgeNote = isHostBridgePreset(p) ? " · MCC bridge" : "";
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

function renderPadGrid() {
  ensurePadGridStructure();
  const slots = state.padSlots || emptySlots();
  const presets = document.querySelectorAll(".pad-preset[data-preset]");
  for (const col of presets) {
    const p = Number(col.getAttribute("data-preset"));
    const list = col.querySelector("[data-slots]");
    if (!list || Number.isNaN(p)) continue;
    list.replaceChildren();
    for (let a = 0; a < ACTION_COUNT; a++) {
      const slot = slots[p * ACTION_COUNT + a] || { mode: 0, mod: 0, key: 0, label: "" };
      const key = `${p}-${a}`;
      const bound = state.actions.find((x) => x.id === state.padBindings[key]);
      const bridge = isHostBridgePreset(p);
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
  state.editingSlot = { preset, action };
  const slots = state.padSlots || emptySlots();
  const slot = slots[preset * ACTION_COUNT + action] || {
    mode: 0,
    mod: 0,
    key: 0,
    label: "",
  };
  const bridge = isHostBridgePreset(preset);
  $("#slotTitle").textContent = `${presetDisplayName(preset)} · ${BTN_NAMES[action]}`;
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
  const key = `${preset}-${action}`;
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
  const { preset } = state.editingSlot || {};
  const bridge = isHostBridgePreset(preset);
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
  const { preset, action } = state.editingSlot || {};
  if (preset == null) return;
  if (!state.padSlots) state.padSlots = emptySlots();
  const key = `${preset}-${action}`;
  const actionId = $("#s_action").value;
  const labelIn = $("#s_label").value.slice(0, 23);

  if (isHostBridgePreset(preset)) {
    const bound = state.actions.find((x) => x.id === actionId);
    const chord = bridgeChordForAction(action);
    state.padSlots[preset * ACTION_COUNT + action] = {
      ...chord,
      label: labelIn || (bound ? bound.name.slice(0, 23) : `Ctrl+Alt+${action + 1}`),
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
    state.padSlots[preset * ACTION_COUNT + action] = {
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
    isHostBridgePreset(preset)
      ? "MCC action bound (Sync optional — chord already on pad)"
      : "Slot updated (Sync to pad to write device)"
  );
}

async function refreshPad() {
  if (!isTauri()) return;
  try {
    const st = await tauriInvoke("pad_status", { address: state.padAddress });
    state.padAddress = st.address;
    // Live BLE advertise name is a compatibility identifier (legacy: Cyberdeck Pad).
    const line = `${st.name || "Cyberdeck Pad"} · ${st.address} · ${
      st.connected ? "connected" : "disconnected"
    }${st.paired ? " · paired" : ""}${st.info ? " · " + st.info : ""}`;
    $("#padStatusLine").textContent = line;
    try {
      state.padSlots = await tauriInvoke("pad_read_slots", { address: state.padAddress });
      ensureHostBridgeSlots(state.padSlots);
      toast("Read slots from pad");
    } catch (e) {
      if (!state.padSlots) state.padSlots = emptySlots();
      $("#padStatusLine").textContent = line + " · GATT unavailable (flash hybrid firmware?)";
      toast(String(e));
    }
    renderPadGrid();
  } catch (e) {
    $("#padStatusLine").textContent =
      "Cyberpad not found — pair as a keyboard (BLE name may show Cyberdeck Pad)";
    toast(String(e));
  }
}

async function syncPad() {
  if (!isTauri()) return;
  if (!state.padSlots || state.padSlots.length !== SLOT_COUNT) {
    toast("No slots to sync — Refresh first");
    return;
  }
  try {
    ensureHostBridgeSlots(state.padSlots);
    await tauriInvoke("pad_write_slots", {
      address: state.padAddress,
      slots: state.padSlots,
    });
    await save();
    renderPadGrid();
    toast(`Synced ${SLOT_COUNT} slots to pad (P3 = Ctrl+Alt bridge)`);
  } catch (e) {
    toast(String(e));
  }
}

async function toggleListen() {
  if (!isTauri()) return;
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

function submitForm(ev) {
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
  if (existing) {
    state.actions = state.actions.map((x) => (x.id === existing.id ? action : x));
  } else {
    state.actions.push(action);
  }
  save();
  $("#dialog").close();
  render();
  toast(existing ? "Saved" : "Added");
}

function exportJson() {
  const payload = {
    actions: state.actions,
    padBindings: state.padBindings,
    padPresetNames: normalizePresetNames(state.padPresetNames),
    composers: normalizeComposers(state.composers),
    allowedCommands: normalizeAllowedCommands(state.allowedCommands),
  };
  const blob = new Blob([JSON.stringify(payload, null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const a = el("a", { href: url, download: "macro-actions.json" });
  document.body.append(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function importJson(file) {
  const reader = new FileReader();
  reader.onload = () => {
    try {
      const data = JSON.parse(reader.result);
      let list = Array.isArray(data) ? data : data.actions;
      if (!Array.isArray(list)) throw new Error("file is not an array / store");
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
      state.actions = cleaned;
      if (!Array.isArray(data) && data.padBindings) state.padBindings = data.padBindings;
      if (!Array.isArray(data) && data.padPresetNames) {
        state.padPresetNames = normalizePresetNames(data.padPresetNames);
      }
      if (!Array.isArray(data) && data.composers) {
        state.composers = normalizeComposers(data.composers);
      }
      save();
      render();
      renderComposerPanel();
      renderPadGrid();
      toast(`Imported ${cleaned.length} actions`);
    } catch (e) {
      toast("Import failed: " + e.message);
    }
  };
  reader.readAsText(file);
}

function renderComposerPanel() {
  const cfg = normalizeComposers(state.composers).ai || defaultComposers().ai;
  const ta = $("#composerCommands");
  const timeout = $("#composerTimeout");
  const sep = $("#composerSeparator");
  if (!ta || !timeout || !sep) return;
  if (document.activeElement !== ta) ta.value = (cfg.commands || []).join("\n");
  if (document.activeElement !== timeout) timeout.value = String(cfg.timeoutMs || 4000);
  if (document.activeElement !== sep) sep.value = cfg.separator ?? " ";
}

function applyComposerPanel() {
  const commands = String($("#composerCommands").value || "")
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  const timeoutMs = Math.max(500, Number($("#composerTimeout").value) || 4000);
  const separator = String($("#composerSeparator").value ?? " ");
  state.composers = normalizeComposers({
    ...state.composers,
    ai: { commands, timeoutMs, separator, resetOn: ["timeout", "explicitClear"] },
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
    `${(window.__MCC_HOME_HINT || "~")}/.config/hk-operator/profiles/dev.json`
  );
  if (!path) return;
  if (
    state.actions.length > 0 &&
    !confirm(`Replace current MCC store with profile from:\n${path}?`)
  ) {
    toast("Import cancelled");
    return;
  }
  try {
    const store = await tauriInvoke("import_profile", { path });
    applyStoreFromRust(store);
    toast("Profile imported");
  } catch (e) {
    toast(String(e));
  }
}

function applyStoreFromRust(store) {
  state.actions = (store.actions || []).map(fromRustAction);
  state.padBindings = store.padBindings || {};
  state.padPresetNames = normalizePresetNames(store.padPresetNames);
  state.composers = normalizeComposers(store.composers);
  state.allowedCommands = normalizeAllowedCommands(store.allowedCommands);
  render();
  renderComposerPanel();
}

function fillGitProfileSelect(profiles, active) {
  const sel = $("#gitProfileSelect");
  if (!sel) return;
  const cur = sel.value;
  sel.replaceChildren();
  const empty = el("option", { value: "", textContent: profiles?.length ? "Select…" : "No profiles yet" });
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
  if (
    state.actions.length > 0 &&
    !confirm(`Pull & replace live MCC store with profile "${name}"?`)
  ) {
    return;
  }
  try {
    const result = await tauriInvoke("git_sync_pull_apply", { name });
    // Backward-compatible: older builds returned the store directly.
    const store = result?.store ?? result;
    applyStoreFromRust(store);
    renderPadGrid();
    const pullMsg = result?.pullMessage || result?.pull_message || "";
    const count =
      result?.actionCount ??
      result?.action_count ??
      store?.actions?.length ??
      state.actions.length;
    const unchanged = !!(result?.unchanged);
    const profile = result?.profile || name;
    const ignored =
      result?.profileAllowlistIgnored ?? result?.profile_allowlist_ignored ?? 0;
    const allowNote =
      ignored > 0
        ? ` Shell allowlist from profile ignored (${ignored} ids) — re-approve in UI if needed.`
        : " Shell approvals stay machine-local (profile allowlist not imported).";
    if (unchanged) {
      toast(
        `Applied "${profile}" (${count} actions) — already matched live store. ${pullMsg}${allowNote}`.trim()
      );
    } else {
      toast(`Applied "${profile}" (${count} actions). ${pullMsg}${allowNote}`.trim());
    }
    await refreshGitSyncStatus();
  } catch (e) {
    toast(String(e));
  }
}

async function gitPushProfile() {
  if (!isTauri()) return toast("Desktop only");
  const suggested = $("#gitProfileSelect")?.value || "dev";
  const name = prompt("Profile name to push", suggested);
  if (!name) return;
  try {
    const msg = await tauriInvoke("git_sync_push_profile", { name });
    toast(String(msg).slice(0, 140));
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
    renderPadGrid();
    renderComposerPanel();
    await tauriListen("macro-fired", async (p) => {
      const msg = String(p.result || "");
      toast(`Macro ${presetDisplayName(p.preset ?? 0)}/${BTN_NAMES[p.action] || p.action}: ${msg}`);
      // If a command was blocked, offer Allow + retry once.
      if (
        (msg.includes("not allowed yet") || msg.includes("value changed since approval")) &&
        p.actionId
      ) {
        const act = state.actions.find((a) => a.id === p.actionId);
        if (
          act &&
          confirm(
            `Allow shell for "${act.name}" so pad macros can run it?\n\n${act.value}`
          )
        ) {
          state.allowedCommands = {
            ...state.allowedCommands,
            [act.id]: commandValueFingerprint(act.value),
          };
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
    $("#padRefreshBtn").addEventListener("click", refreshPad);
    $("#padSyncBtn").addEventListener("click", syncPad);
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
    renderPadGrid();
    renderComposerPanel();
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
