function isSameInProgressTurn(lastTurn, inProgress) {
  if (!lastTurn || !inProgress) return false;

  if (lastTurn.user_message_id && inProgress.user_message_id) {
    return lastTurn.user_message_id === inProgress.user_message_id;
  }

  if (!lastTurn.user_message_id && !inProgress.user_message_id) {
    return !lastTurn.response && lastTurn.turn_number === inProgress.turn_number;
  }

  if (!inProgress.user_message_id && lastTurn.user_input && inProgress.user_input) {
    return !lastTurn.response && lastTurn.user_input === inProgress.user_input;
  }

  return false;
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

  // Show skeleton while loading (only for fresh loads)
  if (!isPaginating) {
    _chatToolActivity.reset(false);
    const chatContainer = document.getElementById('chat-messages');
    chatContainer.innerHTML = '';
    chatContainer.appendChild(renderSkeleton('message', 3));
  }

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
        if (turn.generated_images && turn.generated_images.length > 0) {
          for (const image of turn.generated_images) {
            const resolvedImage = resolveGeneratedImageForRender(currentThreadId, image);
            rememberGeneratedImage(
              currentThreadId,
              image.event_id,
              resolvedImage.dataUrl,
              resolvedImage.path
            );
            addGeneratedImage(
              resolvedImage.dataUrl,
              resolvedImage.path,
              image.event_id,
              false
            );
          }
        }
        if (turn.response) {
          addMessage('assistant', turn.response);
        }
      }
      // Re-inject pending user messages not yet in DB (#2409)
      const pending = _pendingUserMessages.get(currentThreadId);
      let freshPending = [];
      if (pending && pending.length > 0) {
        const now = Date.now();
        freshPending = pending.filter(p => now - p.timestamp < PENDING_MSG_TTL_MS);
        if (freshPending.length > 0) {
          const dbContentsCounts = new Map();
          data.turns
            .map(t => t.user_input)
            .filter(Boolean)
            .forEach(content => {
              dbContentsCounts.set(content, (dbContentsCounts.get(content) || 0) + 1);
            });
          for (const p of freshPending) {
            const count = dbContentsCounts.get(p.content) || 0;
            if (count > 0) {
              dbContentsCounts.set(p.content, count - 1);
            } else {
              const div = addMessage('user', p.content);
              if (p.images && p.images.length > 0) {
                appendImagesToMessage(div, p.images);
              }
            }
          }
          _pendingUserMessages.set(currentThreadId, freshPending);
        } else {
          _pendingUserMessages.delete(currentThreadId);
        }
      }
      container.scrollTop = container.scrollHeight;
      // Show welcome card when history is empty
      if (data.turns.length === 0 && !data.in_progress && freshPending.length === 0) {
        showWelcomeCard();
      }
      // Show processing indicator if the last turn is still in-progress
      var lastTurn = data.turns.length > 0 ? data.turns[data.turns.length - 1] : null;
      if (data.in_progress) {
        const sameLastTurn = isSameInProgressTurn(lastTurn, data.in_progress);
        if (!sameLastTurn && data.in_progress.user_input) {
          addMessage('user', data.in_progress.user_input);
        }
        showActivityThinking(ActivityEntry.t('activity.processing', 'Processing...'));
      } else if (lastTurn && !lastTurn.response && lastTurn.state === 'Processing') {
        showActivityThinking(ActivityEntry.t('activity.processing', 'Processing...'));
      }
      if (data.pending_gate) {
        const gateData = {
          ...data.pending_gate,
          thread_id: data.pending_gate.thread_id || currentThreadId,
        };
        handleGateRequired(gateData);
        const resume = parseGateResumeKind(gateData.resume_kind);
        if (!resume || resume.type === 'approval') {
          aqAddGate(gateData);
        }
      } else {
        // No pending gate for this history view. Keep a global auth overlay if
        // it belongs to a different thread; another tab/thread may still be
        // waiting on it.
        const overlay = getAuthOverlay();
        if (overlay) {
          const overlayThreadId = overlay.getAttribute('data-thread-id');
          if (overlayThreadId && overlayThreadId !== currentThreadId) {
            return;
          }
        }
        removeAuthCard();
        setAuthFlowPending(false);
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
        if (turn.generated_images && turn.generated_images.length > 0) {
          for (const image of turn.generated_images) {
            const resolvedImage = resolveGeneratedImageForRender(currentThreadId, image);
            rememberGeneratedImage(
              currentThreadId,
              image.event_id,
              resolvedImage.dataUrl,
              resolvedImage.path
            );
            fragment.appendChild(
              createGeneratedImageElement(
                resolvedImage.dataUrl,
                resolvedImage.path,
                image.event_id
              )
            );
          }
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
function createMessageElement(role, content, options) {
  const opts = options || {};
  const div = document.createElement('div');
  div.className = 'message ' + role;

  const ts = document.createElement('span');
  ts.className = 'message-timestamp';
  ts.textContent = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  div.appendChild(ts);

  // Message content
  const contentEl = document.createElement('div');
  contentEl.className = 'message-content';
  let copyText = opts.copyText || content;
  let parsedAttachments = opts.attachments || null;
  if (role === 'user' || role === 'system') {
    // User turns can carry an `<attachments>…</attachments>` payload appended
    // by the backend. Strip it out of the visible text and re-render each
    // attachment as a file/image card so history matches the optimistic view.
    // When the caller passed `options.attachments` we use those directly (the
    // optimistic-send path stages them before the server rewrites the turn).
    if (!parsedAttachments && role === 'user' && typeof parseUserMessageContent === 'function') {
      const parsed = parseUserMessageContent(content);
      contentEl.textContent = parsed.text;
      parsedAttachments = parsed.attachments;
      copyText = opts.copyText || parsed.copyText;
    } else {
      contentEl.textContent = content;
    }
  } else {
    div.setAttribute('data-raw', content);
    contentEl.innerHTML = renderMarkdown(content);
    // Upgrade structured data (JSON objects, etc.) into styled cards
    upgradeStructuredData(contentEl);
    // Syntax highlighting for code blocks
    if (typeof hljs !== 'undefined') {
      requestAnimationFrame(() => {
        contentEl.querySelectorAll('pre code').forEach(block => {
          hljs.highlightElement(block);
        });
      });
    }
  }
  div.appendChild(contentEl);

  if (
    role === 'user'
    && parsedAttachments
    && parsedAttachments.length > 0
    && typeof renderMessageAttachments === 'function'
  ) {
    renderMessageAttachments(div, parsedAttachments);
  }

  if (role === 'assistant' || role === 'user') {
    div.classList.add('has-copy');
    div.setAttribute('data-copy-text', copyText);
    const copyBtn = document.createElement('button');
    copyBtn.className = 'message-copy-btn';
    copyBtn.type = 'button';
    copyBtn.setAttribute('aria-label', I18n.t('message.copy'));
    copyBtn.textContent = I18n.t('message.copy');
    copyBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      copyMessage(copyBtn);
    });
    div.appendChild(copyBtn);
  }

  return div;
}

function addToolCallsSummary(toolCalls) {
  const container = document.getElementById('chat-messages');
  container.appendChild(createToolCallsSummaryElement(toolCalls));
  container.scrollTop = container.scrollHeight;
}

function createToolCallsSummaryElement(toolCalls) {
  return createActivityGroupFromHistory(toolCalls);
}

function createActivityGroupFromHistory(toolCalls) {
  return createActivityGroupFromEntries(
    toolCalls.map(normalizeHistoryToolCall),
    {
      includeSummaryDuration: false,
      showCardDurations: false,
      expandErrors: true,
    }
  );
}

function removeScrollSpinner() {
  const spinner = document.getElementById('scroll-load-spinner');
  if (spinner) spinner.remove();
}

// --- Threads ---

function threadTitle(thread) {
  if (thread.title) return thread.title;
  const ch = thread.channel || 'gateway';
  if (thread.thread_type === 'heartbeat') return I18n.t('thread.heartbeatAlerts');
  if (thread.thread_type === 'routine') return I18n.t('thread.routine');
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
  // Show skeleton while loading
  const threadListEl = document.getElementById('thread-list');
  if (threadListEl && threadListEl.children.length === 0) {
    threadListEl.innerHTML = '';
    threadListEl.appendChild(renderSkeleton('row', 4));
  }

  apiFetch('/api/chat/threads').then((data) => {
    const rememberedThreads = [];
    // Pinned assistant thread
    if (data.assistant_thread) {
      assistantThreadId = data.assistant_thread.id;
      rememberedThreads.push({
        threadId: data.assistant_thread.id,
        meta: {
          label: I18n.t('thread.assistant'),
          source: 'chat',
        },
      });
      const el = document.getElementById('assistant-thread');
      const isActive = currentThreadId === assistantThreadId;
      el.className = 'assistant-item' + (isActive ? ' active' : '');
      el.querySelectorAll('.thread-processing').forEach((node) => node.remove());
      const labelEl = document.getElementById('assistant-label');
      if (labelEl) {
        labelEl.textContent = I18n.t('thread.assistant');
      }
      const meta = document.getElementById('assistant-meta');
      meta.textContent = relativeTime(data.assistant_thread.updated_at);
      if (data.assistant_thread.state === 'Processing' && !isActive) {
        const spinner = document.createElement('span');
        spinner.className = 'thread-processing';
        spinner.innerHTML = '<div class="spinner"></div>';
        el.appendChild(spinner);
      }
    }

    // Regular threads
    const list = document.getElementById('thread-list');
    list.innerHTML = '';
    const threads = data.threads || [];
    for (const thread of threads) {
      rememberedThreads.push({
        threadId: thread.id,
        meta: {
          label: threadTitle(thread),
          source: 'chat',
        },
      });
      const item = document.createElement('div');
      const isActive = thread.id === currentThreadId;
      item.className = 'thread-item' + (isActive ? ' active' : '');
      item.setAttribute('data-thread-id', thread.id);

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

      if (aqThreadHasPendingGate(thread.id)) {
        const gateDot = document.createElement('span');
        gateDot.className = 'thread-item-gate-dot';
        gateDot.title = 'Pending approval';
        gateDot.setAttribute('aria-label', 'Pending approval');
        item.appendChild(gateDot);
      }

      // Processing spinner
      if ((thread.state === 'Processing' || processingThreads.has(thread.id)) && !isActive) {
        const spinner = document.createElement('span');
        spinner.className = 'thread-processing';
        spinner.innerHTML = '<div class="spinner"></div>';
        item.appendChild(spinner);
      }

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

    if (data.pending_gates && data.pending_gates.length > 0) {
      for (const gate of data.pending_gates) {
        aqAddGate({
          request_id: gate.request_id,
          tool_name: gate.tool_name,
          description: gate.description,
          parameters: gate.parameters,
          thread_id: gate.thread_id,
          allow_always: true,
        });
      }
    }

    activeWorkStore.rememberThreads(rememberedThreads);

    // Restore thread from URL hash if pending (deferred from restoreFromHash).
    // Switch even when the thread is not in the loaded sidebar list — the
    // list is capped and older threads can age off, but the history API
    // falls back to the DB by thread_id. Silently dropping the URL here was
    // the #1 source of "my thread disappeared" reports.
    if (window._pendingThreadRestore) {
      var pendingId = window._pendingThreadRestore;
      window._pendingThreadRestore = null;
      var inSidebar = (pendingId === assistantThreadId) ||
        threads.some(function(t) { return t.id === pendingId; });
      if (!inSidebar && window.DEBUG_CHAT_RESTORE === true) {
        console.warn('[chat] thread', pendingId, 'not in sidebar list; loading via history API');
      }
      switchThread(pendingId);
      return;
    }

    // Preserve the currently open thread even when it falls outside the
    // sidebar's recency window. The history view can still load that thread
    // directly, and follow-up sends must stay attached to it.

    // Reopen the server's active thread on first load. This keeps the visible
    // chat attached to an in-flight agent turn after a browser refresh, even
    // when the URL does not carry an explicit thread hash.
    if (!currentThreadId) {
      const activeThreadId = data.active_thread || null;
      if (activeThreadId && activeThreadId === assistantThreadId) {
        switchToAssistant();
        return;
      }
      if (activeThreadId && threads.some(t => t.id === activeThreadId)) {
        // Skip external-channel threads (e.g. HTTP, Telegram) — they are
        // read-only in the web UI, so auto-switching to one would leave the
        // chat input disabled.  Fall through to the assistant thread instead.
        const activeThread = threads.find(t => t.id === activeThreadId);
        if (!isReadOnlyChannel(activeThread.channel)) {
          switchThread(activeThreadId);
          return;
        }
      }
      if (assistantThreadId) {
        switchToAssistant();
        return;
      }
    }

    // Enable/disable chat input based on channel type
    if (currentThreadId) {
      const currentThread = currentThreadId === assistantThreadId
        ? data.assistant_thread
        : threads.find(t => t.id === currentThreadId);
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

// Approval Queue Tray — cross-thread pending gate visibility

function aqAddGate(data) {
  if (!data || !data.request_id) return;
  if (pendingGates.has(data.request_id)) return;

  const entry = {
    request_id: data.request_id,
    tool_name: data.tool_name || 'unknown',
    description: data.description || '',
    parameters: data.parameters || null,
    thread_id: data.thread_id || currentThreadId,
    allow_always: data.allow_always !== false,
    created_at: Date.now(),
  };
  pendingGates.set(data.request_id, entry);
  aqRenderItem(entry);
  aqUpdateIndicators();
}

function aqResolveGate(requestId, action) {
  pendingGates.delete(requestId);
  const selector = window.CSS && typeof window.CSS.escape === 'function'
    ? '.aq-item[data-request-id="' + window.CSS.escape(requestId) + '"]'
    : '.aq-item[data-request-id="' + requestId + '"]';
  const itemEl = document.querySelector(selector);
  if (itemEl && !itemEl.classList.contains('resolved')) {
    const actions = itemEl.querySelector('.aq-item-actions');
    const denied = action === 'deny';
    const label = document.createElement('span');
    label.className = 'aq-resolved-label ' + (denied ? 'denied' : 'approved');
    label.textContent = denied
      ? '✗ ' + I18n.t('approval.denied')
      : '✓ ' + I18n.t('approval.approved');
    if (actions) actions.replaceChildren(label);
    itemEl.classList.add('resolved');
    setTimeout(() => {
      itemEl.remove();
      aqUpdateIndicators();
    }, 500);
    return;
  }
  aqUpdateIndicators();
}

function aqRenderItem(entry) {
  const container = document.getElementById('aq-items');
  if (!container) return;

  const item = document.createElement('div');
  item.className = 'aq-item';
  item.setAttribute('data-request-id', entry.request_id);
  item.setAttribute('data-thread-id', entry.thread_id || '');
  item.addEventListener('click', function () {
    if (entry.thread_id && entry.thread_id !== currentThreadId) {
      switchThread(entry.thread_id);
    }
  });

  const top = document.createElement('div');
  top.className = 'aq-item-top';

  const toolSpan = document.createElement('span');
  toolSpan.className = 'aq-item-tool';
  toolSpan.textContent = entry.tool_name;
  top.appendChild(toolSpan);

  const threadSpan = document.createElement('span');
  threadSpan.className = 'aq-item-thread';
  threadSpan.textContent = aqThreadLabel(entry.thread_id);
  top.appendChild(threadSpan);

  const timeSpan = document.createElement('span');
  timeSpan.className = 'aq-item-time';
  timeSpan.textContent = 'just now';
  top.appendChild(timeSpan);

  item.appendChild(top);

  if (entry.description) {
    const desc = document.createElement('div');
    desc.className = 'aq-item-desc';
    desc.textContent = entry.description;
    item.appendChild(desc);
  }

  const actions = document.createElement('div');
  actions.className = 'aq-item-actions';

  const approveBtn = document.createElement('button');
  approveBtn.className = 'aq-btn aq-approve';
  approveBtn.textContent = I18n.t('approval.approve');
  approveBtn.addEventListener('click', function (e) {
    e.stopPropagation();
    sendApprovalAction(entry.request_id, 'approve', entry.thread_id);
  });
  actions.appendChild(approveBtn);

  if (entry.allow_always) {
    const alwaysBtn = document.createElement('button');
    alwaysBtn.className = 'aq-btn aq-always';
    alwaysBtn.textContent = I18n.t('approval.always');
    alwaysBtn.addEventListener('click', function (e) {
      e.stopPropagation();
      sendApprovalAction(entry.request_id, 'always', entry.thread_id);
    });
    actions.appendChild(alwaysBtn);
  }

  const denyBtn = document.createElement('button');
  denyBtn.className = 'aq-btn aq-deny';
  denyBtn.textContent = I18n.t('approval.deny');
  denyBtn.addEventListener('click', function (e) {
    e.stopPropagation();
    sendApprovalAction(entry.request_id, 'deny', entry.thread_id);
  });
  actions.appendChild(denyBtn);

  const viewBtn = document.createElement('button');
  viewBtn.className = 'aq-btn aq-view';
  viewBtn.innerHTML = '&#x2192;';
  viewBtn.title = 'Jump to thread';
  viewBtn.addEventListener('click', function (e) {
    e.stopPropagation();
    if (entry.thread_id) switchThread(entry.thread_id);
  });
  actions.appendChild(viewBtn);

  item.appendChild(actions);
  container.insertBefore(item, container.firstChild);
}

function aqUpdateIndicators() {
  const count = pendingGates.size;
  const tray = document.getElementById('approval-tray');
  const badge = document.getElementById('tab-approval-badge');
  const countEl = document.getElementById('aq-count');

  if (tray) {
    tray.classList.toggle('visible', count > 0);
    if (count > 0 && !tray.classList.contains('collapsed')) {
      tray.setAttribute('aria-expanded', 'true');
    } else if (count === 0) {
      tray.setAttribute('aria-expanded', 'false');
    }
  }
  if (badge) {
    badge.textContent = count;
    badge.classList.toggle('empty', count === 0);
    if (count > 0) {
      badge.classList.remove('pulse');
      void badge.offsetWidth;
      badge.classList.add('pulse');
    }
  }
  if (countEl) {
    countEl.textContent = count;
  }

  aqRefreshGateDots();
}

function aqThreadHasPendingGate(threadId) {
  for (const entry of pendingGates.values()) {
    if (entry.thread_id === threadId) return true;
  }
  return false;
}

function aqRefreshGateDots() {
  const items = document.querySelectorAll('.thread-item[data-thread-id]');
  items.forEach(function (item) {
    const threadId = item.getAttribute('data-thread-id');
    const hasDot = item.querySelector('.thread-item-gate-dot');
    const needsDot = aqThreadHasPendingGate(threadId);
    if (needsDot && !hasDot) {
      const gateDot = document.createElement('span');
      gateDot.className = 'thread-item-gate-dot';
      gateDot.title = 'Pending approval';
      gateDot.setAttribute('aria-label', 'Pending approval');
      item.appendChild(gateDot);
    } else if (!needsDot && hasDot) {
      hasDot.remove();
    }
  });
}

function aqThreadLabel(threadId) {
  if (!threadId || threadId === assistantThreadId) return 'main';
  return threadId.length > 12 ? threadId.substring(0, 8) : threadId;
}

(function initApprovalQueueTray() {
  const header = document.getElementById('aq-header');
  if (header) {
    header.addEventListener('click', function () {
      const tray = document.getElementById('approval-tray');
      if (!tray) return;
      tray.classList.toggle('collapsed');
      tray.setAttribute('aria-expanded', String(!tray.classList.contains('collapsed')));
    });
  }

  function batchApprovalAction(action) {
    const entries = Array.from(pendingGates.values());
    const requests = entries.map(function (entry, index) {
      return new Promise(function (resolve) {
        setTimeout(function () {
          resolve(sendApprovalAction(entry.request_id, action, entry.thread_id));
        }, index * 100);
      });
    });
    return Promise.allSettled(requests);
  }

  const approveAll = document.getElementById('aq-approve-all');
  if (approveAll) {
    approveAll.addEventListener('click', function (e) {
      e.stopPropagation();
      batchApprovalAction('approve');
    });
  }

  const denyAll = document.getElementById('aq-deny-all');
  if (denyAll) {
    denyAll.addEventListener('click', function (e) {
      e.stopPropagation();
      batchApprovalAction('deny');
    });
  }
})();

function disableChatInputReadOnly() {
  const input = document.getElementById('chat-input');
  const btn = document.getElementById('send-btn');
  if (input) {
    input.disabled = true;
    input.placeholder = I18n.t('chat.readOnlyThread');
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
  updateHash();
  if (window.innerWidth <= 768) {
    const sidebar = document.getElementById('thread-sidebar');
    sidebar.classList.remove('expanded-mobile');
    document.getElementById('thread-toggle-btn').innerHTML = '&raquo;';
  }
}

function switchThread(threadId) {
  clearSuggestionChips();
  finalizeActivityGroup();
  _turnResponseReceived = false;
  if (_doneWithoutResponseTimer) {
    clearTimeout(_doneWithoutResponseTimer);
    _doneWithoutResponseTimer = null;
  }
  currentThreadId = threadId;
  unreadThreads.delete(threadId);
  processingThreads.delete(threadId);
  hasMore = false;
  oldestTimestamp = null;
  loadHistory();
  loadThreads();
  updateHash();
  if (window.innerWidth <= 768) {
    const sidebar = document.getElementById('thread-sidebar');
    sidebar.classList.remove('expanded-mobile');
    document.getElementById('thread-toggle-btn').innerHTML = '&raquo;';
  }
}

function createNewThread() {
  apiFetch('/api/chat/thread/new', { method: 'POST' }).then((data) => {
    currentThreadId = data.id || null;
    currentThreadIsReadOnly = false;
    document.getElementById('chat-messages').innerHTML = '';
    showWelcomeCard();
    enableChatInput();
    loadThreads();
    updateHash();
  }).catch((err) => {
    showToast(I18n.t('chat.threadCreateFailed', { message: err.message }), 'error');
  });
}

function toggleThreadSidebar() {
  const sidebar = document.getElementById('thread-sidebar');
  const isMobile = window.innerWidth <= 768;
  if (isMobile) {
    sidebar.classList.toggle('expanded-mobile');
  } else {
    sidebar.classList.toggle('collapsed');
  }
  const btn = document.getElementById('thread-toggle-btn');
  const isOpen = isMobile
    ? sidebar.classList.contains('expanded-mobile')
    : !sidebar.classList.contains('collapsed');
  btn.innerHTML = isOpen ? '&laquo;' : '&raquo;';
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
  const sendBtn = document.getElementById('send-btn');
  if (sendBtn) {
    sendBtn.classList.toggle('active', chatInput.value.trim().length > 0);
  }
});
chatInput.addEventListener('blur', () => {
  // Small delay so mousedown on autocomplete item fires first
  setTimeout(hideSlashAutocomplete, 150);
});

// Infinite scroll: load older messages when scrolled near the top.
// Also toggles the scroll-to-bottom button when the user has scrolled up.
// The handler is rAF-throttled so rapid scroll events coalesce into at most
// one layout read per frame.
let _scrollRafPending = false;
document.getElementById('chat-messages').addEventListener('scroll', function () {
  const container = this;
  if (container.scrollTop < 100 && hasMore && !loadingOlder) {
    loadingOlder = true;
    // Show spinner at top
    const spinner = document.createElement('div');
    spinner.id = 'scroll-load-spinner';
    spinner.className = 'scroll-load-spinner';
    spinner.innerHTML = '<div class="spinner"></div> Loading older messages...';
    container.insertBefore(spinner, container.firstChild);
    loadHistory(oldestTimestamp);
  }
  if (_scrollRafPending) return;
  _scrollRafPending = true;
  requestAnimationFrame(() => {
    _scrollRafPending = false;
    const btn = document.getElementById('scroll-to-bottom-btn');
    if (!btn) return;
    const distanceFromBottom = container.scrollHeight - container.scrollTop - container.clientHeight;
    btn.style.display = distanceFromBottom > 200 ? 'flex' : 'none';
  });
});

document.getElementById('scroll-to-bottom-btn').addEventListener('click', () => {
  const container = document.getElementById('chat-messages');
  container.scrollTo({ top: container.scrollHeight, behavior: 'smooth' });
});

// Keep the scroll-to-bottom button anchored just above the chat input,
// even when the textarea grows to multiple lines.
(() => {
  const input = document.querySelector('.chat-container .chat-input');
  const container = document.querySelector('.chat-container');
  if (!input || !container || typeof ResizeObserver === 'undefined') return;
  const ro = new ResizeObserver((entries) => {
    for (const entry of entries) {
      const h = entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height;
      container.style.setProperty('--chat-input-height', `${Math.ceil(h)}px`);
    }
  });
  ro.observe(input);
})();

function autoResizeTextarea(el) {
  const prev = el.offsetHeight;
  el.style.height = 'auto';
  const target = Math.min(el.scrollHeight, 120);
  el.style.height = prev + 'px';
  requestAnimationFrame(() => {
    el.style.height = target + 'px';
  });
}

// --- Tabs ---

document.querySelectorAll('.tab-bar button[data-tab]').forEach((btn) => {
  btn.addEventListener('click', () => {
    const tab = btn.getAttribute('data-tab');
    switchTab(tab);
  });
});

function switchTab(tab) {
  tab = normalizeTabForEngineMode(tab);
  currentTab = tab;
  // NOTE: this function takes a `tab` argument that may originate from
  // workspace-supplied `layout.tabs.default_tab`, so it must NOT be
  // refactored into a `querySelector('[data-tab="' + tab + '"]')`
  // shape. The current form does string equality on the
  // `getAttribute('data-tab')` value of every button (the loop below)
  // and on `p.id === 'tab-' + tab` for the panel — neither path
  // interpolates `tab` into a CSS selector, so a hostile id can't
  // alter the selector match. If a future change needs to look up a
  // single button by id directly, wrap `tab` in `CSS.escape()` first.
  document.querySelectorAll('.tab-bar button[data-tab]').forEach((b) => {
    b.classList.toggle('active', b.getAttribute('data-tab') === tab);
  });
  document.querySelectorAll('.tab-panel').forEach((p) => {
    p.classList.toggle('active', p.id === 'tab-' + tab);
  });
  applyAriaAttributes();

  if (tab === 'memory') {
    loadMemoryTree();
    // Auto-open README.md on first visit (no file selected yet)
    if (!currentMemoryPath) readMemoryFile('README.md');
  }
  if (tab === 'jobs') loadJobs();
  if (tab === 'projects') {
    loadProjectsOverview();
  } else if (crCurrentProjectId) {
    // Tear down project widgets and reset drill-in state when leaving
    // the Projects tab so widgets don't keep running in the background.
    crBackToOverview();
  }
  if (tab === 'routines') loadRoutines();
  if (tab === 'logs') { connectLogSSE(); applyLogFilters(); }
  else if (logEventSource) { logEventSource.close(); logEventSource = null; }
  if (tab === 'settings') {
    loadSettingsSubtab(currentSettingsSubtab);
  } else {
    stopPairingPoll();
  }
  updateTabIndicator();
  updateHash();
}

function updateTabIndicator() {
  const indicator = document.getElementById('tab-indicator');
  if (!indicator) return;
  const activeBtn = document.querySelector('.tab-bar button[data-tab].active');
  if (!activeBtn) {
    indicator.style.width = '0';
    return;
  }
  const bar = activeBtn.closest('.tab-bar');
  const barRect = bar.getBoundingClientRect();
  const btnRect = activeBtn.getBoundingClientRect();
  indicator.style.left = (btnRect.left - barRect.left) + 'px';
  indicator.style.width = btnRect.width + 'px';
}

window.addEventListener('resize', updateTabIndicator);

// --- Memory (filesystem tree) ---
