#!/bin/bash
# Fetches the .env from Secret Manager at service start.
# Called by ExecStartPre in t3claw.service.
# The secret name is derived from the VM's t3env metadata attribute.
set -e

PROJECT="gen-lang-client-0263867259"

T3ENV=$(curl -sf \
  "http://metadata.google.internal/computeMetadata/v1/instance/attributes/t3env" \
  -H "Metadata-Flavor: Google" 2>/dev/null || echo "staging")

SECRET="t3claw-${T3ENV}-env"

tmp=$(mktemp /opt/t3claw/.env.XXXXXX)
chmod 600 "$tmp"
gcloud secrets versions access latest --secret="${SECRET}" --project="${PROJECT}" > "$tmp"
mv "$tmp" /opt/t3claw/.env
