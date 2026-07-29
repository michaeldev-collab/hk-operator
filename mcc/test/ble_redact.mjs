import { redactBleAddress } from "../src/lib.js";

function check(name, cond) {
  if (!cond) {
    console.error(`  FAIL  ${name}`);
    process.exitCode = 1;
    return;
  }
  console.log(`  PASS  ${name}`);
}

console.log("HK Operator MCC — BLE address redaction (P3-08)\n");

check(
  "keeps last octet uppercase",
  redactBleAddress("aa:bb:cc:dd:ee:ff") === "**:**:**:**:**:FF"
);
check(
  "accepts hyphen separators",
  redactBleAddress("AA-BB-CC-DD-EE-12") === "**:**:**:**:**:12"
);
check("empty → sentinel", redactBleAddress("") === "(no address)");
check("nullish → sentinel", redactBleAddress(null) === "(no address)");
check("garbage fully masked", redactBleAddress("not-a-mac") === "**:**:**:**:**:**");
check(
  "does not leak prefix octets",
  !redactBleAddress("11:22:33:44:55:66").includes("11:22")
);

const failed = process.exitCode === 1;
console.log(`\n${failed ? "FAILED" : "6 passed, 0 failed"} (ble redact)`);
if (failed) process.exit(1);
