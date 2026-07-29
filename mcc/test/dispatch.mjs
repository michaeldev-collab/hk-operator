#!/usr/bin/env node
/**
 * Phase 2 — dispatch schema regressions (HW-independent, no shell).
 * URL scheme check matches Rust `url_gate` (ASCII-case-insensitive http(s)).
 */
import { validateAction, ACTION_TYPES } from "../src/lib.js";

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

const base = { name: "t", category: "URLs", type: "url", value: "https://example.com" };

console.log("HK Operator MCC — dispatch schema regressions\n");

check("ACTION_TYPES includes command and composer", ACTION_TYPES.includes("command") && ACTION_TYPES.includes("composer"));
check("https url ok", validateAction(base).ok);
check("http url ok", validateAction({ ...base, value: "http://example.com" }).ok);
check("ftp url rejected", !validateAction({ ...base, value: "ftp://example.com" }).ok);
check("bare host rejected", !validateAction({ ...base, value: "example.com" }).ok);
check("HTTPS uppercase url ok (aligned with Rust url_gate)", validateAction({ ...base, value: "HTTPS://example.com" }).ok);

check(
  "unknown type rejected",
  !validateAction({ name: "x", category: "URLs", type: "shell", value: "x" }).ok
);
check(
  "command type validates with value",
  validateAction({
    name: "ls",
    category: "Terminal Commands",
    type: "command",
    value: "ls -la",
  }).ok
);
check(
  "command empty value fails",
  !validateAction({
    name: "ls",
    category: "Terminal Commands",
    type: "command",
    value: "  ",
  }).ok
);
check(
  "composer type validates with id",
  validateAction({
    name: "AI",
    category: "Cursor Prompts",
    type: "composer",
    value: "ai",
  }).ok
);
check(
  "path type validates",
  validateAction({
    name: "home",
    category: "Project Paths",
    type: "path",
    value: "~/projects",
  }).ok
);
check(
  "note type validates",
  validateAction({
    name: "snip",
    category: "Prompts",
    type: "note",
    value: "hello",
  }).ok
);

console.log(`\n${pass} passed, ${fail} failed (dispatch schema)`);
process.exit(fail === 0 ? 0 : 1);
