import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test, vi } from "vitest";

import { formatThreadActivityTooltip } from "../../../lib/thread-meta";

vi.mock("../../../design-system/icons", async () => {
  const { createElement } = await import("react");
  return {
    Icon: ({ name, className }) =>
      createElement("span", { className, "data-icon": name }),
  };
});

vi.mock("../../../lib/toast", () => ({ toast: () => {} }));
vi.mock("../../../lib/i18n", () => ({ useT: () => (key) => key }));

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

test("a success result renders a heading, left-aligned field rows, and readable lines", async () => {
  const { CommandResult } = await import("./command-result");
  const html = renderToStaticMarkup(
    React.createElement(CommandResult, {
      response: {
        command: "status",
        result: {
          title: "Status",
          fields: [
            { label: "State", value: "idle" },
            { label: "Run", value: "1b9894d9-3f21-4a10-9abc-def012345678" },
            { label: "Since", value: "2026-07-30T13:18:49Z" },
          ],
          lines: ["The last task was cancelled."],
        },
      },
    }),
  );

  assert.match(html, /data-testid="command-result"/);
  assert.match(html, /<h3[^>]*>Status<\/h3>/, "the result title should render as a heading");
  assert.match(html, /<dl/, "fields should render as a definition list");
  assert.match(html, /<dt[^>]*>State<\/dt>/);
  assert.match(html, /<dd[^>]*>[\s\S]*?idle[\s\S]*?<\/dd>/);
  assert.match(html, /The last task was cancelled\./);
  assert.match(html, /role="list"/, "the lines list should expose real list semantics");
  assert.doesNotMatch(
    html,
    /text-center/,
    "the redesigned result must not regress to centered prose",
  );
});

test("a run-id-shaped field value renders monospace, truncatable, with a copy affordance", async () => {
  const { CommandResult } = await import("./command-result");
  const runId = "1b9894d9-3f21-4a10-9abc-def012345678";
  const html = renderToStaticMarkup(
    React.createElement(CommandResult, {
      response: {
        command: "status",
        result: {
          title: "Status",
          fields: [{ label: "Run", value: runId }],
          lines: [],
        },
      },
    }),
  );

  const valueElement = html.match(new RegExp(`<span[^>]*>${runId}</span>`));
  assert.ok(valueElement, "expected a span wrapping the raw identifier value");
  assert.match(valueElement[0], /font-mono/, "an identifier value should render monospace");
  assert.match(
    valueElement[0],
    new RegExp(`title="${runId}"`),
    "the full value should be available via the native title tooltip (CSS truncation keeps the DOM text intact for a11y)",
  );
  assert.match(
    html,
    /data-icon="copy"/,
    "the identifier value should offer a copy affordance",
  );
});

test("an ISO timestamp field value renders through the existing human-readable date helper, not the raw ISO string", async () => {
  const { CommandResult } = await import("./command-result");
  const iso = "2026-07-30T13:18:49Z";
  const expected = formatThreadActivityTooltip(iso);
  const html = renderToStaticMarkup(
    React.createElement(CommandResult, {
      response: {
        command: "status",
        result: {
          title: "Status",
          fields: [{ label: "Since", value: iso }],
          lines: [],
        },
      },
    }),
  );

  assert.match(html, new RegExp(`dateTime="${iso}"`));
  assert.match(html, new RegExp(`<time[^>]*>${escapeRegExp(expected)}</time>`));
  assert.doesNotMatch(
    html,
    />2026-07-30T13:18:49Z</,
    "the raw ISO string should not be the visible text",
  );
});

test("a plain (non-identifier, non-timestamp) field value renders as ordinary left-aligned text", async () => {
  const { CommandResult } = await import("./command-result");
  const html = renderToStaticMarkup(
    React.createElement(CommandResult, {
      response: {
        command: "status",
        result: {
          title: "Status",
          fields: [{ label: "State", value: "idle" }],
          lines: [],
        },
      },
    }),
  );

  assert.doesNotMatch(html, /data-icon="copy"/);
  assert.doesNotMatch(html, /font-mono/);
});

test("the available-commands rejection renders the server inventory as dropdown-echoing rows, not the raw backend blob", async () => {
  const { CommandResult } = await import("./command-result");
  const commands = [
    { name: "model", title: "Model", description: "Show or switch the active model", usage: "/model" },
    { name: "status", title: "Status", description: "Show what the assistant is doing", usage: "/status" },
  ];
  const html = renderToStaticMarkup(
    React.createElement(CommandResult, {
      response: {
        command: "",
        rejection: {
          kind: "invalid_request",
          message: "Available commands:\n/model\n/status",
        },
      },
      commands,
    }),
  );

  assert.match(html, /chat\.commandListTitle/, "the header uses the localized heading key");
  assert.match(html, /\/model/);
  assert.match(html, /Show or switch the active model/);
  assert.match(html, /\/status/);
  assert.match(html, /role="list"/);
  assert.doesNotMatch(
    html,
    /Available commands:/,
    "the raw backend blob must not be shown verbatim once the real inventory is rendered",
  );
  assert.doesNotMatch(html, /text-center/);
});

test("the available-commands rejection falls back to the backend's plain text when the inventory is unavailable", async () => {
  const { CommandResult } = await import("./command-result");
  const html = renderToStaticMarkup(
    React.createElement(CommandResult, {
      response: {
        command: "",
        rejection: {
          kind: "invalid_request",
          message: "Available commands:\n/model\n/status",
        },
      },
      commands: [],
    }),
  );

  assert.match(html, /data-testid="command-result-list-fallback"/);
  assert.match(html, /role="status"/);
  assert.match(html, /Available commands:/);
  assert.doesNotMatch(html, /text-center/);
});

test("a denial rejection renders as a calm inline notice, not the command-list or success card", async () => {
  const { CommandResult } = await import("./command-result");
  const html = renderToStaticMarkup(
    React.createElement(CommandResult, {
      response: {
        command: "extension_install",
        rejection: {
          kind: "access_denied",
          message: "This command requires an admin account.",
        },
      },
    }),
  );

  assert.match(html, /data-testid="command-result-denial"/);
  assert.match(html, /role="status"/);
  assert.match(html, /This command requires an admin account\./);
  assert.match(html, /data-icon="lock"/);
  assert.doesNotMatch(
    html,
    /data-testid="command-result"/,
    "a denial must not also render the success/command-list card shell",
  );
  assert.doesNotMatch(html, /text-center/);
});

test("a response with neither result nor rejection renders nothing (defensive only; the backend never sends this)", async () => {
  const { CommandResult } = await import("./command-result");
  const html = renderToStaticMarkup(
    React.createElement(CommandResult, { response: { command: "status" } }),
  );
  assert.equal(html, "");
});

// `isIsoTimestampValue` / `isIdentifierValue` are the field-value-shape
// heuristics `FieldValue` (above) dispatches on. They're colocated in
// command-result.tsx (this module is their only caller — see the comment on
// chat-commands.ts) and unit-tested here directly rather than only through
// rendering, since they encode several precise edge cases.
test("isIsoTimestampValue accepts the RFC3339-with-seconds, Z-suffixed shape the backend emits", async () => {
  const { isIsoTimestampValue } = await import("./command-result");
  // Matches `DateTime::to_rfc3339_opts(SecondsFormat::Secs, true)` from
  // `execute_product_status_command`.
  assert.equal(isIsoTimestampValue("2026-07-30T13:18:49Z"), true);
});

test("isIsoTimestampValue accepts a fractional-seconds timestamp with an explicit offset", async () => {
  const { isIsoTimestampValue } = await import("./command-result");
  assert.equal(isIsoTimestampValue("2026-07-30T13:18:49.123+00:00"), true);
});

test("isIsoTimestampValue rejects plain words, bare numbers, and non-timestamp text", async () => {
  const { isIsoTimestampValue } = await import("./command-result");
  assert.equal(isIsoTimestampValue("idle"), false);
  assert.equal(isIsoTimestampValue("12"), false);
  assert.equal(isIsoTimestampValue("2026-07-30"), false);
  assert.equal(isIsoTimestampValue(""), false);
  assert.equal(isIsoTimestampValue(undefined), false);
});

test("isIdentifierValue accepts a run-id-shaped UUID", async () => {
  const { isIdentifierValue } = await import("./command-result");
  assert.equal(isIdentifierValue("1b9894d9-3f21-4a10-9abc-def012345678"), true);
});

test("isIdentifierValue accepts a dotted/hyphenated/slashed package id", async () => {
  const { isIdentifierValue } = await import("./command-result");
  assert.equal(isIdentifierValue("acme-tools/foo-bar@2.1.0"), true);
});

test("isIdentifierValue rejects short plain words even when a label might suggest an id", async () => {
  const { isIdentifierValue } = await import("./command-result");
  assert.equal(isIdentifierValue("idle"), false);
  assert.equal(isIdentifierValue("yes"), false);
  assert.equal(isIdentifierValue("no"), false);
});

test("isIdentifierValue rejects a long plain English word with no structural punctuation", async () => {
  const { isIdentifierValue } = await import("./command-result");
  // e.g. LifecyclePublicState::as_str() -> "uninstalled" — a state WORD, not
  // an opaque identifier.
  assert.equal(isIdentifierValue("uninstalled"), false);
});

test("isIdentifierValue rejects a snake_case state label even though it clears the length bar", async () => {
  const { isIdentifierValue } = await import("./command-result");
  // "setup_needed" — underscore is deliberately excluded from the identifier
  // charset so backend State values never get monospaced.
  assert.equal(isIdentifierValue("setup_needed"), false);
});

test("isIdentifierValue rejects a bare short number (e.g. a Count field)", async () => {
  const { isIdentifierValue } = await import("./command-result");
  assert.equal(isIdentifierValue("12"), false);
});

test("isIdentifierValue rejects prose containing whitespace", async () => {
  const { isIdentifierValue } = await import("./command-result");
  assert.equal(
    isIdentifierValue("No assistant activity in this conversation yet."),
    false,
  );
});

test("isIdentifierValue rejects non-string values", async () => {
  const { isIdentifierValue } = await import("./command-result");
  assert.equal(isIdentifierValue(12), false);
  assert.equal(isIdentifierValue(null), false);
  assert.equal(isIdentifierValue(undefined), false);
});
