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
  var detail = document.getElementById('mission-detail');
  if (detail) detail.style.display = 'none';
  var body = document.getElementById('missions-body');
  if (body) body.style.display = '';

  Promise.all([
    apiFetch('/api/engine/missions/summary'),
    apiFetch('/api/engine/missions'),
    apiFetch('/api/engine/threads').catch(function() { return { threads: [] }; }),
  ]).then(function(results) {
    var summary = results[0];
    var listData = results[1];
    var threadData = results[2];
    currentMissionList = listData.missions || [];
    activeWorkStore.rememberMissions(currentMissionList);
    renderMissionsSummary(summary);
    renderMissionsList(currentMissionList);
    renderMissionsActivity(threadData.threads || []);
    enrichMissionProgress(currentMissionList);
  }).catch(function() {});
}

function renderMissionsSummary(s) {
  document.getElementById('missions-summary').innerHTML =
    '<div class="ms-summary-card"><span class="ms-summary-label">' + escapeHtml(I18n.t('missions.summary.total')) + '</span><span class="ms-summary-value">' + s.total + '</span></div>'
    + '<div class="ms-summary-card"><span class="ms-summary-label">' + escapeHtml(I18n.t('missions.summary.active')) + '</span><span class="ms-summary-value green">' + s.active + '</span></div>'
    + '<div class="ms-summary-card"><span class="ms-summary-label">' + escapeHtml(I18n.t('missions.summary.paused')) + '</span><span class="ms-summary-value amber">' + s.paused + '</span></div>'
    + '<div class="ms-summary-card"><span class="ms-summary-label">' + escapeHtml(I18n.t('missions.summary.completed')) + '</span><span class="ms-summary-value blue">' + s.completed + '</span></div>'
    + '<div class="ms-summary-card"><span class="ms-summary-label">' + escapeHtml(I18n.t('missions.summary.failed')) + '</span><span class="ms-summary-value red">' + s.failed + '</span></div>';
}

function renderMissionsList(missions) {
  var col = document.getElementById('missions-list-col');
  var empty = document.getElementById('missions-empty');
  var body = document.getElementById('missions-body');

  if (!missions || missions.length === 0) {
    if (col) col.innerHTML = '';
    if (body) body.style.display = 'none';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  if (body) body.style.display = '';

  var groups = { Active: [], Paused: [], Completed: [], Failed: [] };
  missions.forEach(function(m) {
    if (groups[m.status]) groups[m.status].push(m);
    else groups.Active.push(m);
  });

  var html = '';
  var order = ['Active', 'Paused', 'Completed', 'Failed'];
  var labels = {
    Active: I18n.t('missions.summary.active'),
    Paused: I18n.t('missions.summary.paused'),
    Completed: I18n.t('missions.summary.completed'),
    Failed: I18n.t('missions.summary.failed')
  };

  order.forEach(function(status) {
    var list = groups[status];
    if (!list.length) return;

    html += '<div class="ms-section-title">' + escapeHtml(labels[status]) + '</div>';
    list.forEach(function(m) {
      var badgeClass = m.status === 'Active' ? 'in_progress'
        : m.status === 'Completed' ? 'completed'
        : m.status === 'Paused' ? 'pending' : 'failed';
      var progress = activeWorkStore.getMissionProgress(m.id);
      var liveHtml = progress
        ? '<span class="ms-live-tag"><span class="ms-live-dot"></span> Running</span>'
        : '';

      html += '<div class="ms-card" data-action="open-mission" data-id="' + escapeHtml(m.id) + '">'
        + '<div class="ms-card-body">'
        + '<div class="ms-card-head">'
        + '<span class="ms-card-name">' + escapeHtml(m.name) + '</span>'
        + '<span class="badge ' + badgeClass + '">' + escapeHtml(m.status) + '</span>'
        + '</div>'
        + '<div class="ms-card-goal">' + escapeHtml(m.goal) + '</div>'
        + '<div class="ms-card-meta">'
        + '<span>' + escapeHtml(m.cadence_description || m.cadence_type || 'manual') + '</span>'
        + '<span>' + m.thread_count + ' threads</span>'
        + '</div>'
        + '</div>'
        + '<div class="ms-card-right">'
        + liveHtml
        + '<div><div class="ms-card-threads-num">' + m.thread_count + '</div>'
        + '<div class="ms-card-threads-label">threads</div></div>'
        + '</div>'
        + '</div>';
    });
  });

  col.innerHTML = html;
}

function renderMissionsActivity(threads) {
  var col = document.getElementById('missions-activity-col');
  if (!col) return;
  if (!threads || !threads.length) {
    col.innerHTML = '<div class="ms-section-title">' + escapeHtml(I18n.t('missions.recentActivity')) + '</div>'
      + '<div style="color:var(--text-dimmed);font-size:var(--text-sm);padding:12px 14px;">No recent activity.</div>';
    return;
  }

  var sorted = threads.slice().sort(function(a, b) {
    return new Date(b.updated_at || b.created_at) - new Date(a.updated_at || a.created_at);
  });

  var html = '<div class="ms-section-title">' + escapeHtml(I18n.t('missions.recentActivity')) + '</div>';
  var lastDay = '';

  sorted.slice(0, 20).forEach(function(t) {
    var d = new Date(t.updated_at || t.created_at);
    var now = new Date();
    var dayLabel = '';
    if (d.toDateString() === now.toDateString()) dayLabel = 'Today';
    else {
      var yesterday = new Date(now);
      yesterday.setDate(yesterday.getDate() - 1);
      if (d.toDateString() === yesterday.toDateString()) dayLabel = 'Yesterday';
      else dayLabel = d.toLocaleDateString();
    }
    if (dayLabel !== lastDay) {
      html += '<div class="ms-day-divider">' + escapeHtml(dayLabel) + '</div>';
      lastDay = dayLabel;
    }

    var dotClass = (t.state === 'Running') ? 'running'
      : (t.state === 'Done' || t.state === 'Completed') ? 'done'
      : (t.state === 'Failed') ? 'failed' : 'done';
    var label = t.title || t.goal || ('Thread ' + (t.id || '').slice(0, 8));
    var costStr = t.total_cost_usd > 0 ? '$' + t.total_cost_usd.toFixed(2) : '';
    var durationStr = '';
    if (t.completed_at && t.created_at) {
      var secs = Math.round((new Date(t.completed_at) - new Date(t.created_at)) / 1000);
      if (secs < 60) durationStr = secs + 's';
      else durationStr = Math.floor(secs / 60) + 'm ' + (secs % 60) + 's';
    }

    html += '<div class="ms-act-row" data-action="open-engine-thread" data-id="' + escapeHtml(t.id) + '">'
      + '<div class="ms-act-dot ' + dotClass + '"></div>'
      + '<div class="ms-act-content">'
      + '<div class="ms-act-label">' + escapeHtml(label) + '</div>'
      + (t.state === 'Running' ? '<div class="ms-act-sub">In progress</div>' : '')
      + (durationStr || costStr ? '<div class="ms-act-metrics">'
        + (durationStr ? '<span>' + escapeHtml(durationStr) + '</span>' : '')
        + (costStr ? '<span>' + escapeHtml(costStr) + '</span>' : '')
        + '</div>' : '')
      + '</div>'
      + '<span class="ms-act-time">' + escapeHtml(formatRelativeTime(t.updated_at || t.created_at)) + '</span>'
      + '</div>';
  });

  col.innerHTML = html;
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
  var body = document.getElementById('missions-body');
  if (body) body.style.display = 'none';
  document.getElementById('missions-empty').style.display = 'none';

  var detail = document.getElementById('mission-detail');
  detail.style.display = 'block';

  var badgeClass = m.status === 'Active' ? 'in_progress'
    : m.status === 'Completed' ? 'completed'
    : m.status === 'Paused' ? 'pending' : 'failed';
  var progress = activeWorkStore.getMissionProgress(m.id);

  var html = '<button class="ms-detail-back" data-action="close-mission-detail">&larr; ' + escapeHtml(I18n.t('common.back')) + '</button>';

  html += '<div class="ms-detail-header">'
    + '<div class="ms-detail-header-left">'
    + '<div class="ms-detail-title-row">'
    + '<span class="ms-detail-title">' + escapeHtml(m.name) + '</span>'
    + '<span class="badge ' + badgeClass + '">' + escapeHtml(m.status) + '</span>'
    + (progress ? '<span class="ms-live-tag"><span class="ms-live-dot"></span> Running</span>' : '')
    + '</div>'
    + '<div class="ms-detail-goal">' + renderMarkdown(m.goal) + '</div>'
    + '</div>'
    + '<div class="ms-detail-actions">';

  if (m.status === 'Active') {
    html += '<button class="ms-btn primary" data-action="fire-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.fireNow')) + '</button>';
    html += '<button class="ms-btn danger" data-action="pause-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.pause')) + '</button>';
  } else if (m.status === 'Paused') {
    html += '<button class="ms-btn primary" data-action="resume-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.resume')) + '</button>';
    html += '<button class="ms-btn" data-action="fire-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.fireOnce')) + '</button>';
  } else if (m.status === 'Failed') {
    html += '<button class="ms-btn primary" data-action="fire-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.retry')) + '</button>';
  }
  html += '</div></div>';

  html += '<div class="ms-meta-grid">'
    + '<div class="ms-meta-cell"><div class="ms-meta-label">' + escapeHtml(I18n.t('missions.cadence')) + '</div><div class="ms-meta-value">' + escapeHtml(m.cadence_description || m.cadence_type || 'manual') + '</div></div>'
    + '<div class="ms-meta-cell"><div class="ms-meta-label">' + escapeHtml(I18n.t('missions.threadsToday')) + '</div><div class="ms-meta-value mono">' + (m.threads_today || 0) + ' / ' + (m.max_threads_per_day || '\u221E') + '</div></div>'
    + '<div class="ms-meta-cell"><div class="ms-meta-label">' + escapeHtml(I18n.t('missions.totalThreads')) + '</div><div class="ms-meta-value mono">' + m.thread_count + '</div></div>'
    + '<div class="ms-meta-cell"><div class="ms-meta-label">' + escapeHtml(I18n.t('missions.nextFire')) + '</div><div class="ms-meta-value">' + (m.next_fire_at ? formatDate(m.next_fire_at) : (m.status === 'Paused' ? '\u2014 paused' : '\u2014')) + '</div></div>'
    + '<div class="ms-meta-cell"><div class="ms-meta-label">' + escapeHtml(I18n.t('missions.created')) + '</div><div class="ms-meta-value">' + formatDate(m.created_at) + '</div></div>'
    + '</div>';

  if (m.current_focus) {
    html += '<div class="ms-section-title">' + escapeHtml(I18n.t('missions.currentFocus')) + '</div>'
      + '<div class="ms-content-block"><p>' + renderMarkdown(m.current_focus) + '</p></div>';
  }

  if (m.success_criteria) {
    html += '<div class="ms-section-title">' + escapeHtml(I18n.t('missions.successCriteria')) + '</div>'
      + '<div class="ms-content-block"><p>' + renderMarkdown(m.success_criteria) + '</p></div>';
  }

  if (m.notify_channels && m.notify_channels.length > 0) {
    html += '<div class="ms-section-title">Notify Channels</div>'
      + '<div class="ms-content-block"><p>' + m.notify_channels.map(escapeHtml).join(', ') + '</p></div>';
  }

  if (m.approach_history && m.approach_history.length > 0) {
    html += '<div class="ms-section-title">' + escapeHtml(I18n.t('missions.approachHistory')) + '</div>'
      + '<div class="ms-approach-list">';
    m.approach_history.forEach(function(a, i) {
      var isLatest = i === m.approach_history.length - 1;
      html += '<div class="ms-approach-entry' + (isLatest ? ' latest' : '') + '">'
        + '<div class="ms-approach-run"><span>Run ' + (i + 1) + '</span></div>'
        + '<div class="ms-approach-body">' + renderMarkdown(a) + '</div>'
        + '</div>';
    });
    html += '</div>';
  }

  if (m.threads && m.threads.length > 0) {
    html += '<div class="ms-section-title">' + escapeHtml(I18n.t('missions.spawnedThreads')) + '</div>'
      + '<div class="ms-thread-list">';
    m.threads.forEach(function(t) {
      var tState = (t.state === 'Done' || t.state === 'Completed') ? 'done'
        : t.state === 'Failed' ? 'failed'
        : t.state === 'Running' ? 'running' : 'pending';
      var costStr = t.total_cost_usd > 0 ? '$' + t.total_cost_usd.toFixed(2) : '';
      html += '<div class="ms-thread-row" data-action="open-engine-thread" data-id="' + escapeHtml(t.id) + '">'
        + '<span class="ms-thread-state ' + tState + '">' + escapeHtml(t.state) + '</span>'
        + '<span class="ms-thread-label">' + escapeHtml(t.goal) + '</span>'
        + '<span class="ms-thread-cost">' + escapeHtml(costStr) + '</span>'
        + '<span class="ms-thread-time">' + formatRelativeTime(t.created_at) + '</span>'
        + '</div>';
    });
    html += '</div>';
  }

  detail.innerHTML = html;
}

function renderEngineThreadDetail(t) {
  var detail = document.getElementById('mission-detail');

  var stateClass = t.state === 'Done' || t.state === 'Completed' ? 'completed'
    : t.state === 'Failed' ? 'failed'
    : t.state === 'Running' ? 'in_progress'
    : 'pending';
  var progress = activeWorkStore.getThreadProgress(t.id);

  var html = '<button class="ms-detail-back" data-action="back-to-mission">' + escapeHtml(I18n.t('missions.backToMission')) + '</button>';

  html += '<div class="ms-detail-header">'
    + '<div class="ms-detail-header-left">'
    + '<div class="ms-detail-title-row">'
    + '<span class="ms-detail-title">' + escapeHtml(t.goal) + '</span>'
    + '<span class="badge ' + stateClass + '">' + escapeHtml(t.state) + '</span>'
    + '</div></div></div>';

  if (progress) {
    html += '<div class="ms-content-block" data-thread-progress-block-id="' + escapeHtml(t.id) + '">'
      + '<p data-thread-progress-text-id="' + escapeHtml(t.id) + '">' + escapeHtml(progress) + '</p></div>';
  }

  html += '<div class="ms-meta-grid">'
    + '<div class="ms-meta-cell"><div class="ms-meta-label">' + escapeHtml(I18n.t('missions.type')) + '</div><div class="ms-meta-value">' + escapeHtml(t.thread_type || '-') + '</div></div>'
    + '<div class="ms-meta-cell"><div class="ms-meta-label">' + escapeHtml(I18n.t('missions.steps')) + '</div><div class="ms-meta-value mono">' + t.step_count + '</div></div>'
    + '<div class="ms-meta-cell"><div class="ms-meta-label">' + escapeHtml(I18n.t('missions.tokens')) + '</div><div class="ms-meta-value mono">' + t.total_tokens.toLocaleString() + '</div></div>'
    + '<div class="ms-meta-cell"><div class="ms-meta-label">' + escapeHtml(I18n.t('missions.cost')) + '</div><div class="ms-meta-value mono">' + (t.total_cost_usd > 0 ? '$' + t.total_cost_usd.toFixed(4) : '-') + '</div></div>'
    + '<div class="ms-meta-cell"><div class="ms-meta-label">' + escapeHtml(I18n.t('missions.created')) + '</div><div class="ms-meta-value">' + formatDate(t.created_at) + '</div></div>'
    + '<div class="ms-meta-cell"><div class="ms-meta-label">' + escapeHtml(I18n.t('jobs.completedLabel')) + '</div><div class="ms-meta-value">' + (t.completed_at ? formatDate(t.completed_at) : '-') + '</div></div>'
    + '</div>';

  if (t.messages && t.messages.length > 0) {
    html += '<div class="ms-section-title">Messages (' + t.messages.length + ')</div>';
    t.messages.forEach(function(msg) {
      var roleClass = msg.role === 'Assistant' ? 'assistant' : msg.role === 'User' ? 'user' : 'system';
      html += '<div class="thread-message thread-msg-' + roleClass + '">'
        + '<div class="thread-msg-role">' + escapeHtml(msg.role) + '</div>'
        + '<div class="thread-msg-content">' + renderMarkdown(msg.content) + '</div>'
        + '</div>';
    });
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
  } else if (currentTab === 'missions') {
    loadMissions();
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

