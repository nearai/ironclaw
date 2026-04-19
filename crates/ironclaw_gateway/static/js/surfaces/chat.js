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
      alert(I18n.t('chat.imageTooBig', { name: file.name, size: (file.size / 1024 / 1024).toFixed(1) }));
      return;
    }
    if (stagedImages.length >= MAX_STAGED_IMAGES) {
      alert(I18n.t('chat.maxImages', { n: MAX_STAGED_IMAGES }));
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

function createGeneratedImageElement(dataUrl, path, eventId) {
  const card = document.createElement('div');
  card.className = 'generated-image-card';
  if (eventId) {
    card.dataset.imageEventId = eventId;
  }

  if (isSafeGeneratedImageDataUrl(dataUrl)) {
    const img = document.createElement('img');
    img.className = 'generated-image';
    img.src = dataUrl;
    img.alt = 'Generated image';
    card.appendChild(img);
  } else {
    const placeholder = document.createElement('div');
    placeholder.className = 'generated-image-placeholder';
    placeholder.textContent = 'Generated image unavailable in history payload';
    card.appendChild(placeholder);
  }

  if (path) {
    const pathLabel = document.createElement('div');
    pathLabel.className = 'generated-image-path';
    pathLabel.textContent = path;
    card.appendChild(pathLabel);
  }

  return card;
}

function isSafeGeneratedImageDataUrl(dataUrl) {
  return typeof dataUrl === 'string' && /^data:image\//i.test(dataUrl);
}

function hasRenderedGeneratedImage(container, eventId) {
  if (!eventId) return false;
  return Array.from(container.querySelectorAll('.generated-image-card')).some((card) => {
    return card.dataset.imageEventId === eventId;
  });
}

function addGeneratedImage(dataUrl, path, eventId, shouldScroll = true) {
  const container = document.getElementById('chat-messages');
  if (hasRenderedGeneratedImage(container, eventId)) {
    return;
  }
  const card = createGeneratedImageElement(dataUrl, path, eventId);
  container.appendChild(card);
  if (shouldScroll) {
    container.scrollTop = container.scrollHeight;
  }
}

function rememberGeneratedImage(threadId, eventId, dataUrl, path) {
  if (!threadId || !eventId || !isSafeGeneratedImageDataUrl(dataUrl)) return;
  const normalizedPath = path || null;
  let images = generatedImagesByThread.get(threadId);
  if (!images) {
    if (generatedImagesByThread.size >= GENERATED_IMAGE_THREAD_CACHE_CAP) {
      const oldestThreadId = generatedImagesByThread.keys().next().value;
      if (oldestThreadId) {
        generatedImagesByThread.delete(oldestThreadId);
      }
    }
    images = [];
    generatedImagesByThread.set(threadId, images);
  } else {
    // Refresh insertion order so recently viewed/updated threads stay cached.
    generatedImagesByThread.delete(threadId);
    generatedImagesByThread.set(threadId, images);
  }
  if (images.some(img => img.eventId === eventId)) {
    return;
  }
  images.push({ eventId, dataUrl, path: normalizedPath });
  while (images.length > GENERATED_IMAGES_PER_THREAD_CAP) {
    images.shift();
  }
}

function getRememberedGeneratedImage(threadId, eventId) {
  if (!threadId || !eventId) return null;
  const images = generatedImagesByThread.get(threadId);
  if (!images) return null;
  return images.find(img => img.eventId === eventId) || null;
}

function resolveGeneratedImageForRender(threadId, image) {
  const normalizedPath = image.path || null;
  if (image.data_url) {
    return { dataUrl: image.data_url, path: normalizedPath };
  }
  const remembered = getRememberedGeneratedImage(threadId, image.event_id);
  if (remembered) {
    return { dataUrl: remembered.dataUrl, path: remembered.path };
  }
  return { dataUrl: null, path: normalizedPath };
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

function sendApprovalAction(requestId, action, threadId) {
  const card = document.querySelector('.approval-card[data-request-id="' + requestId + '"]');
  const targetThreadId = threadId || (card ? card.getAttribute('data-thread-id') : null) || currentThreadId;
  apiFetch('/api/chat/gate/resolve', {
    method: 'POST',
    body: {
      request_id: requestId,
      thread_id: targetThreadId,
      resolution: action === 'deny' ? 'denied' : 'approved',
      always: action === 'always',
    },
  }).catch((err) => {
    addMessage('system', 'Failed to send approval: ' + err.message);
  });

  // Disable buttons and show confirmation on the card
  if (card) {
    const buttons = card.querySelectorAll('.approval-actions button');
    buttons.forEach((btn) => {
      btn.disabled = true;
    });
    const actions = card.querySelector('.approval-actions');
    const label = document.createElement('span');
    label.className = 'approval-resolved gw-approval__resolved' + (action === 'deny' ? '--deny' : '--ok');
    const labelText = action === 'approve' ? I18n.t('approval.approved') : action === 'always' ? I18n.t('approval.alwaysApproved') : I18n.t('approval.denied');
    label.textContent = (action === 'deny' ? '\u2717 ' : '\u2713 ') + labelText;
    actions.appendChild(label);
    // Remove the card after showing the confirmation briefly
    setTimeout(() => { card.remove(); }, 1500);
  }
}

