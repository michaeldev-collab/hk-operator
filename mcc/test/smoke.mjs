// smoke.mjs — Node smoke test for the pure logic layer. No browser needed.
// Run: node test/smoke.mjs
import {
  CATEGORIES,
  ACTION_TYPES,
  validateAction,
  normalizeAction,
  filterActions,
  defaultComposers,
  normalizeComposers,
} from "../src/lib.js";
import { SEED_ACTIONS } from "../src/seed.js";

let pass = 0;
let fail = 0;
function check(name, cond) {
  if (cond) {
    pass++;
    console.log("  PASS  " + name);
  } else {
    fail++;
    console.log("  FAIL  " + name);
  }
}

console.log("HK Operator MCC — smoke test\n");

// 1. constants
check("6 categories defined", CATEGORIES.length === 6);
check("6 action types defined", ACTION_TYPES.length === 6);

// 2. validation: good action
const good = { name: "x", category: "Terminal Commands", type: "command", value: "ls" };
check("valid action passes", validateAction(good).ok === true);

// 3. validation: missing fields
check("missing name fails", validateAction({ category: "URLs", type: "url", value: "https://a.com" }).ok === false);
check("bad category fails", validateAction({ name: "x", category: "Nope", type: "url", value: "https://a.com" }).ok === false);
check("bad type fails", validateAction({ name: "x", category: "URLs", type: "boom", value: "https://a.com" }).ok === false);

// 4. url rule
check("url without http fails", validateAction({ name: "x", category: "URLs", type: "url", value: "example.com" }).ok === false);
check("url with https passes", validateAction({ name: "x", category: "URLs", type: "url", value: "https://example.com" }).ok === true);

// 5. normalize adds id + timestamps + parses tags
const n = normalizeAction({ name: " Hi ", category: "URLs", type: "url", value: "https://a.com", tags: "a, b ,c" });
check("normalize trims name", n.name === "Hi");
check("normalize assigns id", typeof n.id === "string" && n.id.length > 3);
check("normalize parses tags to array", Array.isArray(n.tags) && n.tags.length === 3);
check("normalize sets createdAt", typeof n.createdAt === "string");

// 6. normalize preserves identity on edit
const edited = normalizeAction({ ...n, name: "Hi2" }, n);
check("edit preserves id", edited.id === n.id);
check("edit preserves createdAt", edited.createdAt === n.createdAt);

// 7. seed integrity — every seed action normalizes + validates
let seedOk = true;
for (const s of SEED_ACTIONS) {
  const norm = normalizeAction(s);
  if (!validateAction(norm).ok) {
    seedOk = false;
    console.log("    bad seed:", s.name, validateAction(norm).errors);
  }
}
check(`all ${SEED_ACTIONS.length} seed actions are valid`, seedOk);

// 7b. pad binding names resolve to seed actions
import { SEED_PAD_BINDINGS } from "../src/seed.js";
const seedNames = new Set(SEED_ACTIONS.map((s) => s.name));
check(
  "SEED_PAD_BINDINGS names exist in SEED_ACTIONS",
  Object.values(SEED_PAD_BINDINGS).every((n) => seedNames.has(n))
);
check(
  "SEED_PAD_BINDINGS is empty or valid in public seed",
  Object.keys(SEED_PAD_BINDINGS).length === 0 ||
    ["2-0", "2-1", "2-2"].every((k) => SEED_PAD_BINDINGS[k])
);

// 8. filtering (assertions guard against vacuous pass on empty results)
const sample = SEED_ACTIONS.map((s) => normalizeAction(s));
const byCat = filterActions(sample, { category: "Prompts" });
check("filter by category works", byCat.length > 0 && byCat.every((a) => a.category === "Prompts"));
const byType = filterActions(sample, { type: "command" });
check("filter by type works", byType.length > 0 && byType.every((a) => a.type === "command"));
const byQuery = filterActions(sample, { query: "git" });
check(
  "text query returns only matches that contain the term",
  byQuery.length > 0 &&
    byQuery.every((a) =>
      [a.name, a.description, a.value, a.category, a.type, (a.tags || []).join(" ")]
        .join(" ")
        .toLowerCase()
        .includes("git")
    )
);
check("nonsense query returns none", filterActions(sample, { query: "zzqqx-nothing" }).length === 0);
const favs = filterActions(sample, { favoritesOnly: true });
check("favoritesOnly returns only favorites", favs.length > 0 && favs.every((a) => a.favorite));
const combo = filterActions(sample, { category: "Terminal Commands", type: "command", query: "git" });
check(
  "combined category+type+query works",
  combo.length > 0 && combo.every((a) => a.category === "Terminal Commands" && a.type === "command")
);
check("empty actions array filters to []", filterActions([], { query: "x" }).length === 0);

// 9. sort: favorites first
const sorted = filterActions(sample, {});
const firstNonFavIndex = sorted.findIndex((a) => !a.favorite);
const lastFavIndex = sorted.map((a) => a.favorite).lastIndexOf(true);
check("favorites sort before non-favorites", firstNonFavIndex === -1 || lastFavIndex < firstNonFavIndex);

// 10. validation + normalize edge cases (added after review)
check("validateAction(null) fails", validateAction(null).ok === false);
check("validateAction(undefined) fails", validateAction(undefined).ok === false);
check(
  "whitespace-only value fails",
  validateAction({ name: "x", category: "URLs", type: "note", value: "   " }).ok === false
);
check(
  "uppercase HTTPS url passes",
  validateAction({ name: "x", category: "URLs", type: "url", value: "HTTPS://A.COM" }).ok === true
);
check(
  "ftp scheme url fails",
  validateAction({ name: "x", category: "URLs", type: "url", value: "ftp://a.com" }).ok === false
);
check(
  "url with surrounding whitespace passes after normalize",
  validateAction(normalizeAction({ name: "x", category: "URLs", type: "url", value: "  https://a.com  " })).ok === true
);
check(
  "normalize with no tags yields []",
  normalizeAction({ name: "x", category: "URLs", type: "url", value: "https://a.com" }).tags.length === 0
);
check(
  "normalize empty-string tags yields []",
  normalizeAction({ name: "x", category: "URLs", type: "url", value: "https://a.com", tags: "" }).tags.length === 0
);
check(
  "composer action validates",
  validateAction({ name: "AI cycle", category: "Cursor Prompts", type: "composer", value: "ai" }).ok === true
);
check("default composers has ai", !!defaultComposers().ai);
check(
  "normalizeComposers fills commands",
  normalizeComposers({ ai: { commands: [" /a ", "", "/b"] } }).ai.commands.join(",") === "/a,/b"
);

console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail === 0 ? 0 : 1);
