export class FakeClassList {
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

export class FakeElement {
  constructor(tagName) {
    this.tagName = tagName;
    this.children = [];
    this.parentNode = null;
    this.className = "";
    this.classList = new FakeClassList(this);
    this.attributes = new Map();
    this.style = {};
    this.textContent = "";
    this.content = "";
    this.role = "";
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

  querySelectorAll(selector) {
    if (selector === ".message.assistant, .message.system") {
      return this.children.filter((child) =>
        child.classList.contains("message")
        && (child.classList.contains("assistant") || child.classList.contains("system"))
      );
    }
    if (selector === ".message" || selector === "#chat-messages .message") {
      return this.children.filter((child) => child.classList.contains("message"));
    }
    return [];
  }

  set innerHTML(value) {
    this._innerHTML = String(value);
    this.children = [];
  }

  get innerHTML() {
    return this._innerHTML;
  }
}

export function createMessageElement(role, content = "") {
  const element = new FakeElement("div");
  element.className = `message ${role}`;
  element.role = role;
  element.content = content;
  element.textContent = content;
  return element;
}

export function createChatContainer(initialChildren = []) {
  const container = new FakeElement("div");
  container.className = "chat-messages";
  for (const child of initialChildren) container.appendChild(child);
  return container;
}

export function chatChildKinds(container) {
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
