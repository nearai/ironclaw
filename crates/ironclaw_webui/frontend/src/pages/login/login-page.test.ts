// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../test-support/vm-module-harness";

function component(name) {
  return function TestComponent() {
    return name;
  };
}

// `isLocalDevOrigin` defaults to `true` (local origin) so existing
// providers-only test cases keep exercising the local-install-hint path
// without every call site having to opt in explicitly.
function renderLoginPage({ providers = [], isLocalDev = true } = {}) {
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
    // Imported from `src/lib/browser-origin.ts`; the VM harness strips
    // imports, so it must be injected here like every other dependency.
    isLocalDevOrigin: () => isLocalDev,
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

test("login page shows the local-dev status hint when no OAuth providers are configured on a local origin", () => {
  const rendered = renderLoginPage({ providers: [], isLocalDev: true });

  assert.match(JSON.stringify(rendered), /login\.localDevHint/);
  assert.match(JSON.stringify(rendered), /ironclaw status/);
});

test("login page omits the local-dev hint once an OAuth provider is configured", () => {
  const rendered = renderLoginPage({ providers: ["google"], isLocalDev: true });

  assert.doesNotMatch(JSON.stringify(rendered), /login\.localDevHint/);
  assert.doesNotMatch(JSON.stringify(rendered), /ironclaw status/);
});

// C4 fix: no SSO configured is NOT the same signal as "this is a local
// install" — a hosted token-only deployment also has zero OAuth providers,
// and a remote user viewing it from a non-local origin has no use for a
// CLI command they can't run. The hint must require BOTH conditions.
test("login page omits the local-dev hint on a non-local origin even with no OAuth providers", () => {
  const rendered = renderLoginPage({ providers: [], isLocalDev: false });

  assert.doesNotMatch(JSON.stringify(rendered), /login\.localDevHint/);
  assert.doesNotMatch(JSON.stringify(rendered), /ironclaw status/);
});
