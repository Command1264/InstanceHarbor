// Compares `cargo clippy --message-format=json` output with the warnings recorded in
// src-tauri/clippy-baseline.json, so CI fails only on warnings that develop does not
// already have (#122).
//
//   cargo clippy --locked --all-targets --message-format=json > clippy.json
//   node scripts/clippy-baseline.mjs check clippy.json
//   node scripts/clippy-baseline.mjs write clippy.json   # after fixing warnings
//
// Warnings are counted per lint and file, not per line, so unrelated edits that move
// a warning do not fail the check. The same span reported for the library and its
// test target counts once.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

export const BASELINE = fileURLToPath(new URL("../src-tauri/clippy-baseline.json", import.meta.url));

/** Counts of distinct warning spans, keyed by `lint path`, from clippy JSON lines. */
export function countWarnings(jsonLines) {
  const spans = new Set();
  for (const line of jsonLines.split(/\r?\n/)) {
    if (!line.trim().startsWith("{")) continue;
    const message = JSON.parse(line);
    if (message.reason !== "compiler-message") continue;
    const diagnostic = message.message;
    if (diagnostic.level !== "warning" || !diagnostic.code) continue;
    const span = diagnostic.spans.find((s) => s.is_primary) ?? diagnostic.spans[0];
    const file = span ? span.file_name.replaceAll("\\", "/") : "(no file)";
    const at = span ? `${span.line_start}:${span.column_start}` : "";
    spans.add(JSON.stringify([diagnostic.code.code, file, at]));
  }
  const counts = {};
  for (const entry of spans) {
    const [code, file] = JSON.parse(entry);
    const key = `${code} ${file}`;
    counts[key] = (counts[key] ?? 0) + 1;
  }
  return Object.fromEntries(Object.entries(counts).sort(([a], [b]) => a.localeCompare(b)));
}

/** Keys whose count grew past the baseline, and keys that now have fewer warnings. */
export function compare(baseline, current) {
  const added = [];
  const reduced = [];
  for (const [key, count] of Object.entries(current)) {
    const allowed = baseline[key] ?? 0;
    if (count > allowed) added.push({ key, allowed, count });
  }
  for (const [key, allowed] of Object.entries(baseline)) {
    const count = current[key] ?? 0;
    if (count < allowed) reduced.push({ key, allowed, count });
  }
  return { added, reduced };
}

function main([command, input]) {
  if (!["check", "write"].includes(command) || !input) {
    console.error("Usage: node scripts/clippy-baseline.mjs <check|write> <clippy-json-file>");
    return 2;
  }
  const current = countWarnings(readFileSync(input, "utf8"));
  if (command === "write") {
    writeFileSync(BASELINE, JSON.stringify(current, null, 2) + "\n");
    console.log(`Wrote ${Object.keys(current).length} entries to ${path.basename(BASELINE)}.`);
    return 0;
  }
  const baseline = JSON.parse(readFileSync(BASELINE, "utf8"));
  const { added, reduced } = compare(baseline, current);
  for (const { key, allowed, count } of reduced) {
    console.log(`Fewer warnings than the baseline: ${key} (${allowed} -> ${count}). Run \`write\` to lower the baseline.`);
  }
  if (added.length === 0) {
    console.log("No clippy warnings beyond src-tauri/clippy-baseline.json.");
    return 0;
  }
  for (const { key, allowed, count } of added) {
    console.error(`New clippy warning: ${key} (baseline ${allowed}, now ${count}).`);
  }
  console.error("Fix the new warnings; the cargo clippy output above shows where they are.");
  return 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main(process.argv.slice(2));
}
