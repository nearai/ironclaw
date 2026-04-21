function renderMarkdown(text) {
  if (typeof marked !== 'undefined') {
    // Escape raw HTML error pages instead of rendering them as markup.
    // Only triggers when the text *starts with* a doctype or <html> tag
    // (after optional whitespace), so normal messages that mention HTML
    // tags in prose or code fences are not affected.  See #263.
    if (/^\s*<!doctype\s/i.test(text) || /^\s*<html[\s>]/i.test(text)) {
      return escapeHtml(text);
    }
    let html = marked.parse(text);
    // Sanitize HTML output to prevent XSS from tool output or LLM responses.
    html = sanitizeRenderedHtml(html);
    // Inject copy buttons into <pre> blocks
    html = html.replace(/<pre>/g, '<pre class="code-block-wrapper"><button class="copy-btn" data-action="copy-code">Copy</button>');
    return html;
  }
  return escapeHtml(text);
}

// Sanitize rendered HTML using DOMPurify to prevent XSS from tool output
// or prompt injection in LLM responses. DOMPurify is a DOM-based sanitizer
// that handles all known bypass vectors (SVG onload, newline-split event
// handlers, mutation XSS, etc.) unlike the regex approach it replaces.
function sanitizeRenderedHtml(html) {
  if (typeof DOMPurify !== 'undefined') {
    return DOMPurify.sanitize(html, {
      USE_PROFILES: { html: true },
      FORBID_TAGS: ['style', 'script'],
      FORBID_ATTR: ['style', 'onerror', 'onload']
    });
  }
  // DOMPurify not available (CDN unreachable) — return empty string rather than unsanitized HTML
  return '';
}

// ==================== Structured Data Rendering ====================
//
// Detects JSON objects and key-value data in assistant messages and
// renders them as styled cards instead of raw text. Also supports
// extensible chat renderers via IronClaw.registerChatRenderer().

/**
 * Post-process a .message-content element to upgrade structured data into cards.
 * Runs registered chat renderers first, then falls back to built-in JSON detection.
 */
function upgradeStructuredData(contentEl) {
  // 1. Run registered chat renderers.
  //
  // Each registered renderer receives the live `.message-content` element
  // and the textContent. The renderer is allowed to mutate the element —
  // attach event listeners, set data attributes, swap inner DOM — but any
  // HTML it injects must still pass DOMPurify before it reaches the user.
  // `renderMarkdown` already runs `sanitizeRenderedHtml` on the markdown
  // output BEFORE this function is called, but a renderer that does
  // `contentEl.innerHTML = '<form action="https://attacker">...'` would
  // bypass that sanitization step entirely. Re-run the sanitizer on
  // whatever the renderer leaves behind so the same HTML allowlist
  // applies regardless of how the content got there.
  //
  // CSP already blocks `<script>` execution either way; this guards the
  // form/iframe/object/clickjack-overlay vector that doesn't trip CSP.
  var renderers = (window.IronClaw && IronClaw._chatRenderers) || [];
  for (var i = 0; i < renderers.length; i++) {
    try {
      if (renderers[i].match(contentEl.textContent, contentEl)) {
        renderers[i].render(contentEl, contentEl.textContent);
        // Post-renderer sanitization — DOMPurify is idempotent on
        // already-safe HTML, so the cost on the happy path is bounded
        // by the sanitizer's own walk of the post-renderer subtree.
        contentEl.innerHTML = sanitizeRenderedHtml(contentEl.innerHTML);
        return; // First matching renderer wins
      }
    } catch (e) {
      console.error('[IronClaw] Chat renderer "' + renderers[i].id + '" failed:', e);
    }
  }

  // 2. Built-in: detect and upgrade inline JSON objects.
  //
  // Off by default — the bracket-counting heuristic false-positives on
  // any prose containing balanced `{...}` (e.g. an assistant explaining
  // "set the value to {x: 1, y: 2}"), and the rewrite then mangles the
  // explanation into a styled card. Operators that pipe structured data
  // through chat opt in via `chat.upgrade_inline_json` in
  // `.system/gateway/layout.json`.
  var layoutCfg = window.__IRONCLAW_LAYOUT__;
  if (layoutCfg && layoutCfg.chat && layoutCfg.chat.upgrade_inline_json === true) {
    upgradeInlineJson(contentEl);
  }
}

/**
 * Find JSON-like objects in text nodes and replace them with styled cards.
 *
 * Uses a linear bracket-counting scan instead of a regex with nested
 * quantifiers — the old `/(\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\})/g` exhibited
 * catastrophic backtracking on adversarial input. The current implementation
 * is bounded by two caps:
 *   - MAX_PARA_LEN: skip paragraphs larger than this entirely
 *   - MAX_SCAN:     each `{` scan is capped at this many chars
 *   - MAX_CANDIDATES per paragraph
 */
function upgradeInlineJson(contentEl) {
  var MAX_PARA_LEN = 20000;
  var paragraphs = contentEl.querySelectorAll('p');
  if (paragraphs.length === 0) {
    // No <p> tags — markdown might have produced bare text
    paragraphs = [contentEl];
  }

  paragraphs.forEach(function(p) {
    // Skip code blocks
    if (p.closest('pre') || p.closest('code')) return;

    var html = p.innerHTML;
    if (!html.includes('{')) return; // Fast path: no braces at all
    if (html.length > MAX_PARA_LEN) return; // Bail on very long content

    var candidates = _findJsonCandidates(html);
    if (candidates.length === 0) return;

    // Apply replacements in reverse order so earlier-index positions stay valid.
    var out = html;
    for (var i = candidates.length - 1; i >= 0; i--) {
      var c = candidates[i];
      var card = buildDataCard(c.obj);
      out = out.substring(0, c.start) + card + out.substring(c.end);
    }
    p.innerHTML = out;
  });
}

/**
 * Scan `html` once and return `{start, end, obj}` spans for every balanced
 * `{...}` that parses as a JSON object (not array, not primitive). Positions
 * inside `<code>…</code>` or `<pre>…</pre>` blocks are skipped.
 *
 * Linear in `html` length for typical input; bounded by MAX_SCAN and
 * MAX_CANDIDATES for adversarial input.
 * @private
 */
function _findJsonCandidates(html) {
  var MAX_SCAN = 5000;
  var MAX_CANDIDATES = 32;
  var results = [];
  var n = html.length;
  var i = 0;
  var lowerHtml = html.toLowerCase();

  while (i < n && results.length < MAX_CANDIDATES) {
    var ch = html.charCodeAt(i);

    // Fast-skip past <code>...</code> and <pre>...</pre> regions — avoids
    // counting braces that belong to rendered code samples.
    if (ch === 60 /* < */) {
      if (lowerHtml.substr(i, 5) === '<code') {
        var codeEnd = lowerHtml.indexOf('</code>', i + 5);
        i = codeEnd === -1 ? n : codeEnd + 7;
        continue;
      }
      if (lowerHtml.substr(i, 4) === '<pre') {
        var preEnd = lowerHtml.indexOf('</pre>', i + 4);
        i = preEnd === -1 ? n : preEnd + 6;
        continue;
      }
    }

    if (ch !== 123 /* { */) {
      i++;
      continue;
    }

    // Scan forward with brace counting; respect string literals so that
    // `"a}b"` inside an object doesn't prematurely end the scan.
    var end = _findBalancedEnd(html, i, MAX_SCAN);
    if (end === -1) {
      i++;
      continue;
    }

    var raw = html.substring(i, end);
    // Normalize Python-style single quotes to double quotes so input like
    // `{'k': 'v'}` parses as JSON. The naive `raw.replace(/'/g, '"')`
    // mangled apostrophes inside already-double-quoted string values
    // (e.g., `{"name": "it's"}` → `{"name": "it"s"}` → parse failure).
    // Walk the candidate with the same string-state tracking as
    // `_findBalancedEnd` and only rewrite single quotes that appear OUTSIDE
    // a double-quoted string. This preserves apostrophes inside `"it's"`
    // while still upgrading single-quoted JSON-like input.
    var normalized = _normalizeJsonQuotes(raw);
    try {
      var obj = JSON.parse(normalized);
      if (typeof obj === 'object' && obj !== null && !Array.isArray(obj)) {
        results.push({ start: i, end: end, obj: obj });
        i = end;
        continue;
      }
    } catch (_e) { /* not valid JSON — leave as text */ }

    i++;
  }

  return results;
}

/**
 * Rewrite single quotes that act as string delimiters to double quotes,
 * leaving single quotes that appear inside an already-double-quoted string
 * untouched. Mirrors the string-state tracking in `_findBalancedEnd` so the
 * upgrade is consistent with how the candidate was extracted.
 *
 * `{'k': 'v'}` → `{"k": "v"}`
 * `{"name": "it's"}` → `{"name": "it's"}` (apostrophe preserved)
 * `{'msg': "she said \"hi\""}` → `{"msg": "she said \"hi\""}`
 * @private
 */
function _normalizeJsonQuotes(raw) {
  var out = '';
  var inString = null; // '"' | "'" | null
  for (var k = 0; k < raw.length; k++) {
    var c = raw[k];
    if (inString) {
      // Inside a string literal — copy verbatim, including any single
      // quotes that happen to be apostrophes. Honor backslash escapes so
      // `"\""` doesn't terminate the literal early.
      if (c === '\\' && k + 1 < raw.length) {
        out += c + raw[k + 1];
        k++;
        continue;
      }
      if (c === inString) {
        // Closing quote: emit as `"` regardless of which quote opened the
        // string, so a single-quoted literal becomes a double-quoted one.
        out += '"';
        inString = null;
        continue;
      }
      out += c;
      continue;
    }
    if (c === '"' || c === "'") {
      // Opening quote: normalize to `"` and remember which character
      // closes this literal so apostrophes inside `"it's"` are preserved.
      inString = c;
      out += '"';
      continue;
    }
    out += c;
  }
  return out;
}

/**
 * Return the index one past the matching `}` for the `{` at `start`, or -1
 * if no balanced close is found within `maxLen` characters. Respects single
 * and double-quoted string literals (with backslash escapes) so `"a}b"`
 * doesn't terminate the scan.
 * @private
 */
function _findBalancedEnd(html, start, maxLen) {
  var depth = 0;
  var inString = null; // '"' | "'" | null
  var n = Math.min(html.length, start + maxLen);
  for (var j = start; j < n; j++) {
    var ch = html[j];
    if (inString) {
      if (ch === '\\') { j++; continue; } // skip escaped char
      if (ch === inString) inString = null;
      continue;
    }
    if (ch === '"' || ch === "'") { inString = ch; continue; }
    if (ch === '{') { depth++; continue; }
    if (ch === '}') {
      depth--;
      if (depth === 0) return j + 1;
    }
  }
  return -1;
}

/**
 * Build an HTML data card from a plain object.
 */
function buildDataCard(obj) {
  var keys = Object.keys(obj);
  if (keys.length === 0) return '';

  var rows = '';
  for (var i = 0; i < keys.length; i++) {
    var key = keys[i];
    var value = obj[key];
    var displayKey = key.replace(/_/g, ' ');
    var valueClass = 'data-card-value';
    var valueHtml;

    // Special rendering for known value types
    if (key === 'status' || key === 'state') {
      var badgeClass = 'status-badge';
      var sv = String(value).toLowerCase();
      if (sv === 'created' || sv === 'active' || sv === 'success' || sv === 'completed' || sv === 'ok' || sv === 'running') {
        badgeClass += ' status-success';
      } else if (sv === 'failed' || sv === 'error' || sv === 'cancelled' || sv === 'rejected') {
        badgeClass += ' status-error';
      } else if (sv === 'pending' || sv === 'waiting' || sv === 'queued') {
        badgeClass += ' status-pending';
      }
      valueHtml = '<span class="' + badgeClass + '">' + escapeHtml(String(value)) + '</span>';
    } else if (typeof value === 'object' && value !== null) {
      valueHtml = '<code>' + escapeHtml(JSON.stringify(value)) + '</code>';
    } else {
      // Check if value looks like a UUID or ID
      var strVal = String(value);
      if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(strVal)) {
        valueHtml = '<code class="data-card-id">' + escapeHtml(strVal) + '</code>';
      } else {
        valueHtml = '<span>' + escapeHtml(strVal) + '</span>';
      }
    }

    rows += '<div class="data-card-row">' +
      '<span class="data-card-label">' + escapeHtml(displayKey) + '</span>' +
      '<span class="' + valueClass + '">' + valueHtml + '</span>' +
      '</div>';
  }

  return '<div class="data-card">' + rows + '</div>';
}

function copyCodeBlock(btn) {
  const pre = btn.parentElement;
  const code = pre.querySelector('code');
  const text = code ? code.textContent : pre.textContent;
  navigator.clipboard.writeText(text).then(() => {
    btn.textContent = I18n.t('btn.copied');
    setTimeout(() => { btn.textContent = I18n.t('btn.copy'); }, 1500);
  });
}

function copyMessage(btn) {
  const message = btn.closest('.message');
  if (!message) return;
  const text = message.getAttribute('data-copy-text')
    || message.getAttribute('data-raw')
    || message.textContent
    || '';
  navigator.clipboard.writeText(text).then(() => {
    btn.textContent = I18n.t('message.copied');
    setTimeout(() => { btn.textContent = I18n.t('message.copy'); }, 1200);
  }).catch(() => {
    btn.textContent = I18n.t('common.copyFailed');
    setTimeout(() => { btn.textContent = I18n.t('message.copy'); }, 1200);
  });
}

let _lastMessageDate = null;

function maybeInsertTimeSeparator(container, timestamp) {
  const date = timestamp ? new Date(timestamp) : new Date();
  const dateStr = date.toDateString();
  if (_lastMessageDate === dateStr) return;
  _lastMessageDate = dateStr;

  const now = new Date();
  const today = now.toDateString();
  const yesterday = new Date(now.getTime() - 86400000).toDateString();

  let label;
  if (dateStr === today) label = 'Today';
  else if (dateStr === yesterday) label = 'Yesterday';
  else label = date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });

  const sep = document.createElement('div');
  sep.className = 'time-separator';
  sep.textContent = label;
  container.appendChild(sep);
}

// Remove oldest messages/activity groups from the DOM when the chat container
// exceeds MAX_DOM_MESSAGES elements. Users can scroll up to trigger
// loadHistory() for older content. This prevents unbounded DOM growth during
// long sessions. Elements with data-streaming="true" are preserved to avoid
// breaking mid-stream responses.
// Note: if every element has data-streaming="true", this function will
// under-prune and the DOM may temporarily exceed the cap. This is acceptable
// because streaming completes quickly and the next call will clean up.
function pruneOldMessages() {
  const container = document.getElementById('chat-messages');
  const items = container.querySelectorAll('.message, .activity-group, .time-separator');
  if (items.length <= MAX_DOM_MESSAGES) return;
  let removed = 0;
  const target = items.length - MAX_DOM_MESSAGES;
  for (let i = 0; i < items.length && removed < target; i++) {
    if (items[i].getAttribute('data-streaming') === 'true') continue;
    items[i].remove();
    removed++;
  }
  // Clean up orphaned leading time-separators left after pruning.
  // A separator is orphaned if no .message or .activity-group follows it
  // before the next separator (or end of container).
  const remaining = container.querySelectorAll('.message, .activity-group, .time-separator');
  for (let i = 0; i < remaining.length; i++) {
    if (!remaining[i].classList.contains('time-separator')) break;
    remaining[i].remove();
  }
}

// Append image thumbnails to an existing user message bubble. Used by the
// optimistic display in sendMessage() and by the pending re-inject path in
// loadHistory() so attached images stay visible until DB persistence catches
// up. Reuses the .image-preview class for thumbnail styling.
function appendImagesToMessage(messageDiv, dataUrls) {
  if (!messageDiv || !dataUrls || dataUrls.length === 0) return;
  const wrap = document.createElement('div');
  wrap.className = 'message-images';
  for (const dataUrl of dataUrls) {
    const img = document.createElement('img');
    img.className = 'image-preview';
    img.src = dataUrl;
    img.alt = 'Attached image';
    wrap.appendChild(img);
  }
  messageDiv.appendChild(wrap);
}

function addMessage(role, content, options) {
  const container = document.getElementById('chat-messages');
  maybeInsertTimeSeparator(container);
  const div = createMessageElement(role, content, options);
  container.appendChild(div);
  container.scrollTop = container.scrollHeight;
  return div;
}

function appendToLastAssistant(chunk) {
  const container = document.getElementById('chat-messages');
  const messages = container.querySelectorAll('.message.assistant');
  if (messages.length > 0) {
    const last = messages[messages.length - 1];
    const raw = (last.getAttribute('data-raw') || '') + chunk;
    last.setAttribute('data-raw', raw);
    last.setAttribute('data-copy-text', raw);
    const content = last.querySelector('.message-content');
    if (content) {
      content.innerHTML = renderMarkdown(raw);
      // Syntax highlighting for code blocks
      if (typeof hljs !== 'undefined') {
        requestAnimationFrame(() => {
          content.querySelectorAll('pre code').forEach(block => {
            hljs.highlightElement(block);
          });
        });
      }
    }
    container.scrollTop = container.scrollHeight;
  } else {
    addMessage('assistant', chunk);
  }
}

// --- Inline Tool Activity Cards ---
