#!/usr/bin/env bash
# Vendors the WebUI v2 third-party assets that stay OUTSIDE the esbuild
# bundle into ../static/vendor/, so the SPA loads zero remote origins.
#
#   - Tailwind browser runtime  (DOM-scanning IIFE, not an ES module)
#   - DOMPurify / marked / highlight.js  (consumed as window globals)
#   - Google Fonts CSS + woff2 files  (url()s rewritten to local paths)
#
# Versions are pinned to match what index.html previously pulled from the
# CDNs. Bump them here and re-run; the downloaded files are committed.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENDOR_DIR="$SCRIPT_DIR/../static/vendor"
FONTS_DIR="$VENDOR_DIR/fonts"

TAILWIND_VER="4.3.1"
DOMPURIFY_VER="3.2.3"
MARKED_VER="17.0.2"
HLJS_VER="11.11.1"

# A desktop browser UA so Google Fonts serves modern woff2 @font-face.
UA="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36"
FONTS_QUERY="family=Geist:wght@400;500;600;700&family=Geist+Mono:wght@400;500;600&family=Newsreader:opsz,wght@6..72,500;6..72,600&display=swap"

mkdir -p "$VENDOR_DIR" "$FONTS_DIR"

fetch() {
  # fetch <url> <dest> — retry on flaky CDN connections.
  echo "  GET $1"
  curl -fsSL --connect-timeout 15 --max-time 45 \
    --retry 4 --retry-delay 2 --retry-connrefused \
    -A "$UA" -o "$2" "$1"
}

echo "Vendoring JS libraries…"
fetch "https://cdn.jsdelivr.net/npm/@tailwindcss/browser@${TAILWIND_VER}" "$VENDOR_DIR/tailwindcss-browser.js"
fetch "https://cdnjs.cloudflare.com/ajax/libs/dompurify/${DOMPURIFY_VER}/purify.min.js" "$VENDOR_DIR/purify.min.js"
fetch "https://cdn.jsdelivr.net/npm/marked@${MARKED_VER}/lib/marked.umd.min.js" "$VENDOR_DIR/marked.umd.min.js"
fetch "https://cdnjs.cloudflare.com/ajax/libs/highlight.js/${HLJS_VER}/highlight.min.js" "$VENDOR_DIR/highlight.min.js"

echo "Vendoring Google Fonts…"
RAW_CSS="$(curl -fsSL --max-time 60 -A "$UA" "https://fonts.googleapis.com/css2?${FONTS_QUERY}")"

# Download every gstatic woff2 the CSS references and rewrite each
# absolute URL to a local ./<basename>.woff2 path. `while read` (rather
# than `for url in $(...)`) avoids word-splitting/globbing on the URL list.
CSS="$RAW_CSS"
while IFS= read -r url; do
  [[ -z "$url" ]] && continue
  base="$(basename "$url")"
  fetch "$url" "$FONTS_DIR/$base"
  CSS="${CSS//$url/./$base}"
done < <(printf '%s\n' "$RAW_CSS" | grep -oE 'https://fonts\.gstatic\.com/[^)]+\.woff2' | sort -u)

# Normalize single-quoted font-family / format() names to double quotes to
# match the repo CSS convention (see static/styles/app.css).
CSS="${CSS//\'/\"}"

printf '%s\n' "$CSS" > "$FONTS_DIR/fonts.css"

echo "Done. Vendored assets in static/vendor/"
