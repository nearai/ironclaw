#!/usr/bin/env bash
# Test that kind-prefixed artifact filenames are parsed correctly into
# manifest paths. Mirrors the parsing logic in release.yml.
set -euo pipefail

cd "$(dirname "$0")/.."

PASS=0
FAIL=0

assert_parse() {
    local filename="$1" expected_kind="$2" expected_name="$3"
    local kind name manifest

    kind=$(echo "$filename" | cut -d'-' -f1)
    name=$(echo "$filename" | sed "s/^${kind}-//" | sed 's/-[0-9].*-wasm32-wasip2\.tar\.gz$//')
    manifest="registry/${kind}s/${name}.json"

    if [[ "$kind" != "$expected_kind" ]]; then
        echo "FAIL: $filename → kind=$kind, expected $expected_kind"
        FAIL=$((FAIL + 1))
        return
    fi
    if [[ "$name" != "$expected_name" ]]; then
        echo "FAIL: $filename → name=$name, expected $expected_name"
        FAIL=$((FAIL + 1))
        return
    fi
    echo "OK: $filename → $manifest"
    PASS=$((PASS + 1))
}

# Tool and channel with same name must produce different manifest paths
assert_parse "tool-slack-0.2.1-wasm32-wasip2.tar.gz" "tool" "slack"
assert_parse "channel-slack-0.2.1-wasm32-wasip2.tar.gz" "channel" "slack"

# Same collision case for telegram
assert_parse "tool-telegram-0.2.2-wasm32-wasip2.tar.gz" "tool" "telegram"
assert_parse "channel-telegram-0.2.2-wasm32-wasip2.tar.gz" "channel" "telegram"

# Hyphenated extension names
assert_parse "tool-web-search-0.2.0-wasm32-wasip2.tar.gz" "tool" "web-search"
assert_parse "tool-google-calendar-0.1.0-wasm32-wasip2.tar.gz" "tool" "google-calendar"
assert_parse "tool-google-docs-0.1.0-wasm32-wasip2.tar.gz" "tool" "google-docs"
assert_parse "tool-google-drive-0.1.0-wasm32-wasip2.tar.gz" "tool" "google-drive"
assert_parse "tool-google-sheets-0.1.0-wasm32-wasip2.tar.gz" "tool" "google-sheets"
assert_parse "tool-google-slides-0.1.0-wasm32-wasip2.tar.gz" "tool" "google-slides"

# Simple names
assert_parse "channel-discord-0.2.0-wasm32-wasip2.tar.gz" "channel" "discord"
assert_parse "channel-whatsapp-0.1.0-wasm32-wasip2.tar.gz" "channel" "whatsapp"
assert_parse "tool-github-0.2.0-wasm32-wasip2.tar.gz" "tool" "github"
assert_parse "tool-gmail-0.1.0-wasm32-wasip2.tar.gz" "tool" "gmail"

# Pre-release versions
assert_parse "tool-slack-0.2.1-alpha.1-wasm32-wasip2.tar.gz" "tool" "slack"

echo ""
echo "Results: $PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]] || exit 1
