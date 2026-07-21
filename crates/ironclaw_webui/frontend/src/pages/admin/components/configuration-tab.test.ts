// @ts-nocheck
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";
import { ConfigurationGroup } from "./configuration-tab";

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
          input: "operator",
          provided: true,
          value: null,
        },
        {
          handle: "public_name",
          label: "Public name",
          secret: false,
          required: true,
          input: "provisioned",
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
  assert.match(html, /value="fixture-bot"[^>]*disabled|disabled=""[^>]*value="fixture-bot"/);
  assert.match(html, /Save configuration/);
  assert.doesNotMatch(html, />Install</);
  assert.doesNotMatch(html, />Remove</);
  assert.doesNotMatch(html, />Connect</);
});
