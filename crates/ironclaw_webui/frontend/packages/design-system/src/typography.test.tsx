import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { isValidElement, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";

import { Button } from "./button";
import { Input, Label, Select, Textarea } from "./input";
import { SelectMenu } from "./select-menu";

type StyledElementProps = {
  className?: string;
  children?: unknown;
};

function classNameOf(element: unknown): string {
  assert.ok(isValidElement<StyledElementProps>(element));
  return element.props.className ?? "";
}

test("the root and semantic control type scale stay stable across viewports (#6702)", () => {
  // The `text-ui*` semantic scale lives in the package tokens now,
  // resolved through the canonical --v2-font-size-* tokens so the two
  // scales cannot fork.
  const tokensCss = readFileSync(new URL("./tokens.css", import.meta.url), "utf8");

  assert.match(tokensCss, /--text-ui-sm:\s*var\(--v2-font-size-caption\);/);
  assert.match(tokensCss, /--text-ui:\s*var\(--v2-font-size-body-sm\);/);
  assert.match(tokensCss, /--text-ui-lg:\s*var\(--v2-font-size-body-lg\);/);
  assert.match(tokensCss, /--v2-font-size-caption:\s*0\.75rem;/);
  assert.match(tokensCss, /--v2-font-size-body-sm:\s*0\.8125rem;/);
  assert.match(tokensCss, /--v2-font-size-body-lg:\s*1rem;/);

  const appCss = readFileSync(
    new URL("../../../src/styles/app.css", import.meta.url),
    "utf8"
  );
  assert.match(appCss, /html\s*\{[^}]*font-size:\s*16px;/s);
  assert.match(
    appCss,
    /@layer base\s*\{[^{}]*button, input, select, textarea\s*\{[^}]*font:\s*inherit;/s
  );
  assert.doesNotMatch(
    appCss,
    /@media\s*\(min-width:\s*1024px\)\s*\{[^}]*html\s*\{[^}]*font-size:/s
  );
});

test("medium shared controls use one viewport-independent semantic size (#6702)", () => {
  const button = Button({ children: "Save" }) as ReactElement<StyledElementProps>;
  const input = Input({});
  const textarea = Textarea({});
  const selectWrapper = Select({ children: null });
  const label = Label({ children: "Name" });

  const select = (selectWrapper.props.children as ReactElement<StyledElementProps>[])[0];
  const classes = [
    classNameOf(button),
    classNameOf(input),
    classNameOf(textarea),
    classNameOf(select),
    classNameOf(label),
  ];

  for (const className of classes) {
    assert.match(className, /(?:^|\s)text-ui(?:\s|$)/);
    assert.doesNotMatch(className, /md:text-sm|text-\[13px\]/);
  }

  const selectMenu = renderToStaticMarkup(
    <SelectMenu
      value="default"
      options={[{ value: "default", label: "Default" }]}
      aria-label="Provider"
    />
  );
  assert.match(selectMenu, /class="[^"]*\bfont-sans\b[^"]*\btext-ui\b[^"]*"/);
  assert.doesNotMatch(selectMenu, /\bfont-mono\b|\bmd:text-sm\b|text-\[13px\]/);
});

test("SelectMenu allows monospace only through an explicit override (#6702)", () => {
  const selectMenu = renderToStaticMarkup(
    <SelectMenu
      value="default"
      options={[{ value: "default", label: "Default" }]}
      aria-label="Provider"
      buttonClassName="font-mono"
    />
  );

  assert.match(selectMenu, /<button[^>]*class="[^"]*\bfont-mono\b[^"]*"/);
});
