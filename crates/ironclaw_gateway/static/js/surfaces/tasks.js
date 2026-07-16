// --- Tasks surface extras: card icons, detail sheet, floating chat heads ---
//
// The kanban board (renderMissionsList in projects.js) opens the task detail
// sheet from here. The sheet embeds a mini agent-chat preview scoped to the
// task (MOCK: thread content is simulated — no engine thread is attached),
// and can "pop out" into a floating chat head pinned bottom-right that
// persists across surface navigation (session-scoped, in-memory).

// Keyword → icon mapping for task cards. Inline SVGs match the sidebar set
// (24px viewBox, stroke-based, currentColor).
const TASK_ICON_PATHS = {
  wrench: '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>',
  chart: '<line x1="12" y1="20" x2="12" y2="10"/><line x1="18" y1="20" x2="18" y2="4"/><line x1="6" y1="20" x2="6" y2="16"/>',
  inbox: '<polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>',
  radar: '<path d="M19.07 4.93A10 10 0 0 0 6.99 3.34"/><path d="M4 6h.01"/><path d="M2.29 9.62a10 10 0 1 0 19.02-1.27"/><path d="M16.24 7.76a6 6 0 1 0-8.01 8.91"/><path d="M12 18h.01"/><path d="M17.99 11.66a6 6 0 0 1-2.22 4.75"/><circle cx="12" cy="12" r="2"/><path d="m13.41 10.59 5.66-5.66"/>',
  calendar: '<rect x="3" y="4" width="18" height="18" rx="2" ry="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/>',
  brain: '<path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"/><path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"/><path d="M12 5v13"/>',
  message: '<path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/>',
  sparkles: '<path d="m12 3-1.9 5.8a2 2 0 0 1-1.3 1.3L3 12l5.8 1.9a2 2 0 0 1 1.3 1.3L12 21l1.9-5.8a2 2 0 0 1 1.3-1.3L21 12l-5.8-1.9a2 2 0 0 1-1.3-1.3Z"/>',
};

const TASK_ICON_KEYWORDS = [
  { icon: 'wrench', re: /repair|fix|debug|maintain|patch/i },
  { icon: 'chart', re: /insight|analytic|report|kpi|metric|dashboard|trend/i },
  { icon: 'inbox', re: /email|inbox|mail|triage|message/i },
  { icon: 'radar', re: /monitor|watch|alert|track|ping|health|observ/i },
  { icon: 'calendar', re: /calendar|meeting|schedul|brief|daily|remind/i },
  { icon: 'brain', re: /learn|improve|self|memory|knowledge|skill/i },
  { icon: 'message', re: /conversation|chat|slack|telegram|discord/i },
];

function taskIconName(text) {
  const hay = String(text || '');
  for (const entry of TASK_ICON_KEYWORDS) {
    if (entry.re.test(hay)) return entry.icon;
  }
  return 'sparkles';
}

function taskIconSvg(text, size) {
  const name = taskIconName(text);
  return '<svg width="' + (size || 16) + '" height="' + (size || 16) + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'
    + TASK_ICON_PATHS[name] + '</svg>';
}

// --- Mini chat (shared by the sheet preview and the chat head panels) ---
//
// MOCK: per-task message log lives in memory for the session. The "agent"
// replies are canned — this is a UI preview of task-scoped chat, not a real
// engine thread.

const _taskChatLogs = new Map();

function getTaskChatLog(taskId, taskName) {
  if (!_taskChatLogs.has(taskId)) {
    _taskChatLogs.set(taskId, [{
      role: 'assistant',
      text: I18n.t('tasks.chatIntro', { name: taskName || 'this task' }),
    }]);
  }
  return _taskChatLogs.get(taskId);
}

function renderTaskChatMessages(container, taskId) {
  const log = _taskChatLogs.get(taskId) || [];
  container.innerHTML = log.map((m) =>
    '<div class="task-chat-msg ' + (m.role === 'user' ? 'user' : 'assistant') + '">'
    + escapeHtml(m.text) + '</div>'
  ).join('');
  container.scrollTop = container.scrollHeight;
}

function sendTaskChatMessage(taskId, taskName, text, refresh) {
  const trimmed = (text || '').trim();
  if (!trimmed) return;
  const log = getTaskChatLog(taskId, taskName);
  log.push({ role: 'user', text: trimmed });
  refresh();
  // MOCK: canned agent reply after a beat.
  setTimeout(() => {
    log.push({
      role: 'assistant',
      text: I18n.t('tasks.chatMockReply', { name: taskName || 'this task' }),
    });
    refresh();
  }, 700);
}

// Builds the mini chat DOM (message list + composer) for a task.
function buildTaskMiniChat(taskId, taskName) {
  const wrap = document.createElement('div');
  wrap.className = 'task-mini-chat';

  const messages = document.createElement('div');
  messages.className = 'task-mini-chat-messages';
  wrap.appendChild(messages);

  const composer = document.createElement('form');
  composer.className = 'task-mini-chat-composer';
  const input = document.createElement('input');
  input.type = 'text';
  input.className = 'task-mini-chat-input';
  input.placeholder = I18n.t('tasks.chatPlaceholder');
  composer.appendChild(input);
  const send = document.createElement('button');
  send.type = 'submit';
  send.className = 'task-mini-chat-send';
  send.setAttribute('aria-label', I18n.t('chat.send'));
  send.innerHTML = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="19" x2="12" y2="5"/><polyline points="5 12 12 5 19 12"/></svg>';
  composer.appendChild(send);
  wrap.appendChild(composer);

  const refresh = () => renderTaskChatMessages(messages, taskId);
  getTaskChatLog(taskId, taskName);
  refresh();
  composer.addEventListener('submit', (e) => {
    e.preventDefault();
    sendTaskChatMessage(taskId, taskName, input.value, refresh);
    input.value = '';
  });
  return wrap;
}

// --- Task detail sheet (right-side; full-screen modal on mobile) ---

let _taskSheetId = null;

function openTaskSheet(missionId) {
  const fromList = (typeof currentMissionList !== 'undefined' && currentMissionList || [])
    .find((m) => m.id === missionId) || null;
  _taskSheetId = missionId;
  renderTaskSheet(fromList, null);
  // Enrich with full detail (threads) in the background.
  apiFetch('/api/engine/missions/' + missionId).then((data) => {
    if (_taskSheetId !== missionId) return;
    renderTaskSheet(data.mission || fromList, data.mission && data.mission.threads || []);
  }).catch(() => {});
}

function closeTaskSheet() {
  _taskSheetId = null;
  document.getElementById('task-sheet-root')?.remove();
}

function renderTaskSheet(mission, threads) {
  if (!mission) return;
  closeTaskSheet();
  _taskSheetId = mission.id;

  const root = document.createElement('div');
  root.id = 'task-sheet-root';

  const scrim = document.createElement('div');
  scrim.className = 'task-sheet-scrim';
  scrim.addEventListener('click', () => closeTaskSheet());
  root.appendChild(scrim);

  const sheet = document.createElement('aside');
  sheet.className = 'task-sheet';
  sheet.setAttribute('role', 'dialog');
  sheet.setAttribute('aria-label', mission.name);

  const statusClass = mission.status === 'Active' ? 'in_progress'
    : mission.status === 'Completed' ? 'completed'
    : mission.status === 'Paused' ? 'pending' : 'failed';

  // Two-row header: identity row (icon + name + status + close) over an
  // actions row — keeps the task name readable at sheet width.
  const header = document.createElement('div');
  header.className = 'task-sheet-header';
  header.innerHTML =
    '<div class="task-sheet-header-row">'
    + '<span class="task-sheet-icon" aria-hidden="true">' + taskIconSvg(mission.name + ' ' + (mission.goal || ''), 18) + '</span>'
    + '<div class="task-sheet-titles">'
    + '<div class="task-sheet-name">' + escapeHtml(mission.name) + '</div>'
    + '<span class="badge ' + statusClass + '">' + escapeHtml(mission.status || '') + '</span>'
    + '</div>'
    + '<button type="button" class="task-sheet-close" data-task-action="close" aria-label="' + escapeHtml(I18n.t('btn.close')) + '">'
    + '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>'
    + '</button>'
    + '</div>'
    + '<div class="task-sheet-actions">'
    + (mission.status === 'Active'
      ? '<button type="button" class="task-sheet-action" data-task-action="pause">' + escapeHtml(I18n.t('missions.pause')) + '</button>'
      : (mission.status === 'Paused'
        ? '<button type="button" class="task-sheet-action" data-task-action="resume">' + escapeHtml(I18n.t('missions.resume')) + '</button>'
        : ''))
    + '<button type="button" class="task-sheet-action" data-task-action="full-page">' + escapeHtml(I18n.t('tasks.openFullPage')) + '</button>'
    + '<button type="button" class="task-sheet-action" data-task-action="pop-out">' + escapeHtml(I18n.t('tasks.popOutChat')) + '</button>'
    + '</div>';
  sheet.appendChild(header);

  const body = document.createElement('div');
  body.className = 'task-sheet-body';

  let detailsHtml =
    '<div class="task-sheet-section">'
    + '<div class="task-sheet-section-label">' + escapeHtml(I18n.t('tasks.description')) + '</div>'
    + '<div class="task-sheet-goal">' + escapeHtml(mission.goal || '—') + '</div>'
    + '</div>'
    + '<div class="task-sheet-section">'
    + '<div class="task-sheet-section-label">' + escapeHtml(I18n.t('tasks.trigger')) + '</div>'
    + '<div class="task-sheet-trigger">' + escapeHtml(mission.cadence_description || mission.cadence_type || 'manual') + '</div>'
    + '</div>';

  const threadList = Array.isArray(threads) ? threads.slice(0, 6) : [];
  detailsHtml += '<div class="task-sheet-section">'
    + '<div class="task-sheet-section-label">' + escapeHtml(I18n.t('tasks.recentActivity')) + '</div>';
  if (threadList.length === 0) {
    detailsHtml += '<div class="task-sheet-empty">' + escapeHtml(I18n.t('tasks.noActivity')) + '</div>';
  } else {
    detailsHtml += threadList.map((t) =>
      '<div class="task-sheet-thread">'
      + '<span class="task-sheet-thread-dot ' + (t.state === 'Running' ? 'running' : (t.state === 'Failed' ? 'failed' : 'done')) + '"></span>'
      + '<span class="task-sheet-thread-label">' + escapeHtml(t.goal || t.title || ('Thread ' + String(t.id || '').slice(0, 8))) + '</span>'
      + '</div>'
    ).join('');
  }
  detailsHtml += '</div>';

  const details = document.createElement('div');
  details.className = 'task-sheet-details';
  details.innerHTML = detailsHtml;
  body.appendChild(details);

  // Embedded agent-chat preview scoped to this task (MOCK content).
  const chatSection = document.createElement('div');
  chatSection.className = 'task-sheet-section task-sheet-chat';
  chatSection.innerHTML = '<div class="task-sheet-section-label">' + escapeHtml(I18n.t('tasks.chatPreview')) + '</div>';
  chatSection.appendChild(buildTaskMiniChat(mission.id, mission.name));
  body.appendChild(chatSection);

  sheet.appendChild(body);

  header.addEventListener('click', (e) => {
    const btn = e.target.closest('[data-task-action]');
    if (!btn) return;
    const action = btn.getAttribute('data-task-action');
    if (action === 'close') closeTaskSheet();
    if (action === 'pause' || action === 'resume') {
      closeTaskSheet();
      if (action === 'pause') pauseMission(mission.id);
      else resumeMission(mission.id);
    }
    if (action === 'full-page') {
      closeTaskSheet();
      switchTab('tasks');
      openMissionDetail(mission.id);
    }
    if (action === 'pop-out') {
      closeTaskSheet();
      addTaskChatHead(mission.id, mission.name, mission.goal || '');
    }
  });

  root.appendChild(sheet);
  document.body.appendChild(root);
}

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && _taskSheetId) closeTaskSheet();
});

// --- Floating chat heads (Messenger-style, pinned bottom-right) ---
//
// Session-scoped: heads live in memory and survive surface navigation (the
// container hangs off <body>), but not a reload.

const _taskChatHeads = [];
let _expandedChatHeadId = null;

function ensureChatHeadsRoot() {
  let rootEl = document.getElementById('chat-heads-root');
  if (!rootEl) {
    rootEl = document.createElement('div');
    rootEl.id = 'chat-heads-root';
    document.body.appendChild(rootEl);
  }
  return rootEl;
}

function addTaskChatHead(taskId, taskName, taskGoal) {
  if (!_taskChatHeads.some((h) => h.id === taskId)) {
    _taskChatHeads.push({ id: taskId, name: taskName, goal: taskGoal || '' });
  }
  _expandedChatHeadId = taskId;
  renderChatHeads();
}

function removeTaskChatHead(taskId) {
  const idx = _taskChatHeads.findIndex((h) => h.id === taskId);
  if (idx !== -1) _taskChatHeads.splice(idx, 1);
  if (_expandedChatHeadId === taskId) _expandedChatHeadId = null;
  renderChatHeads();
}

function renderChatHeads() {
  const rootEl = ensureChatHeadsRoot();
  rootEl.innerHTML = '';
  if (_taskChatHeads.length === 0) return;

  // Expanded mini-chat panel (one at a time).
  const expanded = _taskChatHeads.find((h) => h.id === _expandedChatHeadId);
  if (expanded) {
    const panel = document.createElement('div');
    panel.className = 'chat-head-panel';
    const header = document.createElement('div');
    header.className = 'chat-head-panel-header';
    header.innerHTML =
      '<span class="chat-head-panel-icon" aria-hidden="true">' + taskIconSvg(expanded.name + ' ' + expanded.goal, 14) + '</span>'
      + '<span class="chat-head-panel-name">' + escapeHtml(expanded.name) + '</span>'
      + '<button type="button" class="chat-head-panel-btn" data-head-action="minimize" aria-label="' + escapeHtml(I18n.t('tasks.minimize')) + '">'
      + '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="5" y1="12" x2="19" y2="12"/></svg></button>'
      + '<button type="button" class="chat-head-panel-btn" data-head-action="close" aria-label="' + escapeHtml(I18n.t('btn.close')) + '">'
      + '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg></button>';
    header.addEventListener('click', (e) => {
      const btn = e.target.closest('[data-head-action]');
      if (!btn) return;
      if (btn.getAttribute('data-head-action') === 'minimize') {
        _expandedChatHeadId = null;
        renderChatHeads();
      } else {
        removeTaskChatHead(expanded.id);
      }
    });
    panel.appendChild(header);
    panel.appendChild(buildTaskMiniChat(expanded.id, expanded.name));
    rootEl.appendChild(panel);
  }

  // The bubble stack.
  const stack = document.createElement('div');
  stack.className = 'chat-heads-stack';
  _taskChatHeads.forEach((head) => {
    const bubble = document.createElement('button');
    bubble.type = 'button';
    bubble.className = 'chat-head-bubble' + (head.id === _expandedChatHeadId ? ' expanded' : '');
    bubble.title = head.name;
    bubble.innerHTML = taskIconSvg(head.name + ' ' + head.goal, 20)
      + '<span class="chat-head-dismiss" data-dismiss aria-hidden="true">'
      + '<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg></span>';
    bubble.addEventListener('click', (e) => {
      if (e.target.closest('[data-dismiss]')) {
        removeTaskChatHead(head.id);
        return;
      }
      _expandedChatHeadId = _expandedChatHeadId === head.id ? null : head.id;
      renderChatHeads();
    });
    stack.appendChild(bubble);
  });
  rootEl.appendChild(stack);
}

// Delegated open action for kanban cards (kept separate from the big
// data-action switch in ui-helpers.js).
document.addEventListener('click', (e) => {
  const card = e.target.closest('[data-action="open-task-sheet"]');
  if (card) openTaskSheet(card.dataset.id);
});
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Enter' && e.key !== ' ') return;
  const card = e.target.closest && e.target.closest('[data-action="open-task-sheet"]');
  if (card) {
    e.preventDefault();
    openTaskSheet(card.dataset.id);
  }
});
