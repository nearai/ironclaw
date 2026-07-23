// @ts-nocheck
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";
import { runVmModuleForTest } from "../../../test-support/vm-module-harness";
import {
  ConfigurationGroup,
  buildConfigurationSaveMutation,
} from "./configuration-tab";

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (node == null) return;
  fn(node);
  if (typeof node === "object") {
    for (const value of Object.values(node)) visit(value, fn);
  }
}

function findInput(root, type) {
  let found = null;
  visit(root, (node) => {
    if (!found && typeof node === "object" && node.type === "input" && node.props?.type === type) {
      found = node;
    }
  });
  return found;
}

function findForm(root) {
  let found = null;
  visit(root, (node) => {
    if (!found && typeof node === "object" && node.type === "form") {
      found = node;
    }
  });
  return found;
}

function createConfigurationGroupHarness(initialGroup, stateOverrides = {}) {
  const hooks = [];
  const pendingUpdates = [];
  let cursor = 0;
  let pendingEffects = [];
  let group = initialGroup;

  const dependenciesChanged = (previous, next) =>
    !previous || previous.length !== next.length || previous.some((value, index) => value !== next[index]);
  const ReactHarness = {
    useState(initial) {
      const index = cursor++;
      if (!hooks[index]) hooks[index] = { value: typeof initial === "function" ? initial() : initial };
      return [
        hooks[index].value,
        (next) => pendingUpdates.push(() => {
          hooks[index].value = typeof next === "function" ? next(hooks[index].value) : next;
        }),
      ];
    },
    useRef(initial) {
      const index = cursor++;
      if (!hooks[index]) hooks[index] = { value: { current: initial } };
      return hooks[index].value;
    },
    useMemo(factory, dependencies) {
      const index = cursor++;
      if (!hooks[index] || dependenciesChanged(hooks[index].dependencies, dependencies)) {
        hooks[index] = { value: factory(), dependencies };
      }
      return hooks[index].value;
    },
    useEffect(effect, dependencies) {
      const index = cursor++;
      if (!hooks[index] || dependenciesChanged(hooks[index].dependencies, dependencies)) {
        hooks[index] = { dependencies };
        pendingEffects.push(effect);
      }
    },
  };
  const { ConfigurationGroup: Component } = runVmModuleForTest(
    "./configuration-tab.tsx",
    ["ConfigurationGroup"],
    {
      React: ReactHarness,
      Button: "button",
      Input: "input",
      Panel: "section",
      clientActionId: () => "configuration-test-action",
      useAdminConfiguration: () => {},
    },
    import.meta.url,
  );
  const state = {
    isSaving: false,
    savingGroupId: null,
    saveError: null,
    save: async () => {},
    resetSave: () => {},
    ...stateOverrides,
  };
  let rendered;

  const render = () => {
    for (let pass = 0; pass < 10; pass += 1) {
      cursor = 0;
      pendingEffects = [];
      rendered = Component({ group, state });
      for (const effect of pendingEffects) effect();
      if (pendingUpdates.length === 0) return rendered;
      while (pendingUpdates.length > 0) pendingUpdates.shift()();
    }
    throw new Error("configuration group did not settle");
  };

  return {
    render,
    flushUpdates() {
      while (pendingUpdates.length > 0) pendingUpdates.shift()();
    },
    setGroup(nextGroup) {
      group = nextGroup;
    },
  };
}

test("configuration save mutation carries the loaded revision and client idempotency key", () => {
  const mutation = buildConfigurationSaveMutation(
    {
      group_id: "fixture.shared",
      revision: 12,
      fields: [
        { handle: "fixture_secret" },
        { handle: "public_name" },
      ],
    },
    {
      fixture_secret: "write-only",
      public_name: "fixture-bot",
    },
    "save-12-client-generated",
  );

  assert.deepEqual(mutation, {
    groupId: "fixture.shared",
    expectedRevision: 12,
    idempotencyKey: "save-12-client-generated",
    values: [
      { handle: "fixture_secret", value: "write-only" },
      { handle: "public_name", value: "fixture-bot" },
    ],
  });
});

test("configuration group renders generic operator fields and no lifecycle actions", () => {
  const html = renderToStaticMarkup(React.createElement(ConfigurationGroup, {
    group: {
      group_id: "fixture.shared",
      display_name: "Fixture credentials",
      description: "Shared deployment setup",
      complete: false,
      used_by: [
        { package_id: "fixture-one", display_name: "Fixture One", installed: false },
        { package_id: "fixture-two", display_name: "Fixture Two", installed: true },
      ],
      fields: [
        {
          handle: "fixture_secret",
          label: "Client secret",
          secret: true,
          required: true,
          provided: true,
          value: null,
        },
        {
          handle: "public_name",
          label: "Public name",
          secret: false,
          required: true,
          provided: true,
          value: "fixture-bot",
        },
      ],
    },
    state: {
      isSaving: false,
      savingGroupId: null,
      saveError: null,
      save: async () => {},
      resetSave: () => {},
    },
  }));

  assert.match(html, /Fixture credentials/);
  assert.match(html, /Client secret/);
  assert.match(html, /Configured\. Leave blank to keep/);
  assert.match(html, /value="fixture-bot"/);
  assert.doesNotMatch(html, /Set automatically by the provider/);
  assert.match(html, /Save configuration/);
  assert.doesNotMatch(html, />Install</);
  assert.doesNotMatch(html, />Remove</);
  assert.doesNotMatch(html, />Connect</);
});

test("configuration group keeps repeated secret pastes mounted and dirty across a manifest refetch", () => {
  const fixtureGroup = {
    group_id: "vendor.fixture.credentials",
    display_name: "Fixture deployment credentials",
    description: "Manifest-declared fixture fields",
    revision: 4,
    complete: true,
    used_by: [{ package_id: "vendor.fixture", display_name: "Fixture", installed: false }],
    fields: [
      {
        handle: "fixture_client_secret",
        label: "Client secret",
        secret: true,
        required: true,
        provided: true,
        value: "server-secret-material-must-never-render",
      },
      {
        handle: "fixture_endpoint",
        label: "Endpoint",
        secret: false,
        required: true,
        provided: true,
        value: "https://old.example.test",
      },
    ],
  };
  const harness = createConfigurationGroupHarness(fixtureGroup);
  let rendered = harness.render();
  assert.equal(findInput(rendered, "password").props.value, "");

  const pasted = "fixture-write-only-value";
  for (let count = 1; count <= 3; count += 1) {
    const event = { currentTarget: { value: pasted.repeat(count) } };
    findInput(rendered, "password").props.onChange(event);
    event.currentTarget = null;
    harness.flushUpdates();
    rendered = harness.render();
  }

  harness.setGroup({
    ...fixtureGroup,
    revision: 5,
    fields: fixtureGroup.fields.map((field) => field.secret
      ? { ...field, value: null }
      : { ...field, value: "https://refetched.example.test" }),
  });
  rendered = harness.render();

  assert.equal(findInput(rendered, "password").props.value, pasted.repeat(3));
  assert.equal(findInput(rendered, "text").props.value, "https://refetched.example.test");
});

test("configuration group reseeds from the group returned by save, not the pre-save values", async () => {
  const fixtureGroup = {
    group_id: "vendor.fixture.credentials",
    display_name: "Fixture deployment credentials",
    description: "Manifest-declared fixture fields",
    revision: 4,
    complete: true,
    used_by: [{ package_id: "vendor.fixture", display_name: "Fixture", installed: false }],
    fields: [
      {
        handle: "fixture_client_secret",
        label: "Client secret",
        secret: true,
        required: true,
        provided: true,
        value: "server-secret-material-must-never-render",
      },
      {
        handle: "fixture_endpoint",
        label: "Endpoint",
        secret: false,
        required: true,
        provided: true,
        value: "https://old.example.test",
      },
    ],
  };
  // `save` resolves WITH the persisted group — the non-secret value differs from
  // both the loaded value and whatever the operator typed, and the secret is
  // returned write-only (value: null).
  const savedGroup = {
    ...fixtureGroup,
    revision: 5,
    fields: fixtureGroup.fields.map((field) => field.secret
      ? { ...field, value: null }
      : { ...field, value: "https://saved.example.test" }),
  };
  const harness = createConfigurationGroupHarness(fixtureGroup, {
    save: async () => savedGroup,
  });
  let rendered = harness.render();

  // Type a throwaway value into the non-secret field so we can prove the
  // post-save reseed comes from the SAVED group, not this pre-save input.
  const typeEvent = { currentTarget: { value: "https://typed.example.test" } };
  findInput(rendered, "text").props.onChange(typeEvent);
  typeEvent.currentTarget = null;
  harness.flushUpdates();
  rendered = harness.render();
  assert.equal(findInput(rendered, "text").props.value, "https://typed.example.test");

  // Submit; the mutation resolves with the saved group.
  await findForm(rendered).props.onSubmit({ preventDefault() {} });
  harness.flushUpdates();
  rendered = harness.render();

  // Reseeded from the SAVED group's non-secret value (exercises the
  // `savedGroup?.fields` branch), not the typed value nor the loaded value.
  assert.equal(findInput(rendered, "text").props.value, "https://saved.example.test");
  // The secret stays blank after save — never the server-returned material.
  assert.equal(findInput(rendered, "password").props.value, "");
});
