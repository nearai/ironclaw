#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <apt-package>..." >&2
  exit 2
fi

# GitHub-hosted Ubuntu images carry Microsoft apt sources -- the Azure CLI repo
# (packages.microsoft.com/repos/azure-cli) and the prod repo
# (packages.microsoft.com/ubuntu/.../prod) -- that transiently return 403 /
# "no longer signed". When any of them breaks, `apt-get update` fails before CI
# can install the small linker packages these jobs need. None are required here,
# so strip every packages.microsoft.com source, not just the Azure CLI one.
while IFS= read -r -d '' source_file; do
  if sudo grep -q "packages.microsoft.com" "${source_file}"; then
    echo "Removing unavailable Microsoft apt source: ${source_file}" >&2
    sudo rm -f "${source_file}"
  fi
done < <(sudo find /etc/apt -type f \( -name "*.list" -o -name "*.sources" \) -print0)

# Even with broken sources removed, the remaining mirrors occasionally return
# transient errors, so retry `apt-get update` a few times before giving up.
update_ok=false
for attempt in 1 2 3; do
  if sudo apt-get update; then
    update_ok=true
    break
  fi
  echo "apt-get update failed (attempt ${attempt}/3); retrying in $((attempt * 5))s..." >&2
  sleep "$((attempt * 5))"
done
if [ "${update_ok}" != true ]; then
  echo "apt-get update failed after 3 attempts" >&2
  exit 1
fi

sudo apt-get install -y "$@"
