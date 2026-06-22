import { describe, expect, it } from "vitest";
import { type BundledHostRuntime, startBundledHost } from "../helpers/bundled-host";

function createRuntimeConfig(urls: { baseUrl: string; uiAssetsUrl: string }) {
  return {
    env: "production",
    account: "dev.everything.near",
    domain: "everything.dev",
    networkId: "mainnet",
    title: "everything.dev",
    description: "Bundled SSR regression runtime",
    repository: "https://github.com/nearbuilders/everything-dev",
    host: {
      name: "host",
      url: urls.baseUrl,
      entry: `${urls.baseUrl}/mf-manifest.json`,
      source: "remote",
    },
    ui: {
      name: "ui",
      url: urls.uiAssetsUrl,
      entry: `${urls.uiAssetsUrl}/mf-manifest.json`,
      source: "remote",
      ssrUrl: urls.uiAssetsUrl,
    },
    api: {
      name: "api",
      url: urls.baseUrl,
      entry: `${urls.baseUrl}/mf-manifest.json`,
      source: "remote",
    },
  } as const;
}

describe.skip("bundled host SSR runtime", () => {
  // Skipped: this test needs auth config (auth.variables.siwn.recipients) in
  // the RuntimeConfig + a deployed auth remote. The app's _layout.tsx beforeLoad
  // calls sessionQueryOptions which requires an auth client with proper SIWN vars.
  // Un-skip once the test can supply auth config or mock the auth endpoint.

  let runtime: BundledHostRuntime | null = null;

  it("renders real SSR markup from the bundled UI remote through the host", async () => {
    runtime = await startBundledHost((urls) => createRuntimeConfig(urls));

    const response = await fetch(`${runtime.baseUrl}/skill`);
    const html = await response.text();

    expect(response.status).toBe(200);
    expect(html).toContain("Setup Skill");
    expect(html).not.toContain("Server Error");

    await runtime.stop();
  }, 120000);
});
