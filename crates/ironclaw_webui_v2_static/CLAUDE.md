# `ironclaw_webui_v2_static`

Serves the WebUI v2 SPA. `static/js/**` is a React + htm app bundled by
`frontend/build.mjs` into the committed `static/dist/`. `build.rs` embeds
every file under `static/` as a served asset **except** `*.test.js` /
`*.test.mjs` (colocated `node --test` unit tests). Keep dev-only docs out
of `static/` so they are not shipped to clients — this file lives at the
crate root for that reason.

After editing any `static/js/**` file you MUST rebuild the bundle:
`cd frontend && ./build.sh` (or `node build.mjs` if deps are installed),
then commit the regenerated `static/dist/`.

## Router basename: `<Link to>` must be basename-RELATIVE

The app is served under `/v2` and `static/js/app/app.js` mounts
`<BrowserRouter basename=${ROUTER_BASENAME}>` (`ROUTER_BASENAME = "/v2"`,
defined once in `static/js/app/router-href.js`).

react-router prepends the basename to every router-navigation target, so
those targets must be basename-RELATIVE:

- `<Link to>`, `<NavLink to>`, `<Navigate to>`, and `navigate(...)` take
  paths like `"/logs"` — never `"/v2/logs"`. A `/v2`-prefixed target
  resolves to the doubled, dead path `/v2/v2/logs`.
- Only raw `<a href>` navigations (full-page loads that bypass the
  router, e.g. the OAuth login redirect) carry the explicit `/v2` prefix.
- Path builders consumed by router navigation (e.g. `buildScopedLogsPath`
  in `static/js/pages/logs/lib/logs-data.js`) return basename-relative
  paths only.

Regression history: PR #5235 swapped the chat composer "Logs" link from a
raw `<a href="/v2/logs">` to a `<Link to="/v2/logs">` without dropping the
`/v2` prefix, so react-router doubled it to `/v2/v2/logs`. Guarded by
`static/js/app/link-basename.test.mjs` (scans every literal router target)
plus the chat / logs-data unit tests, which resolve `to` through the real
basename instead of asserting the raw `to` prop.
