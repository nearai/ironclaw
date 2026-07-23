# E2E skip/xfail debt inventory

The removed gateway scenarios and their fixture-only skip/xfail clusters were
retired after replacement coverage moved to `ironclaw serve`. This file tracks
only debt in the current Python scenario tree.

| Cluster | File | Debt | Follow-up |
| --- | --- | --- | --- |
| Mutable operator tool availability | `test_reborn_webui_v2_legacy_tool_permissions.py` | One runtime skip when the test catalog contains no mutable operator tool | Provide a deterministic mutable tool in the serve harness, then make the assertion required. |
| Optional Emulate CLI | `conftest.py` | Local provider-contract runs skip when neither `IRONCLAW_EMULATE_CLI` nor the fallback runner is available; CI fails instead | Keep local setup documented and the CI path mandatory. |

## Policy

- Blocking serve-backed contracts fail when required harness prerequisites are
  absent.
- Runtime skips are allowed only for optional local integrations, and the skip
  reason must name the missing prerequisite.
- UI lifecycle tests use local route or provider doubles instead of live
  network dependencies.
- Placeholder tests do not remain as skipped `pass` bodies; track future work
  in an issue or design document.
