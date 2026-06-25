// Single source of truth for the WebUI v2 SPA's router basename.
//
// The app is served under "/v2", so `<BrowserRouter basename>` and every test
// that resolves a `<Link to>` href must agree on this exact value.
//
// IMPORTANT: react-router prepends this basename to every router navigation
// target (`<Link to>`, `<Navigate to>`, `navigate()`). Those targets must
// therefore be basename-RELATIVE (e.g. "/logs", never "/v2/logs") — a
// "/v2"-prefixed target resolves to the broken doubled path "/v2/v2/logs".
// Only raw `<a href>` navigations, which bypass the router, carry the "/v2"
// prefix.
export const ROUTER_BASENAME = "/v2";
