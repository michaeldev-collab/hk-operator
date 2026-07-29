#!/usr/bin/env node
/**
 * Phase 4 P3-03 — command value fingerprint + allowlist helpers.
 */
import {
  commandValueFingerprint,
  normalizeAllowedCommands,
  isCommandAllowed,
} from "../src/lib.js";

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

console.log("HK Operator MCC — command allowlist fingerprint\n");

const fp = commandValueFingerprint("ls -la");
check("fingerprint is 16 hex chars", /^[0-9a-f]{16}$/.test(fp));
check("fingerprint stable", commandValueFingerprint("ls -la") === fp);
check("fingerprint changes with value", commandValueFingerprint("ls -la /tmp") !== fp);
// Golden vector shared with Rust dispatch::command_value_fingerprint("ls")
check(
  "golden vector for 'ls'",
  commandValueFingerprint("ls") === "08ad4d07b5541ae8"
);

check("legacy array normalizes empty", Object.keys(normalizeAllowedCommands(["a1"])).length === 0);
check(
  "object allowlist preserved",
  normalizeAllowedCommands({ a1: fp }).a1 === fp
);

const action = {
  id: "a1",
  type: "command",
  value: "ls -la",
};
check("isCommandAllowed true when fp matches", isCommandAllowed(action, { a1: fp }));
check(
  "isCommandAllowed false when value drifts",
  !isCommandAllowed({ ...action, value: "rm -rf /" }, { a1: fp })
);
check("isCommandAllowed false when missing", !isCommandAllowed(action, {}));

console.log(`\n${pass} passed, ${fail} failed (allowlist fingerprint)`);
process.exit(fail === 0 ? 0 : 1);
