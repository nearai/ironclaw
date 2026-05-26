#!/bin/sh
set -eu

if [ -n "${SSL_CERT_FILE:-}" ] && [ -f "${SSL_CERT_FILE}" ]; then
  cp "${SSL_CERT_FILE}" /usr/local/share/ca-certificates/ironclaw-broker.crt
  update-ca-certificates >/dev/null
fi

if [ "${IRONCLAW_EGRESS_LOCKDOWN:-}" = "broker-only" ]; then
  if [ -z "${IRONCLAW_BROKER_PROXY:-}" ]; then
    echo "IRONCLAW_BROKER_PROXY is required for broker-only lockdown" >&2
    exit 65
  fi

  broker_scheme="$(printf '%s' "${IRONCLAW_BROKER_PROXY}" | sed -E 's#^([a-zA-Z][a-zA-Z0-9+.-]*).*$#\1#')"
  broker_host="$(printf '%s' "${IRONCLAW_BROKER_PROXY}" | sed -E 's#^[a-zA-Z][a-zA-Z0-9+.-]*://([^/:]+).*$#\1#')"
  broker_port="$(printf '%s' "${IRONCLAW_BROKER_PROXY}" | sed -E 's#^[a-zA-Z][a-zA-Z0-9+.-]*://[^/:]+:([0-9]+).*$#\1#')"
  if [ "${broker_port}" = "${IRONCLAW_BROKER_PROXY}" ]; then
    if [ "${broker_scheme}" = "https" ]; then
      broker_port=443
    else
      broker_port=80
    fi
  fi

  if printf '%s' "${broker_host}" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$'; then
    broker_ips="${broker_host}"
  else
    broker_ips="$(getent hosts "${broker_host}" | awk '{print $1}')"
  fi
  if [ -z "${broker_ips}" ]; then
    echo "failed to resolve broker host" >&2
    exit 65
  fi

  iptables -P OUTPUT DROP
  iptables -A OUTPUT -o lo -j ACCEPT
  for broker_ip in ${broker_ips}; do
    iptables -A OUTPUT -p tcp -d "${broker_ip}" --dport "${broker_port}" -j ACCEPT
  done
  iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
fi

exec capsh --drop=all --user=sandbox -- -c 'exec "$@"' process-sandbox "$@"
