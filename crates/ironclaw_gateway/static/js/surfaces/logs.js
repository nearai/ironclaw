const LOG_MAX_ENTRIES = 2000;
let logsPaused = false;
let logBuffer = []; // buffer while paused
let downloadLogEntries = []; // entries available for JSONL download

function connectLogSSE() {
  if (logEventSource) logEventSource.close();

  const logSseUrl = (token && !oidcProxyAuth)
    ? '/api/logs/events?token=' + encodeURIComponent(token)
    : '/api/logs/events';
  logEventSource = new EventSource(logSseUrl);

  logEventSource.addEventListener('log', (e) => {
    const entry = JSON.parse(e.data);
    rememberLogEntryForDownload(entry);
    if (logsPaused) {
      logBuffer.push(entry);
      return;
    }
    prependLogEntry(entry);
  });

  logEventSource.onerror = () => {
    // Silent reconnect
  };
}

function rememberLogEntryForDownload(entry) {
  downloadLogEntries.push(entry);
  while (downloadLogEntries.length > LOG_MAX_ENTRIES) {
    downloadLogEntries.shift();
  }
}

function serializeLogEntriesAsJsonl(entries) {
  return entries.map(entry => JSON.stringify(entry)).join('\n') + (entries.length ? '\n' : '');
}

function logsDownloadFilename() {
  const stamp = new Date()
    .toISOString()
    .replace(/[-:]/g, '')
    .replace(/\.\d{3}Z$/, 'Z')
    .replace('T', '-');
  return 'ironclaw-logs-' + stamp + '.jsonl';
}

function downloadLogsJsonl() {
  const blob = new Blob([serializeLogEntriesAsJsonl(downloadLogEntries)], {
    type: 'application/x-ndjson;charset=utf-8',
  });
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = logsDownloadFilename();
  link.style.display = 'none';
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

function prependLogEntry(entry) {
  const output = document.getElementById('logs-output');

  // Level filter
  const levelFilter = document.getElementById('logs-level-filter').value;
  const targetFilter = document.getElementById('logs-target-filter').value.trim().toLowerCase();

  const div = document.createElement('div');
  div.className = 'log-entry level-' + entry.level;
  div.setAttribute('data-level', entry.level);
  div.setAttribute('data-target', entry.target);
  div.setAttribute('data-msg', entry.message);

  const ts = document.createElement('span');
  ts.className = 'log-ts';
  ts.textContent = entry.timestamp.substring(11, 23);
  div.appendChild(ts);

  const lvl = document.createElement('span');
  lvl.className = 'log-level log-level-' + entry.level.toLowerCase();
  lvl.textContent = entry.level;
  div.appendChild(lvl);

  const tgt = document.createElement('span');
  tgt.className = 'log-target';
  tgt.textContent = entry.target;
  div.appendChild(tgt);

  const msg = document.createElement('span');
  msg.className = 'log-msg';
  msg.textContent = entry.message;
  div.appendChild(msg);

  div.addEventListener('click', () => div.classList.toggle('expanded'));

  // Apply current filters as visibility (search spans target + message).
  const matchesLevel = levelFilter === 'all' || entry.level === levelFilter;
  const matchesQuery = !targetFilter
    || entry.target.toLowerCase().includes(targetFilter)
    || entry.message.toLowerCase().includes(targetFilter);
  if (!matchesLevel || !matchesQuery) {
    div.style.display = 'none';
  }

  output.prepend(div);

  // Cap entries (remove oldest at the bottom)
  while (output.children.length > LOG_MAX_ENTRIES) {
    output.removeChild(output.lastChild);
  }

  syncLogsEmptyState();

  // Auto-scroll to top (newest entries are at the top)
  if (document.getElementById('logs-autoscroll').checked) {
    output.scrollTop = 0;
  }
}

// Toggle the null state depending on whether any row is visible.
function syncLogsEmptyState() {
  const output = document.getElementById('logs-output');
  const empty = document.getElementById('logs-empty');
  if (!output || !empty) return;
  let anyVisible = false;
  for (const el of output.children) {
    if (el.style.display !== 'none') { anyVisible = true; break; }
  }
  empty.style.display = anyVisible ? 'none' : '';
  output.style.display = anyVisible ? '' : 'none';
}

function toggleLogsPause() {
  logsPaused = !logsPaused;
  const btn = document.getElementById('logs-pause-btn');
  btn.textContent = logsPaused ? I18n.t('logs.resume') : I18n.t('logs.pause');

  if (!logsPaused) {
    // Flush buffer: oldest-first + prepend naturally puts newest at top
    for (const entry of logBuffer) {
      prependLogEntry(entry);
    }
    logBuffer = [];
  }
}

function clearLogs() {
  if (!confirm(I18n.t('logs.confirmClear'))) return;
  document.getElementById('logs-output').innerHTML = '';
  logBuffer = [];
  downloadLogEntries = [];
}

// Re-apply filters when level or target changes
document.getElementById('logs-level-filter').addEventListener('change', applyLogFilters);
document.getElementById('logs-target-filter').addEventListener('input', applyLogFilters);

function applyLogFilters() {
  const levelFilter = document.getElementById('logs-level-filter').value;
  const targetFilter = document.getElementById('logs-target-filter').value.trim().toLowerCase();
  const entries = document.querySelectorAll('#logs-output .log-entry');
  for (const el of entries) {
    const matchesLevel = levelFilter === 'all' || el.getAttribute('data-level') === levelFilter;
    const matchesQuery = !targetFilter
      || el.getAttribute('data-target').toLowerCase().includes(targetFilter)
      || (el.getAttribute('data-msg') || '').toLowerCase().includes(targetFilter);
    el.style.display = (matchesLevel && matchesQuery) ? '' : 'none';
  }
  syncLogsEmptyState();
}

// --- Server-side log level control ---

function setServerLogLevel(level) {
  apiFetch('/api/logs/level', {
    method: 'PUT',
    body: { level },
  })
    .then(data => {
      document.getElementById('logs-server-level').value = data.level;
    })
    .catch(err => console.error('Failed to set server log level:', err));
}

function loadServerLogLevel() {
  apiFetch('/api/logs/level')
    .then(data => {
      document.getElementById('logs-server-level').value = data.level;
    })
    .catch(() => {}); // ignore if not available
}

// --- Extensions ---

var kindLabels = { 'wasm_channel': 'Channel', 'wasm_tool': 'Tool', 'mcp_server': 'MCP' };

