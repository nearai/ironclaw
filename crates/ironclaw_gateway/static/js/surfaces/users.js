function loadUsers() {
  apiFetch('/api/admin/users').then(function(data) {
    renderUsersList(data.users || []);
  }).catch(function(err) {
    var tbody = document.getElementById('users-tbody');
    var empty = document.getElementById('users-empty');
    if (tbody) tbody.innerHTML = '';
    if (empty) {
      empty.style.display = 'block';
      if (err.status === 403 || err.status === 401) {
        empty.textContent = I18n.t('users.adminRequired');
      } else {
        empty.textContent = I18n.t('users.failedToLoad') + ': ' + err.message;
      }
    }
  });
}

function renderUsersList(users) {
  var tbody = document.getElementById('users-tbody');
  var empty = document.getElementById('users-empty');
  if (!users || users.length === 0) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    empty.textContent = I18n.t('users.emptyState');
    return;
  }
  empty.style.display = 'none';
  tbody.innerHTML = users.map(function(u) {
    var statusClass = u.status === 'active' ? 'active' : 'failed';
    var roleLabel = u.role === 'admin' ? '<span class="badge badge-admin">' + I18n.t('users.roleAdmin') + '</span>' : '<span class="badge">' + I18n.t('users.roleMember') + '</span>';
    var actions = '';
    if (u.status === 'active') {
      actions += '<button class="btn-small btn-danger" data-action="suspend-user" data-user-id="' + escapeHtml(u.id) + '">' + I18n.t('users.suspend') + '</button> ';
    } else {
      actions += '<button class="btn-small btn-primary" data-action="activate-user" data-user-id="' + escapeHtml(u.id) + '">' + I18n.t('users.activate') + '</button> ';
    }
    if (u.role === 'member') {
      actions += '<button class="btn-small" data-action="change-role" data-user-id="' + escapeHtml(u.id) + '" data-role="admin">' + I18n.t('users.makeAdmin') + '</button> ';
    } else {
      actions += '<button class="btn-small" data-action="change-role" data-user-id="' + escapeHtml(u.id) + '" data-role="member">' + I18n.t('users.makeMember') + '</button> ';
    }
    actions += '<button class="btn-small" data-action="create-token" data-user-id="' + escapeHtml(u.id) + '" data-user-name="' + escapeHtml(u.display_name) + '">' + I18n.t('users.addToken') + '</button>';
    return '<tr>'
      + '<td class="user-id" title="' + escapeHtml(u.id) + '">' + escapeHtml(u.id.substring(0, 8)) + '…</td>'
      + '<td>' + escapeHtml(u.display_name) + '</td>'
      + '<td>' + escapeHtml(u.email || '—') + '</td>'
      + '<td>' + roleLabel + '</td>'
      + '<td><span class="status-badge ' + statusClass + '">' + escapeHtml(u.status) + '</span></td>'
      + '<td>' + (u.job_count || 0) + '</td>'
      + '<td>' + formatCost(u.total_cost) + '</td>'
      + '<td>' + (u.last_active_at ? formatRelativeTime(u.last_active_at) : '—') + '</td>'
      + '<td>' + formatRelativeTime(u.created_at) + '</td>'
      + '<td>' + actions + '</td>'
      + '</tr>';
  }).join('');
}

function suspendUser(userId) {
  apiFetch('/api/admin/users/' + userId + '/suspend', { method: 'POST' })
    .then(function() { loadUsers(); })
    .catch(function(e) { alert(I18n.t('users.failedSuspend') + ': ' + e.message); });
}

function activateUser(userId) {
  apiFetch('/api/admin/users/' + userId + '/activate', { method: 'POST' })
    .then(function() { loadUsers(); })
    .catch(function(e) { alert(I18n.t('users.failedActivate') + ': ' + e.message); });
}

function changeUserRole(userId, newRole) {
  apiFetch('/api/admin/users/' + userId, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ role: newRole })
  })
    .then(function() { loadUsers(); })
    .catch(function(e) { alert(I18n.t('users.failedRoleChange') + ': ' + e.message); });
}

function createTokenForUser(userId, displayName) {
  var tokenName = prompt('Token name for ' + displayName + ':', 'api-token');
  if (!tokenName) return;
  apiFetch('/api/tokens', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: tokenName, user_id: userId }),
  }).then(function(data) {
    showTokenBanner(data.token, I18n.t('users.tokenCreated'));
  }).catch(function(e) { alert(I18n.t('users.failedCreate') + ': ' + e.message); });
}

function showTokenBanner(tokenValue, title) {
  var banner = document.getElementById('users-token-result');
  if (!banner) return;
  var heading = title || I18n.t('users.tokenCreated');
  var loginUrl = window.location.origin + '/?token=' + encodeURIComponent(tokenValue);
  banner.style.display = 'block';
  banner.innerHTML = '<strong>' + escapeHtml(heading) + '</strong> ' + I18n.t('users.tokenShareMessage') + '<br>'
    + '<code class="token-display" id="token-copy-value">' + escapeHtml(loginUrl) + '</code>'
    + '<button class="btn-small" id="token-copy-link">Copy Link</button>'
    + '<br><span style="font-size:0.8em;color:var(--text-muted)">' + I18n.t('users.rawToken') + ' ' + escapeHtml(tokenValue) + '</span>';
  document.getElementById('token-copy-link').addEventListener('click', function() {
    navigator.clipboard.writeText(loginUrl);
    this.textContent = I18n.t('users.copied');
  });
}

// Delegated click handler for user action buttons (CSP-safe, no inline onclick)
document.getElementById('users-table')?.addEventListener('click', function(e) {
  var btn = e.target.closest('[data-action]');
  if (!btn) return;
  var action = btn.getAttribute('data-action');
  var userId = btn.getAttribute('data-user-id');
  var userName = btn.getAttribute('data-user-name');
  if (action === 'suspend-user') suspendUser(userId);
  else if (action === 'activate-user') activateUser(userId);
  else if (action === 'change-role') changeUserRole(userId, btn.getAttribute('data-role'));
  else if (action === 'create-token') createTokenForUser(userId, userName || '');
});

// Wire up Users tab create form
document.getElementById('users-create-btn')?.addEventListener('click', function() {
  document.getElementById('users-create-form').style.display = 'flex';
  document.getElementById('users-token-result').style.display = 'none';
  document.getElementById('user-display-name').focus();
});

document.getElementById('users-create-cancel')?.addEventListener('click', function() {
  document.getElementById('users-create-form').style.display = 'none';
});

document.getElementById('users-create-submit')?.addEventListener('click', function() {
  var displayName = document.getElementById('user-display-name').value.trim();
  var email = document.getElementById('user-email').value.trim();
  var role = document.getElementById('user-role').value;
  if (!displayName) { alert(I18n.t('users.displayNameRequired')); return; }

  apiFetch('/api/admin/users', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      display_name: displayName,
      email: email || undefined,
      role: role,
    }),
  }).then(function(data) {
    document.getElementById('users-create-form').style.display = 'none';
    document.getElementById('user-display-name').value = '';
    document.getElementById('user-email').value = '';
    if (data.token) {
      showTokenBanner(data.token, I18n.t('users.userCreated'));
    }
    loadUsers();
  }).catch(function(e) { alert(I18n.t('users.failedCreate') + ': ' + e.message); });
});

// --- Gateway status widget ---
