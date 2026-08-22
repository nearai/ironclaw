#!/usr/bin/env bash
# Regression tests for the CI apt installer's hang handling.
#
# GitHub's Ubuntu image lists azure.archive.ubuntu.com as the priority:1 apt
# mirror. During actions/runner-images incident #5183 that host started
# accepting the TCP connection and then stalling mid-transfer instead of
# failing, so `apt-get` blocked forever, the installer's retry loop (which only
# fires on a non-zero exit) never ran, and jobs burned their whole 30-120 minute
# cap. These tests pin the property that makes that impossible: every apt
# invocation is bounded, so a hang becomes a fast, loud failure.
#
# Every apt invocation here is stubbed, so this runs in seconds and downloads
# nothing.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALLER="$SCRIPT_DIR/install-ci-apt-packages.sh"
failures=0

# How long a stubbed hang sleeps. Must exceed the per-attempt timeouts the
# cases below pass in, so "bounded" and "hung" are distinguishable.
STUB_HANG_SECONDS=120

# Run the installer with a stubbed `sudo`, `apt-get`, and apt source tree.
#
# PATH is REPLACED, not prepended, so a real apt-get can never satisfy a lookup
# and make a case silently exercise the wrong branch.
run_installer() {
    local mode="$1"
    shift
    local sandbox
    sandbox="$(mktemp -d)"

    # `sudo` that just runs the command. The installer must keep working when
    # every privileged call is a plain exec.
    cat >"$sandbox/sudo" <<'STUB'
#!/usr/bin/env bash
exec "$@"
STUB

    # `apt-get` whose behaviour is selected by APT_STUB_MODE. `update` and
    # `install` are distinguished by scanning the args, because the installer
    # passes -o options before the subcommand.
    cat >"$sandbox/apt-get" <<STUB
#!/usr/bin/env bash
subcommand=""
for arg in "\$@"; do
    case "\$arg" in
        update|install) subcommand="\$arg"; break ;;
    esac
done
echo "apt-get \$subcommand" >>"\$APT_TEST_LOG"
case "\$APT_STUB_MODE" in
    happy) exit 0 ;;
    hang_update)
        [ "\$subcommand" = update ] && sleep $STUB_HANG_SECONDS
        exit 0 ;;
    hang_install)
        [ "\$subcommand" = install ] && sleep $STUB_HANG_SECONDS
        exit 0 ;;
    flaky_update)
        if [ "\$subcommand" = update ]; then
            attempts=\$(grep -c "apt-get update" "\$APT_TEST_LOG")
            [ "\$attempts" -lt 2 ] && exit 100
        fi
        exit 0 ;;
esac
exit 0
STUB

    chmod +x "$sandbox/sudo" "$sandbox/apt-get"

    # A stand-in apt source tree, so the installer's Microsoft-source stripping
    # is exercised without any risk of touching the real /etc/apt.
    mkdir -p "$sandbox/etc-apt/sources.list.d"
    printf 'deb http://packages.microsoft.com/repos/azure-cli noble main\n' \
        >"$sandbox/etc-apt/sources.list.d/microsoft-prod.list"
    printf 'deb http://azure.archive.ubuntu.com/ubuntu noble main\n' \
        >"$sandbox/etc-apt/sources.list.d/ubuntu.list"

    local status=0
    env -i \
        PATH="$sandbox:/usr/bin:/bin" \
        HOME="$sandbox" \
        APT_TEST_LOG="$sandbox/log" \
        APT_STUB_MODE="$mode" \
        APT_SOURCES_DIR="$sandbox/etc-apt" \
        APT_UPDATE_TIMEOUT=2s \
        APT_INSTALL_TIMEOUT=2s \
        APT_ATTEMPTS=2 \
        bash "$INSTALLER" clang mold >"$sandbox/out" 2>&1 || status=$?

    printf '%s' "$status" >"$sandbox/status"
    last_status="$status"
    last_output="$(cat "$sandbox/out")"
    last_log="$(cat "$sandbox/log" 2>/dev/null || true)"
    last_sandbox="$sandbox"
}

fail() {
    echo "FAIL: $1" >&2
    [ -n "${2:-}" ] && printf '  %s\n' "$2" >&2
    failures=$((failures + 1))
}

expect_status() {
    local label="$1" want="$2"
    if [ "$last_status" != "$want" ]; then
        fail "$label: expected exit $want, got $last_status" "$last_output"
    fi
}

expect_output_contains() {
    local label="$1" needle="$2"
    if ! grep -qF -- "$needle" <<<"$last_output"; then
        fail "$label: output missing '$needle'" "$last_output"
    fi
}

expect_faster_than() {
    local label="$1" limit="$2" elapsed="$3"
    if [ "$elapsed" -ge "$limit" ]; then
        fail "$label: took ${elapsed}s, expected under ${limit}s (it hung)"
    fi
}

time_installer() {
    local start end
    start="$(date +%s)"
    run_installer "$@"
    end="$(date +%s)"
    elapsed=$((end - start))
}

# --- Case 1: `apt-get update` hangs -----------------------------------------
# The incident case. Must fail fast rather than block until the job cap.
time_installer hang_update
expect_faster_than "update hang" "$STUB_HANG_SECONDS" "$elapsed"
expect_status "update hang" 1
expect_output_contains "update hang" "timed out"

# --- Case 2: happy path ------------------------------------------------------
time_installer happy
expect_status "happy path" 0
if ! grep -q "apt-get install" <<<"$last_log"; then
    fail "happy path: install was never invoked" "$last_log"
fi

# --- Case 3: transient `apt-get update` failure is retried -------------------
# The behaviour the original retry loop was written for; bounding must not
# break it.
time_installer flaky_update
expect_status "transient failure" 0
if [ "$(grep -c 'apt-get update' <<<"$last_log")" -lt 2 ]; then
    fail "transient failure: update was not retried" "$last_log"
fi

# --- Case 4: `apt-get install` hangs -----------------------------------------
# `install` had neither a retry nor a timeout, so it hung exactly like update.
time_installer hang_install
expect_faster_than "install hang" "$STUB_HANG_SECONDS" "$elapsed"
expect_status "install hang" 1
expect_output_contains "install hang" "timed out"

# --- Case 5: Microsoft sources are still stripped ----------------------------
# Pins the pre-existing behaviour that the bounding must not regress.
run_installer happy
if [ -e "$last_sandbox/etc-apt/sources.list.d/microsoft-prod.list" ]; then
    fail "source stripping: packages.microsoft.com source was not removed"
fi
if [ ! -e "$last_sandbox/etc-apt/sources.list.d/ubuntu.list" ]; then
    fail "source stripping: a non-Microsoft source was removed"
fi

if [ "$failures" -ne 0 ]; then
    echo "$failures check(s) failed" >&2
    exit 1
fi
echo "All install-ci-apt-packages.sh checks passed"
