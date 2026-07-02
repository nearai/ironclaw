// --- API token manager ---
//
// Self-service UI over the existing /api/tokens endpoints (create / list /
// revoke). Creation requires an authenticated session — tokens are never
// mintable from the logged-out landing — so this lives in the account menu.
// The plaintext is shown exactly once, mirroring the API contract.

function openApiTokensModal() {
  closeApiTokensModal();

  const overlay = document.createElement('div');
  overlay.className = 'api-tokens-overlay';
  overlay.id = 'api-tokens-overlay';
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) closeApiTokensModal();
  });

  const modal = document.createElement('div');
  modal.className = 'api-tokens-modal';
  modal.setAttribute('role', 'dialog');
  modal.setAttribute('aria-modal', 'true');

  const header = document.createElement('div');
  header.className = 'api-tokens-header';
  const title = document.createElement('h3');
  title.textContent = I18n.t('tokens.title');
  header.appendChild(title);
  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'api-tokens-close';
  closeBtn.setAttribute('aria-label', I18n.t('common.close'));
  closeBtn.textContent = '\u00d7';
  closeBtn.addEventListener('click', closeApiTokensModal);
  header.appendChild(closeBtn);
  modal.appendChild(header);

  const desc = document.createElement('p');
  desc.className = 'api-tokens-desc';
  desc.textContent = I18n.t('tokens.description');
  modal.appendChild(desc);

  // Create form
  const form = document.createElement('div');
  form.className = 'api-tokens-form';
  const nameInput = document.createElement('input');
  nameInput.type = 'text';
  nameInput.id = 'api-token-name';
  nameInput.placeholder = I18n.t('tokens.namePlaceholder');
  nameInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') createApiToken();
  });
  form.appendChild(nameInput);
  const createBtn = document.createElement('button');
  createBtn.type = 'button';
  createBtn.className = 'api-tokens-create';
  createBtn.textContent = I18n.t('tokens.create');
  createBtn.addEventListener('click', createApiToken);
  form.appendChild(createBtn);
  modal.appendChild(form);

  // One-time plaintext reveal slot
  const reveal = document.createElement('div');
  reveal.id = 'api-token-reveal';
  modal.appendChild(reveal);

  // Token list
  const list = document.createElement('div');
  list.className = 'api-tokens-list';
  list.id = 'api-tokens-list';
  list.innerHTML = '<div class="empty-state">' + I18n.t('common.loading') + '</div>';
  modal.appendChild(list);

  overlay.appendChild(modal);
  document.body.appendChild(overlay);
  nameInput.focus();

  refreshApiTokensList();
}

function closeApiTokensModal() {
  const overlay = document.getElementById('api-tokens-overlay');
  if (overlay) overlay.remove();
}

function refreshApiTokensList() {
  const list = document.getElementById('api-tokens-list');
  if (!list) return;
  apiFetch('/api/tokens').then((data) => {
    const target = document.getElementById('api-tokens-list');
    if (!target) return;
    const tokens = (data.tokens || []).filter((t) => !t.revoked_at);
    if (tokens.length === 0) {
      target.innerHTML = '<div class="empty-state">' + I18n.t('tokens.none') + '</div>';
      return;
    }
    target.innerHTML = '';
    tokens.forEach((token) => {
      const row = document.createElement('div');
      row.className = 'api-token-row';

      const info = document.createElement('div');
      info.className = 'api-token-info';
      const name = document.createElement('div');
      name.className = 'api-token-name';
      name.textContent = token.name;
      info.appendChild(name);
      const meta = document.createElement('div');
      meta.className = 'api-token-meta';
      const parts = [token.token_prefix + '\u2026'];
      if (token.last_used_at) {
        parts.push(I18n.t('tokens.lastUsed', { time: formatDate(token.last_used_at) }));
      }
      if (token.expires_at) {
        parts.push(I18n.t('tokens.expires', { time: formatDate(token.expires_at) }));
      }
      meta.textContent = parts.join(' \u00b7 ');
      info.appendChild(meta);
      row.appendChild(info);

      const revokeBtn = document.createElement('button');
      revokeBtn.type = 'button';
      revokeBtn.className = 'api-token-revoke';
      revokeBtn.textContent = I18n.t('tokens.revoke');
      revokeBtn.addEventListener('click', () => {
        showConfirmModal(I18n.t('tokens.confirmRevoke', { name: token.name }), '', () => {
          apiFetch('/api/tokens/' + encodeURIComponent(token.id), { method: 'DELETE' })
            .then(() => {
              showToast(I18n.t('tokens.revoked', { name: token.name }), 'success');
              refreshApiTokensList();
            })
            .catch((err) => showToast(I18n.t('tokens.revokeFailed', { message: err.message }), 'error'));
        }, I18n.t('tokens.revoke'), 'btn-danger');
      });
      row.appendChild(revokeBtn);

      target.appendChild(row);
    });
  }).catch((err) => {
    const target = document.getElementById('api-tokens-list');
    if (target) {
      target.innerHTML = '<div class="empty-state">'
        + I18n.t('tokens.loadFailed', { message: escapeHtml(err.message) }) + '</div>';
    }
  });
}

function createApiToken() {
  const input = document.getElementById('api-token-name');
  if (!input) return;
  const name = input.value.trim();
  if (!name) {
    showToast(I18n.t('tokens.nameRequired'), 'error');
    input.focus();
    return;
  }
  const btn = document.querySelector('.api-tokens-create');
  if (btn) btn.disabled = true;

  apiFetch('/api/tokens', { method: 'POST', body: { name } }).then((res) => {
    input.value = '';
    renderApiTokenReveal(res.token);
    refreshApiTokensList();
  }).catch((err) => {
    showToast(I18n.t('tokens.createFailed', { message: err.message }), 'error');
  }).finally(() => {
    if (btn) btn.disabled = false;
  });
}

// Shows the plaintext once with a copy affordance — it is unrecoverable
// after the modal closes, matching the backend contract.
function renderApiTokenReveal(plaintext) {
  const slot = document.getElementById('api-token-reveal');
  if (!slot) return;
  slot.innerHTML = '';

  const box = document.createElement('div');
  box.className = 'api-token-plaintext';

  const warning = document.createElement('div');
  warning.className = 'api-token-warning';
  warning.textContent = I18n.t('tokens.copyWarning');
  box.appendChild(warning);

  const valueRow = document.createElement('div');
  valueRow.className = 'api-token-value-row';
  const value = document.createElement('code');
  value.className = 'api-token-value';
  value.textContent = plaintext;
  valueRow.appendChild(value);
  const copyBtn = document.createElement('button');
  copyBtn.type = 'button';
  copyBtn.className = 'api-token-copy';
  copyBtn.textContent = I18n.t('message.copy');
  copyBtn.addEventListener('click', () => {
    navigator.clipboard.writeText(plaintext).then(() => {
      copyBtn.textContent = '\u2713 ' + I18n.t('tokens.copied');
      setTimeout(() => { copyBtn.textContent = I18n.t('message.copy'); }, 2000);
    }).catch(() => showToast(I18n.t('tokens.copyFailed'), 'error'));
  });
  valueRow.appendChild(copyBtn);
  box.appendChild(valueRow);

  slot.appendChild(box);
}

// Account-menu entry point.
document.getElementById('user-tokens-btn')?.addEventListener('click', function() {
  const dd = document.getElementById('user-dropdown');
  if (dd) dd.style.display = 'none';
  openApiTokensModal();
});
