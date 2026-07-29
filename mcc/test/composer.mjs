#!/usr/bin/env node
/**
 * Phase 2 — composer config normalization regressions (JS layer).
 * Rotate/timeout/stack FSM is covered in Rust mcc-desktop::composer tests.
 */
import { defaultComposers, normalizeComposers, validateAction } from "../src/lib.js";

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

console.log("HK Operator MCC — composer config regressions\n");

const d = defaultComposers();
check("default ai has 3 commands", d.ai.commands.length === 3);
check("default timeout 4000", d.ai.timeoutMs === 4000);
check("default separator is space", d.ai.separator === " ");

check("null raw → defaults", normalizeComposers(null).ai.commands[0] === "/help");
check("empty object keeps ai default", normalizeComposers({}).ai.timeoutMs === 4000);

const n = normalizeComposers({
  ai: { commands: [" /a ", "", " /b "], timeoutMs: 100, separator: "|" },
  custom: { commands: [] },
});
check("trims and filters blank commands", n.ai.commands.join(",") === "/a,/b");
check("timeoutMs floored at 500", n.ai.timeoutMs === 500);
check("custom separator preserved", n.ai.separator === "|");
check("empty commands → [/help]", n.custom.commands.join(",") === "/help");

const badTimeout = normalizeComposers({ ai: { commands: ["/x"], timeoutMs: "nope" } });
check("NaN timeout falls back then floors", badTimeout.ai.timeoutMs === 4000);

check(
  "composer action schema ok",
  validateAction({
    name: "cycle",
    category: "Cursor Prompts",
    type: "composer",
    value: "ai",
  }).ok
);

console.log(`\n${pass} passed, ${fail} failed (composer config)`);
process.exit(fail === 0 ? 0 : 1);
