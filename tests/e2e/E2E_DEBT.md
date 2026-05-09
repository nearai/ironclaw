# E2E skip/xfail debt inventory

This inventory tracks skip/xfail debt found in `tests/e2e/scenarios/` while auditing from `origin/main` for the first E2E debt campaign.

## Summary

| Cluster | Files | Debt type | Determinism | Recommended action |
| --- | --- | --- | --- | --- |
| ClawHub skills search/install | `test_skills.py` | Runtime `pytest.skip` when registry/search/install is unavailable or slow | Deterministic with Playwright route mocks | Fix in this campaign: mock the skills API for frontend lifecycle tests so CI never depends on ClawHub availability. |
| Gmail extension OAuth legacy flow | `test_extension_oauth.py`, `test_routine_oauth_credential_injection.py` | Runtime skips when Gmail install/auth prerequisites are absent | Mostly deterministic if migrated to existing isolated OAuth fixtures | Follow-up: consolidate with hosted OAuth fixtures used by newer v2 OAuth tests; avoid module-order globals. |
| MCP auth flow legacy flow | `test_mcp_auth_flow.py` | Runtime skips when mock MCP install/auth URL prerequisites are absent | Mostly deterministic with fixture refactor | Follow-up: convert install/auth state to fixtures instead of module-order state and make failure explicit. |
| Telegram OAuth URL placeholders | `test_oauth_url_parameters.py` | Static skip placeholders with `pass` bodies | Blocked on Telegram channel E2E fixture/product setup | Follow-up: either implement a fake Telegram channel fixture or move placeholders to issue/docs and remove dead skipped tests. |
| Portfolio widget availability | `test_portfolio.py` | Runtime skip when portfolio widget is not registered in this build | Product/build dependent | Follow-up: decide whether portfolio is a required test fixture or optional extension; if optional, keep documented skip. |
| v2 auth/OAuth matrix xfails | `test_v2_auth_oauth_matrix.py` | Static xfails for known engine-v2 contract/product gaps | Requires product behavior changes or deeper engine debug | Follow-up: split into product issues; do not silently un-xfail without matching contract changes. |
| v2 auth approval/cancel fallback | `test_v2_engine_auth_cancel.py`, `test_v2_engine_auth_flow.py` | Runtime skips when dedicated fixtures remain in approval gating rather than auth gating | Requires fixture/model prompt control | Follow-up: pin mock LLM/tool prompt path so tests reach intended auth state deterministically. |
| v2 Google OAuth binary/refresh prerequisites | `test_v2_engine_oauth_google.py` | Runtime skips when google-drive WASM binary or refresh-token prerequisite is missing | Deterministic if fixture builds/provides WASM and OAuth callback state | Follow-up: prebuild or fixture-install the WASM artifact, then make refresh tests independent. |
| Skill OAuth guided auth fallback | `test_skill_oauth_flow.py` | Runtime skip if auth flow does not trigger under current engine mode | Requires fixture/model control | Follow-up: pin engine mode and mock LLM response to force the guided auth branch. |

## Selected cluster for this campaign

**ClawHub skills search/install** is selected because it is the smallest high-value deterministic cluster: the tests are intended to validate the browser skills UI lifecycle, but currently depend on live ClawHub search results and network timing. The E2E README already recommends `page.route()` for tabs that depend on external data. Mocking `/api/skills`, `/api/skills/search`, `/api/skills/install`, and `DELETE /api/skills/{name}` keeps the test end-to-end at the browser/API contract layer without requiring live external services.

Live ClawHub contract coverage belongs below the browser E2E tier: gateway/API integration tests should validate request/response shape, authentication, error handling, and registry availability separately from deterministic UI lifecycle tests.

## Remaining debt policy

- Runtime skips are acceptable only when the prerequisite is genuinely outside the deterministic E2E harness and the reason names that prerequisite.
- UI lifecycle tests should prefer local route mocks when they are not validating the backend integration itself.
- Placeholder skipped tests with `pass` bodies should either become real tests or move to tracked follow-up documentation/issues.
