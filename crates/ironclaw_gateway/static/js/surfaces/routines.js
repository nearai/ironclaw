let currentRoutineId = null;

function loadRoutines() {
  currentRoutineId = null;

  // Restore list view if detail was open
  const detail = document.getElementById('routine-detail');
  if (detail) detail.style.display = 'none';
  const table = document.getElementById('routines-table');
  if (table) table.style.display = '';

  Promise.all([
    apiFetch('/api/routines/summary'),
    apiFetch('/api/routines'),
  ]).then(([summary, listData]) => {
    renderRoutinesSummary(summary);
    renderRoutinesList(listData.routines);
  }).catch(() => {});
}

function renderRoutinesSummary(s) {
  document.getElementById('routines-summary').innerHTML = ''
    + summaryCard(I18n.t('routines.summary.total'), s.total, '')
    + summaryCard(I18n.t('routines.summary.enabled'), s.enabled, 'active')
    + summaryCard(I18n.t('routines.summary.disabled'), s.disabled, '')
    + summaryCard(I18n.t('routines.summary.unverified'), s.unverified, 'pending')
    + summaryCard(I18n.t('routines.summary.failing'), s.failing, 'failed')
    + summaryCard(I18n.t('routines.summary.runsToday'), s.runs_today, 'completed');
}

function renderRoutinesList(routines) {
  const tbody = document.getElementById('routines-tbody');
  const empty = document.getElementById('routines-empty');

  if (!routines || routines.length === 0) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = routines.map((r) => {
    const statusClass = r.status === 'active' ? 'completed'
      : r.status === 'failing' ? 'failed'
      : r.status === 'attention' ? 'stuck'
      : r.status === 'running' ? 'in_progress'
      : 'pending';

    const toggleLabel = r.enabled ? 'Disable' : 'Enable';
    const toggleClass = r.enabled ? 'btn-cancel' : 'btn-restart';
    const triggerTitle = (r.trigger_type === 'cron' && r.trigger_raw)
      ? ' title="' + escapeHtml(r.trigger_raw) + '"'
      : '';
    const runLabel = (r.verification_status === 'unverified' || r.status === 'unverified')
      ? 'Verify now'
      : 'Run';

    return '<tr class="routine-row" data-action="open-routine" data-id="' + escapeHtml(r.id) + '">'
      + '<td>' + escapeHtml(r.name) + '</td>'
      + '<td' + triggerTitle + '>' + escapeHtml(r.trigger_summary) + '</td>'
      + '<td>' + escapeHtml(r.action_type) + '</td>'
      + '<td>' + formatRelativeTime(r.last_run_at) + '</td>'
      + '<td>' + formatRelativeTime(r.next_fire_at) + '</td>'
      + '<td>' + r.run_count + '</td>'
      + '<td><span class="badge ' + statusClass + '">' + escapeHtml(r.status) + '</span></td>'
      + '<td>'
      + '<button class="' + toggleClass + '" data-action="toggle-routine" data-id="' + escapeHtml(r.id) + '">' + toggleLabel + '</button> '
      + '<button class="btn-restart" data-action="trigger-routine" data-id="' + escapeHtml(r.id) + '">' + runLabel + '</button> '
      + '<button class="btn-cancel" data-action="delete-routine" data-id="' + escapeHtml(r.id) + '" data-name="' + escapeHtml(r.name) + '">Delete</button>'
      + '</td>'
      + '</tr>';
  }).join('');
}

function openRoutineDetail(id) {
  currentRoutineId = id;
  updateHash();
  apiFetch('/api/routines/' + id).then((routine) => {
    renderRoutineDetail(routine);
  }).catch((err) => {
    showToast(I18n.t('routines.loadFailed', { message: err.message }), 'error');
  });
}

function closeRoutineDetail() {
  currentRoutineId = null;
  loadRoutines();
  updateHash();
}

function renderRoutineDetail(routine) {
  const table = document.getElementById('routines-table');
  if (table) table.style.display = 'none';
  document.getElementById('routines-empty').style.display = 'none';

  const detail = document.getElementById('routine-detail');
  detail.style.display = 'block';

  const statusClass = routine.status === 'active' ? 'completed'
    : routine.status === 'failing' ? 'failed'
    : routine.status === 'attention' ? 'stuck'
    : routine.status === 'running' ? 'in_progress'
    : 'pending';
  const statusLabel = routine.status || 'active';

  let html = '<div class="job-detail-header">'
    + '<button class="btn-back" data-action="close-routine-detail">&larr; Back</button>'
    + '<h2>' + escapeHtml(routine.name) + '</h2>'
    + '<span class="badge ' + statusClass + '">' + escapeHtml(statusLabel) + '</span>'
    + '</div>';

  // Metadata grid
  html += '<div class="job-meta-grid">'
    + metaItem(I18n.t('routines.id'), routine.id)
    + metaItem(I18n.t('routines.enabled'), routine.enabled ? I18n.t('settings.on') : I18n.t('settings.off'))
    + metaItem(I18n.t('routines.runCount'), routine.run_count)
    + metaItem(I18n.t('routines.failures'), routine.consecutive_failures)
    + metaItem(I18n.t('routines.lastRun'), formatDate(routine.last_run_at))
    + metaItem(I18n.t('routines.nextFire'), formatDate(routine.next_fire_at))
    + metaItem(I18n.t('routines.created'), formatDate(routine.created_at))
    + '</div>';

  // Description
  if (routine.description) {
    html += '<div class="job-description"><h3>Description</h3>'
      + '<div class="job-description-body">' + escapeHtml(routine.description) + '</div></div>';
  }

  if (routine.verification_status === 'unverified') {
    let verificationCopy = 'Created or updated, but not yet verified with a successful run.';
    if (routine.recent_runs && routine.recent_runs.length > 0) {
      const latestRun = routine.recent_runs[0];
      if (latestRun.status === 'failed') {
        verificationCopy = 'The latest verification attempt failed. Review the run details and verify again after fixing it.';
      } else if (latestRun.status === 'attention') {
        verificationCopy = 'The latest verification attempt needs attention. Review the run details and verify again when ready.';
      }
    }
    html += '<div class="job-description"><h3>Verification</h3>'
      + '<div class="job-description-body">' + escapeHtml(verificationCopy) + '</div></div>';
  }

  // Trigger config
  if (routine.trigger_type === 'cron') {
    const summary = routine.trigger_summary || 'cron';
    const raw = routine.trigger_raw || '';
    const timezone = routine.trigger && routine.trigger.timezone ? String(routine.trigger.timezone) : '';
    html += '<div class="job-description"><h3>Trigger</h3>'
      + '<div class="job-description-body"><strong>' + escapeHtml(summary) + '</strong></div>';
    if (raw) {
      html += '<div class="job-meta-item">'
        + '<span class="job-meta-label">Raw</span>'
        + '<span class="job-meta-value">' + escapeHtml(raw + (timezone ? ' (' + timezone + ')' : '')) + '</span>'
        + '</div>';
    }
    html += '</div>';
  } else {
    html += '<div class="job-description"><h3>Trigger</h3>'
      + '<pre class="action-json">' + escapeHtml(JSON.stringify(routine.trigger, null, 2)) + '</pre></div>';
  }

  html += '<div class="job-description"><h3>Action</h3>'
    + '<pre class="action-json">' + escapeHtml(JSON.stringify(routine.action, null, 2)) + '</pre></div>';

  // Conversation thread link
  if (routine.conversation_id) {
    html += '<div class="job-description">'
      + '<a href="#" data-action="view-routine-thread" data-id="' + escapeHtml(routine.conversation_id) + '" class="btn-primary" style="display:inline-block;margin:0.5rem 0">'
      + 'View Execution Thread</a></div>';
  }

  // Recent runs
  if (routine.recent_runs && routine.recent_runs.length > 0) {
    html += '<div class="job-timeline-section"><h3>Recent Runs</h3>'
      + '<table class="routines-table"><thead><tr>'
      + '<th>Trigger</th><th>Started</th><th>Completed</th><th>Status</th><th>Summary</th><th>Tokens</th>'
      + '</tr></thead><tbody>';
    for (const run of routine.recent_runs) {
      const runStatusClass = run.status === 'ok' ? 'completed'
        : run.status === 'failed' ? 'failed'
        : run.status === 'attention' ? 'stuck'
        : 'in_progress';
      html += '<tr>'
        + '<td>' + escapeHtml(run.trigger_type) + '</td>'
        + '<td>' + formatDate(run.started_at) + '</td>'
        + '<td>' + formatDate(run.completed_at) + '</td>'
        + '<td><span class="badge ' + runStatusClass + '">' + escapeHtml(run.status) + '</span></td>'
        + '<td>' + escapeHtml(run.result_summary || '-')
          + (run.job_id ? ' <a href="#" data-action="view-run-job" data-id="' + escapeHtml(run.job_id) + '">[view job]</a>' : '')
          + '</td>'
        + '<td>' + (run.tokens_used != null ? run.tokens_used : '-') + '</td>'
        + '</tr>';
    }
    html += '</tbody></table></div>';
  }

  detail.innerHTML = html;
}

function triggerRoutine(id) {
  apiFetch('/api/routines/' + id + '/trigger', { method: 'POST' })
    .then(() => {
      showToast(I18n.t('routines.triggered'), 'success');
      if (currentRoutineId === id) openRoutineDetail(id);
      else loadRoutines();
    })
    .catch((err) => showToast(I18n.t('routines.triggerFailed', { message: err.message }), 'error'));
}

function toggleRoutine(id) {
  apiFetch('/api/routines/' + id + '/toggle', { method: 'POST' })
    .then((res) => {
      showToast(I18n.t('routines.toggled', { status: res.status || 'toggled' }), 'success');
      if (currentRoutineId) openRoutineDetail(currentRoutineId);
      else loadRoutines();
    })
    .catch((err) => showToast(I18n.t('routines.toggleFailed', { message: err.message }), 'error'));
}

function deleteRoutine(id, name) {
  if (!confirm(I18n.t('routines.confirmDelete', { name: name }))) return;
  apiFetch('/api/routines/' + id, { method: 'DELETE' })
    .then(() => {
      showToast(I18n.t('routines.deleted'), 'success');
      // Re-check legacy routine count so the v2 user who just deleted
      // their last v1 routine sees the tab fall back to hidden without
      // a page reload (#2982).
      refreshLegacyRoutinesPresence().then(function() {
        applyEngineModeToTabs();
        applyEngineModeUi();
      });
      if (currentRoutineId === id) closeRoutineDetail();
      else loadRoutines();
    })
    .catch((err) => showToast(I18n.t('routines.deleteFailed', { message: err.message }), 'error'));
}

// ── Projects Control Room (engine v2) ─────────────────────
//
// 4-layer control room: attention bar → project cards → drill-in → detail.
// Replaces the legacy missions tab when ENGINE_V2 is enabled.

