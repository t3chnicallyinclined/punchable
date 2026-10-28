// Unit test for src/lib/flags.ts: the /api/health `flags` object -> switches, default on, build-time env override.
//   node --experimental-strip-types --no-warnings tests/flags.test.mjs
import * as F from "../src/lib/flags.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

check(F.readFlags({ ok: true, rev: "1", auth: "required" }).glassAccessory === true, "no flags -> default on");
check(F.readFlags(null).glassAccessory === true && F.readFlags("x").glassAccessory === true && F.readFlags(undefined).glassAccessory === true, "no body -> default on");
check(F.readFlags({ flags: { glass_accessory: false } }).glassAccessory === false, "flags.glass_accessory=false -> off");
check(F.readFlags({ flags: { glass_accessory: "0" } }).glassAccessory === false, "the string 0 -> off");
check(F.readFlags({ flags: { glass_accessory: "off" } }).glassAccessory === false, "the word off -> off");
check(F.readFlags({ flags: { glass_accessory: "maybe" } }).glassAccessory === true, "an unknown word -> default");
check(F.readFlags({ glass_accessory: false }).glassAccessory === false, "a top-level key is accepted");
check(F.readFlags({ glass_accessory: false, flags: { glass_accessory: true } }).glassAccessory === true, "the flags object wins");
check(F.readFlags({ flags: { glass_accessory: false } }, { glassAccessory: false }).glassAccessory === false, "a base of off stays off");
check(F.readFlags({ flags: {} }, { glassAccessory: false }).glassAccessory === false, "no key keeps the base");
check(F.flagsFromEnv({}).glassAccessory === true && F.flagsFromEnv({ EXPO_PUBLIC_GLASS_ACCESSORY: "0" }).glassAccessory === false, "EXPO_PUBLIC_GLASS_ACCESSORY=0 turns it off at build time");
check(F.asBool(1, false) === true && F.asBool(0, true) === false && F.asBool(" TRUE ", false) === true, "numbers and case");

if (fails.length) { console.log(`\n${fails.length} failed`); process.exit(1); }
console.log("\nall passed");
