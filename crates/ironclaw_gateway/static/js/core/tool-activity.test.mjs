import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

class ClassList {
  constructor(element) {
    this.element = element;
    this.classes = new Set();
  }

  add(...names) {
    for (const name of names) this.classes.add(name);
    this.sync();
  }

  remove(...names) {
    for (const name of names) this.classes.delete(name);
    this.sync();
  }

  contains(name) {
    return this.classes.has(name);
  }

  toggle(name, force) {
    const shouldAdd = force === undefined ? !this.classes.has(name) : !!force;
    if (shouldAdd) this.classes.add(name);
    else this.classes.delete(name);
    this.sync();
  }

  sync() {
    this.element._className = Array.from(this.classes).join(" ");
  }
}

class Element {
  constructor(tagName) {
    this.tagName = tagName;
    this.children = [];
    this.parentNode = null;
    this.style = {};
    this.attributes = new Map();
    this.eventListeners = new Map();
    this.classList = new ClassList(this);
    this._className = "";
    this._textContent = "";
  }

  set className(value) {
    this._className = value || "";
    this.classList.classes = new Set(this._className.split(/\s+/).filter(Boolean));
  }

  get className() {
    return this._className;
  }

  set textContent(value) {
    this._textContent = String(value ?? "");
    this.children = [];
  }

  get textContent() {
    return this._textContent + this.children.map((child) => child.textContent).join("");
  }

  set innerHTML(value) {
    this._textContent = String(value ?? "");
    this.children = [];
    for (const match of this._textContent.matchAll(/<([a-z0-9]+)[^>]*class="([^"]+)"/gi)) {
      const child = new Element(match[1]);
      child.className = match[2];
      this.appendChild(child);
    }
  }

  get innerHTML() {
    return this._textContent;
  }

  appendChild(child) {
    child.parentNode = this;
    this.children.push(child);
    return child;
  }

  remove() {
    if (!this.parentNode) return;
    this.parentNode.children = this.parentNode.children.filter((child) => child !== this);
    this.parentNode = null;
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

  addEventListener(type, listener) {
    this.eventListeners.set(type, listener);
  }

  querySelector(selector) {
    if (!selector.startsWith(".")) return null;
    const className = selector.slice(1);
    return this.find((element) => element.classList.contains(className));
  }

  querySelectorAll(selector) {
    if (!selector.startsWith(".")) return [];
    const className = selector.slice(1);
    const matches = [];
    this.find((element) => {
      if (element.classList.contains(className)) matches.push(element);
      return false;
    });
    return matches;
  }

  find(predicate) {
    for (const child of this.children) {
      if (predicate(child)) return child;
      const found = child.find(predicate);
      if (found) return found;
    }
    return null;
  }
}

function createHarness() {
  const chatMessages = new Element("div");
  const context = {
    clearInterval: () => {},
    console,
    Date,
    document: {
      createElement: (tagName) => new Element(tagName),
      getElementById: (id) => (id === "chat-messages" ? chatMessages : null),
    },
    setInterval: () => 1,
  };

  vm.runInNewContext(
    readFileSync(new URL("./tool-activity.js", import.meta.url), "utf8"),
    context,
  );

  return { chatMessages, context };
}

test("active tool cards show sanitized input and live status immediately", () => {
  const { chatMessages, context } = createHarness();

  context._chatToolActivity = context.createToolActivityController({
    containerId: "chat-messages",
  });
  context.addToolCard({
    call_id: "call_1",
    name: "shell",
    detail: "ls -la",
  });

  const body = chatMessages.querySelector(".activity-tool-body");
  const output = chatMessages.querySelector(".activity-tool-output");

  assert.equal(body.classList.contains("expanded"), true);
  assert.match(output.textContent, /Input:\nls -la/);
  assert.match(output.textContent, /Status:\nRunning/);
});

test("completed tool groups stay expanded with output visible", () => {
  const { chatMessages, context } = createHarness();

  context._chatToolActivity = context.createToolActivityController({
    containerId: "chat-messages",
  });
  context.addToolCard({
    call_id: "call_1",
    name: "shell",
    detail: "ls -la",
  });
  context.setToolCardOutput({
    call_id: "call_1",
    name: "shell",
    preview: "file_a\nfile_b",
  });
  context.completeToolCard({
    call_id: "call_1",
    name: "shell",
    success: true,
    duration_ms: 42,
  });
  context.finalizeActivityGroup();

  const summary = chatMessages.querySelector(".activity-summary");
  const cardsContainer = chatMessages.querySelector(".activity-cards-container");
  const output = chatMessages.querySelector(".activity-tool-output");

  assert.equal(summary.getAttribute("aria-expanded"), "true");
  assert.equal(cardsContainer.style.display, "flex");
  assert.match(output.textContent, /Input:\nls -la/);
  assert.match(output.textContent, /Output:\nfile_a\nfile_b/);
});
