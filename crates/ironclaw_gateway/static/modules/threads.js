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

    activeWorkStore.rememberThreads(rememberedThreads);

    // Restore thread from URL hash if pending (deferred from restoreFromHash)
    if (window._pendingThreadRestore) {
      var pendingId = window._pendingThreadRestore;
      window._pendingThreadRestore = null;
      // Verify the thread exists in the loaded list
      var found = (pendingId === assistantThreadId) ||
        threads.some(function(t) { return t.id === pendingId; });
      if (found) {
        switchThread(pendingId);
        return;
      }
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
