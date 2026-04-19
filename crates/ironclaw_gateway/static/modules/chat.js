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
  removeWelcomeCard();
  _turnResponseReceived = false;
  if (_doneWithoutResponseTimer) {
    clearTimeout(_doneWithoutResponseTimer);
    _doneWithoutResponseTimer = null;
  }
  const input = document.getElementById('chat-input');
  if (authFlowPending) {
    showToast(I18n.t('chat.authRequiredBeforeSend'), 'info');
    const tokenField = document.querySelector('.auth-card .auth-token-input input');
    if (tokenField) tokenField.focus();
    return;
  }
  if (!currentThreadId) {
    console.warn('sendMessage: no thread selected, ignoring');
    return;
  }
  if (_sendCooldown) return;
  const content = input.value.trim();
  if (!content && stagedImages.length === 0) return;

  // Intercept approval keywords when an unresolved approval card is pending.
  // Find the most recent unresolved card for the current thread (resolved cards
  // linger 1.5s before removal; cards from other threads must not be matched).
  const approvalCards = Array.from(document.querySelectorAll('.approval-card'));
  const approvalCard = approvalCards.reverse().find(card => {
    if (card.querySelector('.approval-resolved')) return false;
    const cardThreadId = card.getAttribute('data-thread-id');
    return !cardThreadId || cardThreadId === currentThreadId;
  });
  if (approvalCard && content) {
    const lower = content.toLowerCase();
    let action = null;
    if (['yes', 'y', 'approve', 'ok', '/approve', '/yes', '/y'].includes(lower)) {
      action = 'approve';
    } else if (['always', 'a', 'yes always', 'approve always', '/always', '/a'].includes(lower)) {
      action = 'always';
    } else if (['no', 'n', 'deny', 'reject', 'cancel', '/deny', '/no', '/n'].includes(lower)) {
      action = 'deny';
    }
    if (action) {
      input.value = '';
      autoResizeTextarea(input);
      input.focus();
      const requestId = approvalCard.getAttribute('data-request-id');
      const threadId = approvalCard.getAttribute('data-thread-id');
      if (requestId) {
        sendApprovalAction(requestId, action, threadId);
      }
      return;
    }
  }

  // Snapshot attached images before the body block clears stagedImages, so the
  // optimistic display and the pending entry both keep them.
  const attachedImageDataUrls = stagedImages.map(img => img.dataUrl);
  const userMsg = addMessage('user', content || '(images attached)');
  if (attachedImageDataUrls.length > 0) {
    appendImagesToMessage(userMsg, attachedImageDataUrls);
  }
  pruneOldMessages();
  if (currentThreadId) {
    activeWorkStore.updateThread(currentThreadId, {
      statusText: ActivityEntry.t('activity.starting', 'Starting'),
    });
  }
  input.value = '';
  autoResizeTextarea(input);
  input.focus();

  // Track as pending so loadHistory() can re-inject if DB hasn't persisted yet (#2409)
  let pendingId = null;
  const pendingThreadId = currentThreadId;
  if (currentThreadId) {
    const displayContent = content || '(images attached)';
    if (!_pendingUserMessages.has(currentThreadId)) {
      _pendingUserMessages.set(currentThreadId, []);
    }
    pendingId = _nextPendingId++;
    _pendingUserMessages.get(currentThreadId).push({
      id: pendingId,
      content: displayContent,
      images: attachedImageDataUrls,
      timestamp: Date.now(),
    });
  }

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
    // Remove the pending entry so it won't be re-injected on thread switch (#2498)
    if (pendingId !== null && pendingThreadId) {
      const arr = _pendingUserMessages.get(pendingThreadId);
      if (arr) {
        const filtered = arr.filter(p => p.id !== pendingId);
        if (filtered.length > 0) {
          _pendingUserMessages.set(pendingThreadId, filtered);
        } else {
          _pendingUserMessages.delete(pendingThreadId);
        }
      }
    }
    // Handle rate limiting (429)
    if (err.status === 429) {
      showToast(I18n.t('chat.rateLimited'), 'error');
      _sendCooldown = true;
      const sendBtn = document.getElementById('send-btn');
      if (sendBtn) sendBtn.disabled = true;
      setTimeout(() => {
        _sendCooldown = false;
        if (sendBtn) sendBtn.disabled = false;
      }, 2000);
    }
    // Keep the user message in DOM, add a retry link
    if (userMsg) {
      userMsg.classList.add('send-failed');
      userMsg.style.borderStyle = 'dashed';
      const retryLink = document.createElement('a');
      retryLink.className = 'retry-link';
      retryLink.href = '#';
      retryLink.textContent = I18n.t('common.retry');
      retryLink.addEventListener('click', (e) => {
        e.preventDefault();
        if (userMsg.parentNode) userMsg.parentNode.removeChild(userMsg);
        input.value = content;
        sendMessage();
      });
      userMsg.appendChild(retryLink);
    }
  });
}

function enableChatInput() {
  if (currentThreadIsReadOnly || authFlowPending) return;
  const input = document.getElementById('chat-input');
  const btn = document.getElementById('send-btn');
  if (input) {
    input.disabled = false;
    input.placeholder = I18n.t('chat.inputPlaceholder');
  }
  if (btn) btn.disabled = false;
}
// --- Plan Checklist ---

function renderPlanChecklist(data) {
  const chatContainer = document.getElementById('chat-messages');
  const planId = data.plan_id;

  // Find or create the plan container
  let container = chatContainer.querySelector('.plan-container[data-plan-id="' + CSS.escape(planId) + '"]');
  if (!container) {
    container = document.createElement('div');
    container.className = 'plan-container';
    container.setAttribute('data-plan-id', planId);
    chatContainer.appendChild(container);
  }

  // Clear and rebuild
  container.innerHTML = '';

  // Header
  const header = document.createElement('div');
  header.className = 'plan-header';

  const title = document.createElement('span');
  title.className = 'plan-title';
  title.textContent = data.title || planId;
  header.appendChild(title);

  const badge = document.createElement('span');
  badge.className = 'plan-status-badge plan-status-' + (data.status || 'draft');
  badge.textContent = data.status || 'draft';
  header.appendChild(badge);

  container.appendChild(header);

  // Steps
  if (data.steps && data.steps.length > 0) {
    const stepsList = document.createElement('div');
    stepsList.className = 'plan-steps';

    let completed = 0;
    for (const step of data.steps) {
      const stepEl = document.createElement('div');
      stepEl.className = 'plan-step';
      stepEl.setAttribute('data-status', step.status || 'pending');

      const icon = document.createElement('span');
      icon.className = 'plan-step-icon';
      if (step.status === 'completed') {
        icon.textContent = '\u2713'; // checkmark
        completed++;
      } else if (step.status === 'failed') {
        icon.textContent = '\u2717'; // X
      } else if (step.status === 'in_progress') {
        icon.innerHTML = '<span class="plan-spinner"></span>';
      } else {
        icon.textContent = '\u25CB'; // circle
      }
      stepEl.appendChild(icon);

      const text = document.createElement('span');
      text.className = 'plan-step-text';
      text.textContent = step.title;
      stepEl.appendChild(text);

      if (step.result) {
        const result = document.createElement('span');
        result.className = 'plan-step-result';
        result.textContent = step.result;
        stepEl.appendChild(result);
      }

      stepsList.appendChild(stepEl);
    }
    container.appendChild(stepsList);

    // Summary
    const summary = document.createElement('div');
    summary.className = 'plan-summary';
    summary.textContent = completed + ' of ' + data.steps.length + ' steps completed';
    if (data.mission_id) {
      summary.textContent += ' \u00b7 Mission: ' + data.mission_id.substring(0, 8);
    }
    container.appendChild(summary);
  }

  chatContainer.scrollTop = chatContainer.scrollHeight;
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
    browseBtn.rel = 'noopener noreferrer';
    browseBtn.textContent = I18n.t('jobs.browse');
    card.appendChild(browseBtn);
  }

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
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
        handleGateRequired({
          ...data.pending_gate,
          thread_id: data.pending_gate.thread_id || currentThreadId,
        });
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
function createMessageElement(role, content) {
  const div = document.createElement('div');
  div.className = 'message ' + role;

  const ts = document.createElement('span');
  ts.className = 'message-timestamp';
  ts.textContent = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  div.appendChild(ts);

  // Message content
  const contentEl = document.createElement('div');
  contentEl.className = 'message-content';
  if (role === 'user' || role === 'system') {
    contentEl.textContent = content;
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

  if (role === 'assistant' || role === 'user') {
    div.classList.add('has-copy');
    div.setAttribute('data-copy-text', content);
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

function createReasoningBlock(content, opts) {
  var durationMs = (opts && opts.duration_ms) || 0;
  var tokens = (opts && opts.tokens) || 0;

  var wrap = document.createElement('div');
  wrap.className = 'reasoning-wrap';

  var avatar = document.createElement('div');
  avatar.className = 'reasoning-avatar';
  avatar.textContent = '\u2726'; // ✦
  wrap.appendChild(avatar);

  var block = document.createElement('div');
  block.className = 'reasoning-block';

  var header = document.createElement('button');
  header.className = 'reasoning-header';
  header.type = 'button';

  var chevron = document.createElement('span');
  chevron.className = 'reasoning-chevron';
  chevron.textContent = '\u203A'; // ›
  header.appendChild(chevron);

  var label = document.createElement('span');
  label.className = 'reasoning-label';
  label.textContent = I18n.t('message.thinking') || 'Thinking';
  header.appendChild(label);

  if (durationMs || tokens) {
    var meta = document.createElement('span');
    meta.className = 'reasoning-meta';
    var parts = [];
    if (durationMs) parts.push((durationMs / 1000).toFixed(1) + 's');
    if (tokens) parts.push(tokens + ' tokens');
    meta.textContent = parts.join(' \u00B7 ');
    header.appendChild(meta);
  }

  block.appendChild(header);

  var body = document.createElement('div');
  body.className = 'reasoning-body';
  body.textContent = content;
  block.appendChild(body);

  header.addEventListener('click', function() {
    chevron.classList.toggle('open');
    body.classList.toggle('open');
  });

  wrap.appendChild(block);
  return wrap;
}

function addReasoningBlock(content, opts) {
  var container = document.getElementById('chat-messages');
  container.appendChild(createReasoningBlock(content, opts));
  container.scrollTop = container.scrollHeight;
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
