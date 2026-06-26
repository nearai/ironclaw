#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <apt-package>..." >&2
  exit 2
fi

# GitHub-hosted Ubuntu images can carry an Azure CLI apt source that transiently
# returns 403s. When that unrelated source breaks, `apt-get update` fails before
# CI can install the small linker packages these jobs need.
while IFS= read -r -d '' source_file; do
  if sudo grep -q "packages.microsoft.com/repos/azure-cli" "${source_file}"; then
    echo "Removing unavailable Azure CLI apt source: ${source_file}" >&2
    sudo rm -f "${source_file}"
  fi
done < <(sudo find /etc/apt -type f \( -name "*.list" -o -name "*.sources" \) -print0)

sudo apt-get update
sudo apt-get install -y "$@"
