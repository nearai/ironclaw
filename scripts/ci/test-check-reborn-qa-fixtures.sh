#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
checker="$repo_root/scripts/ci/check-reborn-qa-fixtures.sh"
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

write_fixture() {
  local directory=$1
  local expects=$2
  mkdir -p "$directory"
  printf '%s\n' \
    '{"model_name":"test","turns":[{"user_input":"hello","steps":[{"response":{"type":"text","content":"done","input_tokens":0,"output_tokens":0}}],"expects":'"$expects"'}]}' \
    > "$directory/case.json"
}

write_fixture "$tmp_dir/valid" '{"final_response":{"contains":["done"]}}'
"$checker" "$tmp_dir/valid" >/dev/null

write_fixture "$tmp_dir/empty-expects" '{}'
if "$checker" "$tmp_dir/empty-expects" >/dev/null 2>&1; then
  echo "checker accepted a fixture with meaningless empty assertions" >&2
  exit 1
fi

write_fixture "$tmp_dir/meaningless-expects" '{"tools_used":[]}'
if "$checker" "$tmp_dir/meaningless-expects" >/dev/null 2>&1; then
  echo "checker accepted a fixture with only empty assertion values" >&2
  exit 1
fi

write_fixture "$tmp_dir/candidate" '{"final_response":{"contains":["done"]}}'
python3 - "$tmp_dir/candidate/case.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
fixture = json.loads(path.read_text(encoding="utf-8"))
fixture["_review"] = {"status": "candidate"}
path.write_text(json.dumps(fixture), encoding="utf-8")
PY
if "$checker" "$tmp_dir/candidate" >/dev/null 2>&1; then
  echo "checker accepted an unpromoted review-required candidate" >&2
  exit 1
fi

unsafe_token='sk-proj-THIS_MUST_NEVER_APPEAR_IN_DIAGNOSTICS_123456789'
write_fixture "$tmp_dir/unsafe" '{"final_response":{"contains":["done"]}}'
python3 - "$tmp_dir/unsafe/case.json" "$unsafe_token" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
fixture = json.loads(path.read_text(encoding="utf-8"))
fixture["turns"][0]["user_input"] = sys.argv[2]
path.write_text(json.dumps(fixture), encoding="utf-8")
PY
unsafe_output="$tmp_dir/unsafe-output"
if "$checker" "$tmp_dir/unsafe" >"$unsafe_output" 2>&1; then
  echo "checker accepted an unsafe secret-shaped fixture" >&2
  exit 1
fi
if grep -qF "$unsafe_token" "$unsafe_output"; then
  echo "checker leaked the raw secret match in diagnostics" >&2
  exit 1
fi

echo "Reborn QA fixture checker self-tests passed"
