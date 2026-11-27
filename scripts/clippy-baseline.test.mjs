import { test } from "node:test";
import assert from "node:assert/strict";
import { compare, countWarnings } from "./clippy-baseline.mjs";

function warning(code, file, line, { level = "warning", target = "lib" } = {}) {
  return JSON.stringify({
    reason: "compiler-message",
    target: { kind: [target], name: "instanceharbor" },
    message: {
      level,
      code: code && { code },
      spans: [{ is_primary: true, file_name: file, line_start: line, column_start: 5 }],
    },
  });
}

test("counts distinct spans per lint and file", () => {
  const output = [
    warning("clippy::ptr_arg", "src\\a.rs", 10),
    warning("clippy::ptr_arg", "src\\a.rs", 10, { target: "test" }),
    warning("clippy::ptr_arg", "src\\a.rs", 20),
    warning("clippy::ptr_arg", "src/b.rs", 1),
    warning(null, "build.rs", 1),
    warning("E0308", "src/a.rs", 3, { level: "error" }),
    JSON.stringify({ reason: "compiler-artifact" }),
    "",
  ].join("\n");
  assert.deepEqual(countWarnings(output), { "clippy::ptr_arg src/a.rs": 2, "clippy::ptr_arg src/b.rs": 1 });
});

test("reports only warnings beyond the baseline as added", () => {
  const baseline = { "clippy::ptr_arg src/a.rs": 2, "clippy::collapsible_if src/c.rs": 1 };
  const current = { "clippy::ptr_arg src/a.rs": 3, "clippy::needless_borrow src/d.rs": 1 };
  assert.deepEqual(compare(baseline, current), {
    added: [
      { key: "clippy::ptr_arg src/a.rs", allowed: 2, count: 3 },
      { key: "clippy::needless_borrow src/d.rs", allowed: 0, count: 1 },
    ],
    reduced: [{ key: "clippy::collapsible_if src/c.rs", allowed: 1, count: 0 }],
  });
});

test("passes when warnings match or drop", () => {
  const baseline = { "clippy::ptr_arg src/a.rs": 2 };
  assert.deepEqual(compare(baseline, { "clippy::ptr_arg src/a.rs": 1 }).added, []);
  assert.deepEqual(compare(baseline, baseline), { added: [], reduced: [] });
});
