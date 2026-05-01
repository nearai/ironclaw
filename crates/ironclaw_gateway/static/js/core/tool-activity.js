const MAX_TOOL_ACTIVITY_RESULT_CHARS = 1000;

function formatToolActivityDurationMs(durationMs) {
  if (typeof durationMs !== 'number' || !isFinite(durationMs) || durationMs < 0) return '';
  if (durationMs === 0) return '<1ms';
  if (durationMs < 1000) return Math.round(durationMs) + 'ms';
  const elapsedSecs = durationMs / 1000;
  return elapsedSecs < 10 ? elapsedSecs.toFixed(1) + 's' : Math.floor(elapsedSecs) + 's';
}

function truncateToolActivityResult(text) {
  if (!text) return '';
  if (text.length <= MAX_TOOL_ACTIVITY_RESULT_CHARS) return text;
  return text.slice(0, MAX_TOOL_ACTIVITY_RESULT_CHARS) + '...';
}

function buildToolFailureText(parameters, error) {
  let detail = '';
  if (parameters) {
    detail += 'Input:\n' + parameters + '\n\n';
  }
  if (error) {
    detail += 'Error:\n' + error;
  }
  return detail;
}

function getToolActivityBodyText(entry) {
  if (!entry) return '';
  return entry.error || entry.result || entry.result_preview || '';
}

function normalizeHistoryToolCall(toolCall) {
  return {
    call_id: toolCall.call_id || null,
    name: toolCall.name || 'tool',
    status: toolCall.has_error ? 'fail' : (toolCall.has_result ? 'success' : 'running'),
    result_preview: toolCall.result_preview || '',
    result: toolCall.result || '',
    error: toolCall.error || '',
    duration_ms: null,
  };
}

function createToolActivitySummary(toolCount, totalDurationMs, includeDuration) {
  const toolWord = toolCount === 1 ? 'tool' : 'tools';
  const summary = document.createElement('button');
  summary.type = 'button';
  summary.className = 'activity-summary';
  summary.setAttribute('aria-expanded', 'false');
  summary.innerHTML = '<span class="activity-summary-chevron"><svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="6 4 10 8 6 12"/></svg></span>'
    + '<span class="activity-summary-text">Used ' + toolCount + ' ' + toolWord + '</span>';
  if (includeDuration) {
    const durationStr = formatToolActivityDurationMs(totalDurationMs);
    if (durationStr) {
      const duration = document.createElement('span');
      duration.className = 'activity-summary-duration';
      duration.textContent = '(' + durationStr + ')';
      summary.appendChild(duration);
    }
  }
  return summary;
}

function setToolActivityCardExpanded(rendered, expanded) {
  rendered.body.classList.toggle('expanded', !!expanded);
  rendered.chevron.classList.toggle('expanded', !!expanded);
  rendered.header.setAttribute('aria-expanded', expanded ? 'true' : 'false');
}

function applyToolActivityCardState(rendered, options) {
  const entry = rendered.entry;
  const status = entry.status || 'running';
  rendered.card.setAttribute('data-tool-name', entry.name);
  if (entry.call_id) {
    rendered.card.setAttribute('data-call-id', entry.call_id);
  } else {
    rendered.card.removeAttribute('data-call-id');
  }
  rendered.card.setAttribute('data-status', status);
  rendered.toolName.textContent = entry.name;

  if (options.showDuration && entry.duration_ms !== null) {
    rendered.duration.textContent = formatToolActivityDurationMs(entry.duration_ms);
  } else {
    rendered.duration.textContent = '';
  }

  if (status === 'fail') {
    rendered.icon.innerHTML = '<span class="activity-icon-fail">&#10007;</span>';
  } else if (status === 'success') {
    rendered.icon.innerHTML = '<span class="activity-icon-success">&#10003;</span>';
  } else {
    rendered.icon.innerHTML = '<div class="spinner"></div>';
  }

  rendered.output.textContent = getToolActivityBodyText(entry);
  const shouldAutoExpand = !!(options.expandErrors && status === 'fail' && rendered.output.textContent);
  setToolActivityCardExpanded(rendered, shouldAutoExpand);
}

function addSkillActivationCard(names, feedback) {
  // The activation card is the first concrete signal that a turn is
  // underway, so the "thinking" dots have done their job — drop them.
  // `startTool` / `completeTool` will re-show them later if nothing
  // else has reported progress by then.
  removeActivityThinking();
  const group = getOrCreateActivityGroup();
  if (!group) return;

  const card = document.createElement('div');
  card.className = 'activity-skill-card';

  const header = document.createElement('div');
  header.className = 'activity-skill-header';
  const icon = document.createElement('span');
  icon.className = 'activity-skill-icon';
  icon.textContent = '\u25C6'; // ◆
  header.appendChild(icon);
  const label = document.createElement('span');
  label.className = 'activity-skill-label';
  if (names.length > 0) {
    label.textContent = 'Skills: ' + names.join(', ');
  } else {
    label.textContent = 'Skill activation';
  }
  header.appendChild(label);
  card.appendChild(header);

  for (const note of feedback) {
    const row = document.createElement('div');
    row.className = 'activity-skill-note';
    row.textContent = note;
    card.appendChild(row);
  }

  group.appendChild(card);
  const container = document.getElementById('chat-messages');
  container.scrollTop = container.scrollHeight;
}

function createToolActivityCard(entry, options) {
  const card = document.createElement('div');
  card.className = 'activity-tool-card';

  const header = document.createElement('button');
  header.type = 'button';
  header.className = 'activity-tool-header';
  header.setAttribute('aria-expanded', 'false');

  const icon = document.createElement('span');
  icon.className = 'activity-tool-icon';

  const toolName = document.createElement('span');
  toolName.className = 'activity-tool-name';

  const duration = document.createElement('span');
  duration.className = 'activity-tool-duration';

  const chevron = document.createElement('span');
  chevron.className = 'activity-tool-chevron';
  chevron.innerHTML = '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="6 4 10 8 6 12"/></svg>';

  header.appendChild(icon);
  header.appendChild(toolName);
  header.appendChild(duration);
  header.appendChild(chevron);

  const body = document.createElement('div');
  body.className = 'activity-tool-body';

  const output = document.createElement('pre');
  output.className = 'activity-tool-output';
  body.appendChild(output);

  const rendered = { entry, card, header, icon, toolName, duration, chevron, body, output, timer: null };
  header.addEventListener('click', () => {
    const willExpand = !body.classList.contains('expanded');
    setToolActivityCardExpanded(rendered, willExpand);
  });

  card.appendChild(header);
  card.appendChild(body);
  applyToolActivityCardState(rendered, options);
  return rendered;
}

function createActivityGroupFromEntries(entries, options) {
  const hasError = entries.some(entry => entry.status === 'fail');
  const group = document.createElement('div');
  group.className = 'activity-group' + (hasError ? '' : ' collapsed');

  let totalDurationMs = 0;
  let hasDuration = false;
  for (const entry of entries) {
    if (typeof entry.duration_ms === 'number' && isFinite(entry.duration_ms)) {
      totalDurationMs += entry.duration_ms;
      hasDuration = true;
    }
  }

  const summary = createToolActivitySummary(entries.length, totalDurationMs, options.includeSummaryDuration && hasDuration);
  if (hasError) {
    summary.querySelector('.activity-summary-chevron').classList.add('expanded');
    summary.setAttribute('aria-expanded', 'true');
  }

  const cardsContainer = document.createElement('div');
  cardsContainer.className = 'activity-cards-container';
  cardsContainer.style.display = hasError ? 'flex' : 'none';

  for (const entry of entries) {
    const rendered = createToolActivityCard(entry, {
      showDuration: !!options.showCardDurations,
      expandErrors: options.expandErrors !== false,
    });
    cardsContainer.appendChild(rendered.card);
  }

  summary.addEventListener('click', () => {
    const isOpen = cardsContainer.style.display !== 'none';
    cardsContainer.style.display = isOpen ? 'none' : 'flex';
    summary.setAttribute('aria-expanded', isOpen ? 'false' : 'true');
    summary.querySelector('.activity-summary-chevron').classList.toggle('expanded', !isOpen);
  });

  group.appendChild(summary);
  group.appendChild(cardsContainer);
  return group;
}

function createToolActivityController(options) {
  let activeGroup = null;
  let activeEntries = [];
  let entriesByCallId = new Map();
  let entriesByName = new Map();
  let thinkingEl = null;

  function getContainer() {
    return document.getElementById(options.containerId);
  }

  function scrollToBottom() {
    const container = getContainer();
    if (container) container.scrollTop = container.scrollHeight;
  }

  function clearTimer(rendered) {
    if (rendered && rendered.timer) {
      clearInterval(rendered.timer);
      rendered.timer = null;
    }
  }

  function rememberRendered(rendered) {
    activeEntries.push(rendered);
    if (rendered.entry.call_id) {
      entriesByCallId.set(rendered.entry.call_id, rendered);
    }
    const byName = entriesByName.get(rendered.entry.name) || [];
    byName.push(rendered);
    entriesByName.set(rendered.entry.name, byName);
  }

  function findRendered(callId, name, predicate) {
    if (callId) {
      const rendered = entriesByCallId.get(callId);
      if (rendered && (!predicate || predicate(rendered))) return rendered;
      return null;
    }

    const byName = entriesByName.get(name) || [];
    for (const rendered of byName) {
      if (!predicate || predicate(rendered)) return rendered;
    }
    return null;
  }

  function reset(removeDom) {
    if (removeDom && activeGroup) {
      activeGroup.remove();
    }
    if (thinkingEl) {
      thinkingEl.remove();
      thinkingEl = null;
    }
    for (const rendered of activeEntries) {
      clearTimer(rendered);
    }
    activeGroup = null;
    activeEntries = [];
    entriesByCallId = new Map();
    entriesByName = new Map();
  }

  function getOrCreateGroup() {
    if (activeGroup) return activeGroup;
    const container = getContainer();
    if (!container) return null;
    activeGroup = document.createElement('div');
    activeGroup.className = 'activity-group';
    container.appendChild(activeGroup);
    scrollToBottom();
    return activeGroup;
  }

  function showThinking(message) {
    const group = getOrCreateGroup();
    if (!group) return;
    if (thinkingEl) {
      thinkingEl.style.display = '';
      thinkingEl.querySelector('.activity-thinking-text').textContent = message;
    } else {
      thinkingEl = document.createElement('div');
      thinkingEl.className = 'activity-thinking';
      thinkingEl.innerHTML =
        '<span class="activity-thinking-dots">'
        + '<span class="activity-thinking-dot"></span>'
        + '<span class="activity-thinking-dot"></span>'
        + '<span class="activity-thinking-dot"></span>'
        + '</span>'
        + '<span class="activity-thinking-text"></span>';
      group.appendChild(thinkingEl);
      thinkingEl.querySelector('.activity-thinking-text').textContent = message;
    }
    scrollToBottom();
  }

  function removeThinking() {
    if (thinkingEl) {
      thinkingEl.remove();
      thinkingEl = null;
    }
  }

  function startTool(event) {
    if (thinkingEl) thinkingEl.style.display = 'none';
    const group = getOrCreateGroup();
    if (!group) return;

    const entry = {
      call_id: event.call_id || null,
      name: event.name || 'tool',
      status: 'running',
      result_preview: '',
      result: '',
      error: '',
      duration_ms: null,
      started_at_ms: Date.now(),
    };
    const rendered = createToolActivityCard(entry, {
      showDuration: true,
      expandErrors: true,
    });

    rendered.timer = setInterval(() => {
      const elapsedMs = Date.now() - rendered.entry.started_at_ms;
      if (elapsedMs > 300000) {
        clearTimer(rendered);
        return;
      }
      rendered.duration.textContent = formatToolActivityDurationMs(elapsedMs);
    }, 100);

    group.appendChild(rendered.card);
    rememberRendered(rendered);
    scrollToBottom();
  }

  function completeTool(event) {
    const rendered = findRendered(
      event.call_id || null,
      event.name || '',
      candidate => candidate.entry.status === 'running'
    ) || findRendered(event.call_id || null, event.name || '');
    if (!rendered) return;

    clearTimer(rendered);
    rendered.entry.status = event.success ? 'success' : 'fail';
    rendered.entry.duration_ms = typeof event.duration_ms === 'number'
      ? event.duration_ms
      : (Date.now() - rendered.entry.started_at_ms);

    if (!event.success && (event.error || event.parameters)) {
      rendered.entry.error = buildToolFailureText(event.parameters, event.error);
    }

    applyToolActivityCardState(rendered, {
      showDuration: true,
      expandErrors: true,
    });
  }

  function setResult(event) {
    const rendered = findRendered(
      event.call_id || null,
      event.name || '',
      candidate => !candidate.entry.result
    ) || findRendered(event.call_id || null, event.name || '');
    if (!rendered) return;

    const preview = truncateToolActivityResult(event.preview || '');
    rendered.entry.result = preview;
    if (!rendered.entry.result_preview) {
      rendered.entry.result_preview = preview;
    }
    applyToolActivityCardState(rendered, {
      showDuration: true,
      expandErrors: true,
    });
  }

  function finalizeGroup() {
    removeThinking();
    if (!activeGroup) return;

    for (const rendered of activeEntries) {
      clearTimer(rendered);
      if (rendered.entry.duration_ms === null) {
        rendered.entry.duration_ms = Date.now() - rendered.entry.started_at_ms;
      }
      applyToolActivityCardState(rendered, {
        showDuration: true,
        expandErrors: true,
      });
    }

    if (activeEntries.length === 0) {
      activeGroup.remove();
      reset(false);
      return;
    }

    const totalDurationMs = activeEntries.reduce((sum, rendered) => {
      return sum + (typeof rendered.entry.duration_ms === 'number' ? rendered.entry.duration_ms : 0);
    }, 0);

    const cardsContainer = document.createElement('div');
    cardsContainer.className = 'activity-cards-container';
    cardsContainer.style.display = 'none';
    for (const rendered of activeEntries) {
      cardsContainer.appendChild(rendered.card);
    }

    const summary = createToolActivitySummary(activeEntries.length, totalDurationMs, true);
    summary.addEventListener('click', () => {
      const isOpen = cardsContainer.style.display !== 'none';
      cardsContainer.style.display = isOpen ? 'none' : 'flex';
      summary.setAttribute('aria-expanded', isOpen ? 'false' : 'true');
      summary.querySelector('.activity-summary-chevron').classList.toggle('expanded', !isOpen);
    });

    activeGroup.innerHTML = '';
    activeGroup.classList.add('collapsed');
    activeGroup.appendChild(summary);
    activeGroup.appendChild(cardsContainer);

    activeGroup = null;
    activeEntries = [];
    entriesByCallId = new Map();
    entriesByName = new Map();
  }

  return {
    getOrCreateGroup,
    showThinking,
    removeThinking,
    startTool,
    completeTool,
    setResult,
    finalizeGroup,
    reset,
  };
}

function getOrCreateActivityGroup() {
  return _chatToolActivity.getOrCreateGroup();
}

function showActivityThinking(message) {
  _chatToolActivity.showThinking(message);
}

function removeActivityThinking() {
  _chatToolActivity.removeThinking();
}

function addToolCard(toolEvent) {
  _chatToolActivity.startTool(toolEvent);
}

function completeToolCard(toolEvent) {
  _chatToolActivity.completeTool(toolEvent);
}

function setToolCardOutput(toolEvent) {
  _chatToolActivity.setResult(toolEvent);
}

function finalizeActivityGroup() {
  _chatToolActivity.finalizeGroup();
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
  return false;
}
