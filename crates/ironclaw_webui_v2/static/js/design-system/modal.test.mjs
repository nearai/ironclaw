import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const modalSource = readFileSync(new URL("./modal.js", import.meta.url), "utf8");

test("Modal title shortcut passes localized close labels to ModalHeader", () => {
  assert.match(
    modalSource,
    /import \{ useT \} from "\.\.\/lib\/i18n\.js";/,
    "ModalHeader should read the active i18n context",
  );
  assert.match(
    modalSource,
    /<\$\{ModalHeader\} onClose=\$\{onClose\} closeLabel=\$\{closeLabel\}>/,
    "Modal should pass closeLabel through its title shortcut",
  );
  assert.match(
    modalSource,
    /const effectiveCloseLabel = closeLabel \|\| t\("common\.close"\);/,
    "ModalHeader should fall back to the localized close label",
  );
});
