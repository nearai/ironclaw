#!/usr/bin/env node
// Assembles a fully static, server-less cut of the IronClaw gateway webui
// into demo/dist/, suitable for `vercel deploy` (or any static host).
//
// The bundle is byte-identical to what the Rust gateway serves — app.js and
// style.css are concatenated from the SAME ordered module lists the gateway
// compiles in (parsed out of crates/ironclaw_gateway/src/assets.rs, so the
// two can never drift) — plus one injected <script> that flips the SPA into
// demo mode (`window.__IRONCLAW_DEMO__ = true`), which activates the
// in-browser mock backend in static/js/core/mock-backend.js.

import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const demoDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(demoDir, '..');
const gatewayDir = join(repoRoot, 'crates', 'ironclaw_gateway');
const staticDir = join(gatewayDir, 'static');
const distDir = join(demoDir, 'dist');

const assetsRs = readFileSync(join(gatewayDir, 'src', 'assets.rs'), 'utf8');

// Extract the ordered include_str!() list for a given `pub const NAME`.
function includeList(constName) {
  const constStart = assetsRs.indexOf(`pub const ${constName}`);
  if (constStart === -1) throw new Error(`const ${constName} not found in assets.rs`);
  const constEnd = assetsRs.indexOf(';', constStart);
  const block = assetsRs.slice(constStart, constEnd);
  const files = [...block.matchAll(/include_str!\("\.\.\/(static\/[^"]+)"\)/g)]
    .map((m) => m[1]);
  if (files.length === 0) throw new Error(`no include_str! entries under ${constName}`);
  return files;
}

function concatModules(constName) {
  return includeList(constName)
    .map((rel) => readFileSync(join(gatewayDir, rel), 'utf8'))
    .join('\n');
}

rmSync(distDir, { recursive: true, force: true });
mkdirSync(distDir, { recursive: true });

// 1. Concatenated bundles (mirrors APP_JS / STYLE_CSS in assets.rs).
writeFileSync(join(distDir, 'app.js'), concatModules('APP_JS'));
writeFileSync(join(distDir, 'style.css'), concatModules('STYLE_CSS'));

// 2. Standalone assets served by the gateway router (static_files.rs).
const passthrough = [
  ['index.html', 'index.html'],
  ['theme.css', 'theme.css'],
  ['theme-init.js', 'theme-init.js'],
  ['debug-init.js', 'debug-init.js'],
  ['debug-panel.js', 'debug-panel.js'],
  ['debug-panel.css', 'debug-panel.css'],
  ['i18n-app.js', 'i18n-app.js'],
  ['favicon.ico', 'favicon.ico'],
  ['i18n', 'i18n'],
  ['fonts', 'fonts'],
];
for (const [src, dest] of passthrough) {
  cpSync(join(staticDir, src), join(distDir, dest), { recursive: true });
}

// 2b. Vercel config rides along inside the deployable directory.
cpSync(join(demoDir, 'vercel.json'), join(distDir, 'vercel.json'));

// 3. Flip the SPA into demo mode. The flag must be set before app.js parses
//    (the mock backend wraps fetch/EventSource at parse time), so inject it
//    into <head> ahead of every other script.
const indexPath = join(distDir, 'index.html');
const indexHtml = readFileSync(indexPath, 'utf8');
const demoFlag = '  <script>window.__IRONCLAW_DEMO__ = true;</script>\n';
const marker = indexHtml.indexOf('<script');
if (marker === -1) throw new Error('no <script> tag found in index.html');
writeFileSync(indexPath, indexHtml.slice(0, marker) + demoFlag.trim() + '\n  ' + indexHtml.slice(marker));

console.log(`demo bundle written to ${distDir}`);
console.log('serve locally:  python3 -m http.server 8321 -d demo/dist');
console.log('deploy:         cd demo && vercel deploy dist --prod');
