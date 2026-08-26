import { readFileSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

interface ManifestChunk {
  file: string;
  imports?: string[];
}

type Manifest = Record<string, ManifestChunk>;

const here = dirname(fileURLToPath(import.meta.url));
const distDir = resolve(here, "..", "dist");
const manifestPath = resolve(distDir, ".vite", "manifest.json");

const LOGIN_GZIP_BUDGET = 180_000;
// The composer's slash-command menu (chat-input.tsx) and its supporting
// classification helpers (chat-commands.ts) are eager, always-on parts of
// the chat composer and its own well-covered vm-harness test suite
// (chat-input.test.ts) — pulling the menu out into its own lazy chunk would
// mean rebuilding that harness around indirection it can't currently express,
// which is out of scope for the command-palette PR. Everything that COULD be
// deferred already is: the ⌘K launcher (command-palette.tsx) and the
// command-result card (command-result.tsx) both load as separate chunks via
// React.lazy (see gateway-layout.tsx / message-bubble.tsx), and their
// presentation-only field-value helpers moved out of the eager
// chat-commands.ts into command-result.tsx. What's left (~0.45 KB over the
// previous 210.0 KB budget) is the composer menu's own necessary weight. React
// Router 8 and its required React 19.2 upgrade then moved the measured initial
// route from 211 KB to 213.8 KB gzip, so retain about 1.2 KB of explicit
// headroom for that supported dependency upgrade.
// Hosted MCP registration (custom MCP server connect) added ~44
// `extensions.customMcp*` i18n keys plus 2 `common.*` keys to `en.ts`, the
// fallback locale pack `i18n.tsx` (~line 36) loads eagerly in `main.tsx` so it
// ships on every page including /chat. The other 10 locale packs are lazy
// per-locale dynamic imports (`i18n.tsx` ~41-50) and don't count against this
// budget, and the registration modal itself is not the cause: `ExtensionsPage`
// is already route-level `React.lazy`'d in `app.tsx` (~49-53), outside the
// /chat closure. Copy was trimmed first: `en.ts` went from 211,326 to 211,218
// bytes (108 gzip bytes saved) before further cuts hit sharp diminishing
// returns (0.39 gzip bytes saved per raw byte cut, down to 0.17) and would
// require gutting minimal labels like "OAuth" or "Server ID". The remaining
// growth is ~44 new i18n keys of string content for hosted MCP registration,
// not new eager code weight; the 215.0 KB budget below (already raised for the
// React Router 8 / React 19.2 upgrade) still covers the measured total with
// headroom to spare, so it was not raised further there. Workspace-file link
// previews then added strict path parsing plus delegated-click verification to
// the eager Markdown renderer. That validation must be present before a link
// can open in the in-app preview, including for cached messages rendered on the
// initial route, and prevents model-authored `data-workspace-path` metadata
// from becoming trusted. The measured /chat closure is 215.8 KB gzip; 217.0 KB
// retains about 1.2 KB of explicit headroom without weakening the feature.
// The inspector shell then brought current `main` to 216.9 KB gzip. The SSE
// reconnect coordinator adds ~0.4 KB of deterministic retry/backpressure logic
// to the eager chat transport path, where it must be available before the first
// stream opens. Two more changes then landed on the merged tree, each adding a
// little to /chat:
//  - Web Push notifications added ~13 `automations.notificationChannels.devicePush.*`
//    keys + a reworded `noSelectionHelper` to the eager `en.ts` fallback pack.
//    Eager code was kept OUT of /chat: `registerServiceWorker` is in the
//    dependency-free `lib/register-sw.ts`, and the enrollment UI rides the
//    already-lazy automations route — so this is localized string content.
//  - The shared native file-picker interaction (#7337) replaced three
//    route-local impls; Vite emits its ~0.3 KB gzip helper as a shared chunk
//    (Chat/Settings/Extensions all consume it).
// Re-measured on the MERGED tree with `vite build` + this check. `main`
// concurrently re-measured 217.7 KB gzip after its own changes (#7480, #7284
// et al.); both deltas are in the measurement below.
// The device-link card then added a multi-step link flow to /chat.
// Everything deferrable was deferred FIRST, and measured at each step:
//  - `auth-device-link-card.tsx` and the `device-link-panel` /
//    `device-link-api` modules behind it load through `React.lazy` from
//    `chat.tsx` (the inspector-panel pattern), so the card, its step machine,
//    its polling, and its input forms emit as their own chunks and cost the
//    initial route nothing. Measured: 220.5 KB eager -> 219.5 KB.
// What stays eager, and why it cannot move:
//  - `link-payload-panel.tsx` — the extracted QR/countdown/copy/renew
//    presentation. It is not new weight so much as relocated weight: it
//    replaces the identical implementation that already shipped inside the
//    eager `pairing-web-code-panel.tsx`, and it is now shared with the lazy
//    device-link chunk, so Vite hoists it into a shared chunk the eager
//    pairing panel pulls anyway. Keeping a second copy to save the hoist is
//    precisely the drift the extraction exists to prevent.
//  - `lib/device-link-frame.ts` — the frame normalizer. The gate selector
//    (`gates.ts`, eager) has to normalize a device-link gate to decide which
//    card to render at all, so it cannot sit behind the lazy boundary; a
//    second eager normalizer would let a polled frame and a gate frame
//    disagree about the same wire object.
//  - ~31 `deviceLink.*` keys in `en.ts`, which `i18n.tsx` loads eagerly as the
//    fallback pack on every page. Measured at 0.5 KB gzip; the other ten
//    locale packs stay lazy per-locale imports and cost nothing here.
// Measured /chat closure on the merged tree is 220.4 KB gzip; 222.0 KB
// retains about 1.6 KB of explicit headroom.
// The always-on OOBE first-run suggestion surface — its cards, the
// `useSuggestions` data hook,
// the suggestions API client, and the brand-icon SVGs — is lazy-loaded from
// `empty-state.tsx` (`React.lazy` + `Suspense`), so it lands in its own chunk
// and costs the eager /chat closure nothing. Only the lazy-import decision
// stays eager (already accounted for above).
// The OOBE drawer's section close/restore interaction then added a small eager
// increment in `empty-state.tsx`: the drawer-visibility gate (open/dismissed/
// gone) plus the `chat.oobe.showSuggestions` / `hideSuggestions` keys in the
// eager `en.ts` fallback pack (the restore pill's own markup is lazy —
// `oobe-restore-pill.tsx` loads only after the drawer is dismissed). Measured
// /chat closure 222.5 KB gzip; 223.0 KB restores ~0.5 KB of explicit headroom.
// retained about 1.6 KB of explicit headroom.
//
// The server-backed notification center consumed nearly all of that: the
// inbox hook, its presenters and the panel are eager parts of the gateway
// layout's header, so they land in the /chat closure and moved the measured
// total to 221.9 KB. Wiring the archive control (the durable inbox exposes
// archive at every backend layer, and without a caller a recipient's records
// only ever accumulate toward the fail-closed retention cap) added the last
// 0.2 KB. Weight was removed first rather than budgeted around: the
// mark-read and archive mutations now share one optimistic cache transform
// (`inboxCacheAfter`/`optimisticHandlers` in `hooks/useNotifications.ts`)
// instead of carrying two near-identical `onMutate` blocks, and the control
// reuses the existing `common.dismiss` label rather than minting a
// `notifications.archive` key in the eagerly loaded `en.ts` fallback pack.
// What remains is the control's own necessary weight. 223.0 KB restores
// about 0.9 KB of headroom over the measured 222.1 KB.
//
// Re-measured 223.0 -> 224.0 on the merge of this branch with main. Both sides
// independently set 223.0 for their own eager increment — main for the OOBE
// drawer's close/restore gate, this branch for the notification centre and its
// archive control — and on the merged tree those increments add: main alone
// measures 222.5 KB, the merged tree 224.0 KB. Neither side is spending the
// other's headroom by mistake; the ceiling simply had two claimants.
//
// Re-ratcheted 224.0 -> 223.2 in the same change by taking that split rather
// than banking the raise: `notification-panel.tsx` now holds the opened panel
// (rows, load-more, empty/error states) and loads through `React.lazy` from
// `notification-center.tsx`, the way `CommandPalette` already does in
// `layout/gateway-layout.tsx`. The bell and its unread dot stay eager because
// the badge is always on screen. The panel leaves the entry closure as its own
// 5.39 kB chunk and /chat measures 223.2 KB, so the ceiling follows the
// measurement down instead of holding headroom this tree does not need.
//
// Then raised 223.2 -> 223.4 with a margin, because ratcheting to the measured
// byte turned out to be a trap: rebasing onto main breached the ceiling by
// roughly a dozen bytes on commits that touch no frontend code at all. The
// margin is under a kilobyte, so the ratchet still catches anything that
// actually ships weight into the entry closure, and the two budgets beside
// this one were never held to the byte either.
//
// Shared page-shell and skeleton primitives remain outside the /chat closure,
// but their lazy-route chunk hashes change the eager preload map enough to
// perturb gzip by 16 bytes on the merged tree. Keep a sub-0.1 KB margin for
// that hash-only variance without budgeting new eager product code.
const CHAT_GZIP_BUDGET = 223_500;
const CHUNK_RAW_BUDGET = 500_000;

export function resolveBundleAsset(distRoot: string, file: string): string {
  const root = resolve(distRoot);
  const fullPath = resolve(root, file);
  const relativePath = relative(root, fullPath);
  if (
    relativePath === ".." ||
    relativePath.startsWith(`..${sep}`) ||
    isAbsolute(relativePath)
  ) {
    throw new Error(`Vite manifest asset escapes the dist directory: ${file}`);
  }
  return fullPath;
}

export function createBundleAssetReader(
  distRoot: string,
): (file: string) => Buffer {
  const cache = new Map<string, Buffer>();

  return (file) => {
    const fullPath = resolveBundleAsset(distRoot, file);
    const cached = cache.get(fullPath);
    if (cached) return cached;

    const contents = readFileSync(fullPath);
    cache.set(fullPath, contents);
    return contents;
  };
}

function importClosure(manifest: Manifest, roots: string[]): Set<string> {
  const visited = new Set<string>();

  function visit(key: string) {
    if (visited.has(key)) return;
    const chunk = manifest[key];
    if (!chunk) {
      throw new Error(`Vite manifest is missing expected chunk: ${key}`);
    }

    visited.add(key);
    for (const dependency of chunk.imports ?? []) {
      visit(dependency);
    }
  }

  for (const root of roots) {
    visit(root);
  }
  return visited;
}

function javascriptFiles(manifest: Manifest, keys: Iterable<string>): Set<string> {
  const files = new Set<string>();
  for (const key of keys) {
    const file = manifest[key]?.file;
    if (file?.endsWith(".js")) files.add(file);
  }
  return files;
}

function gzipBytes(
  files: Iterable<string>,
  readAsset: (file: string) => Buffer,
): number {
  let total = 0;
  for (const file of files) {
    total += gzipSync(readAsset(file)).byteLength;
  }
  return total;
}

function assertAtMost(label: string, actual: number, budget: number) {
  if (actual > budget) {
    throw new Error(
      `${label} is ${(actual / 1_000).toFixed(1)} KB, exceeding the ` +
        `${(budget / 1_000).toFixed(1)} KB budget`,
    );
  }
}

function headroom(actual: number, budget: number): string {
  return `${((budget - actual) / 1_000).toFixed(1)} KB headroom`;
}

function runCli(): void {
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as Manifest;
  const readAsset = createBundleAssetReader(distDir);
  const loginFiles = javascriptFiles(
    manifest,
    importClosure(manifest, ["index.html"]),
  );
  const chatFiles = javascriptFiles(
    manifest,
    importClosure(manifest, [
      "index.html",
      "src/layout/gateway-layout.tsx",
      "src/pages/chat/chat-page.tsx",
    ]),
  );
  const loginGzipBytes = gzipBytes(loginFiles, readAsset);
  const chatGzipBytes = gzipBytes(chatFiles, readAsset);

  assertAtMost("Login entry JavaScript (gzip)", loginGzipBytes, LOGIN_GZIP_BUDGET);
  assertAtMost("Initial /chat JavaScript (gzip)", chatGzipBytes, CHAT_GZIP_BUDGET);

  const emittedJavascript = new Set(
    Object.values(manifest)
      .map(({ file }) => file)
      .filter((file) => file.endsWith(".js")),
  );
  let largestChunk = { file: "", bytes: 0 };
  for (const file of emittedJavascript) {
    const bytes = readAsset(file).byteLength;
    if (bytes > largestChunk.bytes) largestChunk = { file, bytes };
    assertAtMost(`JavaScript chunk ${file} (raw)`, bytes, CHUNK_RAW_BUDGET);
  }

  console.log(
    [
      `Bundle budgets passed: login ${(loginGzipBytes / 1_000).toFixed(1)} KB gzip (${headroom(loginGzipBytes, LOGIN_GZIP_BUDGET)})`,
      `/chat ${(chatGzipBytes / 1_000).toFixed(1)} KB gzip (${headroom(chatGzipBytes, CHAT_GZIP_BUDGET)})`,
      `largest chunk ${largestChunk.file} ${(largestChunk.bytes / 1_000).toFixed(1)} KB raw (${headroom(largestChunk.bytes, CHUNK_RAW_BUDGET)})`,
    ].join("; "),
  );
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : undefined;
if (invokedPath === fileURLToPath(import.meta.url)) runCli();
