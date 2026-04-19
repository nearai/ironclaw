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
    var isApproved = action === 'approve' || action === 'always';
    label.className = 'approval-resolved ' + (isApproved ? 'is-approved' : 'is-denied');
    var icon = document.createElement('span');
    icon.className = 'approval-resolved-icon';
    icon.textContent = isApproved ? '\u2713 ' : '\u2717 ';
    label.appendChild(icon);
    const labelText = action === 'approve' ? I18n.t('approval.approved') : action === 'always' ? I18n.t('approval.alwaysApproved') : I18n.t('approval.denied');
    label.appendChild(document.createTextNode(labelText));
    actions.appendChild(label);
    // Remove the card after showing the confirmation briefly
    setTimeout(() => { card.remove(); }, 1500);
  }
}
function showApproval(data) {
  // Avoid duplicate cards on reconnect/history refresh.
  const existing = document.querySelector('.approval-card[data-request-id="' + CSS.escape(data.request_id) + '"]');
  if (existing) return;

  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'approval-card';
  card.setAttribute('data-request-id', data.request_id);
  const cardThreadId = data.thread_id || currentThreadId;
  if (cardThreadId) {
    card.setAttribute('data-thread-id', cardThreadId);
  }

  var tag = document.createElement('span');
  tag.className = 'approval-tag';
  tag.textContent = I18n.t('approval.tag') || 'approval required';
  card.appendChild(tag);

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
  approveBtn.textContent = I18n.t('approval.approveOnce') || 'Approve once';
  approveBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'approve', cardThreadId));

  const denyBtn = document.createElement('button');
  denyBtn.className = 'deny';
  denyBtn.textContent = I18n.t('approval.deny');
  denyBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'deny', cardThreadId));

  actions.appendChild(approveBtn);
  if (data.allow_always !== false) {
    const alwaysBtn = document.createElement('button');
    alwaysBtn.className = 'always';
    alwaysBtn.textContent = I18n.t('approval.alwaysAllow') || 'Always allow';
    alwaysBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'always', cardThreadId));
    actions.appendChild(alwaysBtn);
  }
  actions.appendChild(denyBtn);
  card.appendChild(actions);

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
}
