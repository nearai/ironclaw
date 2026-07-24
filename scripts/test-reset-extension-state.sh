#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo_root/scripts/reset-extension-state.sh"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/ironclaw-extension-reset.XXXXXX")"
storage_root="$fixture_root/storage"
database="$storage_root/reborn-local-dev.db"

cleanup() {
  find "$fixture_root" -depth -delete
}
trap cleanup EXIT

mkdir -p "$storage_root/system/extensions/slack"
printf 'fixture\n' >"$storage_root/system/extensions/slack/manifest.toml"

sqlite3 "$database" <<'SQL'
CREATE TABLE root_filesystem_entries (path TEXT PRIMARY KEY, value TEXT);
CREATE TABLE root_filesystem_events (id INTEGER PRIMARY KEY, path TEXT);
CREATE TABLE root_filesystem_sequences (path TEXT PRIMARY KEY, value INTEGER);
CREATE TABLE trigger_records (id TEXT PRIMARY KEY, value TEXT);
CREATE TABLE trigger_run_history (id TEXT PRIMARY KEY, value TEXT);

INSERT INTO root_filesystem_entries(path, value) VALUES
  ('/system/extensions', 'extension-root'),
  ('/system/extensions/.installations/slack', 'installed'),
  ('/system/extensions/slack/package.wasm', 'package'),
  ('/system/extensions-backup/keep', 'not-an-extension'),
  ('/tenants/default/shared/extension-admin-configuration/groups/extension.slack.json', 'admin'),
  ('/tenants/default/shared/extension-admin-configuration/groups/extension.slack/revisions/1.json', 'admin-revision'),
  ('/tenants/default/users/__ironclaw_tenant_shared_admin__/secrets/secrets/admin-slack-token.json', 'encrypted-admin-secret'),
  ('/tenants/default/shared/channel-identities/slack/U123', 'identity'),
  ('/tenants/default/shared/channel-dm-targets/slack/U123', 'dm-target'),
  ('/tenants/default/shared/reply-contexts/slack/C123', 'reply-context'),
  ('/tenants/default/shared/channel-pairing/slack/U123', 'pairing'),
  ('/tenants/default/shared/conversations/thread-1', 'conversation');

INSERT INTO root_filesystem_events(path) VALUES
  ('/system/extensions/.installations/slack'),
  ('/tenants/default/shared/extension-admin-configuration/groups/extension.slack.json'),
  ('/tenants/default/shared/channel-identities/slack/U123'),
  ('/tenants/default/shared/conversations/thread-1');

INSERT INTO root_filesystem_sequences(path, value) VALUES
  ('/system/extensions/.installations/slack', 1),
  ('/tenants/default/shared/extension-admin-configuration/groups/extension.slack.json', 1),
  ('/tenants/default/shared/channel-pairing/slack/U123', 1),
  ('/tenants/default/shared/conversations/thread-1', 1);

INSERT INTO trigger_records(id, value) VALUES
  ('automation-1', 'keep this automation');
INSERT INTO trigger_run_history(id, value) VALUES
  ('run-1', 'keep this run');
SQL

resettable_extension_entry_count() {
  sqlite3 "$database" "
    SELECT COUNT(*) FROM root_filesystem_entries
    WHERE path = '/system/extensions'
       OR path LIKE '/system/extensions/%'
       OR path LIKE '/tenants/%/shared/channel-identities'
       OR path LIKE '/tenants/%/shared/channel-identities/%'
       OR path LIKE '/tenants/%/shared/channel-dm-targets'
       OR path LIKE '/tenants/%/shared/channel-dm-targets/%'
       OR path LIKE '/tenants/%/shared/reply-contexts'
       OR path LIKE '/tenants/%/shared/reply-contexts/%'
       OR path LIKE '/tenants/%/shared/channel-pairing'
       OR path LIKE '/tenants/%/shared/channel-pairing/%';
  "
}

assert_equals() {
  expected="$1"
  actual="$2"
  message="$3"
  if [ "$actual" != "$expected" ]; then
    printf 'FAIL: %s (expected %s, got %s)\n' "$message" "$expected" "$actual" >&2
    exit 1
  fi
}

"$script" --local-root "$storage_root" >"$fixture_root/dry-run.txt"
assert_equals 7 "$(resettable_extension_entry_count)" "dry-run must not change extension rows"
assert_equals 1 "$(find "$storage_root/system/extensions" -type f | wc -l | tr -d ' ')" \
  "dry-run must not remove package files"

if "$script" --local-root "$storage_root" --apply >"$fixture_root/missing-confirmation.txt" 2>&1; then
  printf 'FAIL: --apply must require confirmation that IronClaw is stopped\n' >&2
  exit 1
fi

"$script" --local-root "$storage_root" --apply --server-stopped \
  >"$fixture_root/apply.txt"

assert_equals 0 "$(resettable_extension_entry_count)" "extension package/install entries must be removed"
assert_equals 0 "$(sqlite3 "$database" "
  SELECT COUNT(*) FROM root_filesystem_events
  WHERE path = '/system/extensions'
     OR path LIKE '/system/extensions/%'
     OR path LIKE '/tenants/%/shared/channel-identities'
     OR path LIKE '/tenants/%/shared/channel-identities/%';
")" "extension events must be removed"
assert_equals 0 "$(sqlite3 "$database" "
  SELECT COUNT(*) FROM root_filesystem_sequences
  WHERE path = '/system/extensions'
     OR path LIKE '/system/extensions/%'
     OR path LIKE '/tenants/%/shared/channel-pairing'
     OR path LIKE '/tenants/%/shared/channel-pairing/%';
")" "extension sequences must be removed"
assert_equals 0 "$(sqlite3 "$database" "
  SELECT COUNT(*) FROM root_filesystem_entries
  WHERE path IN (
    '/tenants/default/shared/channel-identities/slack/U123',
    '/tenants/default/shared/channel-dm-targets/slack/U123',
    '/tenants/default/shared/reply-contexts/slack/C123',
    '/tenants/default/shared/channel-pairing/slack/U123'
  );
")" "channel connection state must be removed"
assert_equals 5 "$(sqlite3 "$database" "
  SELECT COUNT(*) FROM root_filesystem_entries
  WHERE path IN (
    '/tenants/default/shared/extension-admin-configuration/groups/extension.slack.json',
    '/tenants/default/shared/extension-admin-configuration/groups/extension.slack/revisions/1.json',
    '/tenants/default/users/__ironclaw_tenant_shared_admin__/secrets/secrets/admin-slack-token.json'
  )
  UNION ALL
  SELECT COUNT(*) FROM root_filesystem_events
  WHERE path = '/tenants/default/shared/extension-admin-configuration/groups/extension.slack.json'
  UNION ALL
  SELECT COUNT(*) FROM root_filesystem_sequences
  WHERE path = '/tenants/default/shared/extension-admin-configuration/groups/extension.slack.json';
" | awk '{ total += $1 } END { print total }')" \
  "admin configuration records and managed secrets must remain"
assert_equals 4 "$(sqlite3 "$database" "
  SELECT COUNT(*) FROM root_filesystem_entries
  WHERE path IN (
    '/system/extensions-backup/keep',
    '/tenants/default/shared/conversations/thread-1'
  )
  UNION ALL
  SELECT COUNT(*) FROM root_filesystem_events
  WHERE path = '/tenants/default/shared/conversations/thread-1'
  UNION ALL
  SELECT COUNT(*) FROM root_filesystem_sequences
  WHERE path = '/tenants/default/shared/conversations/thread-1';
" | awk '{ total += $1 } END { print total }')" "unrelated filesystem state must remain"
assert_equals "automation-1|keep this automation" \
  "$(sqlite3 "$database" "SELECT id || '|' || value FROM trigger_records;")" \
  "automation definitions must remain unchanged"
assert_equals "run-1|keep this run" \
  "$(sqlite3 "$database" "SELECT id || '|' || value FROM trigger_run_history;")" \
  "automation history must remain unchanged"
assert_equals 0 "$(find "$storage_root/system/extensions" -mindepth 1 | wc -l | tr -d ' ')" \
  "extension package files must be removed"

"$script" --local-root "$storage_root" --apply --server-stopped \
  >"$fixture_root/second-apply.txt"
assert_equals 1 "$(sqlite3 "$database" "SELECT COUNT(*) FROM trigger_records;")" \
  "rerunning the reset must preserve automations"

printf 'PASS: extension reset preserves only admin configuration, automations, and unrelated state\n'
