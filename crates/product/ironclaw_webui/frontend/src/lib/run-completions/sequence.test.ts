import assert from "node:assert/strict";
import { test } from "vitest";

import { compareSequences } from "./sequence";

test("sequences compare as decimal u64 strings, not lexically", () => {
  assert.ok(compareSequences("9", "10") < 0);
  assert.ok(compareSequences("99", "100") < 0);
  assert.ok(compareSequences("100", "99") > 0);
  assert.equal(compareSequences("42", "42"), 0);
  assert.ok(compareSequences("18446744073709551615", "18446744073709551614") > 0);
});
