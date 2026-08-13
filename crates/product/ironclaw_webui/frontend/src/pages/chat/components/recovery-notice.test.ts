// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import { componentSourceForTest } from "../../../lib/vm-component-harness";

test("RecoveryNotice routes its recovery action through i18n", () => {
  const context = {
    globalThis: {},
    useT: () => (key) => `translated:${key}`,
  };
  vm.runInNewContext(
    componentSourceForTest(
      new URL("./recovery-notice.tsx", import.meta.url),
      "RecoveryNotice",
    ),
    context,
  );

  const rendered = context.globalThis.__testExports.RecoveryNotice({
    notice: { status: "ready", message: "fixture recovery detail" },
    onRecover() {},
  });
  assert.match(JSON.stringify(rendered), /translated:chat\.reloadHistory/);
});
