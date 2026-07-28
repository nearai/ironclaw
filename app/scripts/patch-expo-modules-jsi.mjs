import { readFile, writeFile } from "node:fs/promises";

const path =
  "node_modules/expo-modules-jsi/apple/Sources/ExpoModulesJSI/Coding/JavaScriptCodable+Date.swift";
const original = `  guard milliseconds.isFinite, abs(milliseconds) <= maxJavaScriptDateMilliseconds else {
    throw InvalidDateException()
  }
  return Date(timeIntervalSince1970: milliseconds.rounded(.towardZero) / 1000.0)`;
const patched = `  let magnitude: Double = Swift.abs(milliseconds)
  guard milliseconds.isFinite, magnitude <= maxJavaScriptDateMilliseconds else {
    throw InvalidDateException()
  }
  let clippedMilliseconds = milliseconds.rounded(FloatingPointRoundingRule.towardZero)
  return Date(timeIntervalSince1970: clippedMilliseconds / 1000.0)`;

const source = await readFile(path, "utf8");
if (source.includes(patched)) {
  process.exit(0);
}
if (!source.includes(original)) {
  throw new Error(`ExpoModulesJSI source changed; review the Xcode compatibility patch at ${path}`);
}
await writeFile(path, source.replace(original, patched));
console.log("Applied ExpoModulesJSI Xcode 26 compatibility patch");
