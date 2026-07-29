import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";

import { Heading, Text } from "./text";

test("Text variants resolve through the TYPE_TOKENS scale", () => {
  const body = renderToStaticMarkup(<Text>Body copy</Text>);
  assert.match(body, /^<p /, "body variants render a paragraph by default");
  assert.match(body, /text-\[length:var\(--v2-font-size-body\)\]/);
  assert.match(body, /text-\[var\(--v2-text\)\]/);

  const caption = renderToStaticMarkup(
    <Text variant="caption" tone="muted">
      Last synced 2 minutes ago
    </Text>,
  );
  assert.match(caption, /^<span /, "meta variants render inline by default");
  assert.match(caption, /text-\[length:var\(--v2-font-size-caption\)\]/);
  assert.match(caption, /text-\[var\(--v2-text-muted\)\]/);

  const eyebrow = renderToStaticMarkup(<Text variant="eyebrow">Admin</Text>);
  assert.match(eyebrow, /font-mono/);
  assert.match(eyebrow, /uppercase/);
  assert.match(eyebrow, /tracking-\[var\(--v2-tracking-caps\)\]/);
  assert.match(eyebrow, /text-\[length:var\(--v2-font-size-label\)\]/);
});

test("Text honors the `as` element override and extra props", () => {
  const html = renderToStaticMarkup(
    <Text as="h3" variant="eyebrow" tone="accent" data-testid="eyebrow">
      Admin
    </Text>,
  );
  assert.match(html, /^<h3 /);
  assert.match(html, /data-testid="eyebrow"/);
  assert.match(html, /text-\[var\(--v2-accent-text\)\]/);
});

test("Heading maps levels onto the type scale with semantic elements", () => {
  const h1 = renderToStaticMarkup(<Heading level={1}>Page</Heading>);
  assert.match(h1, /^<h1 /);
  assert.match(h1, /text-\[length:var\(--v2-font-size-display\)\]/);
  assert.match(h1, /text-\[var\(--v2-text-strong\)\]/, "headings default to the strong tone");

  const h3 = renderToStaticMarkup(<Heading level={3}>Panel</Heading>);
  assert.match(h3, /^<h3 /);
  assert.match(h3, /text-\[length:var\(--v2-font-size-title\)\]/);

  const visualOverride = renderToStaticMarkup(
    <Heading level={2} variant="title">
      Semantically h2, visually title
    </Heading>,
  );
  assert.match(visualOverride, /^<h2 /);
  assert.match(visualOverride, /text-\[length:var\(--v2-font-size-title\)\]/);
});
