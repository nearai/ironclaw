// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../test-support/vm-module-harness";

function component(name) {
  return function TestComponent() {
    return name;
  };
}

// `isLoopbackBrowserOrigin` defaults to `true` (local origin) so existing
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
    // A string component type remains visible in the serialized element tree,
    // allowing the layout assertion to verify the OAuth section itself.
    OAuthProviderButtons: "OAuthProviderButtons",
    useOAuthProviders: () => providers,
    // Imported from `src/lib/browser-origin.ts`; the VM harness strips
    // imports, so it must be injected here like every other dependency.
    isLoopbackBrowserOrigin: () => isLocalDev,
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

// The OAuth providers (e.g. "Continue with Google") render above the gateway
// token form, separated by the "or continue with" divider. The serializable
// OAuthProviderButtons sentinel makes the assertion fail if the component is
// omitted or rendered elsewhere in the page.
test("login page renders the OAuth providers above the gateway token form", () => {
  const serialized = JSON.stringify(
    renderLoginPage({ providers: ["google"], isLocalDev: false }),
  );

  const providersIndex = serialized.indexOf("OAuthProviderButtons");
  const dividerIndex = serialized.indexOf("login.oauthDivider");
  const tokenIndex = serialized.indexOf("login.tokenLabel");

  assert.notEqual(providersIndex, -1, "expected the OAuth providers to render");
  assert.notEqual(dividerIndex, -1, "expected the OAuth divider to render");
  assert.notEqual(tokenIndex, -1, "expected the gateway token field to render");
  assert.ok(
    providersIndex < dividerIndex && dividerIndex < tokenIndex,
    "expected OAuth providers, divider, and token form in that order",
  );
});

// With no OAuth providers configured (single-user local install), there is
// nothing to place above the token form and no divider to draw.
test("login page omits the OAuth divider when no providers are configured", () => {
  const serialized = JSON.stringify(
    renderLoginPage({ providers: [], isLocalDev: true }),
  );

  assert.doesNotMatch(serialized, /login\.oauthDivider/);
});

// The gateway token form's Connect button reads as a plain, neutral action
// (matching the OAuth buttons' visual weight) rather than the app's primary
// blue-gradient CTA — no longer the visually loudest element on the card.
test("login page renders the Connect button without the primary color treatment", () => {
  const rendered = renderLoginPage({ providers: ["google"], isLocalDev: false });
  const serialized = JSON.stringify(rendered);

  assert.match(serialized, /login\.connect/);
  assert.doesNotMatch(
    serialized,
    /"variant":"primary"/,
    "expected the Connect button not to use the primary (blue) variant",
  );
});
