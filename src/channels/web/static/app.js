// IronClaw Web Gateway - Client

// --- Theme Management (dark / light / system) ---
// Icon switching is handled by pure CSS via data-theme-mode on <html>.

function getSystemTheme() {
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

const VALID_THEME_MODES = { dark: true, light: true, system: true };

function getThemeMode() {
  const stored = localStorage.getItem('ironclaw-theme');
  return (stored && VALID_THEME_MODES[stored]) ? stored : 'system';
}

function resolveTheme(mode) {
  return mode === 'system' ? getSystemTheme() : mode;
}

function applyTheme(mode) {
  const resolved = resolveTheme(mode);
  document.documentElement.setAttribute('data-theme', resolved);
  document.documentElement.setAttribute('data-theme-mode', mode);
  const titleKeys = { dark: 'theme.tooltipDark', light: 'theme.tooltipLight', system: 'theme.tooltipSystem' };
  const btn = document.getElementById('theme-toggle');
  if (btn) btn.title = (typeof I18n !== 'undefined' && titleKeys[mode]) ? I18n.t(titleKeys[mode]) : ('Theme: ' + mode);
  const announce = document.getElementById('theme-announce');
  if (announce) announce.textContent = (typeof I18n !== 'undefined') ? I18n.t('theme.announce', { mode: mode }) : ('Theme: ' + mode);
}

function toggleTheme() {
  const cycle = { dark: 'light', light: 'system', system: 'dark' };
  const current = getThemeMode();
  const next = cycle[current] || 'dark';
  localStorage.setItem('ironclaw-theme', next);
  applyTheme(next);
}

// Apply theme immediately (FOUC prevention is done via inline script in <head>,
// but we call again here to ensure tooltip is set after DOM is ready).
applyTheme(getThemeMode());

// Delay enabling theme transition to avoid flash on initial load.
requestAnimationFrame(function() {
  requestAnimationFrame(function() {
    document.body.classList.add('theme-transition');
  });
});

// Listen for OS theme changes — only re-apply when in 'system' mode.
const mql = window.matchMedia('(prefers-color-scheme: light)');
const onSchemeChange = function() {
  if (getThemeMode() === 'system') {
    applyTheme('system');
  }
};
if (mql.addEventListener) {
  mql.addEventListener('change', onSchemeChange);
} else if (mql.addListener) {
  mql.addListener(onSchemeChange);
}

// Bind theme toggle button (CSP-compliant — no inline onclick).
document.getElementById('theme-toggle').addEventListener('click', toggleTheme);

let token = '';
let eventSource = null;
let logEventSource = null;
let currentTab = 'chat';
let currentThreadId = null;
let currentThreadIsReadOnly = false;
let assistantThreadId = null;
let hasMore = false;
let oldestTimestamp = null;
let loadingOlder = false;
let sseHasConnectedBefore = false;
let jobEvents = new Map(); // job_id -> Array of events
let jobListRefreshTimer = null;
let pairingPollInterval = null;
let unreadThreads = new Map(); // thread_id -> unread count
let _loadThreadsTimer = null;
const JOB_EVENTS_CAP = 500;
const MEMORY_SEARCH_QUERY_MAX_LENGTH = 100;
let stagedImages = [];
let authFlowPending = false;
let _ghostSuggestion = '';
let currentSettingsSubtab = 'inference';

// --- Slash Commands ---

const SLASH_COMMANDS = [
  { cmd: '/status',     desc: 'Show all jobs, or /status <id> for one job' },
  { cmd: '/list',       desc: 'List all jobs' },
  { cmd: '/cancel',     desc: '/cancel <job-id> — cancel a running job' },
  { cmd: '/undo',       desc: 'Revert the last turn' },
  { cmd: '/redo',       desc: 'Re-apply an undone turn' },
  { cmd: '/compact',    desc: 'Compress the context window' },
  { cmd: '/clear',      desc: 'Clear thread and start fresh' },
  { cmd: '/interrupt',  desc: 'Stop the current turn' },
  { cmd: '/heartbeat',  desc: 'Trigger manual heartbeat check' },
  { cmd: '/summarize',  desc: 'Summarize the current thread' },
  { cmd: '/suggest',    desc: 'Suggest next steps' },
  { cmd: '/help',       desc: 'Show help' },
  { cmd: '/version',    desc: 'Show version info' },
  { cmd: '/tools',      desc: 'List available tools' },
  { cmd: '/skills',     desc: 'List installed skills' },
  { cmd: '/model',      desc: 'Show or switch the LLM model' },
  { cmd: '/thread new', desc: 'Create a new conversation thread' },
];

let _slashSelected = -1;
let _slashMatches = [];

// --- Tool Activity State ---
let _activeGroup = null;
let _activeToolCards = {};
let _activityThinking = null;

// --- Auth ---

function authenticate() {
  token = document.getElementById('token-input').value.trim();
  if (!token) {
    document.getElementById('auth-error').textContent = I18n.t('auth.errorRequired');
    return;
  }

  // Test the token against the health-ish endpoint (chat/threads requires auth)
  apiFetch('/api/chat/threads')
    .then(() => {
      sessionStorage.setItem('ironclaw_token', token);
      document.getElementById('auth-screen').style.display = 'none';
      document.getElementById('app').style.display = 'flex';
      // Strip token and log_level from URL so they're not visible in the address bar
      const cleaned = new URL(window.location);
      const urlLogLevel = cleaned.searchParams.get('log_level');
      cleaned.searchParams.delete('token');
      cleaned.searchParams.delete('log_level');
      window.history.replaceState({}, '', cleaned.pathname + cleaned.search);
      connectSSE();
      connectLogSSE();
      startGatewayStatusPolling();
      checkTeeStatus();
      loadThreads();
      loadMemoryTree();
      loadJobs();
      // Apply URL log_level param if present, otherwise just sync the dropdown
      if (urlLogLevel) {
        setServerLogLevel(urlLogLevel);
      } else {
        loadServerLogLevel();
      }
    })
    .catch(() => {
      sessionStorage.removeItem('ironclaw_token');
      document.getElementById('auth-screen').style.display = '';
      document.getElementById('app').style.display = 'none';
      document.getElementById('auth-error').textContent = I18n.t('auth.errorInvalid');
    });
}

document.getElementById('token-input').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') authenticate();
});

// --- Static element event bindings (CSP-compliant, no inline handlers) ---
document.getElementById('auth-connect-btn').addEventListener('click', () => authenticate());
document.getElementById('restart-overlay').addEventListener('click', () => cancelRestart());
document.getElementById('restart-close-btn').addEventListener('click', () => cancelRestart());
document.getElementById('restart-cancel-btn').addEventListener('click', () => cancelRestart());
document.getElementById('restart-confirm-btn').addEventListener('click', () => confirmRestart());
document.getElementById('language-btn').addEventListener('click', () => toggleLanguageMenu());
// Language option clicks handled by delegated data-action="switch-language" handler.
document.getElementById('restart-btn').addEventListener('click', () => triggerRestart());
document.getElementById('thread-new-btn').addEventListener('click', () => createNewThread());
document.getElementById('thread-toggle-btn').addEventListener('click', () => toggleThreadSidebar());
document.getElementById('assistant-thread').addEventListener('click', () => switchToAssistant());
document.getElementById('send-btn').addEventListener('click', () => sendMessage());
document.getElementById('memory-edit-btn').addEventListener('click', () => startMemoryEdit());
document.getElementById('memory-save-btn').addEventListener('click', () => saveMemoryEdit());
document.getElementById('memory-cancel-btn').addEventListener('click', () => cancelMemoryEdit());
document.getElementById('logs-server-level').addEventListener('change', function() { setServerLogLevel(this.value); });
document.getElementById('logs-pause-btn').addEventListener('click', () => toggleLogsPause());
document.getElementById('logs-clear-btn').addEventListener('click', () => clearLogs());
document.getElementById('wasm-install-btn').addEventListener('click', () => installWasmExtension());
document.getElementById('mcp-add-btn').addEventListener('click', () => addMcpServer());
document.getElementById('skill-search-btn').addEventListener('click', () => searchClawHub());
document.getElementById('skill-install-btn').addEventListener('click', () => installSkillFromForm());

// Auto-authenticate from URL param or saved session
(function autoAuth() {
  const params = new URLSearchParams(window.location.search);
  const urlToken = params.get('token');
  if (urlToken) {
    document.getElementById('token-input').value = urlToken;
    authenticate();
    return;
  }
  const saved = sessionStorage.getItem('ironclaw_token');
  if (saved) {
    document.getElementById('token-input').value = saved;
    // Hide auth screen immediately to prevent flash, authenticate() will
    // restore it if the token turns out to be invalid.
    document.getElementById('auth-screen').style.display = 'none';
    document.getElementById('app').style.display = 'flex';
    authenticate();
  }
})();

// --- API helper ---

function apiFetch(path, options) {
  const opts = options || {};
  opts.headers = opts.headers || {};
  opts.headers['Authorization'] = 'Bearer ' + token;
  if (opts.body && typeof opts.body === 'object') {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(opts.body);
  }
  return fetch(path, opts).then((res) => {
    if (!res.ok) {
      return res.text().then(function(body) {
        throw new Error(body || (res.status + ' ' + res.statusText));
      });
    }
    if (res.status === 204) return null;
    return res.json();
  });
}

// --- Restart Feature ---

let isRestarting = false; // Track if we're currently restarting
let restartEnabled = false; // Track if restart is available in this deployment

function triggerRestart() {
  if (!currentThreadId) {
    alert(I18n.t('error.startConversation'));
    return;
  }

  // Show the confirmation modal
  const confirmModal = document.getElementById('restart-confirm-modal');
  confirmModal.style.display = 'flex';
}

function confirmRestart() {
  if (!currentThreadId) {
    alert(I18n.t('error.startConversation'));
    return;
  }

  // Hide confirmation modal
  const confirmModal = document.getElementById('restart-confirm-modal');
  confirmModal.style.display = 'none';

  const restartBtn = document.getElementById('restart-btn');
  const restartIcon = document.getElementById('restart-icon');

  // Mark as restarting
  isRestarting = true;
  restartBtn.disabled = true;
  if (restartIcon) restartIcon.classList.add('spinning');

  // Show progress modal
  const loaderEl = document.getElementById('restart-loader');
  loaderEl.style.display = 'flex';

  // Send restart command via chat
  console.log('[confirmRestart] Sending /restart command to server');
  apiFetch('/api/chat/send', {
    method: 'POST',
    body: {
      content: '/restart',
      thread_id: currentThreadId,
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    },
  })
    .then((response) => {
      console.log('[confirmRestart] API call succeeded, response:', response);
    })
    .catch((err) => {
      console.error('[confirmRestart] Restart request failed:', err);
      addMessage('system', I18n.t('error.restartFailed', { message: err.message }));
      isRestarting = false;
      restartBtn.disabled = false;
      if (restartIcon) restartIcon.classList.remove('spinning');
      loaderEl.style.display = 'none';
    });
}

function cancelRestart() {
  const confirmModal = document.getElementById('restart-confirm-modal');
  confirmModal.style.display = 'none';
}

function tryShowRestartModal() {
  // Defensive callback for when restart is detected in messages.
  if (!isRestarting) {
    isRestarting = true;
    const restartBtn = document.getElementById('restart-btn');
    const restartIcon = document.getElementById('restart-icon');
    restartBtn.disabled = true;
    if (restartIcon) restartIcon.classList.add('spinning');

    // Show progress modal
    const loaderEl = document.getElementById('restart-loader');
    loaderEl.style.display = 'flex';
  }
}

function updateRestartButtonVisibility() {
  const restartBtn = document.getElementById('restart-btn');
  if (restartBtn) {
    restartBtn.style.display = restartEnabled ? 'block' : 'none';
  }
}

// --- SSE ---

function connectSSE() {
  if (eventSource) eventSource.close();

  eventSource = new EventSource('/api/chat/events?token=' + encodeURIComponent(token));

  eventSource.onopen = () => {
    document.getElementById('sse-dot').classList.remove('disconnected');
    document.getElementById('sse-status').textContent = I18n.t('status.connected');

    // If we were restarting, close the modal and reset button now that server is back
    if (isRestarting) {
      const loaderEl = document.getElementById('restart-loader');
      if (loaderEl) loaderEl.style.display = 'none';
      const restartBtn = document.getElementById('restart-btn');
      const restartIcon = document.getElementById('restart-icon');
      if (restartBtn) restartBtn.disabled = false;
      if (restartIcon) restartIcon.classList.remove('spinning');
      isRestarting = false;
    }

    if (sseHasConnectedBefore && currentThreadId) {
      finalizeActivityGroup();
      loadHistory();
    }
    sseHasConnectedBefore = true;
  };

  eventSource.onerror = () => {
    document.getElementById('sse-dot').classList.add('disconnected');
    document.getElementById('sse-status').textContent = I18n.t('status.reconnecting');
  };

  eventSource.addEventListener('response', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) {
      if (data.thread_id) {
        unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
        debouncedLoadThreads();
      }
      return;
    }
    finalizeActivityGroup();
    addMessage('assistant', data.content);
    enableChatInput();
    // Refresh thread list so new titles appear after first message
    loadThreads();

    // Show restart modal if the response indicates restart was initiated
    if (data.content && data.content.toLowerCase().includes('restart initiated')) {
      setTimeout(() => tryShowRestartModal(), 500);
    }
  });

  eventSource.addEventListener('thinking', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) {
      if (data.thread_id) debouncedLoadThreads();
      return;
    }
    clearSuggestionChips();
    showActivityThinking(data.message);
  });

  eventSource.addEventListener('suggestions', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    if (data.suggestions && data.suggestions.length > 0) {
      showSuggestionChips(data.suggestions);
    }
  });

  eventSource.addEventListener('tool_started', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    addToolCard(data.name);
  });

  eventSource.addEventListener('tool_completed', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    completeToolCard(data.name, data.success, data.error, data.parameters);

    // Show restart modal only when the restart tool succeeds
    if (data.name.toLowerCase() === 'restart' && data.success) {
      setTimeout(() => tryShowRestartModal(), 500);
    }
  });

  eventSource.addEventListener('tool_result', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    setToolCardOutput(data.name, data.preview);
  });

  eventSource.addEventListener('stream_chunk', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    finalizeActivityGroup();
    appendToLastAssistant(data.content);
  });

  eventSource.addEventListener('status', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) {
      if (data.thread_id) debouncedLoadThreads();
      return;
    }
    // "Done" and "Awaiting approval" are terminal signals from the agent:
    // the agentic loop finished, so re-enable input as a safety net in case
    // the response SSE event is empty or lost.
    // Status text is not displayed — inline activity cards handle visual feedback.
    if (data.message === 'Done' || data.message === 'Awaiting approval') {
      finalizeActivityGroup();
      enableChatInput();
    }
  });

  eventSource.addEventListener('job_started', (e) => {
    const data = JSON.parse(e.data);
    showJobCard(data);
  });

  eventSource.addEventListener('approval_needed', (e) => {
    const data = JSON.parse(e.data);
    const hasThread = !!data.thread_id;
    const forCurrentThread = !hasThread || isCurrentThread(data.thread_id);

    if (forCurrentThread) {
      showApproval(data);
    } else {
      // Keep thread list fresh when approval is requested in a background thread.
      unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
      debouncedLoadThreads();
    }

    // Extension setup flows can surface approvals from any settings subtab.
    if (currentTab === 'settings') refreshCurrentSettingsTab();
  });

  eventSource.addEventListener('auth_required', (e) => {
    handleAuthRequired(JSON.parse(e.data));
  });

  eventSource.addEventListener('auth_completed', (e) => {
    const data = JSON.parse(e.data);
    handleAuthCompleted(data);
  });

  eventSource.addEventListener('extension_status', (e) => {
    if (currentTab === 'settings') refreshCurrentSettingsTab();
  });

  eventSource.addEventListener('image_generated', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    addGeneratedImage(data.data_url, data.path);
  });

  eventSource.addEventListener('error', (e) => {
    if (e.data) {
      const data = JSON.parse(e.data);
      if (!isCurrentThread(data.thread_id)) return;
      finalizeActivityGroup();
      addMessage('system', 'Error: ' + data.message);
      enableChatInput();
    }
  });

  // Job event listeners (activity stream for all sandbox jobs)
  const jobEventTypes = [
    'job_message', 'job_tool_use', 'job_tool_result',
    'job_status', 'job_result'
  ];
  for (const evtType of jobEventTypes) {
    eventSource.addEventListener(evtType, (e) => {
      const data = JSON.parse(e.data);
      const jobId = data.job_id;
      if (!jobId) return;
      if (!jobEvents.has(jobId)) jobEvents.set(jobId, []);
      const events = jobEvents.get(jobId);
      events.push({ type: evtType, data: data, ts: Date.now() });
      // Cap per-job events to prevent memory leak
      while (events.length > JOB_EVENTS_CAP) events.shift();
      // If the Activity tab is currently visible for this job, refresh it
      refreshActivityTab(jobId);
      // Auto-refresh job list when on jobs tab (debounced)
      if ((evtType === 'job_result' || evtType === 'job_status') && currentTab === 'jobs' && !currentJobId) {
        clearTimeout(jobListRefreshTimer);
        jobListRefreshTimer = setTimeout(loadJobs, 200);
      }
      // Clean up finished job events after a viewing window
      if (evtType === 'job_result') {
        setTimeout(() => jobEvents.delete(jobId), 60000);
      }
    });
  }
}

// Check if an SSE event belongs to the currently viewed thread.
// Events without a thread_id are dropped (prevents notification leaking).
function isCurrentThread(threadId) {
  if (!threadId) return false;
  if (!currentThreadId) return true;
  return threadId === currentThreadId;
}

// --- Suggestion Chips ---

function showSuggestionChips(suggestions) {
  // Clear previous chips/ghost without restoring placeholder (we'll set it below)
  _ghostSuggestion = '';
  const container = document.getElementById('suggestion-chips');
  container.innerHTML = '';
  const ghost = document.getElementById('ghost-text');
  ghost.style.display = 'none';
  const wrapper = document.querySelector('.chat-input-wrapper');
  if (wrapper) wrapper.classList.remove('has-ghost');

  _ghostSuggestion = suggestions[0] || '';
  const input = document.getElementById('chat-input');
  suggestions.forEach(text => {
    const chip = document.createElement('button');
    chip.className = 'suggestion-chip';
    chip.textContent = text;
    chip.addEventListener('click', () => {
      input.value = text;
      clearSuggestionChips();
      autoResizeTextarea(input);
      input.focus();
      sendMessage();
    });
    container.appendChild(chip);
  });
  container.style.display = 'flex';
  // Show first suggestion as ghost text in the input so user knows Tab works
  if (_ghostSuggestion && input.value === '') {
    ghost.textContent = _ghostSuggestion;
    ghost.style.display = 'block';
    input.closest('.chat-input-wrapper').classList.add('has-ghost');
  }
}

function clearSuggestionChips() {
  _ghostSuggestion = '';
  const container = document.getElementById('suggestion-chips');
  if (container) {
    container.innerHTML = '';
    container.style.display = 'none';
  }
  const ghost = document.getElementById('ghost-text');
  if (ghost) ghost.style.display = 'none';
  const wrapper = document.querySelector('.chat-input-wrapper');
  if (wrapper) wrapper.classList.remove('has-ghost');
}

// --- Chat ---

function sendMessage() {
  clearSuggestionChips();
  const input = document.getElementById('chat-input');
  if (authFlowPending) {
    showToast('Complete the auth step before sending chat messages.', 'info');
    const tokenField = document.querySelector('.auth-card .auth-token-input input');
    if (tokenField) tokenField.focus();
    return;
  }
  if (!currentThreadId) {
    console.warn('sendMessage: no thread selected, ignoring');
    return;
  }
  const content = input.value.trim();
  if (!content && stagedImages.length === 0) return;

  addMessage('user', content || '(images attached)');
  input.value = '';
  autoResizeTextarea(input);
  input.focus();

  const body = { content, thread_id: currentThreadId || undefined, timezone: Intl.DateTimeFormat().resolvedOptions().timeZone };
  if (stagedImages.length > 0) {
    body.images = stagedImages.map(img => ({ media_type: img.media_type, data: img.data }));
    stagedImages = [];
    renderImagePreviews();
  }

  apiFetch('/api/chat/send', {
    method: 'POST',
    body: body,
  }).catch((err) => {
    addMessage('system', 'Failed to send: ' + err.message);
  });
}

function enableChatInput() {
  if (currentThreadIsReadOnly || authFlowPending) return;
  const input = document.getElementById('chat-input');
  const btn = document.getElementById('send-btn');
  if (input) {
    input.disabled = false;
  }
  if (btn) btn.disabled = false;
}

// --- Image Upload ---

function renderImagePreviews() {
  const strip = document.getElementById('image-preview-strip');
  strip.innerHTML = '';
  stagedImages.forEach((img, idx) => {
    const container = document.createElement('div');
    container.className = 'image-preview-container';

    const preview = document.createElement('img');
    preview.className = 'image-preview';
    preview.src = img.dataUrl;
    preview.alt = 'Attached image';

    const removeBtn = document.createElement('button');
    removeBtn.className = 'image-preview-remove';
    removeBtn.textContent = '\u00d7';
    removeBtn.addEventListener('click', () => {
      stagedImages.splice(idx, 1);
      renderImagePreviews();
    });

    container.appendChild(preview);
    container.appendChild(removeBtn);
    strip.appendChild(container);
  });
}

const MAX_IMAGE_SIZE_BYTES = 5 * 1024 * 1024; // 5 MB per image
const MAX_STAGED_IMAGES = 5;

function handleImageFiles(files) {
  Array.from(files).forEach(file => {
    if (!file.type.startsWith('image/')) return;
    if (file.size > MAX_IMAGE_SIZE_BYTES) {
      alert(`Image "${file.name}" exceeds 5 MB limit (${(file.size / 1024 / 1024).toFixed(1)} MB)`);
      return;
    }
    if (stagedImages.length >= MAX_STAGED_IMAGES) {
      alert(`Maximum ${MAX_STAGED_IMAGES} images allowed per message`);
      return;
    }
    const reader = new FileReader();
    reader.onload = function(e) {
      const dataUrl = e.target.result;
      const commaIdx = dataUrl.indexOf(',');
      const meta = dataUrl.substring(0, commaIdx); // e.g. "data:image/png;base64"
      const base64 = dataUrl.substring(commaIdx + 1);
      const mediaType = meta.replace('data:', '').replace(';base64', '');
      stagedImages.push({ media_type: mediaType, data: base64, dataUrl: dataUrl });
      renderImagePreviews();
    };
    reader.readAsDataURL(file);
  });
}

document.getElementById('attach-btn').addEventListener('click', () => {
  document.getElementById('image-file-input').click();
});

document.getElementById('image-file-input').addEventListener('change', (e) => {
  handleImageFiles(e.target.files);
  e.target.value = '';
});

document.getElementById('chat-input').addEventListener('paste', (e) => {
  const items = (e.clipboardData || e.originalEvent.clipboardData).items;
  for (let i = 0; i < items.length; i++) {
    if (items[i].kind === 'file' && items[i].type.startsWith('image/')) {
      const file = items[i].getAsFile();
      if (file) handleImageFiles([file]);
    }
  }
});

const chatMessagesEl = document.getElementById('chat-messages');
chatMessagesEl.addEventListener('copy', (e) => {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed) return;
  const anchorNode = selection.anchorNode;
  const focusNode = selection.focusNode;
  if (!anchorNode || !focusNode) return;
  if (!chatMessagesEl.contains(anchorNode) || !chatMessagesEl.contains(focusNode)) return;
  const text = selection.toString();
  if (!text || !e.clipboardData) return;
  // Force plain-text clipboard output so dark-theme styling never leaks on paste.
  e.preventDefault();
  e.clipboardData.clearData();
  e.clipboardData.setData('text/plain', text);
});

function addGeneratedImage(dataUrl, path) {
  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'generated-image-card';

  const img = document.createElement('img');
  img.className = 'generated-image';
  img.src = dataUrl;
  img.alt = 'Generated image';

  card.appendChild(img);

  if (path) {
    const pathLabel = document.createElement('div');
    pathLabel.className = 'generated-image-path';
    pathLabel.textContent = path;
    card.appendChild(pathLabel);
  }

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
}

// --- Slash Autocomplete ---

function showSlashAutocomplete(matches) {
  const el = document.getElementById('slash-autocomplete');
  if (!el || matches.length === 0) { hideSlashAutocomplete(); return; }
  _slashMatches = matches;
  _slashSelected = -1;
  el.innerHTML = '';
  matches.forEach((item, i) => {
    const row = document.createElement('div');
    row.className = 'slash-ac-item';
    row.dataset.index = i;
    var cmdSpan = document.createElement('span');
    cmdSpan.className = 'slash-ac-cmd';
    cmdSpan.textContent = item.cmd;
    var descSpan = document.createElement('span');
    descSpan.className = 'slash-ac-desc';
    descSpan.textContent = item.desc;
    row.appendChild(cmdSpan);
    row.appendChild(descSpan);
    row.addEventListener('mousedown', (e) => {
      e.preventDefault(); // prevent blur
      selectSlashItem(item.cmd);
    });
    el.appendChild(row);
  });
  el.style.display = 'block';
}

function hideSlashAutocomplete() {
  const el = document.getElementById('slash-autocomplete');
  if (el) el.style.display = 'none';
  _slashSelected = -1;
  _slashMatches = [];
}

function selectSlashItem(cmd) {
  const input = document.getElementById('chat-input');
  input.value = cmd + ' ';
  input.focus();
  hideSlashAutocomplete();
  autoResizeTextarea(input);
}

function updateSlashHighlight() {
  const items = document.querySelectorAll('#slash-autocomplete .slash-ac-item');
  items.forEach((el, i) => el.classList.toggle('selected', i === _slashSelected));
  if (_slashSelected >= 0 && items[_slashSelected]) {
    items[_slashSelected].scrollIntoView({ block: 'nearest' });
  }
}

function filterSlashCommands(value) {
  if (!value.startsWith('/')) { hideSlashAutocomplete(); return; }
  // Only show autocomplete when the input is just a slash command prefix (no spaces except /thread new)
  const lower = value.toLowerCase();
  const matches = SLASH_COMMANDS.filter((c) => c.cmd.startsWith(lower));
  if (matches.length === 0 || (matches.length === 1 && matches[0].cmd === lower.trimEnd())) {
    hideSlashAutocomplete();
  } else {
    showSlashAutocomplete(matches);
  }
}

function sendApprovalAction(requestId, action) {
  apiFetch('/api/chat/approval', {
    method: 'POST',
    body: { request_id: requestId, action: action, thread_id: currentThreadId },
  }).catch((err) => {
    addMessage('system', 'Failed to send approval: ' + err.message);
  });

  // Disable buttons and show confirmation on the card
  const card = document.querySelector('.approval-card[data-request-id="' + requestId + '"]');
  if (card) {
    const buttons = card.querySelectorAll('.approval-actions button');
    buttons.forEach((btn) => {
      btn.disabled = true;
    });
    const actions = card.querySelector('.approval-actions');
    const label = document.createElement('span');
    label.className = 'approval-resolved';
    const labelText = action === 'approve' ? 'Approved' : action === 'always' ? 'Always approved' : 'Denied';
    label.textContent = labelText;
    actions.appendChild(label);
    // Remove the card after showing the confirmation briefly
    setTimeout(() => { card.remove(); }, 1500);
  }
}

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
    btn.textContent = 'Copied';
    setTimeout(() => { btn.textContent = 'Copy'; }, 1200);
  }).catch(() => {
    btn.textContent = 'Failed';
    setTimeout(() => { btn.textContent = 'Copy'; }, 1200);
  });
}

function addMessage(role, content) {
  const container = document.getElementById('chat-messages');
  const div = createMessageElement(role, content);
  container.appendChild(div);
  container.scrollTop = container.scrollHeight;
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
    }
    container.scrollTop = container.scrollHeight;
  } else {
    addMessage('assistant', chunk);
  }
}

// --- Inline Tool Activity Cards ---

function getOrCreateActivityGroup() {
  if (_activeGroup) return _activeGroup;
  const container = document.getElementById('chat-messages');
  const group = document.createElement('div');
  group.className = 'activity-group';
  container.appendChild(group);
  container.scrollTop = container.scrollHeight;
  _activeGroup = group;
  _activeToolCards = {};
  return group;
}

function showActivityThinking(message) {
  const group = getOrCreateActivityGroup();
  if (_activityThinking) {
    // Already exists — just update text and un-hide
    _activityThinking.style.display = '';
    _activityThinking.querySelector('.activity-thinking-text').textContent = message;
  } else {
    _activityThinking = document.createElement('div');
    _activityThinking.className = 'activity-thinking';
    _activityThinking.innerHTML =
      '<span class="activity-thinking-dots">'
      + '<span class="activity-thinking-dot"></span>'
      + '<span class="activity-thinking-dot"></span>'
      + '<span class="activity-thinking-dot"></span>'
      + '</span>'
      + '<span class="activity-thinking-text"></span>';
    group.appendChild(_activityThinking);
    _activityThinking.querySelector('.activity-thinking-text').textContent = message;
  }
  const container = document.getElementById('chat-messages');
  container.scrollTop = container.scrollHeight;
}

function removeActivityThinking() {
  if (_activityThinking) {
    _activityThinking.remove();
    _activityThinking = null;
  }
}

function addToolCard(name) {
  // Hide thinking instead of destroying — it may reappear between tool rounds
  if (_activityThinking) _activityThinking.style.display = 'none';
  const group = getOrCreateActivityGroup();

  const card = document.createElement('div');
  card.className = 'activity-tool-card';
  card.setAttribute('data-tool-name', name);
  card.setAttribute('data-status', 'running');

  const header = document.createElement('div');
  header.className = 'activity-tool-header';

  const icon = document.createElement('span');
  icon.className = 'activity-tool-icon';
  icon.innerHTML = '<div class="spinner"></div>';

  const toolName = document.createElement('span');
  toolName.className = 'activity-tool-name';
  toolName.textContent = name;

  const duration = document.createElement('span');
  duration.className = 'activity-tool-duration';
  duration.textContent = '';

  const chevron = document.createElement('span');
  chevron.className = 'activity-tool-chevron';
  chevron.innerHTML = '&#9656;';

  header.appendChild(icon);
  header.appendChild(toolName);
  header.appendChild(duration);
  header.appendChild(chevron);

  const body = document.createElement('div');
  body.className = 'activity-tool-body';
  body.style.display = 'none';

  const output = document.createElement('pre');
  output.className = 'activity-tool-output';
  body.appendChild(output);

  header.addEventListener('click', () => {
    const isOpen = body.style.display !== 'none';
    body.style.display = isOpen ? 'none' : 'block';
    chevron.classList.toggle('expanded', !isOpen);
  });

  card.appendChild(header);
  card.appendChild(body);
  group.appendChild(card);

  const startTime = Date.now();
  const timerInterval = setInterval(() => {
    const elapsed = (Date.now() - startTime) / 1000;
    if (elapsed > 300) { clearInterval(timerInterval); return; }
    duration.textContent = elapsed < 10 ? elapsed.toFixed(1) + 's' : Math.floor(elapsed) + 's';
  }, 100);

  if (!_activeToolCards[name]) _activeToolCards[name] = [];
  _activeToolCards[name].push({ card, startTime, timer: timerInterval, duration, icon, finalDuration: null });

  const container = document.getElementById('chat-messages');
  container.scrollTop = container.scrollHeight;
}

function completeToolCard(name, success, error, parameters) {
  const entries = _activeToolCards[name];
  if (!entries || entries.length === 0) return;
  // Find first running card
  let entry = null;
  for (let i = 0; i < entries.length; i++) {
    if (entries[i].card.getAttribute('data-status') === 'running') {
      entry = entries[i];
      break;
    }
  }
  if (!entry) entry = entries[entries.length - 1];

  clearInterval(entry.timer);
  const elapsed = (Date.now() - entry.startTime) / 1000;
  entry.finalDuration = elapsed;
  entry.duration.textContent = elapsed < 10 ? elapsed.toFixed(1) + 's' : Math.floor(elapsed) + 's';
  entry.icon.innerHTML = success
    ? '<span class="activity-icon-success">&#10003;</span>'
    : '<span class="activity-icon-fail">&#10007;</span>';
  entry.card.setAttribute('data-status', success ? 'success' : 'fail');

  // For failed tools, populate the body with error details and auto-expand
  if (!success && (error || parameters)) {
    const output = entry.card.querySelector('.activity-tool-output');
    if (output) {
      let detail = '';
      if (parameters) {
        detail += 'Input:\n' + parameters + '\n\n';
      }
      if (error) {
        detail += 'Error:\n' + error;
      }
      output.textContent = detail;

      // Auto-expand so the error is immediately visible
      const body = entry.card.querySelector('.activity-tool-body');
      const chevron = entry.card.querySelector('.activity-tool-chevron');
      if (body) body.style.display = 'block';
      if (chevron) chevron.classList.add('expanded');
    }
  }
}

function setToolCardOutput(name, preview) {
  const entries = _activeToolCards[name];
  if (!entries || entries.length === 0) return;
  // Find first card with empty output
  let entry = null;
  for (let i = 0; i < entries.length; i++) {
    const out = entries[i].card.querySelector('.activity-tool-output');
    if (out && !out.textContent) {
      entry = entries[i];
      break;
    }
  }
  if (!entry) entry = entries[entries.length - 1];

  const output = entry.card.querySelector('.activity-tool-output');
  if (output) {
    const truncated = preview.length > 2000 ? preview.substring(0, 2000) + '\n... (truncated)' : preview;
    output.textContent = truncated;
  }
}

function finalizeActivityGroup() {
  removeActivityThinking();
  if (!_activeGroup) return;

  // Stop all timers
  for (const name in _activeToolCards) {
    const entries = _activeToolCards[name];
    for (let i = 0; i < entries.length; i++) {
      clearInterval(entries[i].timer);
    }
  }

  // Count tools and total duration
  let toolCount = 0;
  let totalDuration = 0;
  for (const tname in _activeToolCards) {
    const tentries = _activeToolCards[tname];
    for (let j = 0; j < tentries.length; j++) {
      const entry = tentries[j];
      toolCount++;
      if (entry.finalDuration !== null) {
        totalDuration += entry.finalDuration;
      } else {
        // Tool was still running when finalized
        totalDuration += (Date.now() - entry.startTime) / 1000;
      }
    }
  }

  if (toolCount === 0) {
    // No tools were used — remove the empty group
    _activeGroup.remove();
    _activeGroup = null;
    _activeToolCards = {};
    return;
  }

  // Wrap existing cards into a hidden container
  const cardsContainer = document.createElement('div');
  cardsContainer.className = 'activity-cards-container';
  cardsContainer.style.display = 'none';

  const cards = _activeGroup.querySelectorAll('.activity-tool-card');
  for (let k = 0; k < cards.length; k++) {
    cardsContainer.appendChild(cards[k]);
  }

  // Build summary line
  const durationStr = totalDuration < 10 ? totalDuration.toFixed(1) + 's' : Math.floor(totalDuration) + 's';
  const toolWord = toolCount === 1 ? 'tool' : 'tools';
  const summary = document.createElement('div');
  summary.className = 'activity-summary';
  summary.innerHTML = '<span class="activity-summary-chevron">&#9656;</span>'
    + '<span class="activity-summary-text">Used ' + toolCount + ' ' + toolWord + '</span>'
    + '<span class="activity-summary-duration">(' + durationStr + ')</span>';

  summary.addEventListener('click', () => {
    const isOpen = cardsContainer.style.display !== 'none';
    cardsContainer.style.display = isOpen ? 'none' : 'block';
    summary.querySelector('.activity-summary-chevron').classList.toggle('expanded', !isOpen);
  });

  // Clear group and add summary + hidden cards
  _activeGroup.innerHTML = '';
  _activeGroup.classList.add('collapsed');
  _activeGroup.appendChild(summary);
  _activeGroup.appendChild(cardsContainer);

  _activeGroup = null;
  _activeToolCards = {};
}

function humanizeToolName(rawName) {
  if (!rawName) return '';
  return String(rawName)
    .replace(/[_-]+/g, ' ')
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/^tool([a-zA-Z])/, 'tool $1')
    .replace(/\s+/g, ' ')
    .trim();
}

function shouldShowChannelConnectedMessage(extensionName, success) {
  if (!success || !extensionName) return false;
  return String(extensionName).toLowerCase().includes('telegram');
}

function showApproval(data) {
  // Avoid duplicate cards on reconnect/history refresh.
  const existing = document.querySelector('.approval-card[data-request-id="' + CSS.escape(data.request_id) + '"]');
  if (existing) return;

  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'approval-card';
  card.setAttribute('data-request-id', data.request_id);

  const header = document.createElement('div');
  header.className = 'approval-header';
  header.textContent = I18n.t('approval.title');
  card.appendChild(header);

  const toolName = document.createElement('div');
  toolName.className = 'approval-tool-name';
  toolName.textContent = humanizeToolName(data.tool_name);
  card.appendChild(toolName);

  if (data.description) {
    const desc = document.createElement('div');
    desc.className = 'approval-description';
    desc.textContent = data.description;
    card.appendChild(desc);
  }

  if (data.parameters) {
    const paramsToggle = document.createElement('button');
    paramsToggle.className = 'approval-params-toggle';
    paramsToggle.textContent = I18n.t('approval.showParams');
    const paramsBlock = document.createElement('pre');
    paramsBlock.className = 'approval-params';
    paramsBlock.textContent = data.parameters;
    paramsBlock.style.display = 'none';
    paramsToggle.addEventListener('click', () => {
      const visible = paramsBlock.style.display !== 'none';
      paramsBlock.style.display = visible ? 'none' : 'block';
      paramsToggle.textContent = visible ? I18n.t('approval.showParams') : I18n.t('approval.hideParams');
    });
    card.appendChild(paramsToggle);
    card.appendChild(paramsBlock);
  }

  const actions = document.createElement('div');
  actions.className = 'approval-actions';

  const approveBtn = document.createElement('button');
  approveBtn.className = 'approve';
  approveBtn.textContent = I18n.t('approval.approve');
  approveBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'approve'));

  const denyBtn = document.createElement('button');
  denyBtn.className = 'deny';
  denyBtn.textContent = I18n.t('approval.deny');
  denyBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'deny'));

  actions.appendChild(approveBtn);
  if (data.allow_always !== false) {
    const alwaysBtn = document.createElement('button');
    alwaysBtn.className = 'always';
    alwaysBtn.textContent = I18n.t('approval.always');
    alwaysBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'always'));
    actions.appendChild(alwaysBtn);
  }
  actions.appendChild(denyBtn);
  card.appendChild(actions);

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
}

function showJobCard(data) {
  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'job-card';

  const icon = document.createElement('span');
  icon.className = 'job-card-icon';
  icon.textContent = '\u2692';
  card.appendChild(icon);

  const info = document.createElement('div');
  info.className = 'job-card-info';

  const title = document.createElement('div');
  title.className = 'job-card-title';
  title.textContent = data.title || I18n.t('sandbox.job');
  info.appendChild(title);

  const id = document.createElement('div');
  id.className = 'job-card-id';
  id.textContent = (data.job_id || '').substring(0, 8);
  info.appendChild(id);

  card.appendChild(info);

  const viewBtn = document.createElement('button');
  viewBtn.className = 'job-card-view';
  viewBtn.textContent = I18n.t('jobs.viewJob');
  viewBtn.addEventListener('click', () => {
    switchTab('jobs');
    openJobDetail(data.job_id);
  });
  card.appendChild(viewBtn);

  if (data.browse_url) {
    const browseBtn = document.createElement('a');
    browseBtn.className = 'job-card-browse';
    browseBtn.href = data.browse_url;
    browseBtn.target = '_blank';
    browseBtn.textContent = I18n.t('jobs.browse');
    card.appendChild(browseBtn);
  }

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
}

// --- Auth card ---

function handleAuthRequired(data) {
  if (data.auth_url) {
    setAuthFlowPending(true, data.instructions);
    // OAuth flow: show the global auth prompt with an OAuth button + optional token paste field.
    showAuthCard(data);
  } else {
    if (getConfigureOverlay(data.extension_name)) return;
    setAuthFlowPending(true, data.instructions);
    // Setup flow: fetch the extension's credential schema and show the multi-field
    // configure modal (the same UI used by the Extensions tab "Setup" button).
    showConfigureModal(data.extension_name);
  }
}

function handleAuthCompleted(data) {
  showToast(data.message, data.success ? 'success' : 'error');
  // Dismiss only the matching extension's UI so stale prompts are cleared.
  removeAuthCard(data.extension_name);
  closeConfigureModal(data.extension_name);
  if (!data.success) {
    setAuthFlowPending(false);
    if (currentTab === 'extensions') loadExtensions();
    enableChatInput();
    return;
  }
  setAuthFlowPending(false);
  if (shouldShowChannelConnectedMessage(data.extension_name, data.success)) {
    addMessage('system', 'Telegram is now connected. You can message me there and I can send you notifications.');
  }
  if (currentTab === 'settings') refreshCurrentSettingsTab();
  enableChatInput();
}

function queryByDataAttribute(selector, attributeName, attributeValue) {
  if (typeof attributeValue !== 'string') return document.querySelector(selector);

  if (window.CSS && typeof window.CSS.escape === 'function') {
    return document.querySelector(
      selector + '[' + attributeName + '="' + window.CSS.escape(attributeValue) + '"]'
    );
  }

  const candidates = document.querySelectorAll(selector);
  for (const candidate of candidates) {
    if (candidate.getAttribute(attributeName) === attributeValue) return candidate;
  }
  return null;
}

function getAuthOverlay(extensionName) {
  return queryByDataAttribute('.auth-overlay', 'data-extension-name', extensionName);
}

function getAuthCard(extensionName) {
  return queryByDataAttribute('.auth-card', 'data-extension-name', extensionName);
}

function getConfigureOverlay(extensionName) {
  return queryByDataAttribute('.configure-overlay', 'data-extension-name', extensionName);
}

function showAuthCard(data) {
  // Keep a single global auth prompt so the experience is consistent across tabs.
  const existing = getAuthOverlay();
  if (existing) existing.remove();

  const overlay = document.createElement('div');
  overlay.className = 'auth-overlay';
  overlay.setAttribute('data-extension-name', data.extension_name);
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) cancelAuth(data.extension_name);
  });

  const card = document.createElement('div');
  card.className = 'auth-card auth-modal';
  card.setAttribute('data-extension-name', data.extension_name);

  const header = document.createElement('div');
  header.className = 'auth-header';
  header.textContent = I18n.t('authRequired.title', {name: data.extension_name});
  card.appendChild(header);

  if (data.instructions) {
    const instr = document.createElement('div');
    instr.className = 'auth-instructions';
    instr.textContent = data.instructions;
    card.appendChild(instr);
  }

  const links = document.createElement('div');
  links.className = 'auth-links';

  if (data.auth_url) {
    const oauthBtn = document.createElement('button');
    oauthBtn.className = 'auth-oauth';
    oauthBtn.textContent = I18n.t('authRequired.authenticateWith', {name: data.extension_name});
    oauthBtn.addEventListener('click', () => {
      openOAuthUrl(data.auth_url);
    });
    links.appendChild(oauthBtn);
  }

  if (data.setup_url) {
    const setupLink = document.createElement('a');
    setupLink.href = data.setup_url;
    setupLink.target = '_blank';
    setupLink.textContent = I18n.t('authRequired.getToken');
    links.appendChild(setupLink);
  }

  if (links.children.length > 0) {
    card.appendChild(links);
  }

  // Token input
  const tokenRow = document.createElement('div');
  tokenRow.className = 'auth-token-input';

  const tokenInput = document.createElement('input');
  tokenInput.type = 'password';
  tokenInput.placeholder = data.instructions
    || I18n.t('auth.extensionTokenPlaceholder')
    || I18n.t('auth.tokenPlaceholder');
  tokenInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') submitAuthToken(data.extension_name, tokenInput.value);
  });
  tokenRow.appendChild(tokenInput);
  card.appendChild(tokenRow);

  // Error display (hidden initially)
  const errorEl = document.createElement('div');
  errorEl.className = 'auth-error';
  errorEl.style.display = 'none';
  card.appendChild(errorEl);

  // Action buttons
  const actions = document.createElement('div');
  actions.className = 'auth-actions';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'auth-submit';
  submitBtn.textContent = I18n.t('btn.submit');
  submitBtn.addEventListener('click', () => submitAuthToken(data.extension_name, tokenInput.value));

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'auth-cancel';
  cancelBtn.textContent = I18n.t('btn.cancel');
  cancelBtn.addEventListener('click', () => cancelAuth(data.extension_name));

  actions.appendChild(submitBtn);
  actions.appendChild(cancelBtn);
  card.appendChild(actions);

  overlay.appendChild(card);
  document.body.appendChild(overlay);
  tokenInput.focus();
}

function removeAuthCard(extensionName) {
  const overlay = getAuthOverlay(extensionName);
  if (overlay) {
    overlay.remove();
    return;
  }
  const card = getAuthCard(extensionName);
  if (card) {
    const parentOverlay = card.closest('.auth-overlay');
    if (parentOverlay) parentOverlay.remove();
    else card.remove();
  }
}

function submitAuthToken(extensionName, tokenValue) {
  if (!tokenValue || !tokenValue.trim()) return;

  // Disable submit button while in flight
  const card = getAuthCard(extensionName);
  if (card) {
    const btns = card.querySelectorAll('button');
    btns.forEach((b) => { b.disabled = true; });
  }

  apiFetch('/api/chat/auth-token', {
    method: 'POST',
    body: { extension_name: extensionName, token: tokenValue.trim() },
  }).then((result) => {
    if (result.success) {
      // Close immediately for responsiveness; the authoritative success UX
      // (toast + extensions refresh) still comes from auth_completed SSE.
      removeAuthCard(extensionName);
      enableChatInput();
    } else {
      showAuthCardError(extensionName, result.message);
    }
  }).catch((err) => {
    showAuthCardError(extensionName, 'Failed: ' + err.message);
  });
}

function cancelAuth(extensionName) {
  apiFetch('/api/chat/auth-cancel', {
    method: 'POST',
    body: { extension_name: extensionName },
  }).catch(() => {});
  removeAuthCard(extensionName);
  setAuthFlowPending(false);
  enableChatInput();
}

function showAuthCardError(extensionName, message) {
  const card = getAuthCard(extensionName);
  if (!card) return;
  // Re-enable buttons
  const btns = card.querySelectorAll('button');
  btns.forEach((b) => { b.disabled = false; });
  // Show error
  const errorEl = card.querySelector('.auth-error');
  if (errorEl) {
    errorEl.textContent = message;
    errorEl.style.display = 'block';
  }
}

function setAuthFlowPending(pending, instructions) {
  authFlowPending = !!pending;
  const input = document.getElementById('chat-input');
  const btn = document.getElementById('send-btn');
  if (!input || !btn) return;
  if (authFlowPending) {
    input.disabled = true;
    btn.disabled = true;
    return;
  }
  if (!currentThreadIsReadOnly) {
    input.disabled = false;
    btn.disabled = false;
  }
}

function loadHistory(before) {
  clearSuggestionChips();
  let historyUrl = '/api/chat/history?limit=50';
  if (currentThreadId) {
    historyUrl += '&thread_id=' + encodeURIComponent(currentThreadId);
  }
  if (before) {
    historyUrl += '&before=' + encodeURIComponent(before);
  }

  const isPaginating = !!before;
  if (isPaginating) loadingOlder = true;

  apiFetch(historyUrl).then((data) => {
    const container = document.getElementById('chat-messages');

    if (!isPaginating) {
      // Fresh load: clear and render
      container.innerHTML = '';
      for (const turn of data.turns) {
        if (turn.user_input) {
          addMessage('user', turn.user_input);
        }
        if (turn.tool_calls && turn.tool_calls.length > 0) {
          addToolCallsSummary(turn.tool_calls);
        }
        if (turn.response) {
          addMessage('assistant', turn.response);
        }
      }
      // Show processing indicator if the last turn is still in-progress
      var lastTurn = data.turns.length > 0 ? data.turns[data.turns.length - 1] : null;
      if (lastTurn && !lastTurn.response && lastTurn.state === 'Processing') {
        showActivityThinking('Processing...');
      }
      // Re-render pending approval card if the thread is awaiting approval
      if (data.pending_approval) {
        showApproval(data.pending_approval);
      }
    } else {
      // Pagination: prepend older messages
      const savedHeight = container.scrollHeight;
      const fragment = document.createDocumentFragment();
      for (const turn of data.turns) {
        if (turn.user_input) {
          const userDiv = createMessageElement('user', turn.user_input);
          fragment.appendChild(userDiv);
        }
        if (turn.tool_calls && turn.tool_calls.length > 0) {
          fragment.appendChild(createToolCallsSummaryElement(turn.tool_calls));
        }
        if (turn.response) {
          const assistantDiv = createMessageElement('assistant', turn.response);
          fragment.appendChild(assistantDiv);
        }
      }
      container.insertBefore(fragment, container.firstChild);
      // Restore scroll position so the user doesn't jump
      container.scrollTop = container.scrollHeight - savedHeight;
    }

    hasMore = data.has_more || false;
    oldestTimestamp = data.oldest_timestamp || null;
  }).catch(() => {
    // No history or no active thread
  }).finally(() => {
    loadingOlder = false;
    removeScrollSpinner();
  });
}

// Create a message DOM element without appending it (for prepend operations)
function createMessageElement(role, content) {
  const div = document.createElement('div');
  div.className = 'message ' + role;

  if (role === 'assistant' || role === 'user') {
    div.classList.add('has-copy');
    div.setAttribute('data-copy-text', content);
    const copyBtn = document.createElement('button');
    copyBtn.className = 'message-copy-btn';
    copyBtn.type = 'button';
    copyBtn.setAttribute('aria-label', 'Copy message');
    copyBtn.textContent = 'Copy';
    copyBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      copyMessage(copyBtn);
    });
    div.appendChild(copyBtn);
  }

  const body = document.createElement('div');
  body.className = 'message-content';
  if (role === 'user' || role === 'system') {
    body.textContent = content;
  } else {
    div.setAttribute('data-raw', content);
    body.innerHTML = renderMarkdown(content);
  }
  div.appendChild(body);
  return div;
}

function addToolCallsSummary(toolCalls) {
  const container = document.getElementById('chat-messages');
  container.appendChild(createToolCallsSummaryElement(toolCalls));
  container.scrollTop = container.scrollHeight;
}

function createToolCallsSummaryElement(toolCalls) {
  const div = document.createElement('div');
  div.className = 'tool-calls-summary';

  const header = document.createElement('div');
  header.className = 'tool-calls-header';
  header.textContent = toolCalls.length + ' tool' + (toolCalls.length !== 1 ? 's' : '') + ' used';
  div.appendChild(header);

  const list = document.createElement('div');
  list.className = 'tool-calls-list';

  for (const tc of toolCalls) {
    const item = document.createElement('div');
    item.className = 'tool-call-item' + (tc.has_error ? ' tool-error' : '');

    const icon = tc.has_error ? '\u2717' : '\u2713';
    const nameSpan = document.createElement('span');
    nameSpan.className = 'tool-call-name';
    nameSpan.textContent = icon + ' ' + tc.name;
    item.appendChild(nameSpan);

    if (tc.result_preview) {
      const preview = document.createElement('div');
      preview.className = 'tool-call-preview';
      preview.textContent = tc.result_preview;
      item.appendChild(preview);
    }
    if (tc.error) {
      const errDiv = document.createElement('div');
      errDiv.className = 'tool-call-error-text';
      errDiv.textContent = tc.error;
      item.appendChild(errDiv);
    }

    list.appendChild(item);
  }

  div.appendChild(list);

  header.style.cursor = 'pointer';
  header.addEventListener('click', () => {
    list.classList.toggle('expanded');
    header.classList.toggle('expanded');
  });

  return div;
}

function removeScrollSpinner() {
  const spinner = document.getElementById('scroll-load-spinner');
  if (spinner) spinner.remove();
}

// --- Threads ---

function threadTitle(thread) {
  if (thread.title) return thread.title;
  const ch = thread.channel || 'gateway';
  if (thread.thread_type === 'heartbeat') return 'Heartbeat Alerts';
  if (thread.thread_type === 'routine') return 'Routine';
  if (ch !== 'gateway') return ch.charAt(0).toUpperCase() + ch.slice(1);
  if (thread.turn_count === 0) return 'New chat';
  return thread.id.substring(0, 8);
}

function relativeTime(isoStr) {
  if (!isoStr) return '';
  const diff = Date.now() - new Date(isoStr).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'now';
  if (mins < 60) return mins + 'm ago';
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return hrs + 'h ago';
  const days = Math.floor(hrs / 24);
  return days + 'd ago';
}

function isReadOnlyChannel(channel) {
  return channel && channel !== 'gateway' && channel !== 'routine' && channel !== 'heartbeat';
}

function debouncedLoadThreads() {
  if (_loadThreadsTimer) clearTimeout(_loadThreadsTimer);
  _loadThreadsTimer = setTimeout(() => { _loadThreadsTimer = null; loadThreads(); }, 500);
}

function loadThreads() {
  apiFetch('/api/chat/threads').then((data) => {
    // Pinned assistant thread
    if (data.assistant_thread) {
      assistantThreadId = data.assistant_thread.id;
      const el = document.getElementById('assistant-thread');
      const isActive = currentThreadId === assistantThreadId;
      el.className = 'assistant-item' + (isActive ? ' active' : '');
      const labelEl = document.getElementById('assistant-label');
      if (labelEl) {
        const at = data.assistant_thread;
        labelEl.textContent = 'Assistant';
      }
      const meta = document.getElementById('assistant-meta');
      meta.textContent = relativeTime(data.assistant_thread.updated_at);
    }

    // Regular threads
    const list = document.getElementById('thread-list');
    list.innerHTML = '';
    const threads = data.threads || [];
    for (const thread of threads) {
      const item = document.createElement('div');
      const isActive = thread.id === currentThreadId;
      item.className = 'thread-item' + (isActive ? ' active' : '');

      // Channel badge for non-gateway threads
      const ch = thread.channel || 'gateway';
      if (ch !== 'gateway') {
        const badge = document.createElement('span');
        badge.className = 'thread-badge thread-badge-' + ch;
        badge.textContent = ch;
        item.appendChild(badge);
      }

      const label = document.createElement('span');
      label.className = 'thread-label';
      label.textContent = threadTitle(thread);
      label.title = (thread.title || '') + ' (' + thread.id + ')';
      item.appendChild(label);

      const meta = document.createElement('span');
      meta.className = 'thread-meta';
      meta.textContent = relativeTime(thread.updated_at);
      item.appendChild(meta);

      // Unread dot
      const unread = unreadThreads.get(thread.id) || 0;
      if (unread > 0 && !isActive) {
        const dot = document.createElement('span');
        dot.className = 'thread-unread';
        dot.textContent = unread > 9 ? '9+' : String(unread);
        item.appendChild(dot);
      }

      item.addEventListener('click', () => switchThread(thread.id));
      list.appendChild(item);
    }

    // Default to assistant thread on first load if no thread selected
    if (!currentThreadId && assistantThreadId) {
      switchToAssistant();
    }

    // Enable/disable chat input based on channel type
    if (currentThreadId) {
      const currentThread = threads.find(t => t.id === currentThreadId);
      const ch = currentThread ? currentThread.channel : 'gateway';
      currentThreadIsReadOnly = isReadOnlyChannel(ch);
      if (currentThreadIsReadOnly) {
        disableChatInputReadOnly();
      } else {
        enableChatInput();
      }
    }
  }).catch(() => {});
}

function disableChatInputReadOnly() {
  const input = document.getElementById('chat-input');
  const btn = document.getElementById('send-btn');
  if (input) {
    input.disabled = true;
    input.placeholder = 'Read-only thread (external channel)';
  }
  if (btn) btn.disabled = true;
}

function switchToAssistant() {
  if (!assistantThreadId) return;
  finalizeActivityGroup();
  currentThreadId = assistantThreadId;
  currentThreadIsReadOnly = false;
  unreadThreads.delete(assistantThreadId);
  hasMore = false;
  oldestTimestamp = null;
  loadHistory();
  loadThreads();
}

function switchThread(threadId) {
  clearSuggestionChips();
  finalizeActivityGroup();
  currentThreadId = threadId;
  unreadThreads.delete(threadId);
  hasMore = false;
  oldestTimestamp = null;
  loadHistory();
  loadThreads();
}

function createNewThread() {
  apiFetch('/api/chat/thread/new', { method: 'POST' }).then((data) => {
    currentThreadId = data.id || null;
    document.getElementById('chat-messages').innerHTML = '';
    loadThreads();
  }).catch((err) => {
    showToast('Failed to create thread: ' + err.message, 'error');
  });
}

function toggleThreadSidebar() {
  const sidebar = document.getElementById('thread-sidebar');
  sidebar.classList.toggle('collapsed');
  const btn = document.getElementById('thread-toggle-btn');
  btn.innerHTML = sidebar.classList.contains('collapsed') ? '&raquo;' : '&laquo;';
}

// Chat input auto-resize and keyboard handling
const chatInput = document.getElementById('chat-input');
chatInput.addEventListener('keydown', (e) => {
  const acEl = document.getElementById('slash-autocomplete');
  const acVisible = acEl && acEl.style.display !== 'none';

  // Accept first suggestion with Tab (plain Tab only, not Shift+Tab)
  if (e.key === 'Tab' && !e.shiftKey && !acVisible && _ghostSuggestion && chatInput.value === '') {
    e.preventDefault();
    chatInput.value = _ghostSuggestion;
    clearSuggestionChips();
    autoResizeTextarea(chatInput);
    return;
  }

  if (acVisible) {
    const items = acEl.querySelectorAll('.slash-ac-item');
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      _slashSelected = Math.min(_slashSelected + 1, items.length - 1);
      updateSlashHighlight();
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      _slashSelected = Math.max(_slashSelected - 1, -1);
      updateSlashHighlight();
      return;
    }
    if (e.key === 'Tab' || e.key === 'Enter') {
      e.preventDefault();
      const pick = _slashSelected >= 0 ? _slashMatches[_slashSelected] : _slashMatches[0];
      if (pick) selectSlashItem(pick.cmd);
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      hideSlashAutocomplete();
      return;
    }
  }

  // Safari fires compositionend before keydown, so e.isComposing is already false
  // when Enter confirms IME input. keyCode 229 (VK_PROCESS) catches this case.
  // See https://bugs.webkit.org/show_bug.cgi?id=165004
  if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.keyCode !== 229) {
    e.preventDefault();
    hideSlashAutocomplete();
    sendMessage();
  }
});
chatInput.addEventListener('input', () => {
  autoResizeTextarea(chatInput);
  filterSlashCommands(chatInput.value);
  const ghost = document.getElementById('ghost-text');
  const wrapper = chatInput.closest('.chat-input-wrapper');
  if (chatInput.value !== '') {
    ghost.style.display = 'none';
    wrapper.classList.remove('has-ghost');
  } else if (_ghostSuggestion) {
    ghost.textContent = _ghostSuggestion;
    ghost.style.display = 'block';
    wrapper.classList.add('has-ghost');
  }
});
chatInput.addEventListener('blur', () => {
  // Small delay so mousedown on autocomplete item fires first
  setTimeout(hideSlashAutocomplete, 150);
});

// Infinite scroll: load older messages when scrolled near the top
document.getElementById('chat-messages').addEventListener('scroll', function () {
  if (this.scrollTop < 100 && hasMore && !loadingOlder) {
    loadingOlder = true;
    // Show spinner at top
    const spinner = document.createElement('div');
    spinner.id = 'scroll-load-spinner';
    spinner.className = 'scroll-load-spinner';
    spinner.innerHTML = '<div class="spinner"></div> Loading older messages...';
    this.insertBefore(spinner, this.firstChild);
    loadHistory(oldestTimestamp);
  }
});

function autoResizeTextarea(el) {
  el.style.height = 'auto';
  el.style.height = Math.min(el.scrollHeight, 120) + 'px';
}

// --- Tabs ---

document.querySelectorAll('.tab-bar button[data-tab]').forEach((btn) => {
  btn.addEventListener('click', () => {
    const tab = btn.getAttribute('data-tab');
    switchTab(tab);
  });
});

function switchTab(tab) {
  currentTab = tab;
  document.querySelectorAll('.tab-bar button[data-tab]').forEach((b) => {
    b.classList.toggle('active', b.getAttribute('data-tab') === tab);
  });
  document.querySelectorAll('.tab-panel').forEach((p) => {
    p.classList.toggle('active', p.id === 'tab-' + tab);
  });

  if (tab === 'memory') loadMemoryTree();
  if (tab === 'jobs') loadJobs();
  if (tab === 'routines') loadRoutines();
  if (tab === 'logs') applyLogFilters();
  if (tab === 'settings') {
    loadSettingsSubtab(currentSettingsSubtab);
  } else {
    stopPairingPoll();
  }
}

// --- Memory (filesystem tree) ---

let memorySearchTimeout = null;
let currentMemoryPath = null;
let currentMemoryContent = null;
// Tree state: nested nodes persisted across renders
// { name, path, is_dir, children: [] | null, expanded: bool, loaded: bool }
let memoryTreeState = null;

document.getElementById('memory-search').addEventListener('input', (e) => {
  clearTimeout(memorySearchTimeout);
  const query = e.target.value.trim();
  if (!query) {
    loadMemoryTree();
    return;
  }
  memorySearchTimeout = setTimeout(() => searchMemory(query), 300);
});

function loadMemoryTree() {
  // Only load top-level on first load (or refresh)
  apiFetch('/api/memory/list?path=').then((data) => {
    memoryTreeState = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    renderTree();
  }).catch(() => {});
}

function renderTree() {
  const container = document.getElementById('memory-tree');
  container.innerHTML = '';
  if (!memoryTreeState || memoryTreeState.length === 0) {
    container.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No files in workspace</div>';
    return;
  }
  renderNodes(memoryTreeState, container, 0);
}

function renderNodes(nodes, container, depth) {
  for (const node of nodes) {
    const row = document.createElement('div');
    row.className = 'tree-row';
    row.style.paddingLeft = (depth * 16 + 8) + 'px';
    row.tabIndex = 0;
    row.setAttribute('role', 'treeitem');

    if (node.is_dir) {
      row.setAttribute('aria-expanded', node.expanded ? 'true' : 'false');
      const arrow = document.createElement('span');
      arrow.className = 'expand-arrow' + (node.expanded ? ' expanded' : '');
      arrow.textContent = '\u25B6';
      row.appendChild(arrow);

      const label = document.createElement('span');
      label.className = 'tree-label dir';
      label.textContent = node.name;
      row.appendChild(label);

      row.addEventListener('click', () => toggleExpand(node));
      row.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleExpand(node); }
      });
    } else {
      const spacer = document.createElement('span');
      spacer.className = 'expand-arrow-spacer';
      row.appendChild(spacer);

      const label = document.createElement('span');
      label.className = 'tree-label file';
      label.textContent = node.name;
      row.appendChild(label);

      row.addEventListener('click', () => readMemoryFile(node.path));
      row.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); readMemoryFile(node.path); }
      });
    }

    container.appendChild(row);

    if (node.is_dir && node.expanded && node.children) {
      const childContainer = document.createElement('div');
      childContainer.className = 'tree-children';
      renderNodes(node.children, childContainer, depth + 1);
      container.appendChild(childContainer);
    }
  }
}

function toggleExpand(node) {
  if (node.expanded) {
    node.expanded = false;
    renderTree();
    return;
  }

  if (node.loaded) {
    node.expanded = true;
    renderTree();
    return;
  }

  // Lazy-load children
  apiFetch('/api/memory/list?path=' + encodeURIComponent(node.path)).then((data) => {
    node.children = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    node.loaded = true;
    node.expanded = true;
    renderTree();
  }).catch(() => {});
}

function readMemoryFile(path) {
  currentMemoryPath = path;
  // Update breadcrumb
  document.getElementById('memory-breadcrumb-path').innerHTML = buildBreadcrumb(path);
  document.getElementById('memory-edit-btn').style.display = 'inline-block';

  // Exit edit mode if active
  cancelMemoryEdit();

  apiFetch('/api/memory/read?path=' + encodeURIComponent(path)).then((data) => {
    currentMemoryContent = data.content;
    const viewer = document.getElementById('memory-viewer');
    // Render markdown if it's a .md file
    if (path.endsWith('.md')) {
      viewer.innerHTML = '<div class="memory-rendered">' + renderMarkdown(data.content) + '</div>';
      viewer.classList.add('rendered');
    } else {
      viewer.textContent = data.content;
      viewer.classList.remove('rendered');
    }
  }).catch((err) => {
    currentMemoryContent = null;
    document.getElementById('memory-viewer').innerHTML = '<div class="empty">Error: ' + escapeHtml(err.message) + '</div>';
  });
}

function startMemoryEdit() {
  if (!currentMemoryPath || currentMemoryContent === null) return;
  document.getElementById('memory-viewer').style.display = 'none';
  const editor = document.getElementById('memory-editor');
  editor.style.display = 'flex';
  const textarea = document.getElementById('memory-edit-textarea');
  textarea.value = currentMemoryContent;
  textarea.focus();
}

function cancelMemoryEdit() {
  document.getElementById('memory-viewer').style.display = '';
  document.getElementById('memory-editor').style.display = 'none';
}

function saveMemoryEdit() {
  if (!currentMemoryPath) return;
  const content = document.getElementById('memory-edit-textarea').value;
  apiFetch('/api/memory/write', {
    method: 'POST',
    body: { path: currentMemoryPath, content: content },
  }).then(() => {
    showToast('Saved ' + currentMemoryPath, 'success');
    cancelMemoryEdit();
    readMemoryFile(currentMemoryPath);
  }).catch((err) => {
    showToast('Save failed: ' + err.message, 'error');
  });
}

function buildBreadcrumb(path) {
  const parts = path.split('/');
  let html = '<a data-action="breadcrumb-root" href="#">workspace</a>';
  let current = '';
  for (const part of parts) {
    current += (current ? '/' : '') + part;
    html += ' / <a data-action="breadcrumb-file" data-path="' + escapeHtml(current) + '" href="#">' + escapeHtml(part) + '</a>';
  }
  return html;
}

function searchMemory(query) {
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) return;

  apiFetch('/api/memory/search', {
    method: 'POST',
    body: { query: normalizedQuery, limit: 20 },
  }).then((data) => {
    const tree = document.getElementById('memory-tree');
    tree.innerHTML = '';
    if (data.results.length === 0) {
      tree.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No results</div>';
      return;
    }
    for (const result of data.results) {
      const item = document.createElement('div');
      item.className = 'search-result';
      const snippet = snippetAround(result.content, normalizedQuery, 120);
      item.innerHTML = '<div class="path">' + escapeHtml(result.path) + '</div>'
        + '<div class="snippet">' + highlightQuery(snippet, normalizedQuery) + '</div>';
      item.addEventListener('click', () => readMemoryFile(result.path));
      tree.appendChild(item);
    }
  }).catch(() => {});
}

function normalizeSearchQuery(query) {
  return (typeof query === 'string' ? query : '').slice(0, MEMORY_SEARCH_QUERY_MAX_LENGTH);
}

function snippetAround(text, query, len) {
  const normalizedQuery = normalizeSearchQuery(query);
  const lower = text.toLowerCase();
  const idx = lower.indexOf(normalizedQuery.toLowerCase());
  if (idx < 0) return text.substring(0, len);
  const start = Math.max(0, idx - Math.floor(len / 2));
  const end = Math.min(text.length, start + len);
  let s = text.substring(start, end);
  if (start > 0) s = '...' + s;
  if (end < text.length) s = s + '...';
  return s;
}

function highlightQuery(text, query) {
  if (!query) return escapeHtml(text);
  const escaped = escapeHtml(text);
  const normalizedQuery = normalizeSearchQuery(query);
  const queryEscaped = normalizedQuery.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const re = new RegExp('(' + queryEscaped + ')', 'gi');
  return escaped.replace(re, '<mark>$1</mark>');
}
// --- Logs ---

const LOG_MAX_ENTRIES = 2000;
let logsPaused = false;
let logBuffer = []; // buffer while paused

function connectLogSSE() {
  if (logEventSource) logEventSource.close();

  logEventSource = new EventSource('/api/logs/events?token=' + encodeURIComponent(token));

  logEventSource.addEventListener('log', (e) => {
    const entry = JSON.parse(e.data);
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

function prependLogEntry(entry) {
  const output = document.getElementById('logs-output');

  // Level filter
  const levelFilter = document.getElementById('logs-level-filter').value;
  const targetFilter = document.getElementById('logs-target-filter').value.trim().toLowerCase();

  const div = document.createElement('div');
  div.className = 'log-entry level-' + entry.level;
  div.setAttribute('data-level', entry.level);
  div.setAttribute('data-target', entry.target);

  const ts = document.createElement('span');
  ts.className = 'log-ts';
  ts.textContent = entry.timestamp.substring(11, 23);
  div.appendChild(ts);

  const lvl = document.createElement('span');
  lvl.className = 'log-level';
  lvl.textContent = entry.level.padEnd(5);
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

  // Apply current filters as visibility
  const matchesLevel = levelFilter === 'all' || entry.level === levelFilter;
  const matchesTarget = !targetFilter || entry.target.toLowerCase().includes(targetFilter);
  if (!matchesLevel || !matchesTarget) {
    div.style.display = 'none';
  }

  output.prepend(div);

  // Cap entries (remove oldest at the bottom)
  while (output.children.length > LOG_MAX_ENTRIES) {
    output.removeChild(output.lastChild);
  }

  // Auto-scroll to top (newest entries are at the top)
  if (document.getElementById('logs-autoscroll').checked) {
    output.scrollTop = 0;
  }
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
  if (!confirm('Clear all logs?')) return;
  document.getElementById('logs-output').innerHTML = '';
  logBuffer = [];
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
    const matchesTarget = !targetFilter || el.getAttribute('data-target').toLowerCase().includes(targetFilter);
    el.style.display = (matchesLevel && matchesTarget) ? '' : 'none';
  }
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

function loadExtensions() {
  const extList = document.getElementById('extensions-list');
  const wasmList = document.getElementById('available-wasm-list');
  extList.innerHTML = renderCardsSkeleton(3);

  // Fetch extensions and registry in parallel
  Promise.all([
    apiFetch('/api/extensions').catch(() => ({ extensions: [] })),
    apiFetch('/api/extensions/registry').catch(function(err) { console.warn('registry fetch failed:', err); return { entries: [] }; }),
  ]).then(([extData, registryData]) => {
    // Render installed extensions (exclude wasm_channel and mcp_server — shown in their own tabs)
    var nonChannelExts = extData.extensions.filter(function(e) {
      return e.kind !== 'wasm_channel' && e.kind !== 'mcp_server';
    });
    if (nonChannelExts.length === 0) {
      extList.innerHTML = '<div class="empty-state">' + I18n.t('extensions.noInstalled') + '</div>';
    } else {
      extList.innerHTML = '';
      for (const ext of nonChannelExts) {
        extList.appendChild(renderExtensionCard(ext));
      }
    }

    // Available extensions (exclude MCP servers and channels — they have their own tabs)
    var wasmEntries = registryData.entries.filter(function(e) {
      return e.kind !== 'mcp_server' && e.kind !== 'wasm_channel' && e.kind !== 'channel' && !e.installed;
    });

    var wasmSection = document.getElementById('available-wasm-section');
    if (wasmEntries.length === 0) {
      if (wasmSection) wasmSection.style.display = 'none';
    } else {
      if (wasmSection) wasmSection.style.display = '';
      wasmList.innerHTML = '';
      for (const entry of wasmEntries) {
        wasmList.appendChild(renderAvailableExtensionCard(entry));
      }
    }

  });
}

function renderAvailableExtensionCard(entry) {
  const card = document.createElement('div');
  card.className = 'ext-card ext-available';

  const header = document.createElement('div');
  header.className = 'ext-header';

  const name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = entry.display_name;
  header.appendChild(name);

  const kind = document.createElement('span');
  kind.className = 'ext-kind kind-' + entry.kind;
  kind.textContent = kindLabels[entry.kind] || entry.kind;
  header.appendChild(kind);

  if (entry.version) {
    const ver = document.createElement('span');
    ver.className = 'ext-version';
    ver.textContent = 'v' + entry.version;
    header.appendChild(ver);
  }

  card.appendChild(header);

  const desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = entry.description;
  card.appendChild(desc);

  if (entry.keywords && entry.keywords.length > 0) {
    const kw = document.createElement('div');
    kw.className = 'ext-keywords';
    kw.textContent = entry.keywords.join(', ');
    card.appendChild(kw);
  }

  const actions = document.createElement('div');
  actions.className = 'ext-actions';

  const installBtn = document.createElement('button');
  installBtn.className = 'btn-ext install';
  installBtn.textContent = I18n.t('extensions.install');
  installBtn.addEventListener('click', function() {
    installBtn.disabled = true;
    installBtn.textContent = I18n.t('extensions.installing');
    apiFetch('/api/extensions/install', {
      method: 'POST',
      body: { name: entry.name, kind: entry.kind },
    }).then(function(res) {
      if (res.success) {
        showToast(I18n.t('extensions.installedSuccess', {name: entry.display_name}), 'success');
        // OAuth popup if auth started during install (builtin creds)
        if (res.auth_url) {
          showAuthCard({
            extension_name: entry.name,
            auth_url: res.auth_url,
          });
          showToast('Opening authentication for ' + entry.display_name, 'info');
          openOAuthUrl(res.auth_url);
        }
        refreshCurrentSettingsTab();
        // Auto-open configure for WASM channels
        if (entry.kind === 'wasm_channel') {
          showConfigureModal(entry.name);
        }
      } else {
        showToast('Install: ' + (res.message || 'unknown error'), 'error');
        refreshCurrentSettingsTab();
      }
    }).catch(function(err) {
      showToast('Install failed: ' + err.message, 'error');
      refreshCurrentSettingsTab();
    });
  });
  actions.appendChild(installBtn);

  card.appendChild(actions);
  return card;
}

function renderMcpServerCard(entry, installedExt) {
  var card = document.createElement('div');
  card.className = 'ext-card' + (installedExt ? '' : ' ext-available');

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = entry.display_name;
  header.appendChild(name);

  var kind = document.createElement('span');
  kind.className = 'ext-kind kind-mcp_server';
  kind.textContent = kindLabels['mcp_server'] || 'mcp_server';
  header.appendChild(kind);

  if (installedExt) {
    var authDot = document.createElement('span');
    authDot.className = 'ext-auth-dot ' + (installedExt.authenticated ? 'authed' : 'unauthed');
    authDot.title = installedExt.authenticated ? 'Authenticated' : 'Not authenticated';
    header.appendChild(authDot);
  }

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = entry.description;
  card.appendChild(desc);

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  if (installedExt) {
    if (!installedExt.active) {
      var activateBtn = document.createElement('button');
      activateBtn.className = 'btn-ext activate';
      activateBtn.textContent = I18n.t('common.activate');
      activateBtn.addEventListener('click', function() { activateExtension(installedExt.name); });
      actions.appendChild(activateBtn);
    } else {
      var activeLabel = document.createElement('span');
      activeLabel.className = 'ext-active-label';
      activeLabel.textContent = I18n.t('ext.active');
      actions.appendChild(activeLabel);
    }
    if (installedExt.needs_setup || (installedExt.has_auth && installedExt.authenticated)) {
      var configBtn = document.createElement('button');
      configBtn.className = 'btn-ext configure';
      configBtn.textContent = installedExt.authenticated ? I18n.t('ext.reconfigure') : I18n.t('ext.configure');
      configBtn.addEventListener('click', function() { showConfigureModal(installedExt.name); });
      actions.appendChild(configBtn);
    }
    var removeBtn = document.createElement('button');
    removeBtn.className = 'btn-ext remove';
    removeBtn.textContent = I18n.t('ext.remove');
    removeBtn.addEventListener('click', function() { removeExtension(installedExt.name); });
    actions.appendChild(removeBtn);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-ext install';
    installBtn.textContent = I18n.t('ext.install');
    installBtn.addEventListener('click', function() {
      installBtn.disabled = true;
      installBtn.textContent = I18n.t('ext.installing');
      apiFetch('/api/extensions/install', {
        method: 'POST',
        body: { name: entry.name, kind: entry.kind },
      }).then(function(res) {
        if (res.success) {
          showToast(I18n.t('extensions.installedSuccess', { name: entry.display_name }), 'success');
        } else {
          showToast(I18n.t('ext.install') + ': ' + (res.message || 'unknown error'), 'error');
        }
        loadMcpServers();
      }).catch(function(err) {
        showToast(I18n.t('ext.installFailed', { message: err.message }), 'error');
        loadMcpServers();
      });
    });
    actions.appendChild(installBtn);
  }

  card.appendChild(actions);
  return card;
}

function createReconfigureButton(extName) {
  var btn = document.createElement('button');
  btn.className = 'btn-ext configure';
  btn.textContent = I18n.t('ext.reconfigure');
  btn.addEventListener('click', function() { showConfigureModal(extName); });
  return btn;
}

function renderExtensionCard(ext) {
  const card = document.createElement('div');
  var stateClass = 'state-inactive';
  if (ext.kind === 'wasm_channel') {
    var s = ext.activation_status || 'installed';
    if (s === 'active') stateClass = 'state-active';
    else if (s === 'failed') stateClass = 'state-error';
    else if (s === 'pairing') stateClass = 'state-pairing';
  } else if (ext.active) {
    stateClass = 'state-active';
  }
  card.className = 'ext-card ' + stateClass;

  const header = document.createElement('div');
  header.className = 'ext-header';

  const name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = ext.display_name || ext.name;
  header.appendChild(name);

  const kind = document.createElement('span');
  kind.className = 'ext-kind kind-' + ext.kind;
  kind.textContent = kindLabels[ext.kind] || ext.kind;
  header.appendChild(kind);

  if (ext.version) {
    const ver = document.createElement('span');
    ver.className = 'ext-version';
    ver.textContent = 'v' + ext.version;
    header.appendChild(ver);
  }

  // Auth dot only for non-WASM-channel extensions (channels use the stepper instead)
  if (ext.kind !== 'wasm_channel') {
    const authDot = document.createElement('span');
    authDot.className = 'ext-auth-dot ' + (ext.authenticated ? 'authed' : 'unauthed');
    authDot.title = ext.authenticated ? 'Authenticated' : 'Not authenticated';
    header.appendChild(authDot);
  }

  card.appendChild(header);

  // WASM channels get a progress stepper
  if (ext.kind === 'wasm_channel') {
    card.appendChild(renderWasmChannelStepper(ext));
  }

  if (ext.description) {
    const desc = document.createElement('div');
    desc.className = 'ext-desc';
    desc.textContent = ext.description;
    card.appendChild(desc);
  }

  if (ext.url) {
    const url = document.createElement('div');
    url.className = 'ext-url';
    url.textContent = ext.url;
    url.title = ext.url;
    card.appendChild(url);
  }

  if (ext.tools && ext.tools.length > 0) {
    const tools = document.createElement('div');
    tools.className = 'ext-tools';
    tools.textContent = 'Tools: ' + ext.tools.join(', ');
    card.appendChild(tools);
  }

  // Show activation error for WASM channels
  if (ext.kind === 'wasm_channel' && ext.activation_error) {
    const errorDiv = document.createElement('div');
    errorDiv.className = 'ext-error';
    errorDiv.textContent = ext.activation_error;
    card.appendChild(errorDiv);
  }


  const actions = document.createElement('div');
  actions.className = 'ext-actions';

  if (ext.kind === 'wasm_channel') {
    // WASM channels: state-based buttons (no generic Activate)
    var status = ext.activation_status || 'installed';
    if (status === 'active') {
      var activeLabel = document.createElement('span');
      activeLabel.className = 'ext-active-label';
      activeLabel.textContent = I18n.t('ext.active');
      actions.appendChild(activeLabel);
      actions.appendChild(createReconfigureButton(ext.name));
    } else if (status === 'pairing') {
      var pairingLabel = document.createElement('span');
      pairingLabel.className = 'ext-pairing-label';
      pairingLabel.textContent = I18n.t('status.awaitingPairing');
      actions.appendChild(pairingLabel);
      actions.appendChild(createReconfigureButton(ext.name));
    } else if (status === 'failed') {
      actions.appendChild(createReconfigureButton(ext.name));
    } else {
      // installed or configured: show Setup button
      var setupBtn = document.createElement('button');
      setupBtn.className = 'btn-ext configure';
      setupBtn.textContent = I18n.t('ext.setup');
      setupBtn.addEventListener('click', function() { showConfigureModal(ext.name); });
      actions.appendChild(setupBtn);
    }
  } else {
    // WASM tools / MCP servers
    const activeLabel = document.createElement('span');
    activeLabel.className = 'ext-active-label';
    activeLabel.textContent = ext.active ? I18n.t('ext.active') : I18n.t('status.installed');
    actions.appendChild(activeLabel);

    // MCP servers and channel-relay extensions may be installed but inactive — show Activate button
    if ((ext.kind === 'mcp_server' || ext.kind === 'channel_relay') && !ext.active) {
      const activateBtn = document.createElement('button');
      activateBtn.className = 'btn-ext activate';
      activateBtn.textContent = I18n.t('common.activate');
      activateBtn.addEventListener('click', () => activateExtension(ext.name));
      actions.appendChild(activateBtn);
    }

    // Show Configure/Reconfigure button when there are secrets to enter.
    // Skip when has_auth is true but needs_setup is false and not yet authenticated —
    // this means OAuth credentials resolve automatically (builtin/env) and the user
    // just needs to complete the OAuth flow, not fill in a config form.
    if (ext.needs_setup || (ext.has_auth && ext.authenticated)) {
      const configBtn = document.createElement('button');
      configBtn.className = 'btn-ext configure';
      configBtn.textContent = ext.authenticated ? I18n.t('ext.reconfigure') : I18n.t('ext.configure');
      configBtn.addEventListener('click', () => showConfigureModal(ext.name));
      actions.appendChild(configBtn);
    }
  }

  const removeBtn = document.createElement('button');
  removeBtn.className = 'btn-ext remove';
  removeBtn.textContent = I18n.t('ext.remove');
  removeBtn.addEventListener('click', () => removeExtension(ext.name));
  actions.appendChild(removeBtn);

  card.appendChild(actions);

  // For WASM channels, check for pending pairing requests.
  if (ext.kind === 'wasm_channel') {
    const pairingSection = document.createElement('div');
    pairingSection.className = 'ext-pairing';
    pairingSection.setAttribute('data-channel', ext.name);
    card.appendChild(pairingSection);
    loadPairingRequests(ext.name, pairingSection);
  }

  return card;
}

function refreshCurrentSettingsTab() {
  if (currentSettingsSubtab === 'extensions') loadExtensions();
  if (currentSettingsSubtab === 'channels') loadChannelsStatus();
  if (currentSettingsSubtab === 'mcp') loadMcpServers();
}

function activateExtension(name) {
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/activate', { method: 'POST' })
    .then((res) => {
      if (res.success) {
        // Even on success, the tool may need OAuth (e.g., WASM loaded but no token yet)
        if (res.auth_url) {
          showAuthCard({
            extension_name: name,
            auth_url: res.auth_url,
          });
          showToast('Opening authentication for ' + name, 'info');
          openOAuthUrl(res.auth_url);
        }
        refreshCurrentSettingsTab();
        return;
      }

      if (res.auth_url) {
        showAuthCard({
          extension_name: name,
          auth_url: res.auth_url,
        });
        showToast('Opening authentication for ' + name, 'info');
        openOAuthUrl(res.auth_url);
      } else if (res.awaiting_token) {
        showConfigureModal(name);
      } else {
        showToast('Activate failed: ' + res.message, 'error');
      }
      refreshCurrentSettingsTab();
    })
    .catch((err) => showToast('Activate failed: ' + err.message, 'error'));
}

function removeExtension(name) {
  showConfirmModal(I18n.t('ext.confirmRemove', { name: name }), '', function() {
    apiFetch('/api/extensions/' + encodeURIComponent(name) + '/remove', { method: 'POST' })
      .then((res) => {
        if (!res.success) {
          showToast(I18n.t('ext.removeFailed', { message: res.message }), 'error');
        } else {
          showToast(I18n.t('ext.removed', { name: name }), 'success');
        }
        refreshCurrentSettingsTab();
      })
      .catch((err) => showToast(I18n.t('ext.removeFailed', { message: err.message }), 'error'));
  }, I18n.t('common.remove'), 'btn-danger');
}

function showConfigureModal(name) {
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup')
    .then((setup) => {
      if (!setup.secrets || setup.secrets.length === 0) {
        showToast('No configuration needed for ' + name, 'info');
        return;
      }
      renderConfigureModal(name, setup.secrets);
    })
    .catch((err) => showToast('Failed to load setup: ' + err.message, 'error'));
}

function renderConfigureModal(name, secrets) {
  closeConfigureModal();
  const overlay = document.createElement('div');
  overlay.className = 'configure-overlay';
  overlay.setAttribute('data-extension-name', name);
  overlay.dataset.telegramVerificationState = 'idle';
  overlay.addEventListener('click', (e) => {
    if (e.target !== overlay) return;
    if (name === 'telegram' && overlay.dataset.telegramVerificationState === 'waiting') return;
    closeConfigureModal();
  });

  const modal = document.createElement('div');
  modal.className = 'configure-modal';

  const header = document.createElement('h3');
  header.textContent = I18n.t('config.title', { name: name });
  modal.appendChild(header);

  if (name === 'telegram') {
    const hint = document.createElement('div');
    hint.className = 'configure-hint';
    hint.textContent = I18n.t('config.telegramOwnerHint');
    modal.appendChild(hint);
  }

  const form = document.createElement('div');
  form.className = 'configure-form';

  const fields = [];
  for (const secret of secrets) {
    const field = document.createElement('div');
    field.className = 'configure-field';
    field.dataset.secretName = secret.name;

    const label = document.createElement('label');
    label.textContent = secret.prompt;
    if (secret.optional) {
      const opt = document.createElement('span');
      opt.className = 'field-optional';
      opt.textContent = I18n.t('config.optional');
      label.appendChild(opt);
    }
    field.appendChild(label);

    const inputRow = document.createElement('div');
    inputRow.className = 'configure-input-row';

    const input = document.createElement('input');
    input.type = 'password';
    input.name = secret.name;
    input.placeholder = secret.provided ? I18n.t('config.alreadySet') : '';
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submitConfigureModal(name, fields);
    });
    inputRow.appendChild(input);

    if (secret.provided) {
      const badge = document.createElement('span');
      badge.className = 'field-provided';
      badge.textContent = '\u2713';
      badge.title = I18n.t('config.alreadyConfigured');
      inputRow.appendChild(badge);
    }
    if (secret.auto_generate && !secret.provided) {
      const hint = document.createElement('span');
      hint.className = 'field-autogen';
      hint.textContent = I18n.t('config.autoGenerate');
      inputRow.appendChild(hint);
    }

    field.appendChild(inputRow);
    form.appendChild(field);
    fields.push({ name: secret.name, input: input });
  }

  modal.appendChild(form);

  const error = document.createElement('div');
  error.className = 'configure-inline-error';
  error.style.display = 'none';
  modal.appendChild(error);

  const status = document.createElement('div');
  status.className = 'configure-inline-status';
  status.style.display = 'none';
  modal.appendChild(status);

  const actions = document.createElement('div');
  actions.className = 'configure-actions';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'btn-ext activate';
  submitBtn.textContent = I18n.t('config.save');
  submitBtn.addEventListener('click', () => submitConfigureModal(name, fields));
  actions.appendChild(submitBtn);

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'btn-ext remove';
  cancelBtn.textContent = I18n.t('config.cancel');
  cancelBtn.addEventListener('click', closeConfigureModal);
  actions.appendChild(cancelBtn);

  modal.appendChild(actions);
  overlay.appendChild(modal);
  document.body.appendChild(overlay);

  if (fields.length > 0) fields[0].input.focus();
}

function renderTelegramVerificationChallenge(overlay, verification) {
  if (!overlay || !verification) return;
  const modal = overlay.querySelector('.configure-modal');
  if (!modal) return;
  const telegramField = modal.querySelector('.configure-field[data-secret-name="telegram_bot_token"]');

  let panel = modal.querySelector('.configure-verification');
  if (!panel) {
    panel = document.createElement('div');
    panel.className = 'configure-verification';
  }
  if (telegramField && telegramField.parentNode) {
    telegramField.insertAdjacentElement('afterend', panel);
  } else {
    modal.insertBefore(
      panel,
      modal.querySelector('.configure-inline-error') || modal.querySelector('.configure-actions')
    );
  }

  panel.innerHTML = '';

  const title = document.createElement('div');
  title.className = 'configure-verification-title';
  title.textContent = I18n.t('config.telegramChallengeTitle');
  panel.appendChild(title);

  const instructions = document.createElement('div');
  instructions.className = 'configure-verification-instructions';
  instructions.textContent = verification.instructions;
  panel.appendChild(instructions);

  const commandLabel = document.createElement('div');
  commandLabel.className = 'configure-verification-instructions';
  commandLabel.textContent = I18n.t('config.telegramCommandLabel');
  panel.appendChild(commandLabel);

  const command = document.createElement('code');
  command.className = 'configure-verification-code';
  command.textContent = '/start ' + verification.code;
  panel.appendChild(command);

  if (verification.deep_link) {
    const link = document.createElement('a');
    link.className = 'configure-verification-link';
    link.href = verification.deep_link;
    link.target = '_blank';
    link.rel = 'noreferrer noopener';
    link.textContent = I18n.t('config.telegramOpenBot');
    panel.appendChild(link);
  }
}

function getConfigurePrimaryButton(overlay) {
  return overlay && overlay.querySelector('.configure-actions button.btn-ext.activate');
}

function getConfigureCancelButton(overlay) {
  return overlay && overlay.querySelector('.configure-actions button.btn-ext.remove');
}

function setConfigureInlineError(overlay, message) {
  const error = overlay && overlay.querySelector('.configure-inline-error');
  if (!error) return;
  error.textContent = message || '';
  error.style.display = message ? 'block' : 'none';
}

function clearConfigureInlineError(overlay) {
  setConfigureInlineError(overlay, '');
}

function setConfigureInlineStatus(overlay, message) {
  const status = overlay && overlay.querySelector('.configure-inline-status');
  if (!status) return;
  status.textContent = message || '';
  status.style.display = message ? 'block' : 'none';
}

function setTelegramConfigureState(overlay, fields, state) {
  if (!overlay) return;
  overlay.dataset.telegramVerificationState = state;

  const primaryBtn = getConfigurePrimaryButton(overlay);
  const cancelBtn = getConfigureCancelButton(overlay);
  const waiting = state === 'waiting';
  const retry = state === 'retry';

  setConfigureInlineStatus(overlay, waiting ? I18n.t('config.telegramOwnerWaiting') : '');

  if (primaryBtn) {
    primaryBtn.style.display = waiting ? 'none' : '';
    primaryBtn.disabled = false;
    primaryBtn.textContent = retry ? I18n.t('config.telegramStartOver') : I18n.t('config.save');
  }
  if (cancelBtn) cancelBtn.disabled = waiting;
}

function startTelegramAutoVerify(name, fields) {
  window.setTimeout(() => submitConfigureModal(name, fields, { telegramAutoVerify: true }), 0);
}

function submitConfigureModal(name, fields, options) {
  options = options || {};
  const secrets = {};
  for (const f of fields) {
    if (f.input.value.trim()) {
      secrets[f.name] = f.input.value.trim();
    }
  }

  const overlay = getConfigureOverlay(name) || document.querySelector('.configure-overlay');
  const isTelegram = name === 'telegram';
  clearConfigureInlineError(overlay);

  // Disable buttons to prevent double-submit
  var btns = overlay ? overlay.querySelectorAll('.configure-actions button') : [];
  btns.forEach(function(b) { b.disabled = true; });
  if (overlay && isTelegram) {
    setTelegramConfigureState(overlay, fields, 'waiting');
  }

  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup', {
    method: 'POST',
    body: { secrets },
  })
    .then((res) => {
      if (res.success) {
        if (res.verification && isTelegram) {
          renderTelegramVerificationChallenge(overlay, res.verification);
          fields.forEach(function(f) { f.input.value = ''; });
          setTelegramConfigureState(overlay, fields, 'waiting');
          // Once the verification challenge is rendered inline, the global auth lock
          // should not keep the chat composer disabled for this setup-driven flow.
          setAuthFlowPending(false);
          enableChatInput();
          if (!options.telegramAutoVerify) {
            startTelegramAutoVerify(name, fields);
            return;
          }
          setTelegramConfigureState(overlay, fields, 'retry');
          setConfigureInlineError(overlay, I18n.t('config.telegramStartOverHint'));
          return;
        }

        closeConfigureModal();
        if (res.auth_url) {
          showAuthCard({
            extension_name: name,
            auth_url: res.auth_url,
          });
          showToast('Opening OAuth authorization for ' + name, 'info');
          openOAuthUrl(res.auth_url);
          refreshCurrentSettingsTab();
        }
        // For non-OAuth success: the server always broadcasts auth_completed SSE,
        // which will show the toast and refresh extensions — no need to do it here too.
      } else {
        // Keep modal open so the user can correct their input and retry.
        btns.forEach(function(b) { b.disabled = false; });
        setConfigureInlineError(overlay, res.message || 'Configuration failed');
        if (isTelegram) {
          const hasVerification = overlay && overlay.querySelector('.configure-verification');
          if (options.telegramAutoVerify || hasVerification) {
            setTelegramConfigureState(overlay, fields, 'retry');
          } else {
            setTelegramConfigureState(overlay, fields, 'idle');
          }
        }
        showToast(res.message || 'Configuration failed', 'error');
      }
    })
    .catch((err) => {
      btns.forEach(function(b) { b.disabled = false; });
      setConfigureInlineError(overlay, 'Configuration failed: ' + err.message);
      if (isTelegram) {
        const hasVerification = overlay && overlay.querySelector('.configure-verification');
        if (options.telegramAutoVerify || hasVerification) {
          setTelegramConfigureState(overlay, fields, 'retry');
        } else {
          setTelegramConfigureState(overlay, fields, 'idle');
        }
      }
      showToast('Configuration failed: ' + err.message, 'error');
    });
}

function closeConfigureModal(extensionName) {
  if (typeof extensionName !== 'string') extensionName = null;
  const existing = getConfigureOverlay(extensionName);
  if (existing) existing.remove();
  if (!document.querySelector('.configure-overlay') && !document.querySelector('.auth-card')) {
    setAuthFlowPending(false);
    enableChatInput();
  }
}

// Validate that a server-supplied OAuth URL is HTTPS before opening a popup.
// Rejects javascript:, data:, and other non-HTTPS schemes to prevent URL-injection.
// Uses the URL constructor to safely parse and validate the scheme, which also
// handles non-string values (objects, null, etc.) that would throw on .startsWith().
function openOAuthUrl(url) {
  let parsed;
  try {
    parsed = new URL(url);
    if (parsed.protocol !== 'https:') {
      throw new Error('non-HTTPS protocol: ' + parsed.protocol);
    }
  } catch (e) {
    console.warn('Blocked invalid/non-HTTPS OAuth URL:', url, e.message);
    showToast('Invalid OAuth URL returned by server', 'error');
    return;
  }
  window.open(parsed.href, '_blank', 'width=600,height=700');
}

// --- Pairing ---

function loadPairingRequests(channel, container) {
  apiFetch('/api/pairing/' + encodeURIComponent(channel))
    .then(data => {
      container.innerHTML = '';
      if (!data.requests || data.requests.length === 0) return;

      const heading = document.createElement('div');
      heading.className = 'pairing-heading';
      heading.textContent = 'Pending pairing requests';
      container.appendChild(heading);

      data.requests.forEach(req => {
        const row = document.createElement('div');
        row.className = 'pairing-row';

        const code = document.createElement('span');
        code.className = 'pairing-code';
        code.textContent = req.code;
        row.appendChild(code);

        const sender = document.createElement('span');
        sender.className = 'pairing-sender';
        sender.textContent = 'from ' + req.sender_id;
        row.appendChild(sender);

        const btn = document.createElement('button');
        btn.className = 'btn-ext activate';
        btn.textContent = 'Approve';
        btn.addEventListener('click', () => approvePairing(channel, req.code, container));
        row.appendChild(btn);

        container.appendChild(row);
      });
    })
    .catch(() => {});
}

function approvePairing(channel, code, container) {
  apiFetch('/api/pairing/' + encodeURIComponent(channel) + '/approve', {
    method: 'POST',
    body: { code },
  }).then(res => {
    if (res.success) {
      showToast('Pairing approved', 'success');
      refreshCurrentSettingsTab();
    } else {
      showToast(res.message || 'Approve failed', 'error');
    }
  }).catch(err => showToast('Error: ' + err.message, 'error'));
}

function startPairingPoll() {
  stopPairingPoll();
  pairingPollInterval = setInterval(function() {
    document.querySelectorAll('.ext-pairing[data-channel]').forEach(function(el) {
      loadPairingRequests(el.getAttribute('data-channel'), el);
    });
  }, 10000);
}

function stopPairingPoll() {
  if (pairingPollInterval) {
    clearInterval(pairingPollInterval);
    pairingPollInterval = null;
  }
}

// --- WASM channel stepper ---

function renderWasmChannelStepper(ext) {
  var stepper = document.createElement('div');
  stepper.className = 'ext-stepper';

  var status = ext.activation_status || 'installed';

  var steps = [
    { label: 'Installed', key: 'installed' },
    { label: 'Configured', key: 'configured' },
    { label: status === 'pairing' ? 'Awaiting Pairing' : 'Active', key: 'active' },
  ];

  var reachedIdx;
  if (status === 'active') reachedIdx = 2;
  else if (status === 'pairing') reachedIdx = 2;
  else if (status === 'failed') reachedIdx = 2;
  else if (status === 'configured') reachedIdx = 1;
  else reachedIdx = 0;

  for (var i = 0; i < steps.length; i++) {
    if (i > 0) {
      var connector = document.createElement('div');
      connector.className = 'stepper-connector' + (i <= reachedIdx ? ' completed' : '');
      stepper.appendChild(connector);
    }

    var step = document.createElement('div');
    var stepState;
    if (i < reachedIdx) {
      stepState = 'completed';
    } else if (i === reachedIdx) {
      if (status === 'failed') {
        stepState = 'failed';
      } else if (status === 'pairing') {
        stepState = 'in-progress';
      } else if (status === 'active' || status === 'configured' || status === 'installed') {
        stepState = 'completed';
      } else {
        stepState = 'pending';
      }
    } else {
      stepState = 'pending';
    }
    step.className = 'stepper-step ' + stepState;

    var circle = document.createElement('span');
    circle.className = 'stepper-circle';
    if (stepState === 'completed') circle.textContent = '\u2713';
    else if (stepState === 'failed') circle.textContent = '\u2717';
    step.appendChild(circle);

    var label = document.createElement('span');
    label.className = 'stepper-label';
    label.textContent = steps[i].label;
    step.appendChild(label);

    stepper.appendChild(step);
  }

  return stepper;
}

// --- Jobs ---

let currentJobId = null;
let currentJobSubTab = 'overview';
let jobFilesTreeState = null;

function loadJobs() {
  currentJobId = null;
  jobFilesTreeState = null;

  // Rebuild DOM if renderJobDetail() destroyed it (it wipes .jobs-container innerHTML).
  const container = document.querySelector('.jobs-container');
  if (!document.getElementById('jobs-summary')) {
    container.innerHTML =
      '<div class="jobs-summary" id="jobs-summary"></div>'
      + '<table class="jobs-table" id="jobs-table"><thead><tr>'
      + '<th>ID</th><th>Title</th><th>Status</th><th>Created</th><th>Actions</th>'
      + '</tr></thead><tbody id="jobs-tbody"></tbody></table>'
      + '<div class="empty-state" id="jobs-empty" style="display:none">No jobs found</div>';
  }

  Promise.all([
    apiFetch('/api/jobs/summary'),
    apiFetch('/api/jobs'),
  ]).then(([summary, jobList]) => {
    renderJobsSummary(summary);
    renderJobsList(jobList.jobs);
  }).catch(() => {});
}

function renderJobsSummary(s) {
  document.getElementById('jobs-summary').innerHTML = ''
    + summaryCard(I18n.t('jobs.summary.total'), s.total, '')
    + summaryCard(I18n.t('jobs.summary.inProgress'), s.in_progress, 'active')
    + summaryCard(I18n.t('jobs.summary.completed'), s.completed, 'completed')
    + summaryCard(I18n.t('jobs.summary.failed'), s.failed, 'failed')
    + summaryCard(I18n.t('jobs.summary.stuck'), s.stuck, 'stuck');
}

function summaryCard(label, count, cls) {
  return '<div class="summary-card ' + cls + '">'
    + '<div class="count">' + count + '</div>'
    + '<div class="label">' + label + '</div>'
    + '</div>';
}

function renderJobsList(jobs) {
  const tbody = document.getElementById('jobs-tbody');
  const empty = document.getElementById('jobs-empty');

  if (jobs.length === 0) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = jobs.map((job) => {
    const shortId = job.id.substring(0, 8);
    const stateClass = job.state.replace(' ', '_');

    let actionBtns = '';
    if (job.state === 'pending' || job.state === 'in_progress') {
      actionBtns = '<button class="btn-cancel" data-action="cancel-job" data-id="' + escapeHtml(job.id) + '">Cancel</button>';
    }
    // Retry is only shown in the detail view where can_restart is available.

    return '<tr class="job-row" data-action="open-job" data-id="' + escapeHtml(job.id) + '">'
      + '<td title="' + escapeHtml(job.id) + '">' + shortId + '</td>'
      + '<td>' + escapeHtml(job.title) + '</td>'
      + '<td><span class="badge ' + stateClass + '">' + escapeHtml(job.state) + '</span></td>'
      + '<td>' + formatDate(job.created_at) + '</td>'
      + '<td>' + actionBtns + '</td>'
      + '</tr>';
  }).join('');
}

function cancelJob(jobId) {
  if (!confirm('Cancel this job?')) return;
  apiFetch('/api/jobs/' + jobId + '/cancel', { method: 'POST' })
    .then(() => {
      showToast('Job cancelled', 'success');
      if (currentJobId) openJobDetail(currentJobId);
      else loadJobs();
    })
    .catch((err) => {
      showToast('Failed to cancel job: ' + err.message, 'error');
    });
}

function restartJob(jobId) {
  apiFetch('/api/jobs/' + jobId + '/restart', { method: 'POST' })
    .then((res) => {
      showToast('Job restarted as ' + (res.new_job_id || '').substring(0, 8), 'success');
    })
    .catch((err) => {
      showToast('Failed to restart job: ' + err.message, 'error');
    })
    .finally(() => {
      loadJobs();
    });
}

function openJobDetail(jobId) {
  currentJobId = jobId;
  currentJobSubTab = 'activity';
  apiFetch('/api/jobs/' + jobId).then((job) => {
    renderJobDetail(job);
  }).catch((err) => {
    addMessage('system', 'Failed to load job: ' + err.message);
    closeJobDetail();
  });
}

function closeJobDetail() {
  currentJobId = null;
  jobFilesTreeState = null;
  loadJobs();
}

function renderJobDetail(job) {
  const container = document.querySelector('.jobs-container');
  const stateClass = job.state.replace(' ', '_');

  container.innerHTML = '';

  // Header
  const header = document.createElement('div');
  header.className = 'job-detail-header';

  let headerHtml = '<button class="btn-back" data-action="close-job-detail">&larr; Back</button>'
    + '<h2>' + escapeHtml(job.title) + '</h2>'
    + '<span class="badge ' + stateClass + '">' + escapeHtml(job.state) + '</span>';

  if ((job.state === 'failed' || job.state === 'interrupted') && job.can_restart === true) {
    headerHtml += '<button class="btn-restart" data-action="restart-job" data-id="' + escapeHtml(job.id) + '">Retry</button>';
  }
  if (job.browse_url) {
    headerHtml += '<a class="btn-browse" href="' + escapeHtml(job.browse_url) + '" target="_blank">Browse Files</a>';
  }

  header.innerHTML = headerHtml;
  container.appendChild(header);

  // Sub-tab bar
  const tabs = document.createElement('div');
  tabs.className = 'job-detail-tabs';
  const subtabs = ['overview', 'activity', 'files'];
  for (const st of subtabs) {
    const btn = document.createElement('button');
    btn.textContent = st.charAt(0).toUpperCase() + st.slice(1);
    btn.className = st === currentJobSubTab ? 'active' : '';
    btn.addEventListener('click', () => {
      currentJobSubTab = st;
      renderJobDetail(job);
    });
    tabs.appendChild(btn);
  }
  container.appendChild(tabs);

  // Content
  const content = document.createElement('div');
  content.className = 'job-detail-content';
  container.appendChild(content);

  switch (currentJobSubTab) {
    case 'overview': renderJobOverview(content, job); break;
    case 'files': renderJobFiles(content, job); break;
    case 'activity': renderJobActivity(content, job); break;
  }
}

function metaItem(label, value) {
  return '<div class="meta-item"><div class="meta-label">' + escapeHtml(label)
    + '</div><div class="meta-value">' + escapeHtml(String(value != null ? value : '-'))
    + '</div></div>';
}

function formatDuration(secs) {
  if (secs == null) return '-';
  if (secs < 60) return secs + 's';
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  if (m < 60) return m + 'm ' + s + 's';
  const h = Math.floor(m / 60);
  return h + 'h ' + (m % 60) + 'm';
}

function renderJobOverview(container, job) {
  // Metadata grid
  const grid = document.createElement('div');
  grid.className = 'job-meta-grid';
  grid.innerHTML = metaItem('Job ID', job.id)
    + metaItem('State', job.state)
    + metaItem('Created', formatDate(job.created_at))
    + metaItem('Started', formatDate(job.started_at))
    + metaItem('Completed', formatDate(job.completed_at))
    + metaItem('Duration', formatDuration(job.elapsed_secs))
    + (job.job_mode ? metaItem('Mode', job.job_mode) : '');
  container.appendChild(grid);

  // Description
  if (job.description) {
    const descSection = document.createElement('div');
    descSection.className = 'job-description';
    const descHeader = document.createElement('h3');
    descHeader.textContent = 'Description';
    descSection.appendChild(descHeader);
    const descBody = document.createElement('div');
    descBody.className = 'job-description-body';
    descBody.innerHTML = renderMarkdown(job.description);
    descSection.appendChild(descBody);
    container.appendChild(descSection);
  }

  // State transitions timeline
  if (job.transitions.length > 0) {
    const timelineSection = document.createElement('div');
    timelineSection.className = 'job-timeline-section';
    const tlHeader = document.createElement('h3');
    tlHeader.textContent = 'State Transitions';
    timelineSection.appendChild(tlHeader);

    const timeline = document.createElement('div');
    timeline.className = 'timeline';
    for (const t of job.transitions) {
      const entry = document.createElement('div');
      entry.className = 'timeline-entry';
      const dot = document.createElement('div');
      dot.className = 'timeline-dot';
      entry.appendChild(dot);
      const info = document.createElement('div');
      info.className = 'timeline-info';
      info.innerHTML = '<span class="badge ' + t.from.replace(' ', '_') + '">' + escapeHtml(t.from) + '</span>'
        + ' &rarr; '
        + '<span class="badge ' + t.to.replace(' ', '_') + '">' + escapeHtml(t.to) + '</span>'
        + '<span class="timeline-time">' + formatDate(t.timestamp) + '</span>'
        + (t.reason ? '<div class="timeline-reason">' + escapeHtml(t.reason) + '</div>' : '');
      entry.appendChild(info);
      timeline.appendChild(entry);
    }
    timelineSection.appendChild(timeline);
    container.appendChild(timelineSection);
  }
}

function renderJobFiles(container, job) {
  container.innerHTML = '<div class="job-files">'
    + '<div class="job-files-sidebar"><div class="job-files-tree"></div></div>'
    + '<div class="job-files-viewer"><div class="empty-state">Select a file to view</div></div>'
    + '</div>';

  container._jobId = job ? job.id : null;

  apiFetch('/api/jobs/' + job.id + '/files/list?path=').then((data) => {
    jobFilesTreeState = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    renderJobFilesTree();
  }).catch(() => {
    const treeContainer = document.querySelector('.job-files-tree');
    if (treeContainer) {
      treeContainer.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No project files</div>';
    }
  });
}

function renderJobFilesTree() {
  const treeContainer = document.querySelector('.job-files-tree');
  if (!treeContainer) return;
  treeContainer.innerHTML = '';
  if (!jobFilesTreeState || jobFilesTreeState.length === 0) {
    treeContainer.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No files in workspace</div>';
    return;
  }
  renderJobFileNodes(jobFilesTreeState, treeContainer, 0);
}

function renderJobFileNodes(nodes, container, depth) {
  for (const node of nodes) {
    const row = document.createElement('div');
    row.className = 'tree-row';
    row.style.paddingLeft = (depth * 16 + 8) + 'px';

    if (node.is_dir) {
      const arrow = document.createElement('span');
      arrow.className = 'expand-arrow' + (node.expanded ? ' expanded' : '');
      arrow.textContent = '\u25B6';
      arrow.addEventListener('click', (e) => {
        e.stopPropagation();
        toggleJobFileExpand(node);
      });
      row.appendChild(arrow);

      const label = document.createElement('span');
      label.className = 'tree-label dir';
      label.textContent = node.name;
      label.addEventListener('click', () => toggleJobFileExpand(node));
      row.appendChild(label);
    } else {
      const spacer = document.createElement('span');
      spacer.className = 'expand-arrow-spacer';
      row.appendChild(spacer);

      const label = document.createElement('span');
      label.className = 'tree-label file';
      label.textContent = node.name;
      label.addEventListener('click', () => readJobFile(node.path));
      row.appendChild(label);
    }

    container.appendChild(row);

    if (node.is_dir && node.expanded && node.children) {
      const childContainer = document.createElement('div');
      childContainer.className = 'tree-children';
      renderJobFileNodes(node.children, childContainer, depth + 1);
      container.appendChild(childContainer);
    }
  }
}

function getJobId() {
  const container = document.querySelector('.job-detail-content');
  return (container && container._jobId) || null;
}

function toggleJobFileExpand(node) {
  if (node.expanded) {
    node.expanded = false;
    renderJobFilesTree();
    return;
  }
  if (node.loaded) {
    node.expanded = true;
    renderJobFilesTree();
    return;
  }
  const jobId = getJobId();
  apiFetch('/api/jobs/' + jobId + '/files/list?path=' + encodeURIComponent(node.path)).then((data) => {
    node.children = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    node.loaded = true;
    node.expanded = true;
    renderJobFilesTree();
  }).catch(() => {});
}

function readJobFile(path) {
  const viewer = document.querySelector('.job-files-viewer');
  if (!viewer) return;
  const jobId = getJobId();
  apiFetch('/api/jobs/' + jobId + '/files/read?path=' + encodeURIComponent(path)).then((data) => {
    viewer.innerHTML = '<div class="job-files-path">' + escapeHtml(path) + '</div>'
      + '<pre class="job-files-content">' + escapeHtml(data.content) + '</pre>';
  }).catch((err) => {
    viewer.innerHTML = '<div class="empty-state">Error: ' + escapeHtml(err.message) + '</div>';
  });
}

// --- Activity tab (unified for all sandbox jobs) ---

let activityCurrentJobId = null;
// Track how many live SSE events we've already rendered so refreshActivityTab
// only appends new ones (avoids duplicates on each SSE tick).
let activityRenderedLiveIndex = 0;

function renderJobActivity(container, job) {
  activityCurrentJobId = job ? job.id : null;
  activityRenderedLiveIndex = 0;

  let html = '<div class="activity-toolbar">'
    + '<select id="activity-type-filter">'
    + '<option value="all">All Events</option>'
    + '<option value="message">Messages</option>'
    + '<option value="tool_use">Tool Calls</option>'
    + '<option value="tool_result">Results</option>'
    + '</select>'
    + '<label class="logs-checkbox"><input type="checkbox" id="activity-autoscroll" checked> Auto-scroll</label>'
    + '</div>'
    + '<div class="activity-terminal" id="activity-terminal"></div>';

  if (job && job.can_prompt === true) {
    html += '<div class="activity-input-bar" id="activity-input-bar">'
      + '<input type="text" id="activity-prompt-input" placeholder="Send follow-up prompt..." />'
      + '<button id="activity-send-btn">Send</button>'
      + '<button id="activity-done-btn" title="Signal done">Done</button>'
      + '</div>';
  }

  container.innerHTML = html;

  document.getElementById('activity-type-filter').addEventListener('change', applyActivityFilter);

  const terminal = document.getElementById('activity-terminal');
  const input = document.getElementById('activity-prompt-input');
  const sendBtn = document.getElementById('activity-send-btn');
  const doneBtn = document.getElementById('activity-done-btn');

  if (sendBtn) sendBtn.addEventListener('click', () => sendJobPrompt(job.id, false));
  if (doneBtn) doneBtn.addEventListener('click', () => sendJobPrompt(job.id, true));
  if (input) input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') sendJobPrompt(job.id, false);
  });

  // Load persisted events from DB, then catch up with any live SSE events
  apiFetch('/api/jobs/' + job.id + '/events').then((data) => {
    if (data.events && data.events.length > 0) {
      for (const evt of data.events) {
        appendActivityEvent(terminal, evt.event_type, evt.data);
      }
    }
    appendNewLiveEvents(terminal, job.id);
  }).catch(() => {
    appendNewLiveEvents(terminal, job.id);
  });
}

function appendNewLiveEvents(terminal, jobId) {
  const live = jobEvents.get(jobId) || [];
  for (let i = activityRenderedLiveIndex; i < live.length; i++) {
    const evt = live[i];
    appendActivityEvent(terminal, evt.type.replace('job_', ''), evt.data);
  }
  activityRenderedLiveIndex = live.length;
  const autoScroll = document.getElementById('activity-autoscroll');
  if (!autoScroll || autoScroll.checked) {
    terminal.scrollTop = terminal.scrollHeight;
  }
}

function applyActivityFilter() {
  const filter = document.getElementById('activity-type-filter').value;
  const events = document.querySelectorAll('#activity-terminal .activity-event');
  for (const el of events) {
    if (filter === 'all') {
      el.style.display = '';
    } else {
      el.style.display = el.getAttribute('data-event-type') === filter ? '' : 'none';
    }
  }
}

function appendActivityEvent(terminal, eventType, data) {
  if (!terminal) return;
  const el = document.createElement('div');
  el.className = 'activity-event activity-event-' + eventType;
  el.setAttribute('data-event-type', eventType);

  // Respect current filter
  const filterEl = document.getElementById('activity-type-filter');
  if (filterEl && filterEl.value !== 'all' && filterEl.value !== eventType) {
    el.style.display = 'none';
  }

  switch (eventType) {
    case 'message':
      el.innerHTML = '<span class="activity-role">' + escapeHtml(data.role || 'assistant') + '</span> '
        + '<span class="activity-content">' + escapeHtml(data.content || '') + '</span>';
      break;
    case 'tool_use':
      el.innerHTML = '<details class="activity-tool-block"><summary>'
        + '<span class="activity-tool-icon">&#9881;</span> '
        + escapeHtml(data.tool_name || 'tool')
        + '</summary><pre class="activity-tool-input">'
        + escapeHtml(typeof data.input === 'string' ? data.input : JSON.stringify(data.input, null, 2))
        + '</pre></details>';
      break;
    case 'tool_result': {
      const trSuccess = data.success !== false;
      const trIcon = trSuccess ? '&#10003;' : '&#10007;';
      const trOutput = data.output || data.error || '';
      const trClass = 'activity-tool-block activity-tool-result'
        + (trSuccess ? '' : ' activity-tool-error');
      el.innerHTML = '<details class="' + trClass + '"><summary>'
        + '<span class="activity-tool-icon">' + trIcon + '</span> '
        + escapeHtml(data.tool_name || 'result')
        + '</summary><pre class="activity-tool-output">'
        + escapeHtml(trOutput)
        + '</pre></details>';
      break;
    }
    case 'status':
      el.innerHTML = '<span class="activity-status">' + escapeHtml(data.message || '') + '</span>';
      break;
    case 'result':
      el.className += ' activity-final';
      const success = data.success !== false;
      el.innerHTML = '<span class="activity-result-status" data-success="' + success + '">'
        + escapeHtml(data.message || data.error || data.status || 'done') + '</span>';
      if (data.session_id) {
        el.innerHTML += ' <span class="activity-session-id">session: ' + escapeHtml(data.session_id) + '</span>';
      }
      break;
    default:
      el.innerHTML = '<span class="activity-status">' + escapeHtml(JSON.stringify(data)) + '</span>';
  }

  terminal.appendChild(el);
}

function refreshActivityTab(jobId) {
  if (activityCurrentJobId !== jobId) return;
  if (currentJobSubTab !== 'activity') return;
  const terminal = document.getElementById('activity-terminal');
  if (!terminal) return;
  appendNewLiveEvents(terminal, jobId);
}

function sendJobPrompt(jobId, done) {
  const input = document.getElementById('activity-prompt-input');
  const content = input ? input.value.trim() : '';
  if (!content && !done) return;

  apiFetch('/api/jobs/' + jobId + '/prompt', {
    method: 'POST',
    body: { content: content || '(done)', done: done },
  }).then(() => {
    if (input) input.value = '';
    if (done) {
      const bar = document.getElementById('activity-input-bar');
      if (bar) bar.innerHTML = '<span class="activity-status">Done signal sent</span>';
    }
  }).catch((err) => {
    const terminal = document.getElementById('activity-terminal');
    if (terminal) {
      appendActivityEvent(terminal, 'status', { message: 'Failed to send: ' + err.message });
    }
  });
}

// --- Routines ---

let currentRoutineId = null;

function loadRoutines() {
  currentRoutineId = null;

  // Restore list view if detail was open
  const detail = document.getElementById('routine-detail');
  if (detail) detail.style.display = 'none';
  const table = document.getElementById('routines-table');
  if (table) table.style.display = '';

  Promise.all([
    apiFetch('/api/routines/summary'),
    apiFetch('/api/routines'),
  ]).then(([summary, listData]) => {
    renderRoutinesSummary(summary);
    renderRoutinesList(listData.routines);
  }).catch(() => {});
}

function renderRoutinesSummary(s) {
  document.getElementById('routines-summary').innerHTML = ''
    + summaryCard(I18n.t('routines.summary.total'), s.total, '')
    + summaryCard(I18n.t('routines.summary.enabled'), s.enabled, 'active')
    + summaryCard(I18n.t('routines.summary.disabled'), s.disabled, '')
    + summaryCard(I18n.t('routines.summary.failing'), s.failing, 'failed')
    + summaryCard(I18n.t('routines.summary.runsToday'), s.runs_today, 'completed');
}

function renderRoutinesList(routines) {
  const tbody = document.getElementById('routines-tbody');
  const empty = document.getElementById('routines-empty');

  if (!routines || routines.length === 0) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = routines.map((r) => {
    const statusClass = r.status === 'active' ? 'completed'
      : r.status === 'failing' ? 'failed'
      : 'pending';

    const toggleLabel = r.enabled ? 'Disable' : 'Enable';
    const toggleClass = r.enabled ? 'btn-cancel' : 'btn-restart';
    const triggerTitle = (r.trigger_type === 'cron' && r.trigger_raw)
      ? ' title="' + escapeHtml(r.trigger_raw) + '"'
      : '';

    return '<tr class="routine-row" data-action="open-routine" data-id="' + escapeHtml(r.id) + '">'
      + '<td>' + escapeHtml(r.name) + '</td>'
      + '<td' + triggerTitle + '>' + escapeHtml(r.trigger_summary) + '</td>'
      + '<td>' + escapeHtml(r.action_type) + '</td>'
      + '<td>' + formatRelativeTime(r.last_run_at) + '</td>'
      + '<td>' + formatRelativeTime(r.next_fire_at) + '</td>'
      + '<td>' + r.run_count + '</td>'
      + '<td><span class="badge ' + statusClass + '">' + escapeHtml(r.status) + '</span></td>'
      + '<td>'
      + '<button class="' + toggleClass + '" data-action="toggle-routine" data-id="' + escapeHtml(r.id) + '">' + toggleLabel + '</button> '
      + '<button class="btn-restart" data-action="trigger-routine" data-id="' + escapeHtml(r.id) + '">Run</button> '
      + '<button class="btn-cancel" data-action="delete-routine" data-id="' + escapeHtml(r.id) + '" data-name="' + escapeHtml(r.name) + '">Delete</button>'
      + '</td>'
      + '</tr>';
  }).join('');
}

function openRoutineDetail(id) {
  currentRoutineId = id;
  apiFetch('/api/routines/' + id).then((routine) => {
    renderRoutineDetail(routine);
  }).catch((err) => {
    showToast('Failed to load routine: ' + err.message, 'error');
  });
}

function closeRoutineDetail() {
  currentRoutineId = null;
  loadRoutines();
}

function renderRoutineDetail(routine) {
  const table = document.getElementById('routines-table');
  if (table) table.style.display = 'none';
  document.getElementById('routines-empty').style.display = 'none';

  const detail = document.getElementById('routine-detail');
  detail.style.display = 'block';

  const statusClass = !routine.enabled ? 'pending'
    : routine.consecutive_failures > 0 ? 'failed'
    : 'completed';
  const statusLabel = !routine.enabled ? 'disabled'
    : routine.consecutive_failures > 0 ? 'failing'
    : 'active';

  let html = '<div class="job-detail-header">'
    + '<button class="btn-back" data-action="close-routine-detail">&larr; Back</button>'
    + '<h2>' + escapeHtml(routine.name) + '</h2>'
    + '<span class="badge ' + statusClass + '">' + escapeHtml(statusLabel) + '</span>'
    + '</div>';

  // Metadata grid
  html += '<div class="job-meta-grid">'
    + metaItem('Routine ID', routine.id)
    + metaItem('Enabled', routine.enabled ? 'Yes' : 'No')
    + metaItem('Run Count', routine.run_count)
    + metaItem('Failures', routine.consecutive_failures)
    + metaItem('Last Run', formatDate(routine.last_run_at))
    + metaItem('Next Fire', formatDate(routine.next_fire_at))
    + metaItem('Created', formatDate(routine.created_at))
    + '</div>';

  // Description
  if (routine.description) {
    html += '<div class="job-description"><h3>Description</h3>'
      + '<div class="job-description-body">' + escapeHtml(routine.description) + '</div></div>';
  }

  // Trigger config
  if (routine.trigger_type === 'cron') {
    const summary = routine.trigger_summary || 'cron';
    const raw = routine.trigger_raw || '';
    const timezone = routine.trigger && routine.trigger.timezone ? String(routine.trigger.timezone) : '';
    html += '<div class="job-description"><h3>Trigger</h3>'
      + '<div class="job-description-body"><strong>' + escapeHtml(summary) + '</strong></div>';
    if (raw) {
      html += '<div class="job-meta-item">'
        + '<span class="job-meta-label">Raw</span>'
        + '<span class="job-meta-value">' + escapeHtml(raw + (timezone ? ' (' + timezone + ')' : '')) + '</span>'
        + '</div>';
    }
    html += '</div>';
  } else {
    html += '<div class="job-description"><h3>Trigger</h3>'
      + '<pre class="action-json">' + escapeHtml(JSON.stringify(routine.trigger, null, 2)) + '</pre></div>';
  }

  html += '<div class="job-description"><h3>Action</h3>'
    + '<pre class="action-json">' + escapeHtml(JSON.stringify(routine.action, null, 2)) + '</pre></div>';

  // Recent runs
  if (routine.recent_runs && routine.recent_runs.length > 0) {
    html += '<div class="job-timeline-section"><h3>Recent Runs</h3>'
      + '<table class="routines-table"><thead><tr>'
      + '<th>Trigger</th><th>Started</th><th>Completed</th><th>Status</th><th>Summary</th><th>Tokens</th>'
      + '</tr></thead><tbody>';
    for (const run of routine.recent_runs) {
      const runStatusClass = run.status === 'Ok' ? 'completed'
        : run.status === 'Failed' ? 'failed'
        : run.status === 'Attention' ? 'stuck'
        : 'in_progress';
      html += '<tr>'
        + '<td>' + escapeHtml(run.trigger_type) + '</td>'
        + '<td>' + formatDate(run.started_at) + '</td>'
        + '<td>' + formatDate(run.completed_at) + '</td>'
        + '<td><span class="badge ' + runStatusClass + '">' + escapeHtml(run.status) + '</span></td>'
        + '<td>' + escapeHtml(run.result_summary || '-')
          + (run.job_id ? ' <a href="#" data-action="view-run-job" data-id="' + escapeHtml(run.job_id) + '">[view job]</a>' : '')
          + '</td>'
        + '<td>' + (run.tokens_used != null ? run.tokens_used : '-') + '</td>'
        + '</tr>';
    }
    html += '</tbody></table></div>';
  }

  detail.innerHTML = html;
}

function triggerRoutine(id) {
  apiFetch('/api/routines/' + id + '/trigger', { method: 'POST' })
    .then(() => {
      showToast('Routine triggered', 'success');
      if (currentRoutineId === id) openRoutineDetail(id);
      else loadRoutines();
    })
    .catch((err) => showToast('Trigger failed: ' + err.message, 'error'));
}

function toggleRoutine(id) {
  apiFetch('/api/routines/' + id + '/toggle', { method: 'POST' })
    .then((res) => {
      showToast('Routine ' + (res.status || 'toggled'), 'success');
      if (currentRoutineId) openRoutineDetail(currentRoutineId);
      else loadRoutines();
    })
    .catch((err) => showToast('Toggle failed: ' + err.message, 'error'));
}

function deleteRoutine(id, name) {
  if (!confirm('Delete routine "' + name + '"?')) return;
  apiFetch('/api/routines/' + id, { method: 'DELETE' })
    .then(() => {
      showToast('Routine deleted', 'success');
      if (currentRoutineId === id) closeRoutineDetail();
      else loadRoutines();
    })
    .catch((err) => showToast('Delete failed: ' + err.message, 'error'));
}

function formatRelativeTime(isoString) {
  if (!isoString) return '-';
  const d = new Date(isoString);
  const now = Date.now();
  const diffMs = now - d.getTime();
  const absDiff = Math.abs(diffMs);
  const future = diffMs < 0;

  if (absDiff < 60000) 
    return future ? I18n.t('time.lessThan1MinuteFromNow') : I18n.t('time.lessThan1MinuteAgo');
  if (absDiff < 3600000) {
    const m = Math.floor(absDiff / 60000);
    return future ? I18n.t('time.minutesFromNow', { n: m }) : I18n.t('time.minutesAgo', { n: m });
  }
  if (absDiff < 86400000) {
    const h = Math.floor(absDiff / 3600000);
    return future ? I18n.t('time.hoursFromNow', { n: h }) : I18n.t('time.hoursAgo', { n: h });
  }
  const days = Math.floor(absDiff / 86400000);
  return future ? I18n.t('time.daysFromNow', { n: days }) : I18n.t('time.daysAgo', { n: days });
}

// --- Gateway status widget ---

let gatewayStatusInterval = null;
let gatewayStatusData = null;
let gatewayUpdateData = null;
let gatewayUpdateError = null;
let gatewayUpdatePromise = null;

function startGatewayStatusPolling() {
  fetchGatewayStatus();
  gatewayStatusInterval = setInterval(fetchGatewayStatus, 30000);
}

function formatTokenCount(n) {
  if (n == null || n === 0) return '0';
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'k';
  return '' + n;
}

function formatCost(costStr) {
  if (!costStr) return '$0.00';
  var n = parseFloat(costStr);
  if (n < 0.01) return '$' + n.toFixed(4);
  return '$' + n.toFixed(2);
}

function shortModelName(model) {
  // Strip provider prefix and shorten common model names
  var m = model.indexOf('/') >= 0 ? model.split('/').pop() : model;
  // Shorten dated suffixes
  m = m.replace(/-20\d{6}$/, '');
  return m;
}

function safeExternalUrl(url) {
  if (!url) return null;
  try {
    var parsed = new URL(url, window.location.origin);
    var protocol = parsed.protocol.toLowerCase();
    if (protocol === 'http:' || protocol === 'https:') {
      return parsed.href;
    }
  } catch (e) {}
  return null;
}

function renderGatewayUpdateSection() {
  if (gatewayUpdateData === null && !gatewayUpdatePromise && !gatewayUpdateError) return '';

  var html = '';
  html += '<div class="gw-divider"></div>';
  html += '<div class="gw-section-label">Update</div>';

  if (gatewayUpdatePromise && gatewayUpdateData === null && !gatewayUpdateError) {
    html += '<div class="gw-update-message">Checking for newer release…</div>';
    return html;
  }

  if (gatewayUpdateError && gatewayUpdateData === null) {
    html += '<div class="gw-update-message gw-update-error">' + escapeHtml(gatewayUpdateError) + '</div>';
    return html;
  }

  if (!gatewayUpdateData) return html;

  if (gatewayUpdateData.error) {
    html += '<div class="gw-update-message gw-update-error">' + escapeHtml(gatewayUpdateData.error) + '</div>';
    return html;
  }

  html += '<div class="gw-stat"><span>Current</span><span>' + escapeHtml(gatewayUpdateData.current_version || 'unknown') + '</span></div>';
  if (gatewayUpdateData.latest_version) {
    html += '<div class="gw-stat"><span>Latest</span><span>' + escapeHtml(gatewayUpdateData.latest_version) + '</span></div>';
  }

  if (gatewayUpdateData.update_available) {
    html += '<div class="gw-update-message gw-update-available">Update available</div>';
    if (gatewayUpdateData.release_name || gatewayUpdateData.release_url) {
      var releaseLabel = gatewayUpdateData.release_name ? escapeHtml(gatewayUpdateData.release_name) : 'Open release notes';
      var releaseHref = safeExternalUrl(gatewayUpdateData.release_url);
      if (releaseHref) {
        html += '<div class="gw-update-link-row">'
          + '<a class="gw-update-link" href="' + escapeHtml(releaseHref) + '" target="_blank" rel="noopener noreferrer">'
          + releaseLabel
          + '</a>'
          + '</div>';
      } else {
        html += '<div class="gw-update-message">' + releaseLabel + '</div>';
      }
    }
  } else {
    html += '<div class="gw-update-message gw-update-ok">Up to date</div>';
  }

  return html;
}

function renderGatewayPopover() {
  var popover = document.getElementById('gateway-popover');
  if (!popover) return;

  var data = gatewayStatusData || {};
  var html = '';

  // Version
  if (data.version) {
    html += '<div class="gw-section-label">IronClaw v' + escapeHtml(data.version) + '</div>';
    html += '<div class="gw-divider"></div>';
  }

  // Update check
  html += renderGatewayUpdateSection();

  // Connection info
  html += '<div class="gw-section-label">' + I18n.t('dashboard.connections') + '</div>';
  html += '<div class="gw-stat"><span>' + I18n.t('dashboard.sse') + '</span><span>' + (data.sse_connections || 0) + '</span></div>';
  html += '<div class="gw-stat"><span>' + I18n.t('dashboard.websocket') + '</span><span>' + (data.ws_connections || 0) + '</span></div>';
  html += '<div class="gw-stat"><span>' + I18n.t('dashboard.uptime') + '</span><span>' + formatDuration(data.uptime_secs) + '</span></div>';

  // Cost tracker
  if (data.daily_cost != null) {
    html += '<div class="gw-divider"></div>';
    html += '<div class="gw-section-label">' + I18n.t('dashboard.costToday') + '</div>';
    html += '<div class="gw-stat"><span>' + I18n.t('dashboard.spent') + '</span><span>' + formatCost(data.daily_cost) + '</span></div>';
    if (data.actions_this_hour != null) {
      html += '<div class="gw-stat"><span>' + I18n.t('dashboard.actionsPerHour') + '</span><span>' + data.actions_this_hour + '</span></div>';
    }
  }

  // Per-model token usage
  if (data.model_usage && data.model_usage.length > 0) {
    html += '<div class="gw-divider"></div>';
    html += '<div class="gw-section-label">Token Usage</div>';
    data.model_usage.sort(function(a, b) {
      return (b.input_tokens + b.output_tokens) - (a.input_tokens + a.output_tokens);
    });
    for (var i = 0; i < data.model_usage.length; i++) {
      var m = data.model_usage[i];
      var name = escapeHtml(shortModelName(m.model));
      html += '<div class="gw-model-row">'
        + '<span class="gw-model-name">' + name + '</span>'
        + '<span class="gw-model-cost">' + escapeHtml(formatCost(m.cost)) + '</span>'
        + '</div>';
      html += '<div class="gw-token-detail">'
        + '<span>in: ' + formatTokenCount(m.input_tokens) + '</span>'
        + '<span>out: ' + formatTokenCount(m.output_tokens) + '</span>'
        + '</div>';
    }
  }

  popover.innerHTML = html;
}

function fetchGatewayUpdateStatus() {
  if (gatewayUpdatePromise) return gatewayUpdatePromise;

  gatewayUpdateError = null;
  gatewayUpdatePromise = apiFetch('/api/gateway/update').then(function(data) {
    gatewayUpdateData = data;
    gatewayUpdatePromise = null;
    gatewayUpdateError = null;
    renderGatewayPopover();
    return data;
  }).catch(function(err) {
    gatewayUpdateData = null;
    gatewayUpdateError = err.message || String(err);
    gatewayUpdatePromise = null;
    renderGatewayPopover();
    return null;
  });

  renderGatewayPopover();
  return gatewayUpdatePromise;
}

function fetchGatewayStatus() {
  apiFetch('/api/gateway/status').then(function(data) {
    gatewayStatusData = data;
    // Update restart button visibility
    restartEnabled = data.restart_enabled || false;
    updateRestartButtonVisibility();
    renderGatewayPopover();
  }).catch(function() {});
}

// Show/hide popover on hover
document.getElementById('gateway-status-trigger').addEventListener('mouseenter', () => {
  document.getElementById('gateway-popover').classList.add('visible');
  renderGatewayPopover();
  if (!gatewayUpdateData && !gatewayUpdatePromise) {
    fetchGatewayUpdateStatus();
  }
});
document.getElementById('gateway-status-trigger').addEventListener('mouseleave', () => {
  document.getElementById('gateway-popover').classList.remove('visible');
});

// --- TEE attestation ---

let teeInfo = null;
let teeReportCache = null;
let teeReportLoading = false;

function teeApiBase() {
    var hostname = window.location.hostname;
    // Skip IP addresses (IPv4 and IPv6) and localhost
    if (hostname === "localhost" || /^(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)$/.test(hostname) || hostname.indexOf(":") !== -1) {
        return null;
    }
    var parts = hostname.split(".");
    if (parts.length < 2) return null;
    var domain = parts.slice(1).join(".");
    return window.location.protocol + "//api." + domain;
}

function teeInstanceName() {
  return window.location.hostname.split('.')[0];
}

function checkTeeStatus() {
  var base = teeApiBase();
  if (!base) return;
  var name = teeInstanceName();
  try {
    fetch(base + '/instances/' + encodeURIComponent(name) + '/attestation').then(function(res) {
      if (!res.ok) throw new Error(res.status);
      return res.json();
    }).then(function(data) {
      teeInfo = data;
      document.getElementById('tee-shield').style.display = 'flex';
    }).catch(function(err) {
      console.warn('Failed to fetch TEE attestation:', err);
    });
  } catch (e) {
    console.warn("Failed to check TEE status:", e);
  }
}

function fetchTeeReport() {
  if (teeReportCache) {
    renderTeePopover(teeReportCache);
    return;
  }
  if (teeReportLoading) return;
  teeReportLoading = true;
  var base = teeApiBase();
  if (!base) return;
  var popover = document.getElementById('tee-popover');
  popover.innerHTML = '<div class="tee-popover-loading">Loading attestation report...</div>';
  fetch(base + '/attestation/report').then(function(res) {
    if (!res.ok) throw new Error(res.status);
    return res.json();
  }).then(function(data) {
    teeReportCache = data;
    renderTeePopover(data);
  }).catch(function() {
    popover.innerHTML = '<div class="tee-popover-loading">Could not load attestation report</div>';
  }).finally(function() {
    teeReportLoading = false;
  });
}

function renderTeePopover(report) {
  var popover = document.getElementById('tee-popover');
  var digest = (teeInfo && teeInfo.image_digest) || 'N/A';
  var fingerprint = report.tls_certificate_fingerprint || 'N/A';
  var reportData = report.report_data || '';
  var vmConfig = report.vm_config || 'N/A';
  var truncated = reportData.length > 32 ? reportData.slice(0, 32) + '...' : reportData;
  popover.innerHTML = '<div class="tee-popover-title">'
    + '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>'
    + 'TEE Attestation</div>'
    + '<div class="tee-field"><div class="tee-field-label">Image Digest</div>'
    + '<div class="tee-field-value">' + escapeHtml(digest) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">TLS Certificate Fingerprint</div>'
    + '<div class="tee-field-value">' + escapeHtml(fingerprint) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">Report Data</div>'
    + '<div class="tee-field-value">' + escapeHtml(truncated) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">VM Config</div>'
    + '<div class="tee-field-value">' + escapeHtml(vmConfig) + '</div></div>'
    + '<div class="tee-popover-actions">'
    + '<button class="tee-btn-copy" data-action="copy-tee-report">Copy Full Report</button></div>';
}

function copyTeeReport() {
  if (!teeReportCache) return;
  var combined = Object.assign({}, teeReportCache, teeInfo || {});
  navigator.clipboard.writeText(JSON.stringify(combined, null, 2)).then(function() {
    showToast('Attestation report copied', 'success');
  }).catch(function() {
    showToast('Failed to copy report', 'error');
  });
}

document.getElementById('tee-shield').addEventListener('mouseenter', function() {
  fetchTeeReport();
  document.getElementById('tee-popover').classList.add('visible');
});
document.getElementById('tee-shield').addEventListener('mouseleave', function() {
  document.getElementById('tee-popover').classList.remove('visible');
});

// --- Extension install ---

function installWasmExtension() {
  var name = document.getElementById('wasm-install-name').value.trim();
  if (!name) {
    showToast('Extension name is required', 'error');
    return;
  }
  var url = document.getElementById('wasm-install-url').value.trim();
  if (!url) {
    showToast('URL to .tar.gz bundle is required', 'error');
    return;
  }

  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: name, url: url, kind: 'wasm_tool' },
  }).then(function(res) {
    if (res.success) {
      showToast('Installed ' + name, 'success');
      document.getElementById('wasm-install-name').value = '';
      document.getElementById('wasm-install-url').value = '';
      loadExtensions();
    } else {
      showToast('Install failed: ' + (res.message || 'unknown error'), 'error');
    }
  }).catch(function(err) {
    showToast('Install failed: ' + err.message, 'error');
  });
}

function addMcpServer() {
  var name = document.getElementById('mcp-install-name').value.trim();
  if (!name) {
    showToast('Server name is required', 'error');
    return;
  }
  var url = document.getElementById('mcp-install-url').value.trim();
  if (!url) {
    showToast('MCP server URL is required', 'error');
    return;
  }

  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: name, url: url, kind: 'mcp_server' },
  }).then(function(res) {
    if (res.success) {
      showToast('Added MCP server ' + name, 'success');
      document.getElementById('mcp-install-name').value = '';
      document.getElementById('mcp-install-url').value = '';
      loadMcpServers();
    } else {
      showToast('Failed to add MCP server: ' + (res.message || 'unknown error'), 'error');
    }
  }).catch(function(err) {
    showToast('Failed to add MCP server: ' + err.message, 'error');
  });
}

// --- Skills ---

function loadSkills() {
  var skillsList = document.getElementById('skills-list');
  skillsList.innerHTML = renderCardsSkeleton(3);
  apiFetch('/api/skills').then(function(data) {
    if (!data.skills || data.skills.length === 0) {
      skillsList.innerHTML = '<div class="empty-state">' + I18n.t('skills.noInstalled') + '</div>';
      return;
    }
    skillsList.innerHTML = '';
    for (var i = 0; i < data.skills.length; i++) {
      skillsList.appendChild(renderSkillCard(data.skills[i]));
    }
  }).catch(function(err) {
    skillsList.innerHTML = '<div class="empty-state">' + I18n.t('skills.loadFailed', {message: escapeHtml(err.message)}) + '</div>';
  });
}

function renderSkillCard(skill) {
  var card = document.createElement('div');
  card.className = 'ext-card state-active';

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = skill.name;
  header.appendChild(name);

  var trust = document.createElement('span');
  var trustClass = skill.trust.toLowerCase() === 'trusted' ? 'trust-trusted' : 'trust-installed';
  trust.className = 'skill-trust ' + trustClass;
  trust.textContent = skill.trust;
  header.appendChild(trust);

  var version = document.createElement('span');
  version.className = 'skill-version';
  version.textContent = 'v' + skill.version;
  header.appendChild(version);

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = skill.description;
  card.appendChild(desc);

  if (skill.keywords && skill.keywords.length > 0) {
    var kw = document.createElement('div');
    kw.className = 'ext-keywords';
    kw.textContent = I18n.t('skills.activatesOn') + ': ' + skill.keywords.join(', ');
    card.appendChild(kw);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  // Only show Remove for registry-installed skills, not user-placed trusted skills
  if (skill.trust.toLowerCase() !== 'trusted') {
    var removeBtn = document.createElement('button');
    removeBtn.className = 'btn-ext remove';
    removeBtn.textContent = I18n.t('skills.remove');
    removeBtn.addEventListener('click', function() { removeSkill(skill.name); });
    actions.appendChild(removeBtn);
  }

  card.appendChild(actions);
  return card;
}

function searchClawHub() {
  var input = document.getElementById('skill-search-input');
  var query = input.value.trim();
  if (!query) return;

  var resultsDiv = document.getElementById('skill-search-results');
  resultsDiv.innerHTML = '<div class="empty-state">' + I18n.t('skills.searching') + '</div>';

  apiFetch('/api/skills/search', {
    method: 'POST',
    body: { query: query },
  }).then(function(data) {
    resultsDiv.innerHTML = '';

    // Show registry error as a warning banner if present
    if (data.catalog_error) {
      var warning = document.createElement('div');
      warning.className = 'empty-state';
      warning.style.color = '#f0ad4e';
      warning.style.borderLeft = '3px solid #f0ad4e';
      warning.style.paddingLeft = '12px';
      warning.style.marginBottom = '16px';
      warning.textContent = I18n.t('skills.registryError', {message: data.catalog_error});
      resultsDiv.appendChild(warning);
    }

    // Show catalog results
    if (data.catalog && data.catalog.length > 0) {
      // Build a set of installed skill names for quick lookup
      var installedNames = {};
      if (data.installed) {
        for (var j = 0; j < data.installed.length; j++) {
          installedNames[data.installed[j].name] = true;
        }
      }

      for (var i = 0; i < data.catalog.length; i++) {
        var card = renderCatalogSkillCard(data.catalog[i], installedNames);
        card.style.animationDelay = (i * 0.06) + 's';
        resultsDiv.appendChild(card);
      }
    }

    // Show matching installed skills too
    if (data.installed && data.installed.length > 0) {
      for (var k = 0; k < data.installed.length; k++) {
        var installedCard = renderSkillCard(data.installed[k]);
        installedCard.style.animationDelay = ((data.catalog ? data.catalog.length : 0) + k) * 0.06 + 's';
        installedCard.classList.add('skill-search-result');
        resultsDiv.appendChild(installedCard);
      }
    }

    if (resultsDiv.children.length === 0) {
      resultsDiv.innerHTML = '<div class="empty-state">' + I18n.t('skills.noResults', {query: escapeHtml(query)}) + '</div>';
    }
  }).catch(function(err) {
    resultsDiv.innerHTML = '<div class="empty-state">' + I18n.t('skills.searchFailed', {message: escapeHtml(err.message)}) + '</div>';
  });
}

function renderCatalogSkillCard(entry, installedNames) {
  var card = document.createElement('div');
  card.className = 'ext-card ext-available skill-search-result';

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('a');
  name.className = 'ext-name';
  name.textContent = entry.name || entry.slug;
  name.href = 'https://clawhub.ai/skills/' + encodeURIComponent(entry.slug);
  name.target = '_blank';
  name.rel = 'noopener';
  name.style.textDecoration = 'none';
  name.style.color = 'inherit';
  name.title = 'View on ClawHub';
  header.appendChild(name);

  if (entry.version) {
    var version = document.createElement('span');
    version.className = 'skill-version';
    version.textContent = 'v' + entry.version;
    header.appendChild(version);
  }

  card.appendChild(header);

  if (entry.description) {
    var desc = document.createElement('div');
    desc.className = 'ext-desc';
    desc.textContent = entry.description;
    card.appendChild(desc);
  }

  // Metadata row: owner, stars, downloads, recency
  var meta = document.createElement('div');
  meta.className = 'ext-meta';
  meta.style.fontSize = '11px';
  meta.style.color = '#888';
  meta.style.marginTop = '6px';

  function addMetaSep() {
    if (meta.children.length > 0) {
      meta.appendChild(document.createTextNode(' \u00b7 '));
    }
  }

  if (entry.owner) {
    var ownerSpan = document.createElement('span');
    ownerSpan.textContent = 'by ' + entry.owner;
    meta.appendChild(ownerSpan);
  }

  if (entry.stars != null) {
    addMetaSep();
    var starsSpan = document.createElement('span');
    starsSpan.textContent = entry.stars + ' stars';
    meta.appendChild(starsSpan);
  }

  if (entry.downloads != null) {
    addMetaSep();
    var dlSpan = document.createElement('span');
    dlSpan.textContent = formatCompactNumber(entry.downloads) + ' downloads';
    meta.appendChild(dlSpan);
  }

  if (entry.updatedAt) {
    var ago = formatTimeAgo(entry.updatedAt);
    if (ago) {
      addMetaSep();
      var updatedSpan = document.createElement('span');
      updatedSpan.textContent = 'updated ' + ago;
      meta.appendChild(updatedSpan);
    }
  }

  if (meta.children.length > 0) {
    card.appendChild(meta);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  var slug = entry.slug || entry.name;
  var isInstalled = installedNames[entry.name] || installedNames[slug];

  if (isInstalled) {
    var label = document.createElement('span');
    label.className = 'ext-active-label';
    label.textContent = I18n.t('status.installed');
    actions.appendChild(label);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-ext install';
    installBtn.textContent = I18n.t('extensions.install');
    installBtn.addEventListener('click', (function(s, btn) {
      return function() {
        if (!confirm('Install skill "' + s + '" from ClawHub?')) return;
        btn.disabled = true;
        btn.textContent = I18n.t('extensions.installing');
        installSkill(s, null, btn);
      };
    })(slug, installBtn));
    actions.appendChild(installBtn);
  }

  card.appendChild(actions);
  return card;
}

function formatCompactNumber(n) {
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
  return '' + n;
}

function formatTimeAgo(epochMs) {
  var now = Date.now();
  var diff = now - epochMs;
  if (diff < 0) return null;
  var minutes = Math.floor(diff / 60000);
  if (minutes < 60) return minutes <= 1 ? 'just now' : minutes + 'm ago';
  var hours = Math.floor(minutes / 60);
  if (hours < 24) return hours + 'h ago';
  var days = Math.floor(hours / 24);
  if (days < 30) return days + 'd ago';
  var months = Math.floor(days / 30);
  if (months < 12) return months + 'mo ago';
  return Math.floor(months / 12) + 'y ago';
}

function installSkill(nameOrSlug, url, btn) {
  var body = { name: nameOrSlug, slug: nameOrSlug };
  if (url) body.url = url;

  apiFetch('/api/skills/install', {
    method: 'POST',
    headers: { 'X-Confirm-Action': 'true' },
    body: body,
  }).then(function(res) {
    if (res.success) {
      showToast(I18n.t('skills.installedSuccess', {name: nameOrSlug}), 'success');
    } else {
      showToast('Install failed: ' + (res.message || 'unknown error'), 'error');
    }
    loadSkills();
    if (btn) { btn.disabled = false; btn.textContent = 'Install'; }
  }).catch(function(err) {
    showToast('Install failed: ' + err.message, 'error');
    if (btn) { btn.disabled = false; btn.textContent = 'Install'; }
  });
}

function removeSkill(name) {
  showConfirmModal(I18n.t('skills.confirmRemove', { name: name }), '', function() {
    apiFetch('/api/skills/' + encodeURIComponent(name), {
      method: 'DELETE',
      headers: { 'X-Confirm-Action': 'true' },
    }).then(function(res) {
      if (res.success) {
        showToast(I18n.t('skills.removed', { name: name }), 'success');
      } else {
        showToast(I18n.t('skills.removeFailed', { message: res.message || 'unknown error' }), 'error');
      }
      loadSkills();
    }).catch(function(err) {
      showToast(I18n.t('skills.removeFailed', { message: err.message }), 'error');
    });
  }, I18n.t('common.remove'), 'btn-danger');
}

function installSkillFromForm() {
  var name = document.getElementById('skill-install-name').value.trim();
  if (!name) { showToast('Skill name is required', 'error'); return; }
  var url = document.getElementById('skill-install-url').value.trim() || null;
  if (url && !url.startsWith('https://')) {
    showToast('URL must use HTTPS', 'error');
    return;
  }
  if (!confirm('Install skill "' + name + '"?')) return;
  installSkill(name, url, null);
  document.getElementById('skill-install-name').value = '';
  document.getElementById('skill-install-url').value = '';
}

// Wire up Enter key on search input
document.getElementById('skill-search-input').addEventListener('keydown', function(e) {
  if (e.key === 'Enter') searchClawHub();
});

// --- Keyboard shortcuts ---

document.addEventListener('keydown', (e) => {
  const mod = e.metaKey || e.ctrlKey;
  const tag = (e.target.tagName || '').toLowerCase();
  const inInput = tag === 'input' || tag === 'textarea';

  // Mod+1-5: switch tabs
  if (mod && e.key >= '1' && e.key <= '5') {
    e.preventDefault();
    const tabs = ['chat', 'memory', 'jobs', 'routines', 'settings'];
    const idx = parseInt(e.key) - 1;
    if (tabs[idx]) switchTab(tabs[idx]);
    return;
  }

  // Mod+K: focus chat input or memory search
  if (mod && e.key === 'k') {
    e.preventDefault();
    if (currentTab === 'memory') {
      document.getElementById('memory-search').focus();
    } else {
      document.getElementById('chat-input').focus();
    }
    return;
  }

  // Mod+N: new thread
  if (mod && e.key === 'n' && currentTab === 'chat') {
    e.preventDefault();
    createNewThread();
    return;
  }

  // Escape: close autocomplete, job detail, or blur input
  if (e.key === 'Escape') {
    const acEl = document.getElementById('slash-autocomplete');
    if (acEl && acEl.style.display !== 'none') {
      hideSlashAutocomplete();
      return;
    }
    if (currentJobId) {
      closeJobDetail();
    } else if (inInput) {
      e.target.blur();
    }
    return;
  }
});

// --- Settings Tab ---

document.querySelectorAll('.settings-subtab').forEach(function(btn) {
  btn.addEventListener('click', function() {
    switchSettingsSubtab(btn.getAttribute('data-settings-subtab'));
  });
});

function switchSettingsSubtab(subtab) {
  currentSettingsSubtab = subtab;
  document.querySelectorAll('.settings-subtab').forEach(function(b) {
    b.classList.toggle('active', b.getAttribute('data-settings-subtab') === subtab);
  });
  document.querySelectorAll('.settings-subpanel').forEach(function(p) {
    p.classList.toggle('active', p.id === 'settings-' + subtab);
  });
  // Clear search when switching subtabs so stale filters don't apply
  var searchInput = document.getElementById('settings-search-input');
  if (searchInput && searchInput.value) {
    searchInput.value = '';
    searchInput.dispatchEvent(new Event('input'));
  }
  loadSettingsSubtab(subtab);
}

function loadSettingsSubtab(subtab) {
  if (subtab === 'inference') loadInferenceSettings();
  else if (subtab === 'agent') loadAgentSettings();
  else if (subtab === 'channels') { loadChannelsStatus(); startPairingPoll(); }
  else if (subtab === 'networking') loadNetworkingSettings();
  else if (subtab === 'extensions') { loadExtensions(); startPairingPoll(); }
  else if (subtab === 'mcp') loadMcpServers();
  else if (subtab === 'skills') loadSkills();
  if (subtab !== 'extensions' && subtab !== 'channels') stopPairingPoll();
}

// --- Structured Settings Definitions ---

var INFERENCE_SETTINGS = [
  {
    group: 'cfg.group.llm',
    settings: [
      { key: 'llm_backend', label: 'cfg.llm_backend.label', description: 'cfg.llm_backend.desc',
        type: 'select', options: ['nearai', 'anthropic', 'openai', 'ollama', 'openai_compatible', 'tinfoil', 'bedrock'] },
      { key: 'selected_model', label: 'cfg.selected_model.label', description: 'cfg.selected_model.desc', type: 'text' },
      { key: 'ollama_base_url', label: 'cfg.ollama_base_url.label', description: 'cfg.ollama_base_url.desc', type: 'text',
        showWhen: { key: 'llm_backend', value: 'ollama' } },
      { key: 'openai_compatible_base_url', label: 'cfg.openai_compatible_base_url.label', description: 'cfg.openai_compatible_base_url.desc', type: 'text',
        showWhen: { key: 'llm_backend', value: 'openai_compatible' } },
      { key: 'bedrock_region', label: 'cfg.bedrock_region.label', description: 'cfg.bedrock_region.desc', type: 'text',
        showWhen: { key: 'llm_backend', value: 'bedrock' } },
      { key: 'bedrock_cross_region', label: 'cfg.bedrock_cross_region.label', description: 'cfg.bedrock_cross_region.desc',
        type: 'select', options: ['us', 'eu', 'apac', 'global'],
        showWhen: { key: 'llm_backend', value: 'bedrock' } },
      { key: 'bedrock_profile', label: 'cfg.bedrock_profile.label', description: 'cfg.bedrock_profile.desc', type: 'text',
        showWhen: { key: 'llm_backend', value: 'bedrock' } },
    ]
  },
  {
    group: 'cfg.group.embeddings',
    settings: [
      { key: 'embeddings.enabled', label: 'cfg.embeddings_enabled.label', description: 'cfg.embeddings_enabled.desc', type: 'boolean' },
      { key: 'embeddings.provider', label: 'cfg.embeddings_provider.label', description: 'cfg.embeddings_provider.desc',
        type: 'select', options: ['openai', 'nearai'] },
      { key: 'embeddings.model', label: 'cfg.embeddings_model.label', description: 'cfg.embeddings_model.desc', type: 'text' },
    ]
  },
];

var AGENT_SETTINGS = [
  {
    group: 'cfg.group.agent',
    settings: [
      { key: 'agent.name', label: 'cfg.agent_name.label', description: 'cfg.agent_name.desc', type: 'text' },
      { key: 'agent.max_parallel_jobs', label: 'cfg.agent_max_parallel_jobs.label', description: 'cfg.agent_max_parallel_jobs.desc', type: 'number' },
      { key: 'agent.job_timeout_secs', label: 'cfg.agent_job_timeout.label', description: 'cfg.agent_job_timeout.desc', type: 'number' },
      { key: 'agent.max_tool_iterations', label: 'cfg.agent_max_tool_iterations.label', description: 'cfg.agent_max_tool_iterations.desc', type: 'number' },
      { key: 'agent.use_planning', label: 'cfg.agent_use_planning.label', description: 'cfg.agent_use_planning.desc', type: 'boolean' },
      { key: 'agent.auto_approve_tools', label: 'cfg.agent_auto_approve.label', description: 'cfg.agent_auto_approve.desc', type: 'boolean' },
      { key: 'agent.default_timezone', label: 'cfg.agent_timezone.label', description: 'cfg.agent_timezone.desc', type: 'text' },
      { key: 'agent.session_idle_timeout_secs', label: 'cfg.agent_session_idle.label', description: 'cfg.agent_session_idle.desc', type: 'number' },
      { key: 'agent.stuck_threshold_secs', label: 'cfg.agent_stuck_threshold.label', description: 'cfg.agent_stuck_threshold.desc', type: 'number' },
      { key: 'agent.max_repair_attempts', label: 'cfg.agent_max_repair.label', description: 'cfg.agent_max_repair.desc', type: 'number' },
      { key: 'agent.max_cost_per_day_cents', label: 'cfg.agent_max_cost.label', description: 'cfg.agent_max_cost.desc', type: 'number', min: 0 },
      { key: 'agent.max_actions_per_hour', label: 'cfg.agent_max_actions.label', description: 'cfg.agent_max_actions.desc', type: 'number', min: 0 },
      { key: 'agent.allow_local_tools', label: 'cfg.agent_allow_local.label', description: 'cfg.agent_allow_local.desc', type: 'boolean' },
    ]
  },
  {
    group: 'cfg.group.heartbeat',
    settings: [
      { key: 'heartbeat.enabled', label: 'cfg.heartbeat_enabled.label', description: 'cfg.heartbeat_enabled.desc', type: 'boolean' },
      { key: 'heartbeat.interval_secs', label: 'cfg.heartbeat_interval.label', description: 'cfg.heartbeat_interval.desc', type: 'number' },
      { key: 'heartbeat.notify_channel', label: 'cfg.heartbeat_notify_channel.label', description: 'cfg.heartbeat_notify_channel.desc', type: 'text' },
      { key: 'heartbeat.notify_user', label: 'cfg.heartbeat_notify_user.label', description: 'cfg.heartbeat_notify_user.desc', type: 'text' },
      { key: 'heartbeat.quiet_hours_start', label: 'cfg.heartbeat_quiet_start.label', description: 'cfg.heartbeat_quiet_start.desc', type: 'number', min: 0, max: 23 },
      { key: 'heartbeat.quiet_hours_end', label: 'cfg.heartbeat_quiet_end.label', description: 'cfg.heartbeat_quiet_end.desc', type: 'number', min: 0, max: 23 },
      { key: 'heartbeat.timezone', label: 'cfg.heartbeat_timezone.label', description: 'cfg.heartbeat_timezone.desc', type: 'text' },
    ]
  },
  {
    group: 'cfg.group.sandbox',
    settings: [
      { key: 'sandbox.enabled', label: 'cfg.sandbox_enabled.label', description: 'cfg.sandbox_enabled.desc', type: 'boolean' },
      { key: 'sandbox.policy', label: 'cfg.sandbox_policy.label', description: 'cfg.sandbox_policy.desc',
        type: 'select', options: ['readonly', 'workspace_write', 'full_access'] },
      { key: 'sandbox.timeout_secs', label: 'cfg.sandbox_timeout.label', description: 'cfg.sandbox_timeout.desc', type: 'number', min: 0 },
      { key: 'sandbox.memory_limit_mb', label: 'cfg.sandbox_memory.label', description: 'cfg.sandbox_memory.desc', type: 'number', min: 0 },
      { key: 'sandbox.image', label: 'cfg.sandbox_image.label', description: 'cfg.sandbox_image.desc', type: 'text' },
    ]
  },
  {
    group: 'cfg.group.routines',
    settings: [
      { key: 'routines.max_concurrent', label: 'cfg.routines_max_concurrent.label', description: 'cfg.routines_max_concurrent.desc', type: 'number', min: 0 },
      { key: 'routines.default_cooldown_secs', label: 'cfg.routines_cooldown.label', description: 'cfg.routines_cooldown.desc', type: 'number', min: 0 },
    ]
  },
  {
    group: 'cfg.group.safety',
    settings: [
      { key: 'safety.max_output_length', label: 'cfg.safety_max_output.label', description: 'cfg.safety_max_output.desc', type: 'number', min: 0 },
      { key: 'safety.injection_check_enabled', label: 'cfg.safety_injection_check.label', description: 'cfg.safety_injection_check.desc', type: 'boolean' },
    ]
  },
  {
    group: 'cfg.group.skills',
    settings: [
      { key: 'skills.max_active', label: 'cfg.skills_max_active.label', description: 'cfg.skills_max_active.desc', type: 'number', min: 0 },
      { key: 'skills.max_context_tokens', label: 'cfg.skills_max_tokens.label', description: 'cfg.skills_max_tokens.desc', type: 'number', min: 0 },
    ]
  },
  {
    group: 'cfg.group.search',
    settings: [
      { key: 'search.fusion_strategy', label: 'cfg.search_fusion.label', description: 'cfg.search_fusion.desc',
        type: 'select', options: ['rrf', 'weighted'] },
    ]
  },
];

function renderSettingsSkeleton(rows) {
  var html = '<div class="settings-group" style="border:none;background:none">';
  for (var i = 0; i < (rows || 5); i++) {
    var w1 = 100 + Math.floor(Math.random() * 60);
    var w2 = 140 + Math.floor(Math.random() * 60);
    html += '<div class="skeleton-row"><div class="skeleton-bar" style="width:' + w1 + 'px"></div><div class="skeleton-bar" style="width:' + w2 + 'px"></div></div>';
  }
  html += '</div>';
  return html;
}

function renderCardsSkeleton(count) {
  var html = '';
  for (var i = 0; i < (count || 3); i++) {
    html += '<div class="skeleton-card"><div class="skeleton-bar" style="width:60%;height:14px"></div><div class="skeleton-bar" style="width:90%;height:10px"></div><div class="skeleton-bar" style="width:40%;height:10px"></div></div>';
  }
  return html;
}

function loadInferenceSettings() {
  var container = document.getElementById('settings-inference-content');
  container.innerHTML = renderSettingsSkeleton(6);

  Promise.all([
    apiFetch('/api/settings/export'),
    apiFetch('/api/gateway/status').catch(function() { return {}; }),
    apiFetch('/v1/models').catch(function() { return { data: [] }; })
  ]).then(function(results) {
    var settings = results[0].settings || {};
    var status = results[1];
    var modelsData = results[2];
    var activeValues = {
      'llm_backend': status.llm_backend,
      'selected_model': status.llm_model
    };
    // Inject available model IDs as suggestions for the selected_model field
    var modelIds = (modelsData.data || []).map(function(m) { return m.id; }).filter(Boolean);
    var llmGroup = INFERENCE_SETTINGS[0];
    for (var i = 0; i < llmGroup.settings.length; i++) {
      if (llmGroup.settings[i].key === 'selected_model') {
        llmGroup.settings[i].suggestions = modelIds;
        break;
      }
    }
    container.innerHTML = '';
    renderStructuredSettingsInto(container, INFERENCE_SETTINGS, settings, activeValues);
  }).catch(function(err) {
    container.innerHTML = '<div class="empty-state">' + I18n.t('common.loadFailed') + ': '
      + escapeHtml(err.message) + '</div>';
  });
}

function loadAgentSettings() {
  loadStructuredSettings('settings-agent-content', AGENT_SETTINGS);
}

function loadStructuredSettings(containerId, settingsDefs) {
  var container = document.getElementById(containerId);
  container.innerHTML = renderSettingsSkeleton(8);

  apiFetch('/api/settings/export').then(function(data) {
    var settings = data.settings || {};
    container.innerHTML = '';
    renderStructuredSettingsInto(container, settingsDefs, settings, {});
  }).catch(function(err) {
    container.innerHTML = '<div class="empty-state">' + I18n.t('common.loadFailed') + ': '
      + escapeHtml(err.message) + '</div>';
  });
}

function renderStructuredSettingsInto(container, settingsDefs, settings, activeValues) {
    for (var gi = 0; gi < settingsDefs.length; gi++) {
      var groupDef = settingsDefs[gi];
      var group = document.createElement('div');
      group.className = 'settings-group';

      var title = document.createElement('div');
      title.className = 'settings-group-title';
      title.textContent = I18n.t(groupDef.group);
      group.appendChild(title);

      var rows = [];
      for (var si = 0; si < groupDef.settings.length; si++) {
        var def = groupDef.settings[si];
        var activeVal = activeValues ? activeValues[def.key] : undefined;
        var row = renderStructuredSettingsRow(def, settings[def.key], activeVal);
        if (def.showWhen) {
          row.setAttribute('data-show-when-key', def.showWhen.key);
          row.setAttribute('data-show-when-value', def.showWhen.value);
          var currentVal = settings[def.showWhen.key];
          if (currentVal === def.showWhen.value) {
            row.classList.remove('hidden');
          } else {
            row.classList.add('hidden');
          }
        }
        rows.push(row);
        group.appendChild(row);
      }

      container.appendChild(group);

      // Wire up showWhen reactivity for select fields in this group
      (function(groupRows, allSettings) {
        for (var ri = 0; ri < groupRows.length; ri++) {
          var sel = groupRows[ri].querySelector('.settings-select');
          if (sel) {
            sel.addEventListener('change', function() {
              var changedKey = this.getAttribute('data-setting-key');
              var changedVal = this.value;
              for (var rj = 0; rj < groupRows.length; rj++) {
                var whenKey = groupRows[rj].getAttribute('data-show-when-key');
                var whenVal = groupRows[rj].getAttribute('data-show-when-value');
                if (whenKey === changedKey) {
                  if (changedVal === whenVal) {
                    groupRows[rj].classList.remove('hidden');
                  } else {
                    groupRows[rj].classList.add('hidden');
                  }
                }
              }
            });
          }
        }
      })(rows, settings);
    }

    if (container.children.length === 0) {
      container.innerHTML = '<div class="empty-state">' + I18n.t('settings.noSettings') + '</div>';
    }
}

function renderStructuredSettingsRow(def, value, activeValue) {
  var row = document.createElement('div');
  row.className = 'settings-row';

  var labelWrap = document.createElement('div');
  labelWrap.className = 'settings-label-wrap';

  var label = document.createElement('div');
  label.className = 'settings-label';
  label.textContent = I18n.t(def.label);
  labelWrap.appendChild(label);

  if (def.description) {
    var desc = document.createElement('div');
    desc.className = 'settings-description';
    desc.textContent = I18n.t(def.description);
    labelWrap.appendChild(desc);
  }

  row.appendChild(labelWrap);

  var inputWrap = document.createElement('div');
  inputWrap.style.display = 'flex';
  inputWrap.style.alignItems = 'center';
  inputWrap.style.gap = '8px';

  var ariaLabel = I18n.t(def.label) + (def.description ? '. ' + I18n.t(def.description) : '');
  function formatSettingValue(raw) {
    if (Array.isArray(raw)) return raw.join(', ');
    if (raw === null || raw === undefined) return '';
    return String(raw);
  }

  var activeValueText = formatSettingValue(activeValue);
  var placeholderText = activeValueText ? I18n.t('settings.envValue', { value: activeValueText }) : (def.placeholder || I18n.t('settings.envDefault'));

  if (def.type === 'boolean') {
    var boolSel = document.createElement('select');
    boolSel.className = 'settings-select';
    boolSel.setAttribute('data-setting-key', def.key);
    boolSel.setAttribute('aria-label', ariaLabel);
    var boolDefault = document.createElement('option');
    boolDefault.value = '';
    boolDefault.textContent = activeValue !== undefined && activeValue !== null
      ? '\u2014 ' + I18n.t('settings.envValue', { value: String(activeValue) }) + ' \u2014'
      : '\u2014 ' + I18n.t('settings.useEnvDefault') + ' \u2014';
    if (value === null || value === undefined) boolDefault.selected = true;
    boolSel.appendChild(boolDefault);
    var boolOn = document.createElement('option');
    boolOn.value = 'true';
    boolOn.textContent = I18n.t('settings.on');
    if (value === true) boolOn.selected = true;
    boolSel.appendChild(boolOn);
    var boolOff = document.createElement('option');
    boolOff.value = 'false';
    boolOff.textContent = I18n.t('settings.off');
    if (value === false) boolOff.selected = true;
    boolSel.appendChild(boolOff);
    boolSel.addEventListener('change', (function(k, el) {
      return function() {
        if (el.value === '') saveSetting(k, null);
        else saveSetting(k, el.value === 'true');
      };
    })(def.key, boolSel));
    inputWrap.appendChild(boolSel);
  } else if (def.type === 'select' && def.options) {
    var sel = document.createElement('select');
    sel.className = 'settings-select';
    sel.setAttribute('data-setting-key', def.key);
    sel.setAttribute('aria-label', ariaLabel);
    var emptyOpt = document.createElement('option');
    emptyOpt.value = '';
    emptyOpt.textContent = activeValue ? '\u2014 ' + I18n.t('settings.envValue', { value: activeValue }) + ' \u2014' : '\u2014 ' + I18n.t('settings.useEnvDefault') + ' \u2014';
    if (!value && value !== false && value !== 0) emptyOpt.selected = true;
    sel.appendChild(emptyOpt);
    for (var oi = 0; oi < def.options.length; oi++) {
      var opt = document.createElement('option');
      opt.value = def.options[oi];
      opt.textContent = def.options[oi];
      if (String(value) === def.options[oi]) opt.selected = true;
      sel.appendChild(opt);
    }
    sel.addEventListener('change', (function(k, el) {
      return function() { saveSetting(k, el.value === '' ? null : el.value); };
    })(def.key, sel));
    inputWrap.appendChild(sel);
  } else if (def.type === 'number') {
    var numInp = document.createElement('input');
    numInp.type = 'number';
    numInp.step = '1';
    numInp.className = 'settings-input';
    numInp.setAttribute('aria-label', ariaLabel);
    numInp.value = (value === null || value === undefined) ? '' : value;
    if (!value && value !== 0) numInp.placeholder = placeholderText;
    if (def.min !== undefined) numInp.min = def.min;
    if (def.max !== undefined) numInp.max = def.max;
    numInp.addEventListener('change', (function(k, el) {
      return function() {
        if (el.value === '') return saveSetting(k, null);
        var parsed = parseInt(el.value, 10);
        if (isNaN(parsed)) return;
        el.value = parsed;
        saveSetting(k, parsed);
      };
    })(def.key, numInp));
    inputWrap.appendChild(numInp);
  } else if (def.type === 'list') {
    var listInp = document.createElement('input');
    listInp.type = 'text';
    listInp.className = 'settings-input';
    listInp.setAttribute('aria-label', ariaLabel);
    var listValue = '';
    if (Array.isArray(value)) listValue = value.join(', ');
    else if (typeof value === 'string') listValue = value;
    listInp.value = listValue;
    if (!listValue) listInp.placeholder = placeholderText;
    listInp.addEventListener('change', (function(k, el) {
      return function() {
        if (el.value.trim() === '') return saveSetting(k, null);
        var items = el.value.split(/[\n,]/).map(function(item) {
          return item.trim();
        }).filter(Boolean);
        saveSetting(k, items);
      };
    })(def.key, listInp));
    inputWrap.appendChild(listInp);
  } else {
    var textInp = document.createElement('input');
    textInp.type = 'text';
    textInp.className = 'settings-input';
    textInp.setAttribute('aria-label', ariaLabel);
    textInp.value = (value === null || value === undefined) ? '' : String(value);
    if (!value) textInp.placeholder = placeholderText;
    // Attach datalist for autocomplete suggestions (e.g., model list)
    if (def.suggestions && def.suggestions.length > 0) {
      var dlId = 'dl-' + def.key.replace(/\./g, '-');
      var dl = document.createElement('datalist');
      dl.id = dlId;
      for (var di = 0; di < def.suggestions.length; di++) {
        var dlOpt = document.createElement('option');
        dlOpt.value = def.suggestions[di];
        dl.appendChild(dlOpt);
      }
      textInp.setAttribute('list', dlId);
      inputWrap.appendChild(dl);
    }
    textInp.addEventListener('change', (function(k, el) {
      return function() { saveSetting(k, el.value === '' ? null : el.value); };
    })(def.key, textInp));
    inputWrap.appendChild(textInp);
  }

  var saved = document.createElement('span');
  saved.className = 'settings-saved-indicator';
  saved.textContent = '\u2713 ' + I18n.t('settings.saved');
  saved.setAttribute('data-key', def.key);
  saved.setAttribute('role', 'status');
  saved.setAttribute('aria-live', 'polite');
  inputWrap.appendChild(saved);

  row.appendChild(inputWrap);
  return row;
}

var RESTART_REQUIRED_KEYS = ['llm_backend', 'selected_model', 'ollama_base_url', 'openai_compatible_base_url',
  'bedrock_region', 'bedrock_cross_region', 'bedrock_profile', 'embeddings.enabled', 'embeddings.provider', 'embeddings.model',
  'agent.auto_approve_tools', 'tunnel.provider', 'tunnel.public_url', 'gateway.rate_limit', 'gateway.max_connections'];

var _settingsSavedTimers = {};

function saveSetting(key, value) {
  var method = (value === null || value === undefined) ? 'DELETE' : 'PUT';
  var opts = { method: method };
  if (method === 'PUT') opts.body = { value: value };
  apiFetch('/api/settings/' + encodeURIComponent(key), opts).then(function() {
    var indicator = document.querySelector('.settings-saved-indicator[data-key="' + key + '"]');
    if (indicator) {
      if (_settingsSavedTimers[key]) clearTimeout(_settingsSavedTimers[key]);
      indicator.classList.add('visible');
      _settingsSavedTimers[key] = setTimeout(function() { indicator.classList.remove('visible'); }, 2000);
    }
    // Show restart banner for inference settings
    if (RESTART_REQUIRED_KEYS.indexOf(key) !== -1) {
      showRestartBanner();
    }
  }).catch(function(err) {
    showToast('Failed to save ' + key + ': ' + err.message, 'error');
  });
}

function showRestartBanner() {
  var container = document.querySelector('.settings-content');
  if (!container || container.querySelector('.restart-banner')) return;
  var banner = document.createElement('div');
  banner.className = 'restart-banner';
  banner.setAttribute('role', 'alert');
  var textSpan = document.createElement('span');
  textSpan.className = 'restart-banner-text';
  textSpan.textContent = '\u26A0\uFE0F ' + I18n.t('settings.restartRequired');
  banner.appendChild(textSpan);
  var restartBtn = document.createElement('button');
  restartBtn.className = 'restart-banner-btn';
  restartBtn.textContent = I18n.t('settings.restartNow');
  restartBtn.addEventListener('click', function() { triggerRestart(); });
  banner.appendChild(restartBtn);
  container.insertBefore(banner, container.firstChild);
}

function loadMcpServers() {
  var mcpList = document.getElementById('mcp-servers-list');
  mcpList.innerHTML = renderCardsSkeleton(2);

  Promise.all([
    apiFetch('/api/extensions').catch(function() { return { extensions: [] }; }),
    apiFetch('/api/extensions/registry').catch(function() { return { entries: [] }; }),
  ]).then(function(results) {
    var extData = results[0];
    var registryData = results[1];
    var mcpEntries = (registryData.entries || []).filter(function(e) { return e.kind === 'mcp_server'; });
    var installedMcp = (extData.extensions || []).filter(function(e) { return e.kind === 'mcp_server'; });

    mcpList.innerHTML = '';
    var renderedNames = {};

    // Registry entries (cross-referenced with installed)
    for (var i = 0; i < mcpEntries.length; i++) {
      renderedNames[mcpEntries[i].name] = true;
      var installedExt = installedMcp.find(function(e) { return e.name === mcpEntries[i].name; });
      mcpList.appendChild(renderMcpServerCard(mcpEntries[i], installedExt));
    }

    // Custom installed MCP servers not in registry
    for (var j = 0; j < installedMcp.length; j++) {
      if (!renderedNames[installedMcp[j].name]) {
        mcpList.appendChild(renderExtensionCard(installedMcp[j]));
      }
    }

    if (mcpList.children.length === 0) {
      mcpList.innerHTML = '<div class="empty-state">' + I18n.t('mcp.noServers') + '</div>';
    }
  }).catch(function(err) {
    mcpList.innerHTML = '<div class="empty-state">' + I18n.t('common.loadFailed') + ': '
      + escapeHtml(err.message) + '</div>';
  });
}

function loadChannelsStatus() {
  var container = document.getElementById('settings-channels-content');
  container.innerHTML = renderCardsSkeleton(4);

  Promise.all([
    apiFetch('/api/gateway/status').catch(function() { return {}; }),
    apiFetch('/api/extensions').catch(function() { return { extensions: [] }; }),
    apiFetch('/api/extensions/registry').catch(function() { return { entries: [] }; }),
  ]).then(function(results) {
    var status = results[0];
    var extensions = results[1].extensions || [];
    var registry = results[2].entries || [];

    container.innerHTML = '';

    // Built-in Channels section
    var builtinSection = document.createElement('div');
    builtinSection.className = 'extensions-section';
    var builtinTitle = document.createElement('h3');
    builtinTitle.textContent = I18n.t('channels.builtin');
    builtinSection.appendChild(builtinTitle);
    var builtinList = document.createElement('div');
    builtinList.className = 'extensions-list';

    builtinList.appendChild(renderBuiltinChannelCard(
      I18n.t('channels.webGateway'),
      I18n.t('channels.webGatewayDesc'),
      true,
      'SSE: ' + (status.sse_connections || 0) + ' \u00B7 WS: ' + (status.ws_connections || 0)
    ));

    var enabledChannels = status.enabled_channels || [];

    builtinList.appendChild(renderBuiltinChannelCard(
      I18n.t('channels.httpWebhook'),
      I18n.t('channels.httpWebhookDesc'),
      enabledChannels.indexOf('http') !== -1,
      I18n.t('channels.configureVia', { env: 'ENABLE_HTTP=true' })
    ));

    builtinList.appendChild(renderBuiltinChannelCard(
      I18n.t('channels.cli'),
      I18n.t('channels.cliDesc'),
      enabledChannels.indexOf('cli') !== -1,
      I18n.t('channels.runWith', { cmd: 'ironclaw run --cli' })
    ));

    builtinList.appendChild(renderBuiltinChannelCard(
      I18n.t('channels.repl'),
      I18n.t('channels.replDesc'),
      enabledChannels.indexOf('repl') !== -1,
      I18n.t('channels.runWith', { cmd: 'ironclaw run --repl' })
    ));

    builtinSection.appendChild(builtinList);
    container.appendChild(builtinSection);

    // Messaging Channels section — use extension cards with full stepper/pairing UI
    var channelEntries = registry.filter(function(e) {
      return e.kind === 'wasm_channel' || e.kind === 'channel';
    });
    var installedChannels = extensions.filter(function(e) {
      return e.kind === 'wasm_channel';
    });

    if (channelEntries.length > 0 || installedChannels.length > 0) {
      var messagingSection = document.createElement('div');
      messagingSection.className = 'extensions-section';
      var messagingTitle = document.createElement('h3');
      messagingTitle.textContent = I18n.t('channels.messaging');
      messagingSection.appendChild(messagingTitle);
      var messagingList = document.createElement('div');
      messagingList.className = 'extensions-list';

      var renderedNames = {};

      // Registry entries: show full ext card if installed, available card if not
      for (var i = 0; i < channelEntries.length; i++) {
        var entry = channelEntries[i];
        renderedNames[entry.name] = true;
        var installed = null;
        for (var k = 0; k < installedChannels.length; k++) {
          if (installedChannels[k].name === entry.name) { installed = installedChannels[k]; break; }
        }
        if (installed) {
          messagingList.appendChild(renderExtensionCard(installed));
        } else {
          messagingList.appendChild(renderAvailableExtensionCard(entry));
        }
      }

      // Installed channels not in registry (custom installs)
      for (var j = 0; j < installedChannels.length; j++) {
        if (!renderedNames[installedChannels[j].name]) {
          messagingList.appendChild(renderExtensionCard(installedChannels[j]));
        }
      }

      messagingSection.appendChild(messagingList);
      container.appendChild(messagingSection);
    }
  });
}

function renderBuiltinChannelCard(name, description, active, detail) {
  var card = document.createElement('div');
  card.className = 'ext-card ' + (active ? 'state-active' : 'state-inactive');

  var header = document.createElement('div');
  header.className = 'ext-header';

  var nameEl = document.createElement('span');
  nameEl.className = 'ext-name';
  nameEl.textContent = name;
  header.appendChild(nameEl);

  var kindEl = document.createElement('span');
  kindEl.className = 'ext-kind kind-builtin';
  kindEl.textContent = I18n.t('ext.builtin');
  header.appendChild(kindEl);

  var statusDot = document.createElement('span');
  statusDot.className = 'ext-auth-dot ' + (active ? 'authed' : 'unauthed');
  statusDot.title = active ? I18n.t('ext.active') : I18n.t('ext.inactive');
  header.appendChild(statusDot);

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = description;
  card.appendChild(desc);

  if (detail) {
    var detailEl = document.createElement('div');
    detailEl.className = 'ext-url';
    detailEl.textContent = detail;
    card.appendChild(detailEl);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions';
  var label = document.createElement('span');
  label.className = 'ext-active-label';
  label.textContent = active ? I18n.t('ext.active') : I18n.t('ext.inactive');
  actions.appendChild(label);
  card.appendChild(actions);

  return card;
}

// --- Networking Settings ---

var NETWORKING_SETTINGS = [
  {
    group: 'cfg.group.tunnel',
    settings: [
      { key: 'tunnel.provider', label: 'cfg.tunnel_provider.label', description: 'cfg.tunnel_provider.desc',
        type: 'select', options: ['none', 'cloudflare', 'ngrok', 'tailscale', 'custom'] },
      { key: 'tunnel.public_url', label: 'cfg.tunnel_public_url.label', description: 'cfg.tunnel_public_url.desc', type: 'text' },
    ]
  },
  {
    group: 'cfg.group.gateway',
    settings: [
      { key: 'gateway.rate_limit', label: 'cfg.gateway_rate_limit.label', description: 'cfg.gateway_rate_limit.desc', type: 'number', min: 0 },
      { key: 'gateway.max_connections', label: 'cfg.gateway_max_connections.label', description: 'cfg.gateway_max_connections.desc', type: 'number', min: 0 },
    ]
  },
];

function loadNetworkingSettings() {
  var container = document.getElementById('settings-networking-content');
  container.innerHTML = renderSettingsSkeleton(4);

  apiFetch('/api/settings/export').then(function(data) {
    var settings = data.settings || {};
    container.innerHTML = '';
    renderStructuredSettingsInto(container, NETWORKING_SETTINGS, settings, {});
  }).catch(function(err) {
    container.innerHTML = '<div class="empty-state">' + I18n.t('common.loadFailed') + ': '
      + escapeHtml(err.message) + '</div>';
  });
}

// --- Toasts ---

function showToast(message, type) {
  const container = document.getElementById('toasts');
  const toast = document.createElement('div');
  toast.className = 'toast toast-' + (type || 'info');
  toast.textContent = message;
  container.appendChild(toast);
  // Trigger slide-in
  requestAnimationFrame(() => toast.classList.add('visible'));
  setTimeout(() => {
    toast.classList.remove('visible');
    toast.addEventListener('transitionend', () => toast.remove());
  }, 4000);
}

// --- Utilities ---

function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

function formatDate(isoString) {
  if (!isoString) return '-';
  const d = new Date(isoString);
  return d.toLocaleString();
}

// --- Event Listener Registration (CSP-safe, no inline handlers) ---

document.getElementById('auth-connect-btn').addEventListener('click', () => authenticate());
document.getElementById('restart-overlay').addEventListener('click', () => cancelRestart());
document.getElementById('restart-close-btn').addEventListener('click', () => cancelRestart());
document.getElementById('restart-cancel-btn').addEventListener('click', () => cancelRestart());
document.getElementById('restart-confirm-btn').addEventListener('click', () => confirmRestart());
document.getElementById('restart-btn').addEventListener('click', () => triggerRestart());
document.getElementById('thread-new-btn').addEventListener('click', () => createNewThread());
document.getElementById('thread-toggle-btn').addEventListener('click', () => toggleThreadSidebar());
document.getElementById('assistant-thread').addEventListener('click', () => switchToAssistant());
document.getElementById('send-btn').addEventListener('click', () => sendMessage());
document.getElementById('memory-edit-btn').addEventListener('click', () => startMemoryEdit());
document.getElementById('memory-save-btn').addEventListener('click', () => saveMemoryEdit());
document.getElementById('memory-cancel-btn').addEventListener('click', () => cancelMemoryEdit());
document.getElementById('logs-server-level').addEventListener('change', (e) => setServerLogLevel(e.target.value));
document.getElementById('logs-pause-btn').addEventListener('click', () => toggleLogsPause());
document.getElementById('logs-clear-btn').addEventListener('click', () => clearLogs());
document.getElementById('wasm-install-btn').addEventListener('click', () => installWasmExtension());
document.getElementById('mcp-add-btn').addEventListener('click', () => addMcpServer());
document.getElementById('skill-search-btn').addEventListener('click', () => searchClawHub());
document.getElementById('skill-install-btn').addEventListener('click', () => installSkillFromForm());
document.getElementById('settings-export-btn').addEventListener('click', () => exportSettings());
document.getElementById('settings-import-btn').addEventListener('click', () => importSettings());

// --- Delegated Event Handlers (for dynamically generated HTML) ---

document.addEventListener('click', function(e) {
  const el = e.target.closest('[data-action]');
  if (!el) return;
  const action = el.dataset.action;

  switch (action) {
    case 'copy-code':
      copyCodeBlock(el);
      break;
    case 'breadcrumb-root':
      e.preventDefault();
      loadMemoryTree();
      break;
    case 'breadcrumb-file':
      e.preventDefault();
      readMemoryFile(el.dataset.path);
      break;
    case 'cancel-job':
      e.stopPropagation();
      cancelJob(el.dataset.id);
      break;
    case 'open-job':
      openJobDetail(el.dataset.id);
      break;
    case 'close-job-detail':
      closeJobDetail();
      break;
    case 'restart-job':
      restartJob(el.dataset.id);
      break;
    case 'open-routine':
      openRoutineDetail(el.dataset.id);
      break;
    case 'toggle-routine':
      e.stopPropagation();
      toggleRoutine(el.dataset.id);
      break;
    case 'trigger-routine':
      e.stopPropagation();
      triggerRoutine(el.dataset.id);
      break;
    case 'delete-routine':
      e.stopPropagation();
      deleteRoutine(el.dataset.id, el.dataset.name);
      break;
    case 'close-routine-detail':
      closeRoutineDetail();
      break;
    case 'view-run-job':
      e.preventDefault();
      switchTab('jobs');
      openJobDetail(el.dataset.id);
      break;
    case 'copy-tee-report':
      copyTeeReport();
      break;
    case 'switch-language':
      if (typeof switchLanguage === 'function') switchLanguage(el.dataset.lang);
      break;
  }
});

document.getElementById('language-btn').addEventListener('click', function() {
  if (typeof toggleLanguageMenu === 'function') toggleLanguageMenu();
});

// --- Confirmation Modal ---

var _confirmModalCallback = null;

function showConfirmModal(title, message, onConfirm, confirmLabel, confirmClass) {
  var modal = document.getElementById('confirm-modal');
  document.getElementById('confirm-modal-title').textContent = title;
  document.getElementById('confirm-modal-message').textContent = message || '';
  document.getElementById('confirm-modal-message').style.display = message ? '' : 'none';
  var btn = document.getElementById('confirm-modal-btn');
  btn.textContent = confirmLabel || I18n.t('btn.confirm');
  btn.className = confirmClass || 'btn-danger';
  _confirmModalCallback = onConfirm;
  modal.style.display = 'flex';
  btn.focus();
}

function closeConfirmModal() {
  document.getElementById('confirm-modal').style.display = 'none';
  _confirmModalCallback = null;
}

document.getElementById('confirm-modal-btn').addEventListener('click', function() {
  if (_confirmModalCallback) _confirmModalCallback();
  closeConfirmModal();
});
document.getElementById('confirm-modal-cancel-btn').addEventListener('click', closeConfirmModal);
document.getElementById('confirm-modal').addEventListener('click', function(e) {
  if (e.target === this) closeConfirmModal();
});
document.addEventListener('keydown', function(e) {
  if (e.key === 'Escape' && document.getElementById('confirm-modal').style.display === 'flex') {
    closeConfirmModal();
  }
});

// --- Settings Import/Export ---

function exportSettings() {
  apiFetch('/api/settings/export').then(function(data) {
    var blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
    var url = URL.createObjectURL(blob);
    var a = document.createElement('a');
    a.href = url;
    a.download = 'ironclaw-settings.json';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
    showToast(I18n.t('settings.exportSuccess'), 'success');
  }).catch(function(err) {
    showToast(I18n.t('settings.exportFailed', { message: err.message }), 'error');
  });
}

function importSettings() {
  var input = document.createElement('input');
  input.type = 'file';
  input.accept = '.json,application/json';
  input.addEventListener('change', function() {
    if (!input.files || !input.files[0]) return;
    var reader = new FileReader();
    reader.onload = function() {
      try {
        var data = JSON.parse(reader.result);
        apiFetch('/api/settings/import', {
          method: 'POST',
          body: data,
        }).then(function() {
          showToast(I18n.t('settings.importSuccess'), 'success');
          loadSettingsSubtab(currentSettingsSubtab);
        }).catch(function(err) {
          showToast(I18n.t('settings.importFailed', { message: err.message }), 'error');
        });
      } catch (e) {
        showToast(I18n.t('settings.importFailed', { message: e.message }), 'error');
      }
    };
    reader.readAsText(input.files[0]);
  });
  input.click();
}

// --- Settings Search ---

document.getElementById('settings-search-input').addEventListener('input', function() {
  var query = this.value.toLowerCase();
  var activePanel = document.querySelector('.settings-subpanel.active');
  if (!activePanel) return;
  var rows = activePanel.querySelectorAll('.settings-row');
  if (rows.length === 0) return;
  var visibleCount = 0;
  rows.forEach(function(row) {
    var text = row.textContent.toLowerCase();
    if (query === '' || text.indexOf(query) !== -1) {
      row.classList.remove('search-hidden');
      if (!row.classList.contains('hidden')) visibleCount++;
    } else {
      row.classList.add('search-hidden');
    }
  });
  // Show/hide group titles based on visible children
  var groups = activePanel.querySelectorAll('.settings-group');
  groups.forEach(function(group) {
    var visibleRows = group.querySelectorAll('.settings-row:not(.search-hidden):not(.hidden)');
    if (visibleRows.length === 0 && query !== '') {
      group.style.display = 'none';
    } else {
      group.style.display = '';
    }
  });
  // Show/hide empty state
  var existingEmpty = activePanel.querySelector('.settings-search-empty');
  if (existingEmpty) existingEmpty.remove();
  if (query !== '' && visibleCount === 0) {
    var empty = document.createElement('div');
    empty.className = 'settings-search-empty';
    empty.textContent = I18n.t('settings.noMatchingSettings', { query: this.value });
    activePanel.appendChild(empty);
  }
});
