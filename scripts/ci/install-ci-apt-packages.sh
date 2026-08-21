#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <apt-package>..." >&2
  exit 2
fi

# Every apt fetch below is bounded twice, because the two bounds catch
# different failures:
#
#   * Acquire::*::Timeout turns a mirror that accepts the TCP connection and
#     then goes silent into an apt-level *error*, so the retries below can
#     actually run. Without it apt blocks forever mid-transfer -- the failure
#     mode behind actions/runner-images incident #5183, which burned whole
#     30-120 minute job caps. The retry loop never helped, because it only
#     fired on a non-zero exit and a hang never returns one.
#
#     Note where the stall actually lands: across 9 sampled hung jobs, the
#     priority:1 mirror azure.archive.ubuntu.com failed FAST and cleanly (17-26
#     `Ign:` lines in seconds) and apt had already fallen through to
#     archive.ubuntu.com -- which is where it then wedged. So there is no
#     further mirror to fall through to, and retrying is the only recovery.
#   * `timeout` is the backstop for what Acquire::*::Timeout does not cover:
#     DNS that never answers, a wedged dpkg frontend, sudo itself.
#
# Together they convert "hangs until the job cap" into "fails in ~90s", which
# is what the retry loop was always written to handle.
#
# ponytail: this is a workaround for a fleet-side bug, and it has a known
# ceiling -- it guarantees a FAST, LOUD failure, not a green build. When the
# mirror is wedged for longer than the retry budget the step still goes red;
# it just costs ~5 min and a re-queue instead of ~120 min and a dequeue.
# actions/runner-images PR #14596 is the real fix; once it ships, the
# Acquire::* options here become redundant (keep the `timeout` backstop).
APT_UPDATE_TIMEOUT="${APT_UPDATE_TIMEOUT:-90s}"
APT_INSTALL_TIMEOUT="${APT_INSTALL_TIMEOUT:-150s}"
APT_ATTEMPTS="${APT_ATTEMPTS:-3}"

# Overridable only so the companion test can point the source-stripping scan at
# a stand-in tree instead of the real one.
APT_SOURCES_DIR="${APT_SOURCES_DIR:-/etc/apt}"

# `sudo timeout ...`, not `timeout sudo ...`: the signal must reach apt-get
# itself. Signalling sudo risks the SIGKILL landing on the wrapper and
# orphaning a root apt-get that still holds the dpkg lock.
#
# `env DEBIAN_FRONTEND=noninteractive` closes the other unbounded wait: a
# conffile or service-restart prompt blocks on stdin forever.
#
# Acquire::Retries covers the wedged-socket case at the apt level: the hang
# lands on the LAST mirror, so a fresh connection is the only way out. Kept
# low so a genuinely dark host still surfaces inside the timeout budget rather
# than eating it -- the outer loop supplies the rest of the attempts.
apt_get_bounded() {
  local timeout_spec="$1"
  shift
  sudo env DEBIAN_FRONTEND=noninteractive \
    timeout --signal=INT --kill-after=30s "${timeout_spec}" \
    apt-get \
    -o Acquire::http::Timeout=15 \
    -o Acquire::https::Timeout=15 \
    -o Acquire::Retries=2 \
    "$@"
}

# Run one apt-get subcommand with bounded attempts and linear backoff. Says why
# it gave up and returns non-zero, so `set -e` ends the step loudly rather than
# letting a downstream `test -x /usr/bin/mold` be the first sign.
apt_get_with_retry() {
  local label="$1" timeout_spec="$2"
  shift 2
  local attempt status
  for attempt in $(seq 1 "${APT_ATTEMPTS}"); do
    # Capture via `|| status=$?`, never `$?` after `fi`: a failed `if` with no
    # `else` branch exits 0, which would report every failure as success.
    # (`|| ...` also keeps `set -e` from ending the script here.)
    status=0
    apt_get_bounded "${timeout_spec}" "$@" || status=$?
    if [ "${status}" -eq 0 ]; then
      return 0
    fi
    if [ "${status}" -eq 124 ]; then
      echo "${label} timed out after ${timeout_spec} (attempt ${attempt}/${APT_ATTEMPTS})" >&2
    else
      echo "${label} failed with exit ${status} (attempt ${attempt}/${APT_ATTEMPTS})" >&2
    fi
    if [ "${attempt}" -lt "${APT_ATTEMPTS}" ]; then
      sleep "$((attempt * 5))"
    fi
  done
  echo "${label} failed after ${APT_ATTEMPTS} attempts" >&2
  return 1
}

# GitHub-hosted Ubuntu images carry Microsoft apt sources -- the Azure CLI repo
# (packages.microsoft.com/repos/azure-cli) and the prod repo
# (packages.microsoft.com/ubuntu/.../prod) -- that transiently return 403 /
# "no longer signed". When any of them breaks, `apt-get update` fails before CI
# can install the small linker packages these jobs need. None are required here,
# so strip every packages.microsoft.com source, not just the Azure CLI one.
#
# Deliberately NOT extended to azure.archive.ubuntu.com: dropping it would not
# help. It is not where jobs hang -- it already fails fast, and apt already
# falls through to archive.ubuntu.com, which is the host that wedges. Removing
# the in-region mirror would only make the healthy path slower.
while IFS= read -r -d '' source_file; do
  if sudo grep -q "packages.microsoft.com" "${source_file}"; then
    echo "Removing unavailable Microsoft apt source: ${source_file}" >&2
    sudo rm -f "${source_file}"
  fi
done < <(sudo find "${APT_SOURCES_DIR}" -type f \( -name "*.list" -o -name "*.sources" \) -print0)

apt_get_with_retry "apt-get update" "${APT_UPDATE_TIMEOUT}" update
apt_get_with_retry "apt-get install" "${APT_INSTALL_TIMEOUT}" install -y "$@"
