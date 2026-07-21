import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

const FRONTEND_ROOT = new URL("../../", import.meta.url);

function frontendFile(path: string) {
  return readFileSync(new URL(path, FRONTEND_ROOT), "utf8");
}

test("Vite and the SPA shell load browser assets from the root path", () => {
  const viteConfig = frontendFile("vite.config.ts");
  assert.match(viteConfig, /base:\s*"\/"/);
  for (const proxyPath of [
    "/assets",
    "/vendor",
    "/wallet/connect",
    "/wallet-connect.js",
  ]) {
    assert.ok(viteConfig.includes(`"${proxyPath}"`), `missing root proxy ${proxyPath}`);
  }

  const index = frontendFile("index.html");
  for (const assetPath of [
    "/assets/favicon-96x96.png",
    "/assets/favicon.svg",
    "/assets/favicon.ico",
    "/assets/apple-touch-icon.png",
    "/assets/site.webmanifest",
    "/vendor/fonts/fonts.css",
  ]) {
    assert.ok(index.includes(`"${assetPath}"`), `missing root asset ${assetPath}`);
  }
  assert.ok(!index.includes('="/v2/'), "SPA shell must not load assets from /v2");
});

test("PWA and wallet entrypoints use root-scoped URLs", () => {
  const manifest = JSON.parse(frontendFile("public/assets/site.webmanifest"));
  // Keep the identity of PWA installs created before the root-path move.
  assert.equal(manifest.id, "/v2/");
  assert.equal(manifest.start_url, "/");
  assert.equal(manifest.scope, "/");
  assert.deepEqual(
    manifest.icons.map((icon: { src: string }) => icon.src),
    [
      "/assets/web-app-manifest-192x192.png",
      "/assets/web-app-manifest-512x512.png",
    ],
  );

  const wallet = frontendFile("public/wallet-connect.html");
  assert.ok(wallet.includes('src="/wallet-connect.js"'));
  assert.ok(!wallet.includes('src="/v2/'));
});

test("router and login defaults no longer add the legacy mount prefix", () => {
  const app = frontendFile("src/app/app.tsx");
  assert.ok(app.includes("const redirectAfter = from;"));
  assert.match(app, /<BrowserRouter>/);
  assert.ok(!app.includes("basename="));

  const login = frontendFile("src/pages/login/login-page.tsx");
  assert.ok(login.includes('oauthRedirectAfter = "/"'));

  const sidebar = frontendFile("src/components/sidebar.tsx");
  assert.ok(sidebar.includes('src="/assets/logo.jpg"'));
});
