#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
  echo "Run with sudo: sudo scripts/install-ironclaw-watchdog.sh" >&2
  exit 1
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

install -o root -g root -m 0755 \
  "${REPO_ROOT}/scripts/ironclaw-watchdog.sh" \
  /usr/local/sbin/ironclaw-watchdog

install -o root -g root -m 0644 \
  "${REPO_ROOT}/systemd/ironclaw-watchdog.service" \
  /etc/systemd/system/ironclaw-watchdog.service

install -o root -g root -m 0644 \
  "${REPO_ROOT}/systemd/ironclaw-watchdog.timer" \
  /etc/systemd/system/ironclaw-watchdog.timer

touch /var/log/ironclaw-watchdog.log
chown root:root /var/log/ironclaw-watchdog.log
chmod 0644 /var/log/ironclaw-watchdog.log

systemctl daemon-reload
systemctl enable --now ironclaw-watchdog.timer
systemctl start ironclaw-watchdog.service
systemctl status ironclaw-watchdog.timer --no-pager
