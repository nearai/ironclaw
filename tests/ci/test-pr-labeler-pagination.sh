#!/usr/bin/env bash
set -euo pipefail

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

mkdir -p "$tmpdir/bin"
cat > "$tmpdir/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail

if [[ "$1" == "pr" && "$2" == "edit" ]]; then
  printf 'gh pr edit should not be used for label mutation\n' >&2
  exit 1
fi

if [[ "$1" == "api" && "$2" == "repos/enjimi/ironclaw/issues/26/labels" ]]; then
  exit 0
fi

if [[ "$1" == "api" && "$2" == "--method" && "$3" == "POST" && "$4" == "repos/enjimi/ironclaw/issues/26/labels" ]]; then
  exit 0
fi

if [[ "$1" == "api" && "$2" == "repos/enjimi/ironclaw/pulls/26/files" ]]; then
  case "$*" in
    *".changes]"*)
      printf '4976\n0\n'
      ;;
    *".filename"*)
      printf 'src/main.rs\n'
      ;;
    *)
      exit 1
      ;;
  esac
  exit 0
fi

if [[ "$1" == "api" && "$2" == "repos/enjimi/ironclaw/pulls/26" ]]; then
  printf 'mirror-bot\n'
  exit 0
fi

if [[ "$1" == "api" && "$2" == "--method" && "$3" == "GET" && "$4" == "search/issues" ]]; then
  printf '0\n'
  exit 0
fi

printf 'unexpected gh invocation: %q ' "$@" >&2
printf '\n' >&2
exit 1
GH
chmod +x "$tmpdir/bin/gh"

output="$(
  PATH="$tmpdir/bin:$PATH" \
    PR_NUMBER=26 \
    REPO=enjimi/ironclaw \
    bash .github/scripts/pr-labeler.sh
)"

grep -Fq "Size: 4976 changed lines -> size: XL" <<< "$output"
