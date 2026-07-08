import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

class FakeClassList {
  constructor(element) {
    this.element = element;
  }

  add(...names) {
    const classes = new Set(this.element.className.split(/\s+/).filter(Boolean));
    for (const name of names) classes.add(name);
    this.element.className = Array.from(classes).join(" ");
  }

  remove(...names) {
    const remove = new Set(names);
    this.element.className = this.element.className
      .split(/\s+/)
      .filter((name) => name && !remove.has(name))
      .join(" ");
  }

  contains(name) {
    return this.element.className.split(/\s+/).includes(name);
  }

  toggle(name, force) {
    const shouldAdd = force === undefined ? !this.contains(name) : !!force;
    if (shouldAdd) {
      this.add(name);
    } else {
      this.remove(name);
    }
    return shouldAdd;
  }
}

class FakeElement {
  constructor(tagName) {
    this.tagName = tagName.toUpperCase();
    this.attributes = new Map();
    this.children = [];
    this.className = "";
    this.classList = new FakeClassList(this);
    this.listeners = new Map();
    this.parentNode = null;
    this.style = {};
    this._innerHTML = "";
    this._textContent = "";
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
    return this.attributes.get(name) ?? null;
  }

  removeAttribute(name) {
    this.attributes.delete(name);
  }

  addEventListener(type, listener) {
    this.listeners.set(type, listener);
  }

  querySelector(selector) {
    if (!selector.startsWith(".")) return null;
    const className = selector.slice(1);
    return this.find((element) => element.classList.contains(className));
  }

  find(predicate) {
    for (const child of this.children) {
      if (predicate(child)) return child;
      const match = child.find(predicate);
      if (match) return match;
    }
    return null;
  }

  set innerHTML(value) {
    this._innerHTML = String(value);
    this.children = [];
    if (!this._innerHTML) return;
    if (this._innerHTML.includes("activity-summary-chevron")) {
      const chevron = new FakeElement("span");
      chevron.className = "activity-summary-chevron";
      this.appendChild(chevron);
    }
    if (this._innerHTML.includes("activity-summary-text")) {
      const text = new FakeElement("span");
      text.className = "activity-summary-text";
      this.appendChild(text);
    }
  }

  get innerHTML() {
    return this._innerHTML;
  }

  set textContent(value) {
    this._textContent = String(value);
  }

  get textContent() {
    return this._textContent;
  }
}

function createHarness() {
  const chatMessages = new FakeElement("div");
  const document = {
    createElement: (tagName) => new FakeElement(tagName),
    getElementById: (id) => (id === "chat-messages" ? chatMessages : null),
  };
  const context = {
    clearInterval: () => {},
    console,
    document,
    globalThis: {},
    isFinite,
    setInterval: () => 1,
  };

  vm.runInNewContext(
    `${readFileSync(new URL("./tool-activity.js", import.meta.url), "utf8")}
globalThis.__testExports = {
  createActivityGroupFromEntries,
  createToolActivityController,
  normalizeHistoryToolCall,
};`,
    context,
  );

  return {
    chatMessages,
    ...context.globalThis.__testExports,
  };
}

test("live tool cards show input summary while running", () => {
  const { chatMessages, createToolActivityController } = createHarness();
  const controller = createToolActivityController({ containerId: "chat-messages" });

  controller.startTool({
    call_id: "call-1",
    detail: "cmd: ls",
    name: "shell",
  });

  const card = chatMessages.querySelector(".activity-tool-card");
  const body = card.querySelector(".activity-tool-body");
  const output = card.querySelector(".activity-tool-output");

  assert.equal(card.getAttribute("data-status"), "running");
  assert.equal(body.classList.contains("expanded"), true);
  assert.equal(output.textContent, "Input:\ncmd: ls\n\nStatus:\nRunning");
});

test("finalized successful tool groups stay expanded with output details", () => {
  const { chatMessages, createToolActivityController } = createHarness();
  const controller = createToolActivityController({ containerId: "chat-messages" });

  controller.startTool({
    call_id: "call-1",
    detail: "path: README.md",
    name: "read_file",
  });
  controller.setResult({
    call_id: "call-1",
    name: "read_file",
    preview: "file contents",
  });
  controller.completeTool({
    call_id: "call-1",
    duration_ms: 25,
    name: "read_file",
    success: true,
  });
  controller.finalizeGroup();

  const summary = chatMessages.querySelector(".activity-summary");
  const cards = chatMessages.querySelector(".activity-cards-container");
  const body = chatMessages.querySelector(".activity-tool-body");
  const output = chatMessages.querySelector(".activity-tool-output");

  assert.equal(summary.getAttribute("aria-expanded"), "true");
  assert.equal(cards.style.display, "flex");
  assert.equal(body.classList.contains("expanded"), true);
  assert.equal(output.textContent, "Input:\npath: README.md\n\nOutput:\nfile contents");
});

test("failed tool cards avoid duplicate preformatted input labels", () => {
  const { chatMessages, createToolActivityController } = createHarness();
  const controller = createToolActivityController({ containerId: "chat-messages" });

  controller.startTool({
    call_id: "call-1",
    detail: "cmd: fail",
    name: "shell",
  });
  controller.completeTool({
    call_id: "call-1",
    error: "boom",
    name: "shell",
    parameters: '{"cmd":"fail"}',
    success: false,
  });

  const output = chatMessages.querySelector(".activity-tool-output");

  assert.equal(output.textContent, 'Input:\n{"cmd":"fail"}\n\nError:\nboom');
});

test("history tool groups render input summaries expanded by default", () => {
  const {
    createActivityGroupFromEntries,
    normalizeHistoryToolCall,
  } = createHarness();
  const group = createActivityGroupFromEntries(
    [
      normalizeHistoryToolCall({
        has_result: true,
        input_summary: "path: README.md",
        name: "read_file",
        result_preview: "file contents",
      }),
    ],
    {
      expandErrors: true,
      includeSummaryDuration: false,
      showCardDurations: false,
    },
  );

  const cards = group.querySelector(".activity-cards-container");
  const body = group.querySelector(".activity-tool-body");
  const output = group.querySelector(".activity-tool-output");

  assert.equal(group.classList.contains("collapsed"), false);
  assert.equal(cards.style.display, "flex");
  assert.equal(body.classList.contains("expanded"), true);
  assert.equal(output.textContent, "Input:\npath: README.md\n\nOutput:\nfile contents");
});
