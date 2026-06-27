import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function sourceForTest(path, exportNames) {
  const source = readFileSync(new URL(path, import.meta.url), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ${exportNames.join(", ")} };`;
}

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  if (Array.isArray(node.values)) {
    for (const value of node.values) visit(value, fn);
  }
}

function deepValuesAfter(root, fragment) {
  const values = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !Array.isArray(node.values)) return;
    node.strings.forEach((part, index) => {
      if (part.includes(fragment)) values.push(node.values[index]);
    });
  });
  return values;
}

function collectScalars(root) {
  const scalars = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string") scalars.push(value);
    }
  });
  return scalars;
}

function createUseOAuthProvidersHarness(fetchAuthProviders) {
  const state = { providers: undefined, cleanup: null };
  const context = {
    fetchAuthProviders,
    globalThis: {},
    React: {
      useEffect: (fn) => {
        state.cleanup = fn();
      },
      useState: (initial) => {
        if (!Object.hasOwn(state, "providers") || state.providers === undefined) {
          state.providers = typeof initial === "function" ? initial() : initial;
        }
        return [
          state.providers,
          (next) => {
            state.providers = typeof next === "function" ? next(state.providers) : next;
          },
        ];
      },
    },
  };
  vm.runInNewContext(
    sourceForTest("./hooks/useOAuthProviders.js", ["useOAuthProviders"]),
    context,
  );
  const returned = context.globalThis.__testExports.useOAuthProviders();
  return { returned, state };
}

function renderOAuthProviderButtons({ providers, redirectAfter = "/v2/chat" }) {
  const context = {
    Button: "Button",
    Icon: "Icon",
    globalThis: {},
    html,
    useT: () => (key, params = {}) => {
      if (key === "login.oauthProvider") return `Continue with ${params.provider}`;
      return key;
    },
  };
  vm.runInNewContext(
    sourceForTest("./components/oauth-provider-buttons.js", ["OAuthProviderButtons"]),
    context,
  );
  return context.globalThis.__testExports.OAuthProviderButtons({
    providers,
    redirectAfter,
  });
}

test("useOAuthProviders sorts supported providers and drops unknown providers", async () => {
  const { returned, state } = createUseOAuthProvidersHarness(async () => ({
    providers: ["apple", "unknown", "github", "google", "github"],
  }));

  assert.deepEqual(Array.from(returned), []);
  await Promise.resolve();

  assert.deepEqual(Array.from(state.providers), ["google", "github", "apple"]);
});

test("useOAuthProviders hides providers when discovery fails", async () => {
  const { state } = createUseOAuthProvidersHarness(async () => {
    throw new Error("backend unavailable");
  });

  await Promise.resolve();

  assert.deepEqual(Array.from(state.providers), []);
});

test("useOAuthProviders ignores late discovery after unmount", async () => {
  let resolveProviders;
  const pending = new Promise((resolve) => {
    resolveProviders = resolve;
  });
  const { state } = createUseOAuthProvidersHarness(() => pending);

  state.cleanup();
  resolveProviders({ providers: ["google"] });
  await pending;
  await Promise.resolve();

  assert.deepEqual(Array.from(state.providers), []);
});

test("OAuthProviderButtons renders nothing for an empty provider list", () => {
  assert.equal(renderOAuthProviderButtons({ providers: [] }), null);
});

test("OAuthProviderButtons builds encoded login links with stable labels", () => {
  const rendered = renderOAuthProviderButtons({
    providers: ["google", "github", "apple", "custom/id"],
    redirectAfter: "/v2/chat?next=/settings/inference&mode=sso",
  });

  assert.deepEqual(deepValuesAfter(rendered, "href="), [
    "/auth/login/google?redirect_after=%2Fv2%2Fchat%3Fnext%3D%2Fsettings%2Finference%26mode%3Dsso",
    "/auth/login/github?redirect_after=%2Fv2%2Fchat%3Fnext%3D%2Fsettings%2Finference%26mode%3Dsso",
    "/auth/login/apple?redirect_after=%2Fv2%2Fchat%3Fnext%3D%2Fsettings%2Finference%26mode%3Dsso",
    "/auth/login/custom%2Fid?redirect_after=%2Fv2%2Fchat%3Fnext%3D%2Fsettings%2Finference%26mode%3Dsso",
  ]);
  assert.deepEqual(
    collectScalars(rendered).filter((value) => value.startsWith("Continue with ")),
    [
      "Continue with Google",
      "Continue with GitHub",
      "Continue with Apple",
      "Continue with custom/id",
    ],
  );
});
