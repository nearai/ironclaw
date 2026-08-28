#!/usr/bin/env bash
#
# Print the cargo `--test <name>` arguments for the Reborn in-process
# integration-tier test binaries, one per line.
#
# Integration-tier (task T0-COV) is the set of in-process suites under
# tests/integration/ (post-restructure home of the roadmap integration suite;
# see docs/internal/superpowers/specs/2026-06-26-reborn-integration-test-framework-design.md):
#   - tests/integration/<name>.rs        (flat bins; `name = reborn_integration_<name>`)
#   - tests/integration/group_<x>/       (group bins; `name = reborn_group_<x>`)
#   - tests/integration/<domain>/<n>.rs  (domain-folder bins, e.g. auth/;
#                                         `name = reborn_integration_<n>`)
#
# Discovery is registration-driven: the workspace Cargo.toml's `[[test]]`
# entries are the single source of truth, and every entry whose `path` sits
# under tests/integration/ is selected. Deriving candidates from a filesystem
# walk instead has already burned us once: the previous `find -maxdepth 1`
# walk could not see domain-folder bins, so the six tests/integration/auth/
# suites (oauth_connect, oauth_popup_journeys, oauth_refresh, auth_gate,
# auth_failure, reopen_resume_through_gate) ran in NO PR or merge-queue lane
# — their only executor was the push-to-main coverage workflow. Selecting
# from the registration makes a new suite impossible to register without
# also being selected, whatever directory shape it uses, and a registered
# entry whose file was deleted fails the lane loudly ("couldn't read the
# file") instead of being silently skipped.
#
# `#[path]`-mounted shared-fixture siblings (auth/common.rs,
# slack_pairing_fixtures.rs, support/) have no `[[test]]` entry and are
# therefore never selected — same reason the old walk filtered its
# candidates against the registration.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exec python3 "${repo_root}/scripts/ci/lib/integration_test_inventory.py" "${repo_root}"
