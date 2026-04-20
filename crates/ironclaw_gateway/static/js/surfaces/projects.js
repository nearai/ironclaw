let currentMissionId = null;
let crOverview = null; // cached overview response
let crCurrentProjectId = null; // currently drilled-into project

function applyEngineModeToTabs() {
  document.querySelectorAll('.tab-bar [data-v2-only]').forEach(function(el) {
    el.style.display = engineV2Enabled ? '' : 'none';
  });
  document.querySelectorAll('.tab-bar [data-v1-only]').forEach(function(el) {
    el.style.display = engineV2Enabled ? 'none' : '';
  });
  var activeBtn = document.querySelector('.tab-bar button[data-tab].active');
  if (activeBtn && activeBtn.style.display === 'none') switchTab('chat');
  updateTabIndicator();
}

function loadProjectsOverview() {
  apiFetch('/api/engine/projects/overview').then(function(data) {
    crOverview = data;
    renderCrAttention(data.attention || []);
    renderCrCards(data.projects || []);
    // If we were drilled in, stay drilled in (refresh data).
    if (crCurrentProjectId) drillIntoProject(crCurrentProjectId);
  }).catch(function(err) {
    console.error('[projects] Failed to load overview:', err);
    document.getElementById('cr-cards').innerHTML =
      '<div class="cr-empty">Failed to load projects.</div>';
  });
}

function renderCrAttention(items) {
  var el = document.getElementById('cr-attention');
  if (!el) return;
  if (!items.length) { el.style.display = 'none'; return; }
  el.style.display = '';
  el.innerHTML = '<div class="cr-attention-title">Needs attention</div>'
    + items.map(function(a) {
      var icon = a.type === 'gate' ? '<span class="cr-att-icon cr-att-gate">&#x1F511;</span>'
        : '<span class="cr-att-icon cr-att-fail">&#x26A0;</span>';
      return '<button class="cr-att-item" data-action="cr-att-click" data-project="'
        + escapeHtml(a.project_id) + '" data-thread="' + escapeHtml(a.thread_id || '') + '">'
        + icon + '<span class="cr-att-proj">' + escapeHtml(a.project_name) + '</span>'
        + '<span class="cr-att-msg">' + escapeHtml(a.message) + '</span></button>';
    }).join('');
}

function renderCrCards(projects) {
  var el = document.getElementById('cr-cards');
  if (!el) return;

  // Separate default project from user-created projects.
  var defaultProj = projects.find(function(p) { return p.name === 'default'; });
  var userProjects = projects.filter(function(p) { return p.name !== 'default'; });

  var html = '';

  // Default project as a special "General" section.
  if (defaultProj) {
    var dStats = defaultProj.active_missions + ' missions · '
      + defaultProj.threads_today + ' threads today';
    html += '<div class="cr-general">'
      + '<button class="cr-general-card" data-action="cr-drill" data-id="' + escapeHtml(defaultProj.id) + '">'
      + '<div class="cr-general-name">General</div>'
      + '<div class="cr-card-stats">' + escapeHtml(dStats) + '</div>'
      + '</button></div>';
  }

  // User-created project cards.
  if (!userProjects.length && !defaultProj) {
    html += '<div class="cr-empty">No projects yet. Ask the assistant to create one, or use the button below.</div>';
  }
  html += userProjects.map(function(p) {
    var dot = p.health === 'green' ? 'cr-dot-green'
      : p.health === 'yellow' ? 'cr-dot-yellow' : 'cr-dot-red';
    var stats = p.active_missions + ' active · '
      + p.threads_today + ' threads today · $' + (p.cost_today_usd || 0).toFixed(2);
    var lastAct = p.last_activity ? formatRelativeTime(p.last_activity) : 'no activity';
    return '<button class="cr-card" data-action="cr-drill" data-id="' + escapeHtml(p.id) + '">'
      + '<div class="cr-card-head"><span class="cr-dot ' + dot + '"></span>'
      + '<span class="cr-card-name">' + escapeHtml(p.name) + '</span></div>'
      + '<div class="cr-card-stats">' + escapeHtml(stats) + '</div>'
      + '<div class="cr-card-last">Last: ' + escapeHtml(lastAct) + '</div>'
      + '</button>';
  }).join('');

  // "New Project" card.
  html += '<button class="cr-card cr-card-new" data-action="cr-new-project">'
    + '<div class="cr-card-head"><span class="cr-card-name">+ New Project</span></div>'
    + '<div class="cr-card-stats">Create an autonomous workspace</div>'
    + '</button>';

  el.innerHTML = html;
}

function drillIntoProject(projectId) {
  crCurrentProjectId = projectId;
  document.getElementById('cr-cards').style.display = 'none';
  var drill = document.getElementById('cr-drill');
  drill.style.display = '';
  document.getElementById('cr-detail').style.display = 'none';

  // Find project from cached overview.
  var proj = crOverview && crOverview.projects
    ? crOverview.projects.find(function(p) { return p.id === projectId; }) : null;
  var name = proj ? proj.name : 'Project';
  var desc = proj ? proj.description : '';

  document.getElementById('cr-drill-header').innerHTML =
    '<button class="cr-back" data-action="cr-back">&larr; All Projects</button>'
    + '<h2 class="cr-drill-name">' + escapeHtml(name) + '</h2>'
    + (desc ? '<p class="cr-drill-desc">' + escapeHtml(desc) + '</p>' : '');

  // Show goals/metrics if present.
  if (proj && (proj.goals && proj.goals.length || proj.metrics && proj.metrics.length)) {
    var gmHtml = '';
    if (proj.goals && proj.goals.length) {
      gmHtml += '<div class="cr-goals"><div class="cr-section-title">Goals</div>';
      proj.goals.forEach(function(g) {
        gmHtml += '<div class="cr-goal-item">' + escapeHtml(g) + '</div>';
      });
      gmHtml += '</div>';
    }
    // Metrics would come from project detail; overview doesn't include them yet.
    document.getElementById('cr-drill-header').innerHTML += gmHtml;
  }

  // Fetch missions and threads for this project.
  Promise.all([
    apiFetch('/api/engine/missions?project_id=' + encodeURIComponent(projectId)).catch(function(e) { console.error('[projects] missions fetch:', e); return { missions: [] }; }),
    apiFetch('/api/engine/threads?project_id=' + encodeURIComponent(projectId)).catch(function(e) { console.error('[projects] threads fetch:', e); return { threads: [] }; }),
  ]).then(function(res) {
    var missions = res[0].missions || [];
    var threads = res[1].threads || [];
    renderCrDrillMissions(missions);
    renderCrDrillActivity(threads, missions);
  }).catch(function(err) {
    console.error('[projects] Failed to load project details:', err);
  });

  // Load project-scoped widgets into header/section slots.
  loadProjectWidgets(projectId);
}

function crBackToOverview() {
  crCurrentProjectId = null;
  destroyProjectWidgets();
  document.getElementById('cr-drill').style.display = 'none';
  document.getElementById('cr-detail').style.display = 'none';
  document.getElementById('cr-cards').style.display = '';
}

function renderCrDrillMissions(missions) {
  var el = document.getElementById('cr-drill-missions');
  if (!el) return;
  if (!missions.length) {
    el.innerHTML = '<div class="cr-section-title">Missions</div>'
      + '<div class="cr-empty">No missions configured yet.</div>';
    return;
  }
  var html = '<div class="cr-section-title">Missions</div>';
  missions.forEach(function(m) {
    var statusClass = m.status === 'Active' ? 'in_progress'
      : m.status === 'Completed' ? 'completed'
      : m.status === 'Paused' ? 'pending' : 'failed';
    html += '<button class="cr-mission-card" data-action="open-mission" data-id="' + escapeHtml(m.id) + '">'
      + '<div class="cr-mc-head">'
      + '<span class="cr-mc-name">' + escapeHtml(m.name) + '</span>'
      + '<span class="badge ' + statusClass + '">' + escapeHtml(m.status) + '</span></div>'
      + '<div class="cr-mc-sub">'
      + escapeHtml(m.cadence_description || m.cadence_type || 'manual')
      + ' · ' + m.thread_count + ' threads'
      + '</div>'
      + '</button>';
  });
  el.innerHTML = html;
}

function renderCrDrillActivity(threads, missions) {
  var el = document.getElementById('cr-drill-activity');
  if (!el) return;
  if (!threads.length) {
    el.innerHTML = '<div class="cr-section-title">Activity</div>'
      + '<div class="cr-empty">No threads yet.</div>';
    return;
  }
  // Sort by updated_at descending.
  var sorted = threads.slice().sort(function(a, b) {
    return new Date(b.updated_at) - new Date(a.updated_at);
  });
  var html = '<div class="cr-section-title">Recent Activity</div>';
  sorted.slice(0, 30).forEach(function(t) {
    var stateClass = (t.state === 'Done' || t.state === 'Completed') ? 'completed'
      : t.state === 'Failed' ? 'failed'
      : t.state === 'Running' ? 'in_progress' : 'pending';
    var label = t.title || t.goal || ('Thread ' + (t.id || '').slice(0, 8));
    var time = formatRelativeTime(t.updated_at);
    html += '<button class="cr-activity-row" data-action="open-engine-thread" data-id="' + escapeHtml(t.id) + '">'
      + '<span class="badge ' + stateClass + '">' + escapeHtml(t.state) + '</span>'
      + '<span class="cr-act-label">' + escapeHtml(label) + '</span>'
      + '<span class="cr-act-time">' + escapeHtml(time) + '</span>'
      + '</button>';
  });
  el.innerHTML = html;
}

function crShowDetail(html) {
  var detail = document.getElementById('cr-detail');
  detail.style.display = '';
  detail.innerHTML = html;
}

// CR-specific mission detail: renders into the control-room cr-detail panel.
function crOpenMissionDetail(id) {
  currentMissionId = id;
  apiFetch('/api/engine/missions/' + id).then(function(data) {
    renderMissionDetailInCr(data.mission);
  }).catch(function(err) {
    console.error('[projects] Failed to load mission:', err);
    showToast('Failed to load mission: ' + err.message, 'error');
  });
}

function renderMissionDetailInCr(m) {
  var statusClass = m.status === 'Active' ? 'in_progress'
    : m.status === 'Completed' ? 'completed'
    : m.status === 'Paused' ? 'pending' : 'failed';
  var html = '<div class="cr-detail-header">'
    + '<button class="cr-back" data-action="cr-close-detail">&larr; Back</button>'
    + '<h2>' + escapeHtml(m.name) + '</h2>'
    + '<span class="badge ' + statusClass + '">' + escapeHtml(m.status) + '</span></div>';
  html += '<div class="job-description"><h3>Goal</h3>'
    + '<div class="job-description-body">' + renderMarkdown(m.goal) + '</div></div>';
  html += '<div class="job-meta-grid">'
    + metaItem('Cadence', m.cadence_description || m.cadence_type)
    + metaItem('Threads today', m.threads_today + ' / ' + (m.max_threads_per_day || '\u221E'))
    + metaItem('Total threads', m.thread_count)
    + metaItem('Created', formatDate(m.created_at))
    + metaItem('Next fire', m.next_fire_at ? formatDate(m.next_fire_at) : '\u2014')
    + '</div>';
  if (m.current_focus) {
    html += '<div class="job-description"><h3>Current Focus</h3>'
      + '<div class="job-description-body">' + renderMarkdown(m.current_focus) + '</div></div>';
  }
  if (m.approach_history && m.approach_history.length) {
    html += '<div class="job-description"><h3>Approach History</h3>';
    m.approach_history.forEach(function(a, i) {
      html += '<div class="job-description-body" style="margin-bottom:8px">'
        + '<strong>Run ' + (i + 1) + '</strong><br>' + renderMarkdown(a) + '</div>';
    });
    html += '</div>';
  }
  // Action buttons.
  html += '<div style="margin-top:16px;">';
  if (m.status === 'Active') html += '<button class="btn-cancel" data-action="pause-mission" data-id="' + escapeHtml(m.id) + '">Pause</button> ';
  if (m.status === 'Paused') html += '<button class="btn-restart" data-action="resume-mission" data-id="' + escapeHtml(m.id) + '">Resume</button> ';
  html += '<button class="btn-restart" data-action="fire-mission" data-id="' + escapeHtml(m.id) + '">Fire now</button>';
  html += '</div>';
  // Spawned threads.
  if (m.threads && m.threads.length) {
    html += '<div class="job-description"><h3>Spawned Threads</h3>';
    m.threads.forEach(function(t) {
      var tState = (t.state === 'Done' || t.state === 'Completed') ? 'completed'
        : t.state === 'Failed' ? 'failed' : t.state === 'Running' ? 'in_progress' : 'pending';
      html += '<button class="cr-activity-row" data-action="open-engine-thread" data-id="' + escapeHtml(t.id) + '">'
        + '<span class="badge ' + tState + '">' + escapeHtml(t.state) + '</span>'
        + '<span class="cr-act-label">' + escapeHtml(t.goal) + '</span>'
        + '<span class="cr-act-time">' + escapeHtml(formatDate(t.created_at)) + '</span></button>';
    });
    html += '</div>';
  }
  crShowDetail(html);
}

function crOpenEngineThread(threadId) {
  apiFetch('/api/engine/threads/' + threadId).then(function(data) {
    var t = data.thread;
    var stateClass = (t.state === 'Done' || t.state === 'Completed') ? 'completed'
      : t.state === 'Failed' ? 'failed' : t.state === 'Running' ? 'in_progress' : 'pending';
    var html = '<div class="cr-detail-header">'
      + '<button class="cr-back" data-action="cr-close-detail">&larr; Back</button>'
      + '<h2>Thread: ' + escapeHtml(t.goal) + '</h2>'
      + '<span class="badge ' + stateClass + '">' + escapeHtml(t.state) + '</span></div>';
    html += '<div class="job-meta-grid">'
      + metaItem('Type', t.thread_type) + metaItem('Steps', t.step_count)
      + metaItem('Tokens', t.total_tokens.toLocaleString())
      + metaItem('Cost', t.total_cost_usd > 0 ? '$' + t.total_cost_usd.toFixed(4) : '\u2014')
      + metaItem('Created', formatDate(t.created_at))
      + metaItem('Completed', t.completed_at ? formatDate(t.completed_at) : '\u2014')
      + '</div>';
    if (t.messages && t.messages.length) {
      html += '<div class="job-description"><h3>Messages (' + t.messages.length + ')</h3>';
      t.messages.forEach(function(msg) {
        var roleClass = msg.role === 'Assistant' ? 'assistant' : msg.role === 'User' ? 'user' : 'system';
        html += '<div class="thread-message thread-msg-' + roleClass + '">'
          + '<div class="thread-msg-role">' + escapeHtml(msg.role) + '</div>'
          + '<div class="thread-msg-content">' + renderMarkdown(msg.content) + '</div></div>';
      });
      html += '</div>';
    }
    crShowDetail(html);
  }).catch(function(err) {
    console.error('[projects] Failed to load thread:', err);
    showToast('Failed to load thread: ' + err.message, 'error');
  });
}

// ── Project-scoped widgets ─────────────────────────────────
// Loaded dynamically on drill-in, destroyed on back/tab-switch.

var _projectWidgets = []; // { id, destroy }

function loadProjectWidgets(projectId) {
  destroyProjectWidgets();
  apiFetch('/api/engine/projects/' + encodeURIComponent(projectId) + '/widgets')
    .then(function(widgets) {
      if (!Array.isArray(widgets) || !widgets.length) return;
      widgets.forEach(function(w) {
        var manifest = w.manifest;
        var slot = manifest.slot;
        var parentId = slot === 'project_header' ? 'cr-widget-header' : 'cr-widget-sections';
        var parent = document.getElementById(parentId);
        if (!parent) return;

        // Create scoped container.
        var container = document.createElement('div');
        container.setAttribute('data-widget', manifest.id);
        container.setAttribute('data-project-widget', 'true');
        parent.appendChild(container);

        // Inject scoped CSS if present (already scoped server-side via scope_css).
        var style = null;
        if (w.css) {
          style = document.createElement('style');
          style.setAttribute('data-widget', manifest.id);
          style.textContent = w.css;
          document.head.appendChild(style);
        }

        // Eval the JS module to register the widget.
        try {
          var api = typeof IronClaw !== 'undefined' ? IronClaw.api : null;
          var fn = new Function('container', 'api', 'projectId', w.js);
          fn(container, api, projectId);

          _projectWidgets.push({
            id: manifest.id,
            container: container,
            style: style || null,
            destroy: function() {
              container.remove();
              if (style) style.remove();
            }
          });
        } catch (err) {
          console.error('[projects] Failed to mount widget ' + manifest.id + ':', err);
          container.innerHTML = '<div class="cr-empty">Widget error: ' + manifest.id + '</div>';
        }
      });
    })
    .catch(function(err) {
      console.error('[projects] Failed to load project widgets:', err);
    });
}

function destroyProjectWidgets() {
  _projectWidgets.forEach(function(w) {
    try { w.destroy(); } catch (e) { /* ignore */ }
  });
  _projectWidgets = [];
  var header = document.getElementById('cr-widget-header');
  if (header) header.innerHTML = '';
  var sections = document.getElementById('cr-widget-sections');
  if (sections) sections.innerHTML = '';
}

function crNewProject() {
  // Switch to chat tab and pre-fill with a project creation prompt.
  switchTab('chat');
  var input = document.getElementById('chat-input');
  if (input) {
    input.value = 'Create a new project for me. I want to set up an autonomous workspace for: ';
    input.focus();
    autoGrow(input);
  }
}

function enrichMissionProgress(missions) {
  const activeMissions = (missions || []).filter((mission) => mission.status === 'Active');
  activeMissions.forEach((mission) => {
    const cachedMission = missionDetailCache.get(mission.id);
    if (cachedMission) {
      activeWorkStore.rememberMissionThreads(cachedMission);
    }
    fetchMissionDetailForProgress(mission.id, { force: true });
  });
}

function renderMissionProgressMarkup(progress) {
  return progress
    ? '<span class="mission-progress-live">' + escapeHtml(progress) + '</span>'
    : '<span class="mission-progress-idle">Idle</span>';
}

function renderMissionProgressCell(missionId) {
  return '<span data-mission-progress-id="' + escapeHtml(missionId) + '">'
    + renderMissionProgressMarkup(activeWorkStore.getMissionProgress(missionId))
    + '</span>';
}

function renderMissionThreadProgress(threadId) {
  return '<span data-thread-progress-id="' + escapeHtml(threadId) + '">'
    + renderMissionProgressMarkup(activeWorkStore.getThreadProgress(threadId))
    + '</span>';
}

function missionThreadIds(mission) {
  if (!mission || !Array.isArray(mission.threads)) return [];
  return mission.threads.map((thread) => thread.id).filter(Boolean).sort();
}

function haveMissionThreadsChanged(previousMission, nextMission) {
  const previousIds = missionThreadIds(previousMission);
  const nextIds = missionThreadIds(nextMission);
  if (previousIds.length !== nextIds.length) return true;
  for (let i = 0; i < previousIds.length; i += 1) {
    if (previousIds[i] !== nextIds[i]) return true;
  }
  return false;
}

function applyMissionDetailUpdate(mission) {
  if (!mission || !mission.id) return;
  const previousMission = missionDetailCache.get(mission.id) || null;
  missionDetailCache.set(mission.id, mission);
  activeWorkStore.rememberMissions([mission]);
  activeWorkStore.rememberMissionThreads(mission);

  if (currentMissionData && currentMissionData.id === mission.id) {
    const shouldRerenderDetail = haveMissionThreadsChanged(currentMissionData, mission);
    currentMissionData = mission;
    if (currentTab === 'missions' && !currentEngineThreadDetail && shouldRerenderDetail) {
      renderMissionDetail(currentMissionData);
      return;
    }
  }

  let missionListChanged = false;
  if (currentMissionList.length > 0) {
    currentMissionList = currentMissionList.map((entry) => {
      if (!entry || entry.id !== mission.id) return entry;
      const updatedEntry = {
        ...entry,
        status: mission.status,
        thread_count: mission.thread_count,
        current_focus: mission.current_focus,
        next_fire_at: mission.next_fire_at,
      };
      if (
        updatedEntry.status !== entry.status
        || updatedEntry.thread_count !== entry.thread_count
        || updatedEntry.current_focus !== entry.current_focus
        || updatedEntry.next_fire_at !== entry.next_fire_at
      ) {
        missionListChanged = true;
      }
      return updatedEntry;
    });
  }

  if (currentTab === 'missions' && !currentMissionData && !currentEngineThreadDetail && missionListChanged) {
    renderMissionsList(currentMissionList);
    return;
  }

  if (previousMission && haveMissionThreadsChanged(previousMission, mission)) {
    scheduleMissionProgressViewsRefresh();
  }
}

function fetchMissionDetailForProgress(missionId, options = {}) {
  if (!missionId) return Promise.resolve(null);
  if (missionDetailFetchInFlight.has(missionId)) {
    if (options.force) {
      missionMappingsLastRefreshedAt = Date.now();
    }
    return Promise.resolve(null);
  }
  missionDetailFetchInFlight.add(missionId);
  return apiFetch('/api/engine/missions/' + missionId)
    .then((data) => {
      if (!data || !data.mission) return null;
      applyMissionDetailUpdate(data.mission);
      return data.mission;
    })
    .catch(() => null)
    .finally(() => {
      missionDetailFetchInFlight.delete(missionId);
      if (options.force) {
        missionMappingsLastRefreshedAt = Date.now();
      }
    });
}

function refreshPersistentActivityBar() {
  if (activityBarSnapshotInFlight) return;
  activityBarSnapshotInFlight = true;
  Promise.all([
    apiFetch('/api/jobs').catch(() => null),
    engineV2Enabled ? apiFetch('/api/engine/missions').catch(() => null) : Promise.resolve(null),
  ]).then(([jobList, missionList]) => {
    if (jobList && Array.isArray(jobList.jobs)) {
      activeWorkStore.rememberJobs(jobList.jobs);
    }
    if (missionList && Array.isArray(missionList.missions)) {
      activeWorkStore.rememberMissions(missionList.missions);
      missionList.missions
        .filter((mission) => mission && mission.id && mission.status === 'Active')
        .forEach((mission) => {
          fetchMissionDetailForProgress(mission.id, { force: true });
        });
    }
  }).finally(() => {
    activityBarSnapshotInFlight = false;
  });
}

function getTrackedActiveMissionIds() {
  return activeWorkStore.getActiveMissionIds();
}

function scheduleActiveMissionMappingRefresh() {
  const missionIds = getTrackedActiveMissionIds();
  if (missionIds.length === 0 || missionMappingRefreshTimer) return;
  const now = Date.now();
  const refreshDelay = Math.max(0, ACTIVE_MISSION_MAPPING_REFRESH_MS - (now - missionMappingsLastRefreshedAt));
  missionMappingRefreshTimer = window.setTimeout(() => {
    missionMappingRefreshTimer = null;
    missionIds.forEach((missionId) => {
      fetchMissionDetailForProgress(missionId, { force: true });
    });
  }, refreshDelay);
}

function scheduleMissionProgressViewsRefresh() {
  if (missionProgressRefreshScheduled) return;
  missionProgressRefreshScheduled = true;
  window.requestAnimationFrame(() => {
    missionProgressRefreshScheduled = false;
    refreshMissionProgressViews();
  });
}

function refreshMissionProgressViews() {
  document.querySelectorAll('[data-mission-progress-id]').forEach((node) => {
    node.innerHTML = renderMissionProgressMarkup(activeWorkStore.getMissionProgress(node.dataset.missionProgressId));
  });
  document.querySelectorAll('[data-thread-progress-id]').forEach((node) => {
    node.innerHTML = renderMissionProgressMarkup(activeWorkStore.getThreadProgress(node.dataset.threadProgressId));
  });
  document.querySelectorAll('[data-thread-progress-block-id]').forEach((block) => {
    const progress = activeWorkStore.getThreadProgress(block.dataset.threadProgressBlockId);
    const body = block.querySelector('[data-thread-progress-text-id]');
    block.hidden = !progress;
    if (body) body.textContent = progress || '';
  });
  scheduleActiveMissionMappingRefresh();
}

function loadMissions() {
  currentMissionId = null;
  currentMissionData = null;
  currentEngineThreadDetail = null;
  const detail = document.getElementById('mission-detail');
  if (detail) detail.style.display = 'none';
  const table = document.getElementById('missions-table');
  if (table) table.style.display = '';

  Promise.all([
    apiFetch('/api/engine/missions/summary'),
    apiFetch('/api/engine/missions'),
  ]).then(([summary, listData]) => {
    currentMissionList = listData.missions || [];
    activeWorkStore.rememberMissions(currentMissionList);
    renderMissionsSummary(summary);
    renderMissionsList(currentMissionList);
    enrichMissionProgress(currentMissionList);
  }).catch(() => {});
}

function renderMissionsSummary(s) {
  document.getElementById('missions-summary').innerHTML = ''
    + summaryCard(I18n.t('missions.summary.total'), s.total, '')
    + summaryCard(I18n.t('missions.summary.active'), s.active, 'active')
    + summaryCard(I18n.t('missions.summary.paused'), s.paused, '')
    + summaryCard(I18n.t('missions.summary.completed'), s.completed, 'completed')
    + summaryCard(I18n.t('missions.summary.failed'), s.failed, 'failed');
}

function renderMissionsList(missions) {
  const tbody = document.getElementById('missions-tbody');
  const empty = document.getElementById('missions-empty');

  if (!missions || missions.length === 0) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = missions.map((m) => {
    const statusClass = m.status === 'Active' ? 'in_progress'
      : m.status === 'Completed' ? 'completed'
      : m.status === 'Paused' ? 'pending'
      : 'failed';

    return '<tr class="mission-row" data-action="open-mission" data-id="' + escapeHtml(m.id) + '">'
      + '<td>' + escapeHtml(m.name) + '</td>'
      + '<td class="truncate">' + escapeHtml(m.goal) + '</td>'
      + '<td>' + escapeHtml(m.cadence_description || m.cadence_type) + '</td>'
      + '<td>' + m.thread_count + '</td>'
      + '<td><span class="badge ' + statusClass + '">' + escapeHtml(m.status) + '</span></td>'
      + '<td>' + renderMissionProgressCell(m.id) + '</td>'
      + '<td>'
      + (m.status === 'Active' ? '<button class="btn-cancel" data-action="pause-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.pause')) + '</button> ' : '')
      + (m.status === 'Paused' ? '<button class="btn-restart" data-action="resume-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.resume')) + '</button> ' : '')
      + '<button class="btn-restart" data-action="fire-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.fire')) + '</button>'
      + '</td>'
      + '</tr>';
  }).join('');
}

function openMissionDetail(id) {
  currentMissionId = id;
  apiFetch('/api/engine/missions/' + id).then((data) => {
    currentEngineThreadDetail = null;
    currentMissionData = data.mission;
    applyMissionDetailUpdate(data.mission);
    // Route to control room or standalone detail depending on active tab.
    if (currentTab === 'projects') {
      renderMissionDetailInCr(data.mission);
    } else {
      renderMissionDetail(currentMissionData);
    }
  }).catch((err) => {
    showToast(I18n.t('missions.loadFailed', { message: err.message }), 'error');
  });
}

function closeMissionDetail() {
  currentMissionId = null;
  currentMissionData = null;
  currentEngineThreadDetail = null;
  loadMissions();
}

function renderMissionDetail(m) {
  const table = document.getElementById('missions-table');
  if (table) table.style.display = 'none';
  document.getElementById('missions-empty').style.display = 'none';

  const detail = document.getElementById('mission-detail');
  detail.style.display = 'block';

  const statusClass = m.status === 'Active' ? 'in_progress'
    : m.status === 'Completed' ? 'completed'
    : m.status === 'Paused' ? 'pending'
    : 'failed';

  let html = '<div class="job-detail-header">'
    + '<button class="btn-back" data-action="close-mission-detail">' + escapeHtml(I18n.t('common.back')) + '</button>'
    + '<h2>' + escapeHtml(m.name) + '</h2>'
    + '<span class="badge ' + statusClass + '">' + escapeHtml(m.status) + '</span>'
    + '</div>';

  // Goal — full-width markdown block
  html += '<div class="job-description"><h3>Goal</h3>'
    + '<div class="job-description-body">' + renderMarkdown(m.goal) + '</div></div>';

  html += '<div class="job-meta-grid">'
    + metaItem(I18n.t('missions.cadence'), m.cadence_description || m.cadence_type)
    + metaItem(I18n.t('missions.status'), m.status)
    + metaItem(I18n.t('missions.threadsToday'), m.threads_today + ' / ' + (m.max_threads_per_day || '\u221E'))
    + metaItem(I18n.t('missions.totalThreads'), m.thread_count)
    + metaItem(I18n.t('missions.created'), formatDate(m.created_at))
    + metaItem(I18n.t('missions.nextFire'), m.next_fire_at ? formatDate(m.next_fire_at) : I18n.t('common.noData'))
    + '</div>';

  if (m.current_focus) {
    html += '<div class="job-description"><h3>Current Focus</h3>'
      + '<div class="job-description-body">' + renderMarkdown(m.current_focus) + '</div></div>';
  }

  if (m.success_criteria) {
    html += '<div class="job-description"><h3>Success Criteria</h3>'
      + '<div class="job-description-body">' + renderMarkdown(m.success_criteria) + '</div></div>';
  }

  if (m.notify_channels && m.notify_channels.length > 0) {
    html += '<div class="job-description"><h3>Notify Channels</h3>'
      + '<div class="job-description-body">' + m.notify_channels.map(escapeHtml).join(', ') + '</div></div>';
  }

  if (m.approach_history && m.approach_history.length > 0) {
    html += '<div class="job-description"><h3>Approach History</h3>';
    m.approach_history.forEach((a, i) => {
      html += '<div class="job-description-body" style="margin-bottom:8px">'
        + '<strong>Run ' + (i + 1) + '</strong><br>'
        + renderMarkdown(a) + '</div>';
    });
    html += '</div>';
  }

  if (m.threads && m.threads.length > 0) {
    html += '<div class="job-description"><h3>Spawned Threads</h3>'
      + '<table class="missions-table"><thead><tr>'
      + '<th>Goal</th><th>Type</th><th>State</th><th>' + escapeHtml(I18n.t('missions.progress')) + '</th><th>Steps</th><th>Tokens</th><th>Created</th>'
      + '</tr></thead><tbody>';
    m.threads.forEach((t) => {
      var tState = t.state === 'Done' || t.state === 'Completed' ? 'completed'
        : t.state === 'Failed' ? 'failed'
        : t.state === 'Running' ? 'in_progress'
        : 'pending';
      html += '<tr class="mission-row" data-action="open-engine-thread" data-id="' + escapeHtml(t.id) + '">'
        + '<td class="truncate">' + escapeHtml(t.goal) + '</td>'
        + '<td>' + escapeHtml(t.thread_type) + '</td>'
        + '<td><span class="badge ' + tState + '">' + escapeHtml(t.state) + '</span></td>'
        + '<td>' + renderMissionThreadProgress(t.id) + '</td>'
        + '<td>' + t.step_count + '</td>'
        + '<td>' + t.total_tokens.toLocaleString() + '</td>'
        + '<td>' + formatDate(t.created_at) + '</td>'
        + '</tr>';
    });
    html += '</tbody></table></div>';
  }

  // Action buttons
  html += '<div style="margin-top:16px;">';
  if (m.status === 'Active') {
    html += '<button class="btn-cancel" data-action="pause-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.pause')) + '</button> ';
  }
  if (m.status === 'Paused') {
    html += '<button class="btn-restart" data-action="resume-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.resume')) + '</button> ';
  }
  html += '<button class="btn-restart" data-action="fire-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.fireNow')) + '</button>';
  html += '</div>';

  detail.innerHTML = html;
}

function renderEngineThreadDetail(t) {
  var detail = document.getElementById('mission-detail');

  var stateClass = t.state === 'Done' || t.state === 'Completed' ? 'completed'
    : t.state === 'Failed' ? 'failed'
    : t.state === 'Running' ? 'in_progress'
    : 'pending';
  var progress = activeWorkStore.getThreadProgress(t.id);

  var html = '<div class="job-detail-header">'
    + '<button class="btn-back" data-action="back-to-mission">' + escapeHtml(I18n.t('missions.backToMission')) + '</button>'
    + '<h2>Thread: ' + escapeHtml(t.goal) + '</h2>'
    + '<span class="badge ' + stateClass + '">' + escapeHtml(t.state) + '</span>'
    + '</div>';

  html += '<div class="job-description mission-thread-progress" data-thread-progress-block-id="' + escapeHtml(t.id) + '"'
    + (progress ? '' : ' hidden')
    + '><h3>Current Progress</h3>'
    + '<div class="job-description-body" data-thread-progress-text-id="' + escapeHtml(t.id) + '">' + escapeHtml(progress || '') + '</div></div>';

  html += '<div class="job-meta-grid">'
    + metaItem(I18n.t('missions.threadId'), t.id)
    + metaItem(I18n.t('missions.type'), t.thread_type)
    + metaItem(I18n.t('missions.steps'), t.step_count)
    + metaItem(I18n.t('missions.tokens'), t.total_tokens.toLocaleString())
    + metaItem(I18n.t('missions.cost'), t.total_cost_usd > 0 ? '$' + t.total_cost_usd.toFixed(4) : '-')
    + metaItem(I18n.t('missions.maxIterations'), t.max_iterations)
    + metaItem(I18n.t('missions.created'), formatDate(t.created_at))
    + metaItem(I18n.t('jobs.completedLabel'), t.completed_at ? formatDate(t.completed_at) : '-')
    + '</div>';

  if (t.messages && t.messages.length > 0) {
    html += '<div class="job-description"><h3>Messages (' + t.messages.length + ')</h3>';
    t.messages.forEach(function(msg) {
      var roleClass = msg.role === 'Assistant' ? 'assistant' : msg.role === 'User' ? 'user' : 'system';
      html += '<div class="thread-message thread-msg-' + roleClass + '">'
        + '<div class="thread-msg-role">' + escapeHtml(msg.role) + '</div>'
        + '<div class="thread-msg-content">' + renderMarkdown(msg.content) + '</div>'
        + '</div>';
    });
    html += '</div>';
  }

  detail.innerHTML = html;
}

function openEngineThread(threadId) {
  // Route to control room or standalone detail depending on active tab.
  if (currentTab === 'projects') {
    crOpenEngineThread(threadId);
    return;
  }
  apiFetch('/api/engine/threads/' + threadId).then((data) => {
    currentEngineThreadDetail = data.thread;
    renderEngineThreadDetail(currentEngineThreadDetail);
  }).catch(function(err) {
    showToast(I18n.t('missions.threadLoadFailed', { message: err.message }), 'error');
  });
}

function refreshMissionView(missionId) {
  // Refresh the currently visible mission context.
  if (currentMissionId === missionId) {
    openMissionDetail(missionId);
  } else if (crCurrentProjectId) {
    drillIntoProject(crCurrentProjectId);
  }
}

function fireMission(id) {
  apiFetch('/api/engine/missions/' + id + '/fire', { method: 'POST' })
    .then(function(data) {
      if (data.fired) {
        showToast(I18n.t('missions.fired', { id: data.thread_id }), 'success');
      } else {
        showToast(I18n.t('missions.notFired'), 'warning');
      }
      refreshMissionView(id);
    })
    .catch(function(err) { showToast(I18n.t('missions.fireFailed', { message: err.message }), 'error'); });
}

function pauseMission(id) {
  apiFetch('/api/engine/missions/' + id + '/pause', { method: 'POST' })
    .then(function() {
      showToast(I18n.t('missions.paused'), 'success');
      refreshMissionView(id);
    })
    .catch(function(err) { showToast(I18n.t('missions.pauseFailed', { message: err.message }), 'error'); });
}

function resumeMission(id) {
  apiFetch('/api/engine/missions/' + id + '/resume', { method: 'POST' })
    .then(function() {
      showToast(I18n.t('missions.resumed'), 'success');
      refreshMissionView(id);
    })
    .catch(function(err) { showToast(I18n.t('missions.resumeFailed', { message: err.message }), 'error'); });
}

function formatRelativeTime(isoString) {
  if (!isoString) return '-';
  const d = new Date(isoString);
  const now = Date.now();
  const diffMs = now - d.getTime();
  const absDiff = Math.abs(diffMs);
  const future = diffMs < 0;

  if (absDiff < 60000)
    return future ? I18n.t('time.lessThan1MinuteFromNow') : I18n.t('time.lessThan1MinuteAgo');
  if (absDiff < 3600000) {
    const m = Math.floor(absDiff / 60000);
    return future ? I18n.t('time.minutesFromNow', { n: m }) : I18n.t('time.minutesAgo', { n: m });
  }
  if (absDiff < 86400000) {
    const h = Math.floor(absDiff / 3600000);
    return future ? I18n.t('time.hoursFromNow', { n: h }) : I18n.t('time.hoursAgo', { n: h });
  }
  const days = Math.floor(absDiff / 86400000);
  return future ? I18n.t('time.daysFromNow', { n: days }) : I18n.t('time.daysAgo', { n: days });
}

// --- Users (admin) ---


// ─────────────────────────────────────────────────────────────────────
// Project UI — active-project selector, conversation chrome, "!" shell
// mode rendering. Kept at the tail of app.js so it can reference
// already-defined helpers (apiFetch, showToast, I18n, escapeHtml,
// currentThreadId, addTrackedEventListener, ...) without forward
// declarations. Surface is intentionally small — one public object on
// `window.ProjectUI` so future widget-layout hooks can call into it.
// ─────────────────────────────────────────────────────────────────────
(function () {
  const state = {
    projects: [],
    activeProjectId: null,
    currentThreadProject: null,
    loaded: false,
  };

  function t(key, fallback, params) {
    if (typeof I18n !== 'undefined' && typeof I18n.t === 'function') {
      try {
        const s = I18n.t(key, params || {});
        if (s && s !== key) return s;
      } catch (_err) { /* fall through to fallback */ }
    }
    if (fallback && params) {
      return Object.keys(params).reduce(
        (out, k) => out.replace('{' + k + '}', params[k]),
        fallback,
      );
    }
    return fallback || key;
  }

  function contractHome(path) {
    if (!path) return '';
    // Purely cosmetic — we don't know the user's real $HOME from the
    // browser. Strip the common ~/.ironclaw prefix so the chrome shows
    // a short label for default-folder projects.
    return path.replace(/^\/home\/[^/]+\//, '~/').replace(/^\/Users\/[^/]+\//, '~/');
  }

  // The engine creates a per-user fallback project with backend name
  // "default" — the Projects overview has long rendered it as "General".
  // Keep the stable backend identifier but present one consistent label
  // in every UI surface (chrome bar, manage modal, anywhere else the
  // name is user-visible) so people aren't confused by seeing both.
  const DEFAULT_PROJECT_BACKEND_NAME = 'default';
  const DEFAULT_PROJECT_DISPLAY_NAME = 'General';
  function displayProjectName(name) {
    return name === DEFAULT_PROJECT_BACKEND_NAME ? DEFAULT_PROJECT_DISPLAY_NAME : (name || '');
  }

  function ensureChrome() {
    let bar = document.getElementById('project-chrome');
    if (bar) return bar;
    const tabChat = document.getElementById('tab-chat');
    if (!tabChat) return null;
    const chatContainer = tabChat.querySelector('.chat-container');
    if (!chatContainer) return null;
    bar = document.createElement('div');
    bar.id = 'project-chrome';
    bar.className = 'project-chrome';
    bar.hidden = true;
    bar.innerHTML = ''
      + '<div class="project-chrome-inner">'
      + '  <button class="project-chrome-button" id="project-chrome-button" type="button">'
      + '    <span class="project-chrome-icon">📁</span>'
      + '    <span class="project-chrome-name" id="project-chrome-name"></span>'
      + '    <span class="project-chrome-caret">▾</span>'
      + '  </button>'
      + '  <span class="project-chrome-folder" id="project-chrome-folder"></span>'
      + '  <span class="project-chrome-branch" id="project-chrome-branch" hidden></span>'
      + '  <a class="project-chrome-pr" id="project-chrome-pr" target="_blank" rel="noopener" hidden></a>'
      + '  <span class="project-chrome-empty" id="project-chrome-empty" hidden></span>'
      + '</div>';
    chatContainer.insertBefore(bar, chatContainer.firstChild);
    bar.querySelector('#project-chrome-button').addEventListener('click', openProjectModal);
    return bar;
  }

  function refreshChromeFromThread(threadProject) {
    const bar = ensureChrome();
    if (!bar) return;
    state.currentThreadProject = threadProject || null;
    const empty = bar.querySelector('#project-chrome-empty');
    if (!threadProject) {
      bar.hidden = false;
      bar.classList.add('project-chrome-empty-state');
      bar.querySelector('#project-chrome-name').textContent = t(
        'project.chrome.none',
        'No project',
      );
      bar.querySelector('#project-chrome-folder').textContent = '';
      const branchEl = bar.querySelector('#project-chrome-branch');
      branchEl.hidden = true;
      const prEl = bar.querySelector('#project-chrome-pr');
      prEl.hidden = true;
      empty.hidden = false;
      empty.textContent = t(
        'project.chrome.noneHint',
        '→ Manage projects to create or select one',
      );
      return;
    }
    bar.hidden = false;
    bar.classList.remove('project-chrome-empty-state');
    empty.hidden = true;
    bar.querySelector('#project-chrome-name').textContent = displayProjectName(threadProject.name);
    const folderEl = bar.querySelector('#project-chrome-folder');
    folderEl.textContent = contractHome(threadProject.workspace_path || '');
    folderEl.title = threadProject.workspace_path || '';
    const branchEl = bar.querySelector('#project-chrome-branch');
    if (threadProject.branch) {
      branchEl.hidden = false;
      branchEl.textContent = (threadProject.dirty ? '● ' : '') + threadProject.branch;
      branchEl.className =
        'project-chrome-branch' + (threadProject.dirty ? ' dirty' : '');
      branchEl.title = threadProject.dirty_summary || '';
    } else {
      branchEl.hidden = true;
    }
    const prEl = bar.querySelector('#project-chrome-pr');
    if (threadProject.pr) {
      prEl.hidden = false;
      prEl.textContent = '#' + threadProject.pr.number + ' ' + (threadProject.pr.state || 'open');
      prEl.href = threadProject.pr.url || '#';
      prEl.title = threadProject.pr.title || '';
    } else {
      prEl.hidden = true;
    }
  }

  async function fetchProjectList() {
    try {
      const data = await apiFetch('/api/engine/projects');
      state.projects = (data && data.projects) || [];
      return state.projects;
    } catch (e) {
      showToast(t('project.loadFailed', 'Failed to load projects'), 'error');
      return [];
    }
  }

  async function fetchActive() {
    try {
      const data = await apiFetch('/api/engine/projects/active');
      state.activeProjectId = (data && data.project_id) || null;
      return data;
    } catch (e) {
      state.activeProjectId = null;
      return null;
    }
  }

  async function loadProjectsIfNeeded() {
    if (state.loaded) return;
    await Promise.all([fetchProjectList(), fetchActive()]);
    state.loaded = true;
  }

  async function setActive(projectId) {
    try {
      await apiFetch('/api/engine/projects/active', {
        method: 'POST',
        body: { project_id: projectId },
      });
      state.activeProjectId = projectId;
      showToast(t('project.activeSet', 'Active project updated'), 'info');
      // Refresh the chrome for the current thread so inherited chrome
      // reflects the new active project immediately.
      refreshCurrentThread();
    } catch (e) {
      showToast(
        t('project.setActiveFailed', 'Failed to set active project: {message}', {
          message: e.message || '',
        }),
        'error',
      );
    }
  }

  async function assignToCurrentThread(projectId) {
    if (!currentThreadId) return;
    try {
      await apiFetch('/api/chat/threads/' + currentThreadId + '/project', {
        method: 'POST',
        body: projectId ? { project_id: projectId } : {},
      });
      refreshCurrentThread();
    } catch (e) {
      showToast(
        t('project.assignFailed', 'Failed to assign project: {message}', {
          message: e.message || '',
        }),
        'error',
      );
    }
  }

  async function createProject(fields) {
    try {
      const data = await apiFetch('/api/engine/projects', {
        method: 'POST',
        body: fields,
      });
      await fetchProjectList();
      return data && data.project;
    } catch (e) {
      showToast(
        t('project.createFailed', 'Failed to create project: {message}', {
          message: e.message || '',
        }),
        'error',
      );
      throw e;
    }
  }

  async function updateProject(id, fields) {
    try {
      const data = await apiFetch('/api/engine/projects/' + encodeURIComponent(id), {
        method: 'PATCH',
        body: fields,
      });
      await fetchProjectList();
      return data && data.project;
    } catch (e) {
      showToast(
        t('project.updateFailed', 'Failed to update project: {message}', {
          message: e.message || '',
        }),
        'error',
      );
      throw e;
    }
  }

  function refreshCurrentThread() {
    // Fetch history for the current thread; its ThreadInfo.project is
    // already enriched by the backend. We don't need to round-trip —
    // the history endpoint is the cheapest thing that returns it.
    if (!currentThreadId) {
      refreshChromeFromThread(null);
      return;
    }
    apiFetch('/api/chat/threads')
      .then((data) => {
        const all = []
          .concat(data.assistant_thread ? [data.assistant_thread] : [])
          .concat(data.threads || []);
        const match = all.find((th) => th && th.id === currentThreadId);
        refreshChromeFromThread((match && match.project) || null);
      })
      .catch(() => { /* chrome stays in its last state */ });
  }

  function openProjectModal() {
    let modal = document.getElementById('project-modal');
    if (modal) {
      modal.hidden = false;
      renderModal();
      return;
    }
    modal = document.createElement('div');
    modal.id = 'project-modal';
    modal.className = 'project-modal';
    modal.innerHTML = ''
      + '<div class="project-modal-backdrop" id="project-modal-backdrop"></div>'
      + '<div class="project-modal-dialog" role="dialog" aria-modal="true">'
      + '  <div class="project-modal-header">'
      + '    <h3>' + escapeHtml(t('project.modal.title', 'Projects')) + '</h3>'
      + '    <button class="project-modal-close" id="project-modal-close" aria-label="Close">✕</button>'
      + '  </div>'
      + '  <div class="project-modal-body" id="project-modal-body"></div>'
      + '</div>';
    document.body.appendChild(modal);
    modal.querySelector('#project-modal-close').addEventListener('click', closeProjectModal);
    modal.querySelector('#project-modal-backdrop').addEventListener('click', closeProjectModal);
    renderModal();
  }

  function closeProjectModal() {
    const modal = document.getElementById('project-modal');
    if (modal) modal.hidden = true;
  }

  function renderModal() {
    const body = document.getElementById('project-modal-body');
    if (!body) return;
    loadProjectsIfNeeded().then(() => {
      body.innerHTML = '';
      const list = document.createElement('div');
      list.className = 'project-modal-list';
      if (state.projects.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'project-modal-empty';
        empty.textContent = t(
          'project.modal.empty',
          'No projects yet — create one below.',
        );
        list.appendChild(empty);
      } else {
        for (const p of state.projects) {
          list.appendChild(renderProjectRow(p));
        }
      }
      body.appendChild(list);
      body.appendChild(renderCreateForm());
    });
  }

  function renderProjectRow(p) {
    const row = document.createElement('div');
    row.className =
      'project-modal-row' + (p.id === state.activeProjectId ? ' active' : '');
    const repo = (p.metadata && p.metadata.github_repo) || '';
    const defBranch = (p.metadata && p.metadata.default_branch) || '';
    row.innerHTML = ''
      + '<div class="project-modal-row-main">'
      + '  <div class="project-modal-row-name">' + escapeHtml(displayProjectName(p.name)) + '</div>'
      + '  <div class="project-modal-row-meta">'
      +      (p.workspace_path
                ? '<code>' + escapeHtml(contractHome(p.workspace_path)) + '</code>'
                : '')
      +      (repo ? ' · <span class="gh">' + escapeHtml(repo) + '</span>' : '')
      +      (defBranch ? ' · ' + escapeHtml(defBranch) : '')
      + '  </div>'
      + '</div>'
      + '<div class="project-modal-row-actions">'
      + '  <button data-action="activate" ' + (p.id === state.activeProjectId ? 'disabled' : '') + '>'
      +      escapeHtml(p.id === state.activeProjectId
                         ? t('project.modal.active', 'Active')
                         : t('project.modal.setActive', 'Set active'))
      + '  </button>'
      + '  <button data-action="assign-thread">'
      +      escapeHtml(t('project.modal.assignThread', 'Use in this thread'))
      + '  </button>'
      + '  <button data-action="edit">'
      +      escapeHtml(t('project.modal.edit', 'Edit'))
      + '  </button>'
      + '</div>';
    row.querySelector('[data-action="activate"]').addEventListener('click', async () => {
      await setActive(p.id);
      renderModal();
    });
    row.querySelector('[data-action="assign-thread"]').addEventListener('click', async () => {
      await assignToCurrentThread(p.id);
      closeProjectModal();
    });
    row.querySelector('[data-action="edit"]').addEventListener('click', () => {
      showEditForm(p);
    });
    return row;
  }

  function renderCreateForm() {
    const wrap = document.createElement('div');
    wrap.className = 'project-modal-form';
    wrap.innerHTML = ''
      + '<h4>' + escapeHtml(t('project.modal.createTitle', 'New project')) + '</h4>'
      + '<label>' + escapeHtml(t('project.modal.name', 'Name')) + '<input type="text" id="pm-name" required></label>'
      + '<label>' + escapeHtml(t('project.modal.description', 'Description')) + '<input type="text" id="pm-description"></label>'
      + '<label>' + escapeHtml(t('project.modal.githubRepo', 'GitHub repo (owner/repo)')) + '<input type="text" id="pm-github" placeholder="nearai/ironclaw"></label>'
      + '<label>' + escapeHtml(t('project.modal.defaultBranch', 'Default branch')) + '<input type="text" id="pm-branch" placeholder="staging"></label>'
      + '<label>' + escapeHtml(t('project.modal.workspacePath', 'Workspace path (optional)')) + '<input type="text" id="pm-workspace" placeholder="/home/user/my-repo"></label>'
      + '<button class="project-modal-create" id="pm-create">' + escapeHtml(t('project.modal.create', 'Create')) + '</button>';
    wrap.querySelector('#pm-create').addEventListener('click', async () => {
      const name = wrap.querySelector('#pm-name').value.trim();
      if (!name) {
        showToast(t('project.modal.nameRequired', 'Project name is required'), 'error');
        return;
      }
      const fields = { name };
      const desc = wrap.querySelector('#pm-description').value.trim();
      if (desc) fields.description = desc;
      const repo = wrap.querySelector('#pm-github').value.trim();
      if (repo) fields.github_repo = repo;
      const br = wrap.querySelector('#pm-branch').value.trim();
      if (br) fields.default_branch = br;
      const wp = wrap.querySelector('#pm-workspace').value.trim();
      if (wp) fields.workspace_path = wp;
      try {
        const project = await createProject(fields);
        if (project && project.id) {
          await setActive(project.id);
        }
        renderModal();
      } catch (_e) {
        /* toast shown in createProject */
      }
    });
    return wrap;
  }

  function showEditForm(p) {
    const body = document.getElementById('project-modal-body');
    if (!body) return;
    // The auto-created `default` project is a system-managed shared
    // "General" bucket; renaming it would break the backend's name
    // lookup (`bridge::router` finds it by `p.name == "default"`).
    // Lock the name field rather than hide it, so the user still sees
    // the labelled row and understands the intent.
    const isDefault = p.name === DEFAULT_PROJECT_BACKEND_NAME;
    const nameValue = isDefault ? DEFAULT_PROJECT_DISPLAY_NAME : (p.name || '');
    body.innerHTML = '';
    const form = document.createElement('div');
    form.className = 'project-modal-form';
    form.innerHTML = ''
      + '<h4>' + escapeHtml(t('project.modal.editTitle', 'Edit project')) + '</h4>'
      + '<label>Name<input type="text" id="pe-name" value="' + escapeHtml(nameValue) + '"'
      +     (isDefault ? ' disabled title="The shared General project cannot be renamed."' : '')
      +     '></label>'
      + '<label>Description<input type="text" id="pe-description" value="'
      +     escapeHtml(p.description || '') + '"></label>'
      + '<label>GitHub repo<input type="text" id="pe-github" value="'
      +     escapeHtml((p.metadata && p.metadata.github_repo) || '') + '"></label>'
      + '<label>Default branch<input type="text" id="pe-branch" value="'
      +     escapeHtml((p.metadata && p.metadata.default_branch) || '') + '"></label>'
      + '<label>Workspace path<input type="text" id="pe-workspace" value="'
      +     escapeHtml(p.workspace_path || '') + '"></label>'
      + '<div class="project-modal-form-actions">'
      + '  <button id="pe-save">' + escapeHtml(t('project.modal.save', 'Save')) + '</button>'
      + '  <button id="pe-cancel">' + escapeHtml(t('project.modal.cancel', 'Cancel')) + '</button>'
      + '</div>';
    body.appendChild(form);
    form.querySelector('#pe-save').addEventListener('click', async () => {
      // Never round-trip "General" as the name — it would rename the
      // stable backend identifier and break project resolution for
      // every thread that relies on it. Preserve the original `default`
      // for the auto-created row.
      const enteredName = form.querySelector('#pe-name').value.trim();
      const fields = {
        name: isDefault ? DEFAULT_PROJECT_BACKEND_NAME : enteredName,
        description: form.querySelector('#pe-description').value.trim(),
        github_repo: form.querySelector('#pe-github').value.trim(),
        default_branch: form.querySelector('#pe-branch').value.trim(),
        workspace_path: form.querySelector('#pe-workspace').value.trim(),
      };
      try {
        await updateProject(p.id, fields);
        renderModal();
      } catch (_e) { /* toast shown in updateProject */ }
    });
    form.querySelector('#pe-cancel').addEventListener('click', renderModal);
  }

  // ── Shell turn rendering ─────────────────────────────────────────

  function renderShellCard(turnId) {
    const container = document.getElementById('chat-messages');
    if (!container) return null;
    let card = container.querySelector('[data-turn-id="' + turnId + '"]');
    if (card) return card;
    card = document.createElement('div');
    card.className = 'shell-turn';
    card.setAttribute('data-turn-id', turnId || '');
    card.innerHTML = ''
      + '<div class="shell-turn-header">'
      + '  <span class="shell-turn-dollar">$</span>'
      + '  <span class="shell-turn-cmd"></span>'
      + '  <span class="shell-turn-status" hidden></span>'
      + '</div>'
      + '<pre class="shell-turn-body" hidden></pre>';
    container.appendChild(card);
    container.scrollTop = container.scrollHeight;
    return card;
  }

  window.renderShellCommand = function (data) {
    const card = renderShellCard(data.turn_id || ('inline-' + Date.now()));
    if (!card) return;
    card.querySelector('.shell-turn-cmd').textContent = data.command || '';
    if (data.workdir) card.title = data.workdir;
  };

  window.fillShellOutput = function (data) {
    const card = renderShellCard(data.turn_id || ('inline-' + Date.now()));
    if (!card) return;
    const body = card.querySelector('.shell-turn-body');
    body.textContent = [data.stdout || '', data.stderr || '']
      .filter(Boolean)
      .join('\n');
    body.hidden = false;
    const status = card.querySelector('.shell-turn-status');
    status.hidden = false;
    status.textContent = 'exit ' + (typeof data.exit_code === 'number' ? data.exit_code : '?');
    status.classList.toggle('success', data.exit_code === 0);
    status.classList.toggle('failure', data.exit_code !== 0);
    const container = document.getElementById('chat-messages');
    if (container) container.scrollTop = container.scrollHeight;
  };

  window.renderShellTurn = function (shell) {
    // History-replay path — no turn_id to pair command and output; just
    // render a fresh card with both halves filled in at once.
    const turnId = 'hist-' + Date.now() + '-' + Math.random().toString(36).slice(2, 8);
    const card = renderShellCard(turnId);
    if (!card) return;
    card.querySelector('.shell-turn-cmd').textContent = shell.command || '';
    const body = card.querySelector('.shell-turn-body');
    body.textContent = [shell.stdout || '', shell.stderr || '']
      .filter(Boolean)
      .join('\n');
    body.hidden = false;
    const status = card.querySelector('.shell-turn-status');
    status.hidden = false;
    status.textContent = 'exit ' + shell.exit_code;
    status.classList.toggle('success', shell.exit_code === 0);
    status.classList.toggle('failure', shell.exit_code !== 0);
  };

  // ── sendShellCommand ─────────────────────────────────────────────

  window.sendShellCommand = function (command) {
    const body = {
      content: command,
      thread_id: currentThreadId || undefined,
      mode: 'shell',
    };
    apiFetch('/api/chat/send', { method: 'POST', body })
      .catch((err) => {
        const msg = err && err.message ? err.message : String(err);
        showToast(
          t('project.shell.dispatchFailed', 'Shell command failed: {message}', {
            message: msg,
          }),
          'error',
        );
      });
  };

  // Expose a compact surface for hooks + tests.
  window.ProjectUI = {
    refreshCurrentThread,
    openModal: openProjectModal,
    fetchList: fetchProjectList,
    fetchActive,
    setActive,
  };

  function installShellBadgeListener() {
    const input = document.getElementById('chat-input');
    const wrapper = input ? input.closest('.chat-input-wrapper') : null;
    if (!input || !wrapper) return;
    const sync = () => {
      const starts = input.value.startsWith('!');
      wrapper.classList.toggle('shell-mode', starts);
    };
    input.addEventListener('input', sync);
    // Run once at install time so the badge reflects any state the
    // existing app.js restored before we attached the listener.
    sync();
  }

  // Kick off once DOM is ready.
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => {
      loadProjectsIfNeeded().then(refreshCurrentThread);
      installShellBadgeListener();
    });
  } else {
    loadProjectsIfNeeded().then(refreshCurrentThread);
    installShellBadgeListener();
  }

  // Keep chrome in sync when the user switches threads.
  // `core/history.js::switchThread` + `switchToAssistant` +
  // `createNewThread` all fire a `threadchange` CustomEvent on
  // `window` — we listen for it rather than monkey-patching the
  // source functions (they're file-scoped via script concatenation
  // and not reassignable through `window`). The prior monkey-patch
  // targeted a `window.switchToThread` that never existed and was
  // silently a no-op, so chrome stayed stale after thread switches
  // and the `!`-mode toast had no visible effect.
  if (typeof window !== 'undefined') {
    window.addEventListener('threadchange', () => {
      refreshCurrentThread();
    });
  }
})();
