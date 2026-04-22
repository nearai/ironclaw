/* IronClaw Admin Panel */

// TODO(#1968): Inline style attributes throughout this file bypass the
// theme token system in admin.css. Migrate to CSS classes that reference
// CSS custom properties (--space-*, --text-*, --accent, etc.).

(function () {
  'use strict';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  var token = '';
  var oidcProxyAuth = false;
  var currentProfile = null;

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  function escapeHtml(str) {
    if (!str) return '';
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function formatNumber(n) {
    if (n == null) return '0';
    return Number(n).toLocaleString();
  }

  function formatTokenCount(n) {
    if (n == null || n === 0) return '0';
    if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
    if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
    return String(n);
  }

  function formatCost(v) {
    if (v == null) return '$0.00';
    var n = parseFloat(v);
    if (isNaN(n)) return '$0.00';
    return '$' + n.toFixed(2);
  }

  function formatUptime(secs) {
    if (!secs) return '0s';
    var d = Math.floor(secs / 86400);
    var h = Math.floor((secs % 86400) / 3600);
    var m = Math.floor((secs % 3600) / 60);
    if (d > 0) return d + 'd ' + h + 'h';
    if (h > 0) return h + 'h ' + m + 'm';
    return m + 'm';
  }

  function formatRelativeTime(iso) {
    if (!iso) return 'Never';
    var diff = (Date.now() - new Date(iso).getTime()) / 1000;
    if (diff < 0) diff = 0;
    if (diff < 60) return 'Just now';
    if (diff < 3600) return Math.floor(diff / 60) + 'm ago';
    if (diff < 86400) return Math.floor(diff / 3600) + 'h ago';
    if (diff < 2592000) return Math.floor(diff / 86400) + 'd ago';
    return new Date(iso).toLocaleDateString();
  }

  function statusBadge(status) {
    var cls = 'badge badge-' + escapeHtml(status || 'active');
    return '<span class="' + cls + '">' + escapeHtml(status || 'active') + '</span>';
  }

  function roleBadge(role) {
    var cls = 'badge badge-' + escapeHtml(role || 'member');
    return '<span class="' + cls + '">' + escapeHtml(role || 'member') + '</span>';
  }

  function truncateId(id) {
    if (!id) return '';
    return id.length > 12 ? id.slice(0, 12) + '\u2026' : id;
  }

  // ---------------------------------------------------------------------------
  // API
  // ---------------------------------------------------------------------------

  function apiFetch(path, options) {
    var opts = {};
    if (options) {
      for (var k in options) {
        if (Object.prototype.hasOwnProperty.call(options, k)) {
          opts[k] = options[k];
        }
      }
    }
    opts.headers = Object.assign({}, opts.headers || {});
    if (token && !oidcProxyAuth) {
      opts.headers['Authorization'] = 'Bearer ' + token;
    }
    if (opts.body && typeof opts.body === 'object') {
      opts.headers['Content-Type'] = 'application/json';
      opts.body = JSON.stringify(opts.body);
    }
    return fetch(path, opts).then(function (res) {
      if (!res.ok) {
        return res.text().then(function (body) {
          var err = new Error(body || res.status + ' ' + res.statusText);
          err.status = res.status;
          throw err;
        });
      }
      if (res.status === 204) return null;
      return res.json();
    });
  }

  // ---------------------------------------------------------------------------
  // Auth
  // ---------------------------------------------------------------------------

  function showAuth() {
    document.getElementById('auth-screen').style.display = 'flex';
    document.getElementById('access-denied').style.display = 'none';
    document.getElementById('app').style.display = 'none';
  }

  function showAccessDenied() {
    document.getElementById('auth-screen').style.display = 'none';
    document.getElementById('access-denied').style.display = 'flex';
    document.getElementById('app').style.display = 'none';
  }

  function showApp() {
    document.getElementById('auth-screen').style.display = 'none';
    document.getElementById('access-denied').style.display = 'none';
    document.getElementById('app').style.display = 'flex';
  }

  function logout() {
    token = '';
    oidcProxyAuth = false;
    currentProfile = null;
    sessionStorage.removeItem('ironclaw_token');
    showAuth();
  }

  function authenticate(t) {
    oidcProxyAuth = false;
    token = t;
    return apiFetch('/api/profile').then(function (profile) {
      currentProfile = profile;
      if (profile.role !== 'admin') {
        token = '';
        showAccessDenied();
        return false;
      }
      // Security note: sessionStorage is readable by any XSS payload running
      // in this origin. We accept this risk because: (1) the token is session-
      // scoped and cleared on tab close, (2) the CSP restricts script-src to
      // 'self' only (no inline scripts), (3) migration to httpOnly cookies
      // needs server-side session management (larger effort, tracked for
      // follow-up).
      sessionStorage.setItem('ironclaw_token', t);
      showApp();
      route();
      return true;
    }).catch(function (err) {
      token = '';
      throw err;
    });
  }

  function autoAuth() {
    // Check sessionStorage
    var saved = sessionStorage.getItem('ironclaw_token');
    if (saved) {
      authenticate(saved).catch(function () { showAuth(); });
      return;
    }

    // Check implicit auth (e.g. OIDC proxy cookie) by probing profile directly.
    apiFetch('/api/profile').then(function (profile) {
      oidcProxyAuth = true;
      token = '';
      currentProfile = profile;
      if (profile.role !== 'admin') {
        showAccessDenied();
      } else {
        showApp();
        route();
      }
    }).catch(function () {
      oidcProxyAuth = false;
      showAuth();
    });
  }

  // ---------------------------------------------------------------------------
  // Router
  // ---------------------------------------------------------------------------

  function parseHash() {
    var hash = window.location.hash || '#/';
    if (hash.charAt(0) === '#') hash = hash.slice(1);
    if (!hash || hash.charAt(0) !== '/') hash = '/';
    return hash;
  }

  function route() {
    var path = parseHash();
    var content = document.getElementById('content');

    // Update active nav link
    var links = document.querySelectorAll('.nav-link[data-route]');
    for (var i = 0; i < links.length; i++) {
      var r = links[i].getAttribute('data-route');
      var isActive = path === r || (r !== '/' && path.indexOf(r) === 0);
      links[i].classList.toggle('active', isActive);
    }

    // Route dispatch
    if (path === '/') {
      renderDashboard(content);
    } else if (path === '/users') {
      renderUsers(content);
    } else if (path.indexOf('/users/') === 0) {
      var userId = decodeURIComponent(path.slice(7));
      renderUserDetail(content, userId);
    } else if (path === '/usage') {
      renderUsage(content, 'day');
    } else if (path === '/workspaces') {
      renderStub(content, 'Workspaces', 'Workspace management is coming soon.', '#1607');
    } else if (path === '/invitations') {
      renderStub(content, 'Invitations', 'Invitation management is coming soon.', '#1608');
    } else {
      content.innerHTML = '<div class="empty-state"><p>Page not found</p></div>';
    }
  }

  // ---------------------------------------------------------------------------
  // Pages
  // ---------------------------------------------------------------------------

  // --- Dashboard ---

  function renderDashboard(el) {
    el.innerHTML = '<div class="loading">Loading dashboard...</div>';

    Promise.all([
      apiFetch('/api/admin/usage/summary'),
      apiFetch('/api/admin/users')
    ]).then(function (results) {
      var summary = results[0];
      var rawUsers = results[1] || {};
      var users = Array.isArray(rawUsers) ? rawUsers : (rawUsers.users || []);

      var u = summary.users || {};
      var j = summary.jobs || {};
      var usage = summary.usage_30d || {};

      var html = '<div class="page-header"><h1>Dashboard</h1></div>';

      // Metrics
      html += '<div class="metrics-grid">';
      html += metricCard('Total Users', formatNumber(u.total));
      html += metricCard('Active Users', formatNumber(u.active), 'accent');
      html += metricCard('Suspended', formatNumber(u.suspended), u.suspended > 0 ? 'danger' : '');
      html += metricCard('Admins', formatNumber(u.admins));
      html += metricCard('Total Jobs', formatNumber(j.total));
      html += metricCard('30d LLM Calls', formatNumber(usage.llm_calls));
      html += metricCard('30d Cost', formatCost(usage.total_cost), 'accent');
      html += metricCard('Uptime', formatUptime(summary.uptime_seconds));
      html += '</div>';

      // Recent users table
      var recent = users.slice().sort(function (a, b) {
        var ta = a.last_active_at || a.created_at || '';
        var tb = b.last_active_at || b.created_at || '';
        return tb.localeCompare(ta);
      }).slice(0, 5);

      html += '<div class="detail-card">';
      html += '<h2>Recent Users</h2>';
      if (recent.length === 0) {
        html += '<div class="empty-state"><p>No users yet</p></div>';
      } else {
        html += '<table class="data-table"><thead><tr>';
        html += '<th>Name</th><th>Role</th><th>Status</th><th>Jobs</th><th>Last Active</th>';
        html += '</tr></thead><tbody>';
        for (var i = 0; i < recent.length; i++) {
          var ru = recent[i];
          html += '<tr>';
          html += '<td><a href="#/users/' + encodeURIComponent(ru.id) + '" style="color:var(--accent);text-decoration:none">' + escapeHtml(ru.display_name) + '</a></td>';
          html += '<td>' + roleBadge(ru.role) + '</td>';
          html += '<td>' + statusBadge(ru.status) + '</td>';
          html += '<td class="mono">' + formatNumber(ru.job_count) + '</td>';
          html += '<td>' + formatRelativeTime(ru.last_active_at) + '</td>';
          html += '</tr>';
        }
        html += '</tbody></table>';
      }
      html += '</div>';

      el.innerHTML = html;
    }).catch(function (err) {
      el.innerHTML = '<div class="error-message">Failed to load dashboard: ' + escapeHtml(err.message) + '</div>';
    });
  }

  function metricCard(label, value, cls) {
    return '<div class="metric-card">' +
      '<div class="metric-label">' + escapeHtml(label) + '</div>' +
      '<div class="metric-value' + (cls ? ' ' + cls : '') + '">' + escapeHtml(value) + '</div>' +
      '</div>';
  }

  // --- Users List ---

  var usersCache = null;
  var usersFilter = 'all';
  var usersSearch = '';

  function renderUsers(el) {
    el.innerHTML = '<div class="loading">Loading users...</div>';
    usersCache = null;
    usersFilter = 'all';
    usersSearch = '';

    apiFetch('/api/admin/users').then(function (raw) {
      usersCache = Array.isArray(raw) ? raw : (raw && raw.users ? raw.users : []);
      renderUsersPage(el);
    }).catch(function (err) {
      el.innerHTML = '<div class="error-message">Failed to load users: ' + escapeHtml(err.message) + '</div>';
    });
  }

  function renderUsersPage(el) {
    var users = filterUsers(usersCache || []);

    var html = '<div class="page-header"><h1>Users</h1>';
    html += '<button class="btn-primary" data-action="show-create-form">+ New User</button>';
    html += '</div>';

    // Create user form (hidden by default)
    html += '<div id="create-user-form" class="form-card" style="display:none">';
    html += '<div class="form-row">';
    html += '<div class="form-group"><label>Display Name</label><input type="text" id="new-user-name" placeholder="Jane Doe"></div>';
    html += '<div class="form-group"><label>Email (optional)</label><input type="text" id="new-user-email" placeholder="jane@example.com"></div>';
    html += '<div class="form-group"><label>Role</label><select id="new-user-role"><option value="member">Member</option><option value="admin">Admin</option></select></div>';
    html += '<div class="form-group" style="align-self:end"><button class="btn-primary" data-action="create-user">Create</button> ';
    html += '<button class="btn-secondary" data-action="hide-create-form">Cancel</button></div>';
    html += '</div></div>';

    // Token banner
    html += '<div id="user-token-banner" style="display:none"></div>';

    // Toolbar
    html += '<div class="toolbar">';
    html += '<input type="text" class="search-input" id="users-search" placeholder="Search by name or email..." value="' + escapeHtml(usersSearch) + '">';
    html += '<button class="filter-btn' + (usersFilter === 'all' ? ' active' : '') + '" data-action="filter" data-filter="all">All</button>';
    html += '<button class="filter-btn' + (usersFilter === 'active' ? ' active' : '') + '" data-action="filter" data-filter="active">Active</button>';
    html += '<button class="filter-btn' + (usersFilter === 'suspended' ? ' active' : '') + '" data-action="filter" data-filter="suspended">Suspended</button>';
    html += '<button class="filter-btn' + (usersFilter === 'admin' ? ' active' : '') + '" data-action="filter" data-filter="admin">Admins</button>';
    html += '</div>';

    // Table
    if (users.length === 0) {
      html += '<div class="empty-state"><p>No users found</p></div>';
    } else {
      html += '<table class="data-table"><thead><tr>';
      html += '<th>ID</th><th>Name</th><th>Email</th><th>Role</th><th>Status</th>';
      html += '<th>Jobs</th><th>Cost</th><th>Last Active</th><th>Actions</th>';
      html += '</tr></thead><tbody>';
      for (var i = 0; i < users.length; i++) {
        var u = users[i];
        html += '<tr>';
        html += '<td class="mono">' + escapeHtml(truncateId(u.id)) + '</td>';
        html += '<td><a href="#/users/' + encodeURIComponent(u.id) + '" style="color:var(--accent);text-decoration:none">' + escapeHtml(u.display_name) + '</a></td>';
        html += '<td>' + escapeHtml(u.email || '') + '</td>';
        html += '<td>' + roleBadge(u.role) + '</td>';
        html += '<td>' + statusBadge(u.status) + '</td>';
        html += '<td class="mono">' + formatNumber(u.job_count) + '</td>';
        html += '<td class="mono">' + formatCost(u.total_cost) + '</td>';
        html += '<td>' + formatRelativeTime(u.last_active_at) + '</td>';
        html += '<td class="actions">';
        if (u.status === 'active') {
          html += '<button class="btn-small" data-action="suspend" data-id="' + escapeHtml(u.id) + '">Suspend</button>';
        } else {
          html += '<button class="btn-small" data-action="activate" data-id="' + escapeHtml(u.id) + '">Activate</button>';
        }
        if (u.role === 'admin') {
          html += '<button class="btn-small" data-action="change-role" data-id="' + escapeHtml(u.id) + '" data-role="member">Demote</button>';
        } else {
          html += '<button class="btn-small" data-action="change-role" data-id="' + escapeHtml(u.id) + '" data-role="admin">Promote</button>';
        }
        html += '<button class="btn-small" data-action="create-token" data-id="' + escapeHtml(u.id) + '" data-name="' + escapeHtml(u.display_name) + '">Token</button>';
        html += '</td>';
        html += '</tr>';
      }
      html += '</tbody></table>';
    }

    el.innerHTML = html;

    // Search input handler
    var searchEl = document.getElementById('users-search');
    if (searchEl) {
      searchEl.addEventListener('input', function () {
        usersSearch = searchEl.value;
        renderUsersPage(el);
      });
      searchEl.focus();
      searchEl.setSelectionRange(usersSearch.length, usersSearch.length);
    }
  }

  function filterUsers(users) {
    var result = users;
    if (usersFilter === 'active') {
      result = result.filter(function (u) { return u.status === 'active'; });
    } else if (usersFilter === 'suspended') {
      result = result.filter(function (u) { return u.status === 'suspended'; });
    } else if (usersFilter === 'admin') {
      result = result.filter(function (u) { return u.role === 'admin'; });
    }
    if (usersSearch) {
      var q = usersSearch.toLowerCase();
      result = result.filter(function (u) {
        return (u.display_name && u.display_name.toLowerCase().indexOf(q) >= 0) ||
               (u.email && u.email.toLowerCase().indexOf(q) >= 0) ||
               (u.id && u.id.toLowerCase().indexOf(q) >= 0);
      });
    }
    return result;
  }

  // --- User Detail ---

  function renderUserDetail(el, userId) {
    el.innerHTML = '<div class="loading">Loading user...</div>';

    Promise.all([
      apiFetch('/api/admin/users/' + encodeURIComponent(userId)),
      apiFetch('/api/admin/usage?user_id=' + encodeURIComponent(userId) + '&period=month')
    ]).then(function (results) {
      var user = results[0];
      var usageData = results[1];

      var html = '<div class="breadcrumb"><a href="#/users">Users</a><span class="sep">/</span><span>' + escapeHtml(user.display_name) + '</span></div>';

      html += '<div class="page-header"><h1>' + escapeHtml(user.display_name) + '</h1>';
      html += '<div class="actions">';
      if (user.status === 'active') {
        html += '<button class="btn-small" data-action="suspend" data-id="' + escapeHtml(user.id) + '">Suspend</button>';
      } else {
        html += '<button class="btn-small" data-action="activate" data-id="' + escapeHtml(user.id) + '">Activate</button>';
      }
      html += '<button class="btn-small" data-action="create-token" data-id="' + escapeHtml(user.id) + '" data-name="' + escapeHtml(user.display_name) + '">Create Token</button>';
      html += '<button class="btn-small btn-danger" data-action="delete-user" data-id="' + escapeHtml(user.id) + '" data-name="' + escapeHtml(user.display_name) + '">Delete</button>';
      html += '</div></div>';

      // Token banner slot
      html += '<div id="user-token-banner" style="display:none"></div>';

      // Profile + Stats grid
      html += '<div class="detail-grid">';

      // Profile card
      html += '<div class="detail-card"><h2>Profile</h2>';
      html += detailRowRawHtml('ID', '<span class="mono">' + escapeHtml(user.id) + '</span>');
      html += detailRow('Email', user.email || 'Not set');
      html += detailRowRawHtml('Role', roleBadge(user.role));
      html += detailRowRawHtml('Status', statusBadge(user.status));
      html += detailRow('Created', formatRelativeTime(user.created_at));
      html += detailRow('Last Login', formatRelativeTime(user.last_login_at));
      if (user.created_by) {
        html += detailRowRawHtml('Created By', '<span class="mono">' + escapeHtml(truncateId(user.created_by)) + '</span>');
      }
      html += '</div>';

      // Stats card
      html += '<div class="detail-card"><h2>Summary</h2>';
      html += detailRow('Jobs', formatNumber(user.job_count));
      html += detailRow('Total Cost', formatCost(user.total_cost));
      html += detailRow('Last Active', formatRelativeTime(user.last_active_at));
      html += '</div>';

      html += '</div>';

      // Role management
      html += '<div class="detail-card" style="margin-bottom:var(--space-6)">';
      html += '<h2>Role Management</h2>';
      html += '<div class="form-row">';
      html += '<div class="form-group"><label>Current Role</label>';
      html += '<select id="role-select" data-id="' + escapeHtml(user.id) + '">';
      html += '<option value="member"' + (user.role === 'member' ? ' selected' : '') + '>Member</option>';
      html += '<option value="admin"' + (user.role === 'admin' ? ' selected' : '') + '>Admin</option>';
      html += '</select></div>';
      html += '<div class="form-group" style="align-self:end"><button class="btn-primary" data-action="save-role" data-id="' + escapeHtml(user.id) + '">Save Role</button></div>';
      html += '</div></div>';

      // Usage table
      var entries = (usageData && usageData.usage) || [];
      html += '<div class="detail-card">';
      html += '<h2>Usage (Last 30 Days)</h2>';
      if (entries.length === 0) {
        html += '<div class="empty-state"><p>No usage data</p></div>';
      } else {
        html += '<table class="data-table"><thead><tr>';
        html += '<th>Model</th><th>Calls</th><th>Input Tokens</th><th>Output Tokens</th><th>Cost</th>';
        html += '</tr></thead><tbody>';
        for (var i = 0; i < entries.length; i++) {
          var e = entries[i];
          html += '<tr>';
          html += '<td class="mono">' + escapeHtml(e.model) + '</td>';
          html += '<td class="mono">' + formatNumber(e.call_count) + '</td>';
          html += '<td class="mono">' + formatTokenCount(e.input_tokens) + '</td>';
          html += '<td class="mono">' + formatTokenCount(e.output_tokens) + '</td>';
          html += '<td class="mono">' + formatCost(e.total_cost) + '</td>';
          html += '</tr>';
        }
        html += '</tbody></table>';
      }
      html += '</div>';

      el.innerHTML = html;
    }).catch(function (err) {
      el.innerHTML = '<div class="breadcrumb"><a href="#/users">Users</a><span class="sep">/</span><span>Error</span></div>' +
        '<div class="error-message">Failed to load user: ' + escapeHtml(err.message) + '</div>';
    });
  }

  function detailRow(label, value) {
    return '<div class="detail-row"><span class="detail-label">' + escapeHtml(label) + '</span><span class="detail-value">' + escapeHtml(value == null ? '' : String(value)) + '</span></div>';
  }

  // SAFETY: valueHtml is injected as raw HTML — callers MUST pre-escape any
  // user-supplied content via escapeHtml() to prevent XSS. Prefer detailRow()
  // for plain-text values; use this variant only when the value contains
  // trusted markup (badges, <span class="mono">, etc.).
  function detailRowRawHtml(label, valueHtml) {
    return '<div class="detail-row"><span class="detail-label">' + escapeHtml(label) + '</span><span class="detail-value">' + valueHtml + '</span></div>';
  }

  // --- Usage ---

  function renderUsage(el, period) {
    el.innerHTML = '<div class="loading">Loading usage data...</div>';

    apiFetch('/api/admin/usage?period=' + encodeURIComponent(period)).then(function (data) {
      var entries = (data && data.usage) || [];

      var html = '<div class="page-header"><h1>Usage</h1>';
      html += '<div class="period-selector">';
      html += '<button class="period-btn' + (period === 'day' ? ' active' : '') + '" data-action="period" data-period="day">24h</button>';
      html += '<button class="period-btn' + (period === 'week' ? ' active' : '') + '" data-action="period" data-period="week">7d</button>';
      html += '<button class="period-btn' + (period === 'month' ? ' active' : '') + '" data-action="period" data-period="month">30d</button>';
      html += '</div></div>';

      if (entries.length === 0) {
        html += '<div class="empty-state"><p>No usage data for this period</p></div>';
        el.innerHTML = html;
        return;
      }

      // Aggregate by user
      var byUser = {};
      var maxCost = 0;
      for (var i = 0; i < entries.length; i++) {
        var e = entries[i];
        if (!byUser[e.user_id]) {
          byUser[e.user_id] = { user_id: e.user_id, calls: 0, input_tokens: 0, output_tokens: 0, cost: 0 };
        }
        byUser[e.user_id].calls += e.call_count || 0;
        byUser[e.user_id].input_tokens += e.input_tokens || 0;
        byUser[e.user_id].output_tokens += e.output_tokens || 0;
        byUser[e.user_id].cost += parseFloat(e.total_cost) || 0;
      }

      var userList = Object.keys(byUser).map(function (k) { return byUser[k]; });
      userList.sort(function (a, b) { return b.cost - a.cost; });

      for (var j = 0; j < userList.length; j++) {
        if (userList[j].cost > maxCost) maxCost = userList[j].cost;
      }

      // Summary row
      var totalCalls = 0, totalInput = 0, totalOutput = 0, totalCostVal = 0;
      for (var k = 0; k < userList.length; k++) {
        totalCalls += userList[k].calls;
        totalInput += userList[k].input_tokens;
        totalOutput += userList[k].output_tokens;
        totalCostVal += userList[k].cost;
      }

      html += '<div class="metrics-grid" style="margin-bottom:var(--space-6)">';
      html += metricCard('Total Calls', formatNumber(totalCalls));
      html += metricCard('Input Tokens', formatTokenCount(totalInput));
      html += metricCard('Output Tokens', formatTokenCount(totalOutput));
      html += metricCard('Total Cost', formatCost(totalCostVal.toFixed(2)), 'accent');
      html += '</div>';

      // Per-user table
      html += '<div class="detail-card">';
      html += '<h2>Per-User Breakdown</h2>';
      html += '<table class="data-table"><thead><tr>';
      html += '<th>User</th><th>Calls</th><th>Input Tokens</th><th>Output Tokens</th><th>Cost</th><th></th>';
      html += '</tr></thead><tbody>';
      for (var m = 0; m < userList.length; m++) {
        var uu = userList[m];
        var pct = maxCost > 0 ? (uu.cost / maxCost * 100) : 0;
        html += '<tr>';
        html += '<td><a href="#/users/' + encodeURIComponent(uu.user_id) + '" class="mono" style="color:var(--accent);text-decoration:none">' + escapeHtml(truncateId(uu.user_id)) + '</a></td>';
        html += '<td class="mono">' + formatNumber(uu.calls) + '</td>';
        html += '<td class="mono">' + formatTokenCount(uu.input_tokens) + '</td>';
        html += '<td class="mono">' + formatTokenCount(uu.output_tokens) + '</td>';
        html += '<td class="mono">' + formatCost(uu.cost.toFixed(2)) + '</td>';
        html += '<td class="usage-bar-cell"><div class="usage-bar" style="width:' + pct.toFixed(1) + '%"></div></td>';
        html += '</tr>';
      }
      html += '</tbody></table></div>';

      // Per-model table
      var byModel = {};
      for (var n = 0; n < entries.length; n++) {
        var em = entries[n];
        if (!byModel[em.model]) {
          byModel[em.model] = { model: em.model, calls: 0, input_tokens: 0, output_tokens: 0, cost: 0 };
        }
        byModel[em.model].calls += em.call_count || 0;
        byModel[em.model].input_tokens += em.input_tokens || 0;
        byModel[em.model].output_tokens += em.output_tokens || 0;
        byModel[em.model].cost += parseFloat(em.total_cost) || 0;
      }

      var modelList = Object.keys(byModel).map(function (k) { return byModel[k]; });
      modelList.sort(function (a, b) { return b.cost - a.cost; });

      html += '<div class="detail-card" style="margin-top:var(--space-6)">';
      html += '<h2>Per-Model Breakdown</h2>';
      html += '<table class="data-table"><thead><tr>';
      html += '<th>Model</th><th>Calls</th><th>Input Tokens</th><th>Output Tokens</th><th>Cost</th>';
      html += '</tr></thead><tbody>';
      for (var p = 0; p < modelList.length; p++) {
        var mm = modelList[p];
        html += '<tr>';
        html += '<td class="mono">' + escapeHtml(mm.model) + '</td>';
        html += '<td class="mono">' + formatNumber(mm.calls) + '</td>';
        html += '<td class="mono">' + formatTokenCount(mm.input_tokens) + '</td>';
        html += '<td class="mono">' + formatTokenCount(mm.output_tokens) + '</td>';
        html += '<td class="mono">' + formatCost(mm.cost.toFixed(2)) + '</td>';
        html += '</tr>';
      }
      html += '</tbody></table></div>';

      el.innerHTML = html;
    }).catch(function (err) {
      el.innerHTML = '<div class="error-message">Failed to load usage: ' + escapeHtml(err.message) + '</div>';
    });
  }

  // --- Stub pages ---

  function renderStub(el, title, desc, issue) {
    el.innerHTML = '<div class="coming-soon">' +
      '<h2>' + escapeHtml(title) + '</h2>' +
      '<p>' + escapeHtml(desc) + '</p>' +
      '<p style="margin-top:var(--space-3);font-size:var(--text-xs);color:var(--text-dimmed)">Tracking: ' + escapeHtml(issue) + '</p>' +
      '</div>';
  }

  // ---------------------------------------------------------------------------
  // Actions (event delegation)
  // ---------------------------------------------------------------------------

  function handleAction(target) {
    var action = target.getAttribute('data-action');
    if (!action) return;

    var id = target.getAttribute('data-id');
    var content = document.getElementById('content');

    switch (action) {
      case 'show-create-form':
        var form = document.getElementById('create-user-form');
        if (form) form.style.display = 'block';
        break;

      case 'hide-create-form':
        var formH = document.getElementById('create-user-form');
        if (formH) formH.style.display = 'none';
        break;

      case 'create-user':
        createUser(content);
        break;

      case 'suspend':
        apiFetch('/api/admin/users/' + encodeURIComponent(id) + '/suspend', { method: 'POST' })
          .then(function () { refreshCurrentPage(); })
          .catch(function (err) { alert('Failed to suspend: ' + err.message); });
        break;

      case 'activate':
        apiFetch('/api/admin/users/' + encodeURIComponent(id) + '/activate', { method: 'POST' })
          .then(function () { refreshCurrentPage(); })
          .catch(function (err) { alert('Failed to activate: ' + err.message); });
        break;

      case 'change-role':
        var newRole = target.getAttribute('data-role');
        apiFetch('/api/admin/users/' + encodeURIComponent(id), {
          method: 'PATCH',
          body: { role: newRole }
        }).then(function () { refreshCurrentPage(); })
          .catch(function (err) { alert('Failed to change role: ' + err.message); });
        break;

      case 'save-role':
        var sel = document.getElementById('role-select');
        if (sel) {
          apiFetch('/api/admin/users/' + encodeURIComponent(id), {
            method: 'PATCH',
            body: { role: sel.value }
          }).then(function () { refreshCurrentPage(); })
            .catch(function (err) { alert('Failed to save role: ' + err.message); });
        }
        break;

      case 'create-token':
        var userName = target.getAttribute('data-name') || 'user';
        var tokenName = prompt('Token name for ' + userName + ':');
        if (!tokenName) return;
        apiFetch('/api/tokens', {
          method: 'POST',
          body: { name: tokenName, user_id: id }
        }).then(function (res) {
          showTokenBanner(res.token || res.plaintext_token);
        }).catch(function (err) { alert('Failed to create token: ' + err.message); });
        break;

      case 'delete-user':
        var name = target.getAttribute('data-name') || id;
        showConfirmModal(
          'Delete User',
          'Are you sure you want to delete "' + name + '"? This action cannot be undone.',
          'Delete',
          function () {
            apiFetch('/api/admin/users/' + encodeURIComponent(id), { method: 'DELETE' })
              .then(function () { window.location.hash = '#/users'; })
              .catch(function (err) { alert('Failed to delete: ' + err.message); });
          }
        );
        break;

      case 'filter':
        usersFilter = target.getAttribute('data-filter') || 'all';
        renderUsersPage(content);
        break;

      case 'period':
        var period = target.getAttribute('data-period') || 'day';
        renderUsage(content, period);
        break;

      case 'copy-token':
        var tokenVal = target.getAttribute('data-token');
        if (tokenVal && navigator.clipboard) {
          navigator.clipboard.writeText(tokenVal);
          target.textContent = 'Copied!';
          setTimeout(function () { target.textContent = 'Copy'; }, 2000);
        }
        break;

      case 'modal-close':
        closeModal();
        break;
    }
  }

  function createUser(el) {
    var nameEl = document.getElementById('new-user-name');
    var emailEl = document.getElementById('new-user-email');
    var roleEl = document.getElementById('new-user-role');
    if (!nameEl || !nameEl.value.trim()) {
      alert('Display name is required');
      return;
    }
    var body = { display_name: nameEl.value.trim(), role: roleEl ? roleEl.value : 'member' };
    if (emailEl && emailEl.value.trim()) body.email = emailEl.value.trim();

    apiFetch('/api/admin/users', { method: 'POST', body: body }).then(function (res) {
      var formEl = document.getElementById('create-user-form');
      if (formEl) formEl.style.display = 'none';
      if (nameEl) nameEl.value = '';
      if (emailEl) emailEl.value = '';
      var createdToken = res && (res.token || res.plaintext_token);

      // Reload users list
      apiFetch('/api/admin/users').then(function (raw) {
        usersCache = Array.isArray(raw) ? raw : (raw && raw.users ? raw.users : []);
        renderUsersPage(el);
        if (createdToken) {
          showTokenBanner(createdToken);
        }
      });
    }).catch(function (err) {
      alert('Failed to create user: ' + err.message);
    });
  }

  function showTokenBanner(tokenValue) {
    var banner = document.getElementById('user-token-banner');
    if (!banner) return;
    banner.innerHTML = '<div class="token-banner">' +
      '<p><strong>Token created!</strong> Copy this now — it will not be shown again.</p>' +
      '<div class="token-value"><code>' + escapeHtml(tokenValue) + '</code>' +
      '<button class="btn-small" data-action="copy-token" data-token="' + escapeHtml(tokenValue) + '">Copy</button></div>' +
      '<p style="margin-top:var(--space-2);font-size:var(--text-xs);color:var(--text-muted)">Use this token in the admin login field.</p>' +
      '</div>';
    banner.style.display = 'block';
  }

  function refreshCurrentPage() {
    route();
  }

  // ---------------------------------------------------------------------------
  // Modal
  // ---------------------------------------------------------------------------

  /**
   * Show a confirmation modal dialog.
   *
   * All string parameters are HTML-escaped via escapeHtml() before insertion
   * into the DOM, so callers do not need to pre-sanitise user-supplied strings.
   *
   * @param {string} title - Dialog title (escaped before rendering).
   * @param {string} message - Dialog body text (escaped before rendering).
   * @param {string} confirmText - Text for the confirm button (escaped before rendering).
   * @param {Function} onConfirm - Callback invoked when the user confirms.
   */
  function showConfirmModal(title, message, confirmText, onConfirm) {
    var overlay = document.getElementById('modal-overlay');
    var content = document.getElementById('modal-content');
    if (!overlay || !content) return;

    content.innerHTML = '<h2>' + escapeHtml(title) + '</h2>' +
      '<p style="color:var(--text-secondary);margin-top:var(--space-3)">' + escapeHtml(message) + '</p>' +
      '<div class="modal-actions">' +
      '<button class="btn-secondary" data-action="modal-close">Cancel</button>' +
      '<button class="btn-primary btn-danger" id="modal-confirm">' + escapeHtml(confirmText) + '</button>' +
      '</div>';
    overlay.style.display = 'flex';

    var confirmBtn = document.getElementById('modal-confirm');
    if (confirmBtn) {
      confirmBtn.onclick = function () {
        closeModal();
        onConfirm();
      };
    }
  }

  function closeModal() {
    var overlay = document.getElementById('modal-overlay');
    if (overlay) overlay.style.display = 'none';
  }

  // ---------------------------------------------------------------------------
  // Event Listeners
  // ---------------------------------------------------------------------------

  document.addEventListener('click', function (e) {
    var target = e.target;
    // Walk up to find data-action
    while (target && target !== document) {
      if (target.getAttribute && target.getAttribute('data-action')) {
        e.preventDefault();
        handleAction(target);
        return;
      }
      target = target.parentElement;
    }
  });

  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') closeModal();
  });

  // Modal overlay click to close
  var overlay = document.getElementById('modal-overlay');
  if (overlay) {
    overlay.addEventListener('click', function (e) {
      if (e.target === overlay) closeModal();
    });
  }

  // Auth form
  var connectBtn = document.getElementById('connect-btn');
  if (connectBtn) {
    connectBtn.addEventListener('click', function () {
      var input = document.getElementById('token-input');
      var errEl = document.getElementById('auth-error');
      if (!input || !input.value.trim()) return;

      connectBtn.disabled = true;
      connectBtn.textContent = 'Connecting...';

      authenticate(input.value.trim()).catch(function (err) {
        if (errEl) {
          errEl.textContent = 'Authentication failed: ' + err.message;
          errEl.style.display = 'block';
        }
        connectBtn.disabled = false;
        connectBtn.textContent = 'Connect';
      });
    });
  }

  var tokenInput = document.getElementById('token-input');
  if (tokenInput) {
    tokenInput.addEventListener('keydown', function (e) {
      if (e.key === 'Enter' && connectBtn) connectBtn.click();
    });
  }

  // Logout buttons
  var logoutBtn = document.getElementById('logout-btn');
  if (logoutBtn) logoutBtn.addEventListener('click', logout);
  var logoutDenied = document.getElementById('logout-btn-denied');
  if (logoutDenied) logoutDenied.addEventListener('click', logout);

  // Hash-based routing
  window.addEventListener('hashchange', function () {
    if (document.getElementById('app').style.display !== 'none') {
      route();
    }
  });

  // ---------------------------------------------------------------------------
  // Init
  // ---------------------------------------------------------------------------

  autoAuth();

})();
