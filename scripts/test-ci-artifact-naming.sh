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

# Remaining WASM tools
assert_parse "tool-portfolio-0.1.0-wasm32-wasip2.tar.gz" "tool" "portfolio"
assert_parse "tool-telegram_mtproto-0.2.1-wasm32-wasip2.tar.gz" "tool" "telegram_mtproto"

# Telegram collision case: tool and channel with same base name
assert_parse "channel-telegram-0.2.2-wasm32-wasip2.tar.gz" "channel" "telegram"

echo ""
echo "Results: $PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]] || exit 1
