#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Reset IronClaw extension and user channel state while preserving administrator
configuration, managed secrets, and automations.

IronClaw must be stopped before applying the reset.

Local/libSQL:
  scripts/reset-extension-state.sh --local-root PATH
  scripts/reset-extension-state.sh --local-root PATH --apply --server-stopped

PostgreSQL:
  scripts/reset-extension-state.sh --database-url URL
  scripts/reset-extension-state.sh --database-url URL --apply --server-stopped

Without --apply, the script only reports what would be removed.
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

local_root=""
database_url=""
apply=false
server_stopped=false

while [ "$#" -gt 0 ]; do
  case "$1" in
    --local-root)
      [ "$#" -ge 2 ] || die "--local-root requires a path"
      local_root="$2"
      shift 2
      ;;
    --database-url)
      [ "$#" -ge 2 ] || die "--database-url requires a URL"
      database_url="$2"
      shift 2
      ;;
    --apply)
      apply=true
      shift
      ;;
    --server-stopped)
      server_stopped=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

if [ -n "$local_root" ] && [ -n "$database_url" ]; then
  die "choose either --local-root or --database-url, not both"
fi
if [ -z "$local_root" ] && [ -z "$database_url" ]; then
  usage >&2
  exit 1
fi
if [ "$apply" = true ] && [ "$server_stopped" != true ]; then
  die "--apply requires --server-stopped"
fi

path_predicate=$(cat <<'SQL'
(
  path = '/system/extensions'
  OR path LIKE '/system/extensions/%'
  OR path LIKE '/tenants/%/shared/channel-identities'
  OR path LIKE '/tenants/%/shared/channel-identities/%'
  OR path LIKE '/tenants/%/shared/channel-dm-targets'
  OR path LIKE '/tenants/%/shared/channel-dm-targets/%'
  OR path LIKE '/tenants/%/shared/reply-contexts'
  OR path LIKE '/tenants/%/shared/reply-contexts/%'
  OR path LIKE '/tenants/%/shared/channel-pairing'
  OR path LIKE '/tenants/%/shared/channel-pairing/%'
)
SQL
)

extension_count_sql=$(cat <<SQL
SELECT
  (SELECT COUNT(*) FROM root_filesystem_entries WHERE $path_predicate)
  + (SELECT COUNT(*) FROM root_filesystem_events WHERE $path_predicate)
  + (SELECT COUNT(*) FROM root_filesystem_sequences WHERE $path_predicate);
SQL
)

delete_sql=$(cat <<SQL
DELETE FROM root_filesystem_events WHERE $path_predicate;
DELETE FROM root_filesystem_sequences WHERE $path_predicate;
DELETE FROM root_filesystem_entries WHERE $path_predicate;
SQL
)

if [ -n "$local_root" ]; then
  command -v sqlite3 >/dev/null 2>&1 || die "sqlite3 is required for --local-root"
  [ -d "$local_root" ] || die "local storage root does not exist: $local_root"

  local_root="$(cd "$local_root" && pwd -P)"
  [ "$local_root" != "/" ] || die "refusing to use / as the local storage root"

  database="$local_root/reborn-local-dev.db"
  package_root="$local_root/system/extensions"
  [ -f "$database" ] || die "local IronClaw database not found: $database"

  sqlite_query() {
    sqlite3 -bail -noheader "$database" "$1"
  }

  extension_rows_before="$(sqlite_query "$extension_count_sql")"
  automations_before="$(sqlite_query "SELECT COUNT(*) FROM trigger_records;")"
  automation_runs_before="$(sqlite_query "SELECT COUNT(*) FROM trigger_run_history;")"
  package_files_before=0
  if [ -d "$package_root" ]; then
    package_files_before="$(find "$package_root" -type f | wc -l | tr -d ' ')"
  fi

  printf 'Backend: local/libSQL\n'
  printf 'Extension database rows to remove: %s\n' "$extension_rows_before"
  printf 'Extension package files to remove: %s\n' "$package_files_before"
  printf 'Automation definitions to preserve: %s\n' "$automations_before"
  printf 'Automation history rows to preserve: %s\n' "$automation_runs_before"

  if [ "$apply" != true ]; then
    printf 'Dry run only; nothing changed. Add --apply --server-stopped to reset.\n'
    exit 0
  fi

  sqlite3 -bail "$database" <<SQL
BEGIN IMMEDIATE;
$delete_sql
COMMIT;
SQL

  if [ -d "$package_root" ]; then
    find "$package_root" -mindepth 1 -depth -delete
  else
    mkdir -p "$package_root"
  fi

  extension_rows_after="$(sqlite_query "$extension_count_sql")"
  automations_after="$(sqlite_query "SELECT COUNT(*) FROM trigger_records;")"
  automation_runs_after="$(sqlite_query "SELECT COUNT(*) FROM trigger_run_history;")"

elif [ -n "$database_url" ]; then
  command -v psql >/dev/null 2>&1 || die "psql is required for --database-url"

  psql_query() {
    psql "$database_url" -X -v ON_ERROR_STOP=1 -qAt -c "$1"
  }

  extension_rows_before="$(psql_query "$extension_count_sql")"
  automations_before="$(psql_query "SELECT COUNT(*) FROM trigger_records;")"
  automation_runs_before="$(psql_query "SELECT COUNT(*) FROM trigger_run_history;")"

  printf 'Backend: PostgreSQL\n'
  printf 'Extension database rows to remove: %s\n' "$extension_rows_before"
  printf 'Automation definitions to preserve: %s\n' "$automations_before"
  printf 'Automation history rows to preserve: %s\n' "$automation_runs_before"

  if [ "$apply" != true ]; then
    printf 'Dry run only; nothing changed. Add --apply --server-stopped to reset.\n'
    exit 0
  fi

  psql "$database_url" -X -v ON_ERROR_STOP=1 -q <<SQL
BEGIN;
$delete_sql
COMMIT;
SQL

  extension_rows_after="$(psql_query "$extension_count_sql")"
  automations_after="$(psql_query "SELECT COUNT(*) FROM trigger_records;")"
  automation_runs_after="$(psql_query "SELECT COUNT(*) FROM trigger_run_history;")"
fi

[ "$extension_rows_after" = "0" ] ||
  die "extension reset incomplete: $extension_rows_after database rows remain"
[ "$automations_after" = "$automations_before" ] ||
  die "automation count changed from $automations_before to $automations_after"
[ "$automation_runs_after" = "$automation_runs_before" ] ||
  die "automation history count changed from $automation_runs_before to $automation_runs_after"

printf 'Reset complete. Extension package/install and user channel state are empty.\n'
printf 'Administrator configuration, managed secrets, and automations were preserved.\n'
printf 'Start the new IronClaw version and reinstall the extensions it needs.\n'
