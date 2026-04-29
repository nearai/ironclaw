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

async function sendMessage() {
  // Wait for any in-flight FileReader decode so an Enter-press mid-upload
  // still includes the attachment in the next /api/chat/send body.
  if (pendingAttachmentReads.length > 0) {
    await Promise.all([...pendingAttachmentReads]);
  }
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
  if (!content && stagedAttachments.length === 0) return;

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

  // Snapshot attachments before the body block clears them, so the optimistic
  // display, pending entry, and retry handler all see the same view the user
  // pressed Enter on.
  const pendingAttachmentsForDisplay = stagedAttachments.map(att => ({
    kind: att.kind || (att.mime_type && att.mime_type.startsWith('image/') ? 'image' : 'document'),
    filename: att.filename || 'attachment',
    mime_type: att.mime_type || '',
    size_label: att.size_label || '',
    preview_url: att.preview_url || null,
    preview_text: '',
  }));
  const displayContent = content
    || (pendingAttachmentsForDisplay.length > 0 ? '(files attached)' : '');
  const pendingCopyTextParts = [];
  if (displayContent) pendingCopyTextParts.push(displayContent);
  pendingAttachmentsForDisplay.forEach((att) => {
    const suffix = [att.mime_type, att.size_label].filter(Boolean).join(' • ');
    pendingCopyTextParts.push(
      suffix
        ? `[Attachment] ${att.filename || 'attachment'} (${suffix})`
        : `[Attachment] ${att.filename || 'attachment'}`
    );
  });
  const pendingCopyText = pendingCopyTextParts.join('\n');
  const userMsg = addMessage('user', displayContent, {
    attachments: pendingAttachmentsForDisplay,
    copyText: pendingCopyText,
  });
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
    if (!_pendingUserMessages.has(currentThreadId)) {
      _pendingUserMessages.set(currentThreadId, []);
    }
    pendingId = _nextPendingId++;
    _pendingUserMessages.get(currentThreadId).push({
      id: pendingId,
      content: displayContent,
      copyText: pendingCopyText,
      attachments: pendingAttachmentsForDisplay.map((att) => ({ ...att })),
      timestamp: Date.now(),
    });
  }

  const body = { content, thread_id: currentThreadId || undefined, timezone: Intl.DateTimeFormat().resolvedOptions().timeZone };
  // Clone attachments so the retry handler can restore them if send fails
  // without getting mutated by subsequent stagedAttachments clears.
  const pendingAttachments = stagedAttachments.map(att => ({ ...att }));
  if (stagedAttachments.length > 0) {
    body.attachments = stagedAttachments.map(att => ({
      mime_type: att.mime_type,
      filename: att.filename,
      data_base64: att.data_base64,
    }));
    stagedAttachments = [];
    if (typeof renderAttachmentPreviews === 'function') {
      renderAttachmentPreviews();
    }
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
        // Restore the attachments we just cleared so the retry carries the
        // same payload the failed send attempted.
        if (pendingAttachments.length > 0) {
          stagedAttachments = pendingAttachments.map(att => ({ ...att }));
          if (typeof renderAttachmentPreviews === 'function') {
            renderAttachmentPreviews();
          }
        }
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

// The click/change/paste wiring for #attach-btn + #image-file-input lives in
// the `wireAttachmentUI` IIFE below (next to the unified handleAttachmentFiles
// flow). A duplicate set of listeners used to live here and fire first,
// clearing `e.target.value` before the unified listener ran — which emptied
// the FileList and silently dropped every uploaded attachment.

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

let _slashSkillEntries = [];

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

function setSlashSkillEntries(skills) {
  if (!Array.isArray(skills)) {
    _slashSkillEntries = [];
    const input = document.getElementById('chat-input');
    if (input && input.value.startsWith('/')) filterSlashCommands(input.value);
    return;
  }
  _slashSkillEntries = skills
    .filter((skill) => skill && typeof skill.name === 'string' && skill.name.trim() !== '')
    .map((skill) => ({
      cmd: '/' + skill.name.trim(),
      desc: (skill.description || '').trim() || 'Skill',
      kind: 'skill',
    }))
    .sort((a, b) => a.cmd.localeCompare(b.cmd));
  const input = document.getElementById('chat-input');
  if (input && input.value.startsWith('/')) filterSlashCommands(input.value);
}

function getSlashAutocompleteItems() {
  const items = SLASH_COMMANDS.map((cmd) => ({
    cmd: cmd.cmd,
    desc: cmd.desc,
    kind: 'command',
  }));
  const seen = new Set(items.map((item) => item.cmd.toLowerCase()));
  _slashSkillEntries.forEach((item) => {
    const key = item.cmd.toLowerCase();
    if (seen.has(key)) return;
    seen.add(key);
    items.push(item);
  });
  return items;
}

function refreshSlashSkillEntries() {
  return apiFetch('/api/skills')
    .then(function(data) {
      setSlashSkillEntries((data && data.skills) || []);
    })
    .catch(function() {
      // Preserve the last known skill list on transient fetch failures.
    });
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
  const exactLower = lower.trimEnd();
  const matches = getSlashAutocompleteItems().filter((c) => c.cmd.toLowerCase().startsWith(lower));
  if (matches.length === 0 || (matches.length === 1 && matches[0].cmd.toLowerCase() === exactLower)) {
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
    label.className = 'approval-resolved';
    const labelText = action === 'approve' ? I18n.t('approval.approved') : action === 'always' ? I18n.t('approval.alwaysApproved') : I18n.t('approval.denied');
    label.textContent = labelText;
    actions.appendChild(label);
    // Remove the card after showing the confirmation briefly
    setTimeout(() => { card.remove(); }, 1500);
  }
}


// --- Attachment Upload ---

function inferAttachmentMimeType(file) {
  if (file.type) return file.type;
  const name = (file.name || '').toLowerCase();
  if (name.endsWith('.pdf')) return 'application/pdf';
  if (name.endsWith('.pptx')) return 'application/vnd.openxmlformats-officedocument.presentationml.presentation';
  if (name.endsWith('.ppt')) return 'application/vnd.ms-powerpoint';
  if (name.endsWith('.docx')) return 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';
  if (name.endsWith('.doc')) return 'application/msword';
  if (name.endsWith('.xlsx')) return 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';
  if (name.endsWith('.xls')) return 'application/vnd.ms-excel';
  if (name.endsWith('.md')) return 'text/markdown';
  if (name.endsWith('.csv')) return 'text/csv';
  if (name.endsWith('.json')) return 'application/json';
  if (name.endsWith('.xml')) return 'application/xml';
  if (name.endsWith('.rtf')) return 'application/rtf';
  if (name.endsWith('.txt')) return 'text/plain';
  if (name.endsWith('.mp3')) return 'audio/mpeg';
  if (name.endsWith('.ogg')) return 'audio/ogg';
  if (name.endsWith('.wav')) return 'audio/wav';
  if (name.endsWith('.m4a')) return 'audio/x-m4a';
  if (name.endsWith('.mp4')) return 'audio/mp4';
  if (name.endsWith('.aac')) return 'audio/aac';
  if (name.endsWith('.flac')) return 'audio/flac';
  if (name.endsWith('.webm')) return 'audio/webm';
  return 'application/octet-stream';
}

function formatAttachmentSize(bytes) {
  if (typeof bytes !== 'number') return '';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

// Lucide-style SVG icons. Audio gets a music note; everything else falls
// back to the generic file glyph. Video isn't supported by the backend
// allowlist, so it's rejected at `handleAttachmentFiles` and never reaches
// the icon picker.
function attachmentIconSvg(mimeType) {
  const mt = (mimeType || '').toLowerCase();
  if (mt.startsWith('audio/')) {
    return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>';
  }
  return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>';
}

// Short, recognizable type label. Prefer the file extension because users
// know "DOCX"/"MP3" instinctively; fall back to a MIME-derived category
// when the filename is missing or extensionless.
function attachmentTypeLabel(filename, mimeType) {
  if (filename) {
    const dot = filename.lastIndexOf('.');
    if (dot > -1 && dot < filename.length - 1) {
      const ext = filename.slice(dot + 1).toUpperCase();
      // Sanity-cap extension length so an oddly-named file like
      // "report.final-version-with-comments" doesn't blow out the label.
      if (ext.length > 0 && ext.length <= 5) return ext;
    }
  }
  const mt = (mimeType || '').toLowerCase();
  if (mt === 'application/pdf') return 'PDF';
  if (mt.startsWith('image/')) return 'Image';
  if (mt.startsWith('audio/')) return 'Audio';
  if (mt.startsWith('text/')) return 'Text';
  return 'File';
}

// Compose `type • size` for the meta line, dropping either piece if missing.
function attachmentMetaText(filename, mimeType, sizeLabel) {
  return [attachmentTypeLabel(filename, mimeType), sizeLabel || '']
    .filter(Boolean)
    .join(' • ');
}

function appendAttachmentFileCard(container, itemClassName, nameClassName, metaClassName, filename, metaText, mimeType) {
  const item = document.createElement('div');
  item.className = itemClassName;
  // Mirror the staging preview: icon on the left, name + meta on the right.
  const iconEl = document.createElement('div');
  iconEl.className = `${nameClassName.replace('-name', '-icon')}`;
  iconEl.innerHTML = attachmentIconSvg(mimeType);
  item.appendChild(iconEl);
  const metaWrap = document.createElement('div');
  metaWrap.className = `${nameClassName.replace('-name', '-body')}`;
  const nameEl = document.createElement('div');
  nameEl.className = nameClassName;
  nameEl.textContent = filename || 'attachment';
  nameEl.title = filename || '';
  metaWrap.appendChild(nameEl);
  if (metaText) {
    const metaEl = document.createElement('div');
    metaEl.className = metaClassName;
    metaEl.textContent = metaText;
    metaWrap.appendChild(metaEl);
  }
  item.appendChild(metaWrap);
  container.appendChild(item);
}

function renderAttachmentPreviews() {
  const strip = document.getElementById('image-preview-strip');
  if (!strip) return;
  strip.innerHTML = '';
  stagedAttachments.forEach((att, idx) => {
    const container = document.createElement('div');
    container.className = 'attachment-preview-container';

    if (att.kind === 'image' && att.preview_url) {
      const preview = document.createElement('img');
      preview.className = 'image-preview';
      preview.src = att.preview_url;
      preview.alt = att.filename || 'Attached image';
      container.appendChild(preview);
    } else {
      container.classList.add('attachment-preview-file');
      const icon = document.createElement('div');
      icon.className = 'attachment-preview-file-icon';
      // Audio/video get type-specific glyphs so the user can read the card
      // at a glance; everything else falls back to the generic file icon.
      icon.innerHTML = attachmentIconSvg(att.mime_type);
      container.appendChild(icon);
      const meta = document.createElement('div');
      meta.className = 'attachment-preview-file-meta';
      const nameEl = document.createElement('div');
      nameEl.className = 'attachment-preview-file-name';
      nameEl.textContent = att.filename || 'Attached file';
      nameEl.title = att.filename || '';
      meta.appendChild(nameEl);
      const metaText = attachmentMetaText(att.filename, att.mime_type, att.size_label);
      if (metaText) {
        const sizeEl = document.createElement('div');
        sizeEl.className = 'attachment-preview-file-size';
        sizeEl.textContent = metaText;
        meta.appendChild(sizeEl);
      }
      container.appendChild(meta);
    }

    const removeBtn = document.createElement('button');
    removeBtn.className = 'image-preview-remove';
    removeBtn.textContent = '\u00d7';
    removeBtn.addEventListener('click', () => {
      stagedAttachments.splice(idx, 1);
      renderAttachmentPreviews();
    });

    container.appendChild(removeBtn);
    strip.appendChild(container);
  });
}

const MAX_ATTACHMENT_SIZE_BYTES = 7 * 1024 * 1024; // 7 MB per attachment (matches MAX_INLINE_ATTACHMENT_BYTES)
const MAX_TOTAL_ATTACHMENT_BYTES = 10 * 1024 * 1024; // 10 MB decoded per message
const MAX_STAGED_ATTACHMENTS = 5;

function handleAttachmentFiles(files) {
  let projectedCount = stagedAttachments.length + pendingAttachmentCount;
  let projectedTotalBytes = stagedAttachments.reduce((sum, att) => sum + (att.size_bytes || 0), 0) + pendingAttachmentBytes;
  Array.from(files).forEach(file => {
    const mimeType = inferAttachmentMimeType(file);
    // Backend allowlist (src/channels/web/util.rs `is_allowed_attachment_mime`)
    // doesn't include video/*. Reject at the boundary so the user gets a clear
    // alert instead of a server-side rejection after upload.
    if ((file.type || '').toLowerCase().startsWith('video/')
        || mimeType.toLowerCase().startsWith('video/')) {
      alert(I18n.t('chat.videoNotSupported', { name: file.name }));
      return;
    }
    if (file.size > MAX_ATTACHMENT_SIZE_BYTES) {
      alert(I18n.t('chat.fileTooBig', { name: file.name, size: (file.size / 1024 / 1024).toFixed(1) }));
      return;
    }
    if (projectedCount >= MAX_STAGED_ATTACHMENTS) {
      alert(I18n.t('chat.maxAttachments', { n: MAX_STAGED_ATTACHMENTS }));
      return;
    }
    if (projectedTotalBytes + file.size > MAX_TOTAL_ATTACHMENT_BYTES) {
      alert(I18n.t('chat.totalAttachmentsTooBig', { size: (MAX_TOTAL_ATTACHMENT_BYTES / 1024 / 1024).toFixed(0) }));
      return;
    }
    projectedCount += 1;
    projectedTotalBytes += file.size;
    pendingAttachmentCount += 1;
    pendingAttachmentBytes += file.size;

    const reader = new FileReader();
    let resolveRead;
    const readPromise = new Promise((resolve) => { resolveRead = resolve; });
    pendingAttachmentReads.push(readPromise);
    const finalizeRead = () => {
      pendingAttachmentCount = Math.max(0, pendingAttachmentCount - 1);
      pendingAttachmentBytes = Math.max(0, pendingAttachmentBytes - file.size);
      const idx = pendingAttachmentReads.indexOf(readPromise);
      if (idx !== -1) pendingAttachmentReads.splice(idx, 1);
      resolveRead();
    };
    reader.onload = function(e) {
      const dataUrl = e.target.result;
      const commaIdx = dataUrl.indexOf(',');
      const meta = dataUrl.substring(0, commaIdx);
      const base64 = dataUrl.substring(commaIdx + 1);
      const parsedType = meta.replace('data:', '').replace(';base64', '');
      const mediaType = (!parsedType || parsedType === 'application/octet-stream') ? mimeType : parsedType;
      stagedAttachments.push({
        kind: mediaType.startsWith('image/') ? 'image' : 'document',
        mime_type: mediaType,
        filename: file.name || null,
        data_base64: base64,
        preview_url: mediaType.startsWith('image/') ? dataUrl : null,
        size_bytes: file.size,
        size_label: formatAttachmentSize(file.size),
      });
      renderAttachmentPreviews();
      finalizeRead();
    };
    reader.onerror = function() {
      alert(I18n.t('error.unknown'));
      finalizeRead();
    };
    reader.readAsDataURL(file);
  });
}

(function wireAttachmentUI() {
  const attachBtn = document.getElementById('attach-btn');
  if (attachBtn) {
    attachBtn.addEventListener('click', () => {
      const input = document.getElementById('image-file-input');
      if (input) input.click();
    });
  }
  const fileInput = document.getElementById('image-file-input');
  if (fileInput) {
    fileInput.addEventListener('change', (e) => {
      // Snapshot the FileList into an array *before* clearing the input.
      // Some drivers (e.g. Playwright's set_input_files) expose a live
      // FileList that turns empty mid-listener-chain; reading it later
      // silently loses every file. Array.from fixes this by creating a
      // stable copy while the FileList is still populated.
      const files = Array.from(e.target.files || []);
      handleAttachmentFiles(files);
      e.target.value = '';
    });
  }
  const chatInputEl = document.getElementById('chat-input');
  if (chatInputEl) {
    chatInputEl.addEventListener('paste', (e) => {
      const items = (e.clipboardData || e.originalEvent.clipboardData).items;
      for (let i = 0; i < items.length; i++) {
        if (items[i].kind === 'file' && items[i].type.startsWith('image/')) {
          const file = items[i].getAsFile();
          if (file) handleAttachmentFiles([file]);
        }
      }
    });
  }
})();

// --- User message attachment parsing/rendering ---

function decodeXmlText(text) {
  return text
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&');
}

function parseAttachmentAttributes(rawAttrs) {
  const attrs = {};
  const attrRegex = /(\w+)="([^"]*)"/g;
  let match;
  while ((match = attrRegex.exec(rawAttrs)) !== null) {
    attrs[match[1]] = decodeXmlText(match[2]);
  }
  return attrs;
}

// Extract the plain text body and any `<attachments>…</attachments>` payload
// from a user turn's `user_input`. Messages carry their persisted attachment
// index inline so chat history can re-render file cards without a DB roundtrip.
// Only strip the trailing block when at least one `<attachment …>` element is
// parsed out of it — otherwise the user's raw text happens to end in
// `<attachments>…</attachments>` and we must leave it intact.
function parseUserMessageContent(content) {
  const match = content.match(/^([\s\S]*?)(?:\n\n)?<attachments>([\s\S]*?)<\/attachments>\s*$/);
  if (!match) {
    return { text: content, attachments: [], copyText: content };
  }

  const block = match[2];
  const attachments = [];
  const attachmentRegex = /<attachment\b([^>]*)>([\s\S]*?)<\/attachment>/g;
  let attachmentMatch;
  while ((attachmentMatch = attachmentRegex.exec(block)) !== null) {
    const attrs = parseAttachmentAttributes(attachmentMatch[1]);
    attachments.push({
      kind: attrs.type === 'image' ? 'image' : 'document',
      filename: attrs.filename || 'attachment',
      mime_type: attrs.mime || '',
      size_label: attrs.size || '',
      preview_text: decodeXmlText(attachmentMatch[2].trim()),
      preview_url: null,
    });
  }

  if (attachments.length === 0) {
    return { text: content, attachments: [], copyText: content };
  }

  const text = match[1].replace(/\s+$/, '');
  const copyParts = [];
  if (text) copyParts.push(text);
  attachments.forEach((att) => {
    const suffix = [att.mime_type, att.size_label].filter(Boolean).join(' • ');
    copyParts.push(suffix ? `[Attachment] ${att.filename} (${suffix})` : `[Attachment] ${att.filename}`);
  });

  return { text, attachments, copyText: copyParts.join('\n') };
}

function renderMessageAttachments(container, attachments) {
  if (!attachments || attachments.length === 0) return;
  const strip = document.createElement('div');
  strip.className = 'message-attachments';
  attachments.forEach((att) => {
    if (att.kind === 'image' && att.preview_url) {
      const image = document.createElement('img');
      image.className = 'message-attachment-image';
      image.src = att.preview_url;
      image.alt = att.filename || 'Attached image';
      strip.appendChild(image);
      return;
    }
    // Match the staging preview: type-aware icon + filename + (type • size).
    appendAttachmentFileCard(
      strip,
      'message-attachment-file',
      'message-attachment-file-name',
      'message-attachment-file-meta',
      att.filename || 'attachment',
      attachmentMetaText(att.filename, att.mime_type, att.size_label),
      att.mime_type
    );
  });
  container.appendChild(strip);
}

// --- Image lightbox ---

// Lazy-create a fullscreen overlay showing the clicked image at its natural
// size (capped to viewport). Click the backdrop or press ESC to close.
function showImageLightbox(src, alt) {
  // Remove any existing lightbox so rapid clicks don't stack overlays.
  const existing = document.getElementById('image-lightbox');
  if (existing) existing.remove();

  const overlay = document.createElement('div');
  overlay.id = 'image-lightbox';
  overlay.className = 'image-lightbox-overlay';
  overlay.setAttribute('role', 'dialog');
  overlay.setAttribute('aria-modal', 'true');
  overlay.setAttribute('aria-label', 'Image preview');
  overlay.tabIndex = -1;

  const img = document.createElement('img');
  img.className = 'image-lightbox-image';
  img.src = src;
  if (alt) img.alt = alt;
  overlay.appendChild(img);

  const close = () => {
    overlay.remove();
    document.removeEventListener('keydown', onKey);
  };
  const onKey = (e) => { if (e.key === 'Escape') close(); };

  // Close only when the backdrop itself is clicked, never the image.
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) close();
  });
  document.addEventListener('keydown', onKey);

  document.body.appendChild(overlay);
  overlay.focus();
}

// Delegate click handling to document.body so dynamically-added images
// (history rerenders, generated images, staging strip) are picked up
// without rebinding listeners.
(function wireImageLightbox() {
  document.addEventListener('click', (e) => {
    const target = e.target;
    if (!(target instanceof HTMLImageElement)) return;
    if (target.classList.contains('image-preview')
        || target.classList.contains('message-attachment-image')
        || target.classList.contains('generated-image')) {
      if (e.defaultPrevented) return;
      e.preventDefault();
      showImageLightbox(target.src, target.alt || '');
    }
  });
})();
