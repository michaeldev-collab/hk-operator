#!/usr/bin/env node
/**
 * Phase 2 — public example profile shape + hygiene.
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { validateAction, normalizeComposers } from "../src/lib.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const examplePath = join(__dirname, "../../config/examples/dev.json");

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

console.log("HK Operator MCC — config profile regressions\n");

let profile;
try {
  profile = JSON.parse(readFileSync(examplePath, "utf8"));
  check("example JSON parses", true);
} catch {
  check("example JSON parses", false);
  console.log(`\n${pass} passed, ${fail} failed (config profile)`);
  process.exit(1);
}

check("has actions array", Array.isArray(profile.actions));
check("has padBindings object", profile.padBindings && typeof profile.padBindings === "object");
check("has composers.ai", profile.composers && profile.composers.ai);
check("allowedCommands is empty object or array", 
  profile.allowedCommands &&
  (Array.isArray(profile.allowedCommands)
    ? profile.allowedCommands.length === 0
    : Object.keys(profile.allowedCommands).length === 0)
);

const ids = new Set(profile.actions.map((a) => a.id));
for (const a of profile.actions) {
  const v = validateAction(a);
  check(`action ${a.id} validates`, v.ok);
}
for (const [key, actionId] of Object.entries(profile.padBindings || {})) {
  check(`binding ${key} resolves`, ids.has(actionId));
}

const composerActs = profile.actions.filter((a) => a.type === "composer");
check("at least one composer action", composerActs.length >= 1);
check(
  "composer values reference composers map",
  composerActs.every((a) => profile.composers[a.value])
);

const blob = JSON.stringify(profile);
check("no /home/ paths", !blob.includes("/home/"));
check("no Bluetooth MAC", !/\b([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}\b/.test(blob));

const norm = normalizeComposers(profile.composers);
check("normalizeComposers keeps ai commands", norm.ai.commands.length >= 1);

check("malformed actions missing fails clearly", !validateAction(null).ok);

console.log(`\n${pass} passed, ${fail} failed (config profile)`);
process.exit(fail === 0 ? 0 : 1);
