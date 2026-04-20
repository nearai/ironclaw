#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
  echo "Run with sudo: sudo scripts/install-lunarwing-watchdog.sh" >&2
  exit 1
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

# Migration cleanup: older installs used ironclaw-watchdog names. Remove those
# units and binary before installing the LunarWing watchdog to avoid two timers.
systemctl disable --now ironclaw-watchdog.timer >/dev/null 2>&1 || true
systemctl disable --now ironclaw-watchdog.service >/dev/null 2>&1 || true
rm -f \
  /etc/systemd/system/ironclaw-watchdog.service \
  /etc/systemd/system/ironclaw-watchdog.timer \
  /usr/local/sbin/ironclaw-watchdog

install -o root -g root -m 0755 \
  "${REPO_ROOT}/scripts/lunarwing-watchdog.sh" \
  /usr/local/sbin/lunarwing-watchdog

install -o root -g root -m 0644 \
  "${REPO_ROOT}/systemd/lunarwing-watchdog.service" \
  /etc/systemd/system/lunarwing-watchdog.service

install -o root -g root -m 0644 \
  "${REPO_ROOT}/systemd/lunarwing-watchdog.timer" \
  /etc/systemd/system/lunarwing-watchdog.timer

touch /var/log/lunarwing-watchdog.log
chown root:root /var/log/lunarwing-watchdog.log
chmod 0644 /var/log/lunarwing-watchdog.log

systemctl daemon-reload
systemctl enable --now lunarwing-watchdog.timer
systemctl start lunarwing-watchdog.service
systemctl status lunarwing-watchdog.timer --no-pager
