import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

class FakeElement {
  constructor(id) {
    this.id = id;
    this.attributes = new Map();
    this.hidden = false;
    this.listeners = {};
    this.textContent = "";
    this.value = "";
  }

  addEventListener(type, listener) {
    this.listeners[type] = listener;
  }

  dispatch(type) {
    if (this.listeners[type]) this.listeners[type]({ target: this });
  }

  removeAttribute(name) {
    this.attributes.delete(name);
  }

  setAttribute(name, value) {
    this.attributes.set(name, value);
  }

  getAttribute(name) {
    return this.attributes.get(name) || null;
  }
}

function createHarness() {
  const elements = new Map();
  for (const id of [
    "skill-install-name",
    "skill-install-name-error",
    "skill-install-url",
    "skill-install-url-error",
    "skill-search-input",
  ]) {
    elements.set(id, new FakeElement(id));
  }

  const toasts = [];
  const installs = [];
  const context = {
    I18n: {
      t(key, values) {
        if (key === "skills.nameRequired") return "Skill name is required";
        if (key === "skills.httpsRequired") return "URL must use HTTPS";
        if (key === "skills.confirmInstall") return `Install skill "${values.name}"?`;
        return key;
      },
    },
    apiFetch(url, options) {
      installs.push({ url, options });
      return Promise.resolve({ success: true });
    },
    confirm: () => true,
    document: {
      createElement: (tag) => new FakeElement(tag),
      getElementById: (id) => elements.get(id) || null,
    },
    encodeURIComponent,
    formatTimeAgo: () => "",
    loadSkills: () => {},
    Promise,
    setSlashSkillEntries: () => {},
    showConfirmModal: () => {},
    showToast: (message, type) => toasts.push({ message, type }),
  };

  vm.runInNewContext(
    readFileSync(new URL("./skills.js", import.meta.url), "utf8"),
    context,
  );

  return { context, elements, installs, toasts };
}

test("skill install validation clears name error when the name becomes valid", () => {
  const harness = createHarness();
  const name = harness.elements.get("skill-install-name");
  const error = harness.elements.get("skill-install-name-error");

  harness.context.installSkillFromForm();

  assert.equal(error.hidden, false);
  assert.equal(error.textContent, "Skill name is required");
  assert.equal(name.getAttribute("aria-invalid"), "true");

  name.value = "summarizer";
  name.dispatch("input");

  assert.equal(error.hidden, true);
  assert.equal(error.textContent, "");
  assert.equal(name.getAttribute("aria-invalid"), null);
});

test("skill install validation clears url error when the url becomes valid", () => {
  const harness = createHarness();
  const name = harness.elements.get("skill-install-name");
  const url = harness.elements.get("skill-install-url");
  const error = harness.elements.get("skill-install-url-error");
  name.value = "summarizer";
  url.value = "http://example.com/SKILL.md";

  harness.context.installSkillFromForm();

  assert.equal(error.hidden, false);
  assert.equal(error.textContent, "URL must use HTTPS");
  assert.equal(url.getAttribute("aria-invalid"), "true");

  url.value = "https://example.com/SKILL.md";
  url.dispatch("input");

  assert.equal(error.hidden, true);
  assert.equal(error.textContent, "");
  assert.equal(url.getAttribute("aria-invalid"), null);
});
