// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../test-support/vm-module-harness";

function component(name) {
  return function TestComponent() {
    return name;
  };
}

function renderLoginPage({ providers = [] } = {}) {
  const context = {
    Button: component("Button"),
    Card: component("Card"),
    Input: component("Input"),
    FormField: component("FormField"),
    Icon: component("Icon"),
    useInterfaceTheme: () => ({ theme: "dark", toggleTheme: () => {} }),
    useT: () => (key) => key,
    cn: (...classes) => classes.flat().filter(Boolean).join(" "),
    OAuthProviderButtons: component("OAuthProviderButtons"),
    useOAuthProviders: () => providers,
    useForm: () => ({
      formState: { errors: {}, isSubmitting: false },
      handleSubmit: () => () => {},
      register: () => ({}),
    }),
  };
  const { LoginPage } = runVmModuleForTest(
    "./login-page.tsx",
    ["LoginPage"],
    context,
    import.meta.url,
  );
  return LoginPage({ onSubmit: () => {} });
}

test("login page shows the local-dev status hint when no OAuth providers are configured", () => {
  const rendered = renderLoginPage({ providers: [] });

  assert.match(JSON.stringify(rendered), /login\.localDevHint/);
  assert.match(JSON.stringify(rendered), /ironclaw-reborn status/);
});

test("login page omits the local-dev hint once an OAuth provider is configured", () => {
  const rendered = renderLoginPage({ providers: ["google"] });

  assert.doesNotMatch(JSON.stringify(rendered), /login\.localDevHint/);
  assert.doesNotMatch(JSON.stringify(rendered), /ironclaw-reborn status/);
});
