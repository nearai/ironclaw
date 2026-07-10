import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

class FakeClassList {
  constructor(owner) {
    this.owner = owner;
  }

  values() {
    return String(this.owner.className || "")
      .split(/\s+/)
      .filter(Boolean);
  }

  contains(name) {
    return this.values().includes(name);
  }

  add(...names) {
    const next = new Set(this.values());
    for (const name of names) next.add(name);
    this.owner.className = Array.from(next).join(" ");
  }

  remove(...names) {
    const remove = new Set(names);
    this.owner.className = this.values()
      .filter((name) => !remove.has(name))
      .join(" ");
  }

  toggle(name, force) {
    const has = this.contains(name);
    const shouldAdd = force === undefined ? !has : Boolean(force);
    if (shouldAdd) this.add(name);
    else this.remove(name);
    return shouldAdd;
  }
}

class FakeElement {
  constructor(tagName) {
    this.tagName = tagName;
    this.children = [];
    this.parentNode = null;
    this.className = "";
    this.classList = new FakeClassList(this);
    this.attributes = new Map();
    this.style = {};
    this.textContent = "";
    this._innerHTML = "";
  }

  appendChild(child) {
    if (child.parentNode) child.parentNode.removeChild(child);
    child.parentNode = this;
    this.children.push(child);
    return child;
  }

  insertBefore(child, anchor) {
    if (!anchor) return this.appendChild(child);
    const index = this.children.indexOf(anchor);
    if (index < 0) return this.appendChild(child);
    if (child.parentNode) child.parentNode.removeChild(child);
    child.parentNode = this;
    this.children.splice(index, 0, child);
    return child;
  }

  removeChild(child) {
    const index = this.children.indexOf(child);
    if (index >= 0) this.children.splice(index, 1);
    child.parentNode = null;
    return child;
  }

  remove() {
    if (this.parentNode) this.parentNode.removeChild(this);
  }

  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }

  getAttribute(name) {
    return this.attributes.get(name) || null;
  }

  removeAttribute(name) {
    this.attributes.delete(name);
  }

  addEventListener() {}

  querySelector() {
    return null;
  }

  set innerHTML(value) {
    this._innerHTML = String(value);
    this.children = [];
  }

  get innerHTML() {
    return this._innerHTML;
  }
}

function message(role) {
  const element = new FakeElement("div");
  element.className = `message ${role}`;
  return element;
}

function createHarness(initialChildren) {
  const container = new FakeElement("div");
  container.className = "chat-messages";
  for (const child of initialChildren) container.appendChild(child);

  const context = {
    clearInterval: () => {},
    Date,
    document: {
      createElement: (tagName) => new FakeElement(tagName),
      getElementById: (id) => (id === "chat-messages" ? container : null),
    },
    isFinite,
    Map,
    Number,
    setInterval: () => 1,
  };
  vm.runInNewContext(
    readFileSync(new URL("./tool-activity.js", import.meta.url), "utf8"),
    context,
  );

  return {
    container,
    controller: context.createToolActivityController({
      containerId: "chat-messages",
    }),
  };
}

function childKinds(container) {
  return container.children.map((child) => {
    if (child.classList.contains("activity-group")) return "activity";
    if (child.classList.contains("message")) {
      if (child.classList.contains("assistant")) return "assistant";
      if (child.classList.contains("user")) return "user";
      if (child.classList.contains("system")) return "system";
      return "message";
    }
    return child.tagName;
  });
}

test("tool activity started after a trailing assistant reply renders before that reply", () => {
  const { container, controller } = createHarness([
    message("user"),
    message("assistant"),
  ]);

  controller.startTool({
    call_id: "call-extension-search",
    name: "extension_search",
  });

  assert.deepEqual(childKinds(container), ["user", "activity", "assistant"]);

  controller.completeTool({
    call_id: "call-extension-search",
    name: "extension_search",
    success: true,
    duration_ms: 25,
  });
  controller.finalizeGroup();

  assert.deepEqual(childKinds(container), ["user", "activity", "assistant"]);
});

test("tool activity after a follow-up user message stays with the active follow-up turn", () => {
  const { container, controller } = createHarness([
    message("user"),
    message("assistant"),
    message("user"),
  ]);

  controller.startTool({
    call_id: "call-calendar",
    name: "calendar",
  });

  assert.deepEqual(childKinds(container), [
    "user",
    "assistant",
    "user",
    "activity",
  ]);
});

test("tool activity does not skip non-message cards after an assistant reply", () => {
  const card = new FakeElement("div");
  card.className = "auth-card";
  const { container, controller } = createHarness([
    message("user"),
    message("assistant"),
    card,
  ]);

  controller.startTool({
    call_id: "call-drive",
    name: "drive",
  });

  assert.deepEqual(childKinds(container), [
    "user",
    "assistant",
    "div",
    "activity",
  ]);
});
