#!/bin/bash
# gotify.sh - Send notifications via Gotify
# Usage: gotify.sh "title" "message" [priority]
# Priority: 1-3 = low (chat), 5-7 = medium (cron/research), 8-10 = high (urgent)

set -euo pipefail

TITLE="${1:-Baud}"
MESSAGE="${2:-No message}"
PRIORITY="${3:-3}"

curl -s -X POST "$GOTIFY_URL/message" \
  -H "X-Gotify-Key: $GOTIFY_APP_TOKEN" \
  -H "Content-Type: application/json" \
  -d "$(python3 -c "
import json, sys
print(json.dumps({
    'title': sys.argv[1],
    'message': sys.argv[2],
    'priority': int(sys.argv[3])
}))
" "$TITLE" "$MESSAGE" "$PRIORITY")" > /dev/null 2>&1

echo "OK"
