import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

const consumers = [
  {
    file: new URL("../pages/settings/components/settings-toolbar.tsx", import.meta.url),
    wiring: [/onChange=\{onSearchChange\}/, /onClear=\{onSearchClear\}/],
  },
  {
    file: new URL("../pages/extensions/components/registry-tab.tsx", import.meta.url),
    wiring: [/onChange=\{setFilter\}/, /onClear=\{\(\) => setFilter\(""\)\}/],
  },
  {
    file: new URL("../components/sidebar-threads.tsx", import.meta.url),
    wiring: [/onChange=\{setQuery\}/, /onClear=\{\(\) => setQuery\(""\)\}/],
  },
];

test("common list filters keep their state wiring behind SearchField", () => {
  for (const consumer of consumers) {
    const source = readFileSync(consumer.file, "utf8");
    assert.match(source, /import \{ SearchField \} from/);
    assert.match(source, /<SearchField\b/);
    for (const pattern of consumer.wiring) assert.match(source, pattern);
  }
});
