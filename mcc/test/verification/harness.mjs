#!/usr/bin/env node
/**
 * Phase 1 verification harness stub inventory.
 * Does not claim hardware paths are verified — lists capabilities and exits 0
 * when the stub scaffold is intact. Phase 2 fills real assertions.
 *
 * Run: npm run test:harness
 */
import { readFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "../..");
const docsHarness = join(root, "../docs/verification/harness.md");
const examples = join(root, "../config/examples/dev.json");
const protocol = join(root, "protocol/PROTOCOL.md");

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

/** Capability stubs — status is intentional scaffolding, not a green claim. */
const CAPABILITIES = [
  { id: "V-DISCOVER", layer: "hitl", note: "BlueZ discovery by compat name" },
  { id: "V-BOND", layer: "hitl", note: "Reuse bonded HID link" },
  { id: "V-INFO", layer: "hitl", note: "Info characteristic string" },
  { id: "V-SLOTS-R", layer: "tested", note: "Read slots (slots_codec + unit)" },
  { id: "V-SLOTS-W", layer: "partial", note: "Pack round-trip unit; HITL write remains" },
  { id: "V-MACRO", layer: "partial", note: "MacroEvent::from_bytes unit; HITL notify remains" },
  { id: "V-IDX", layer: "tested", note: "Preset/action index bounds (PadSlots::get)" },
  { id: "V-HID-FALLBACK", layer: "hitl", note: "HID with MCC closed" },
  { id: "V-GATT-DOWN", layer: "hitl", note: "Degraded GATT" },
  { id: "V-PROFILE-IO", layer: "partial", note: "Example profile + normalize (config_profile.mjs)" },
  { id: "V-GIT-SYNC", layer: "stub", note: "Pull / pull-apply messaging" },
  { id: "V-COMPOSER", layer: "tested", note: "Rotate/lock/stack pure FSM + JS normalize" },
  { id: "V-DISPATCH", layer: "tested", note: "URL/allowlist/unknown-type gates (no shell)" },
  { id: "V-FAIL", layer: "partial", note: "Gate error strings covered; UI paths HITL" },
];

console.log("HK Operator — verification harness stubs (Phase 1)\n");

check("docs/verification/harness.md exists", existsSync(docsHarness));
check("protocol doc exists", existsSync(protocol));
check("public example profile exists", existsSync(examples));

if (existsSync(examples)) {
  try {
    const profile = JSON.parse(readFileSync(examples, "utf8"));
    check("example profile has actions array", Array.isArray(profile.actions));
    const blob = JSON.stringify(profile);
    check("example profile has no /home/ absolute paths", !blob.includes("/home/"));
    check("example profile has no Bluetooth MAC pattern", !/\b([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}\b/.test(blob));
  } catch (e) {
    check("example profile parses as JSON", false);
  }
}

console.log("\nCapability inventory (Phase 2 fills tested/partial; HITL still open):\n");
for (const c of CAPABILITIES) {
  console.log(`  [${c.layer.padEnd(7)}] ${c.id} — ${c.note}`);
}
check("capability inventory has 14 entries", CAPABILITIES.length === 14);
check(
  "no capability claims full HITL 'verified'",
  CAPABILITIES.every((c) => c.layer !== "verified")
);

console.log(`\n${pass} passed, ${fail} failed (harness stubs)`);
process.exit(fail === 0 ? 0 : 1);
