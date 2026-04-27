function shortDisplayId(id) {
  return typeof id === 'string' && id.length > 8 ? id.substring(0, 8) : (id || '');
}

class ActivityEntry {
  static parseTimestampMs(value) {
    if (!value) return 0;
    const parsed = Date.parse(value);
    return Number.isFinite(parsed) ? parsed : 0;
  }

  static t(key, fallback, params) {
    if (typeof I18n === 'undefined') return fallback;
    const translated = I18n.t(key, params);
    return translated && translated !== key ? translated : fallback;
  }
}

class JobActivityEntry extends ActivityEntry {
  constructor({ id, title, state, statusText, updatedAt }) {
    super();
    this.id = id;
    this.title = title;
    this.state = state;
    this.statusText = statusText;
    this.updatedAt = updatedAt;
  }

  static isActiveState(state) {
    return state === 'pending' || state === 'in_progress' || state === 'running';
  }

  static normalizeState(state) {
    if (state === 'failed' || state === 'error' || state === 'stuck') return 'failed';
    if (state === 'completed' || state === 'done' || state === 'succeeded') return 'done';
    if (JobActivityEntry.isActiveState(state)) return 'running';
    return state || 'done';
  }

  static formatStatus(state, fallback) {
    if (fallback) return fallback;
    if (state === 'pending') return ActivityEntry.t('jobs.statusPending', 'Pending');
    if (state === 'in_progress' || state === 'running') return ActivityEntry.t('jobs.statusRunning', 'Running');
    if (state === 'completed' || state === 'done' || state === 'succeeded') return ActivityEntry.t('jobs.statusCompleted', 'Completed');
    if (state === 'failed' || state === 'error') return ActivityEntry.t('jobs.statusFailed', 'Failed');
    if (state === 'stuck') return ActivityEntry.t('jobs.summary.stuck', 'Stuck');
    return state ? state.replace(/_/g, ' ') : ActivityEntry.t('jobs.statusCompleted', 'Completed');
  }

  static shouldPreserveActiveStatus(existing) {
    if (!existing?.isActive() || !existing.statusText) return false;
    const genericStates = ['pending', 'in_progress', 'running'];
    return !genericStates.some((candidate) => existing.statusText === JobActivityEntry.formatStatus(candidate));
  }

  static fromApi(job, existing) {
    const normalizedState = JobActivityEntry.normalizeState(job.state);
    const nextUpdatedAt = JobActivityEntry.parseTimestampMs(job.started_at || job.created_at)
      || existing?.updatedAt
      || Date.now();
    const shouldPreserveStatus = normalizedState === 'running'
      && JobActivityEntry.shouldPreserveActiveStatus(existing);
    return new JobActivityEntry({
      id: job.id,
      title: job.title || existing?.title || ('Job ' + shortDisplayId(job.id)),
      state: normalizedState,
      statusText: normalizedState === 'running'
        ? JobActivityEntry.formatStatus(job.state, shouldPreserveStatus ? existing.statusText : '')
        : JobActivityEntry.formatStatus(job.state),
      updatedAt: nextUpdatedAt,
    });
  }

  applyPatch(patch) {
    if (patch.title) this.title = patch.title;
    if (patch.state) this.state = patch.state;
    if (patch.statusText) this.statusText = patch.statusText;
    if (patch.active === false && !patch.state) this.state = 'done';
    this.updatedAt = Date.now();
  }

  isActive() {
    return this.state === 'running';
  }

  toBarItem() {
    return {
      kind: 'job',
      id: this.id,
      title: this.title,
      statusText: this.statusText || JobActivityEntry.formatStatus('running'),
      updatedAt: this.updatedAt || 0,
      state: this.state || 'done',
    };
  }
}

class MissionActivityEntry extends ActivityEntry {
  constructor({ id, title, status, state, statusText, updatedAt }) {
    super();
    this.id = id;
    this.title = title;
    this.status = status;
    this.state = state;
    this.statusText = statusText;
    this.updatedAt = updatedAt;
  }

  static normalizeState(status) {
    if (status === 'Active') return 'running';
    if (status === 'Completed') return 'done';
    if (status === 'Failed') return 'failed';
    return 'idle';
  }

  static formatStatus(status, fallback) {
    if (fallback) return fallback;
    if (status === 'Active') return ActivityEntry.t('status.active', 'Active');
    if (status === 'Completed') return ActivityEntry.t('missions.summary.completed', 'Completed');
    if (status === 'Failed') return ActivityEntry.t('missions.summary.failed', 'Failed');
    if (status === 'Paused') return ActivityEntry.t('missions.summary.paused', 'Paused');
    return status || ActivityEntry.t('status.idle', 'Idle');
  }

  static shouldPreserveActiveStatus(existing) {
    if (!existing?.isActive() || !existing.statusText) return false;
    const genericStatuses = ['Active', 'Completed', 'Failed', 'Paused'];
    return !genericStatuses.some((candidate) => existing.statusText === MissionActivityEntry.formatStatus(candidate));
  }

  static fromApi(mission, existing) {
    const normalizedState = MissionActivityEntry.normalizeState(mission.status);
    const nextUpdatedAt = MissionActivityEntry.parseTimestampMs(mission.updated_at || mission.created_at)
      || existing?.updatedAt
      || Date.now();
    const shouldPreserveStatus = normalizedState === 'running'
      && MissionActivityEntry.shouldPreserveActiveStatus(existing);
    return new MissionActivityEntry({
      id: mission.id,
      title: mission.name || existing?.title || ('Mission ' + shortDisplayId(mission.id)),
      status: mission.status || existing?.status || '',
      state: normalizedState,
      statusText: normalizedState === 'running'
        ? MissionActivityEntry.formatStatus(
          mission.status,
          shouldPreserveStatus ? existing.statusText : '',
        )
        : MissionActivityEntry.formatStatus(mission.status),
      updatedAt: nextUpdatedAt,
    });
  }

  applyThreadPatch(meta, patch) {
    this.title = meta.mission_name || this.title || ('Mission ' + shortDisplayId(meta.mission_id));
    this.status = this.status || 'Active';
    this.state = 'running';
    this.statusText = patch.statusText || this.statusText || MissionActivityEntry.formatStatus('Active');
    this.updatedAt = Date.now();
  }

  isActive() {
    return this.state === 'running';
  }

  isVisibleInBar() {
    return this.state !== 'idle';
  }

  toBarItem(liveSnapshot) {
    const liveUpdatedAt = liveSnapshot?.updatedAt || 0;
    const statusText = this.state === 'running'
      ? (liveSnapshot?.progress || MissionActivityEntry.formatStatus(this.status))
      : MissionActivityEntry.formatStatus(this.status, this.statusText);
    return {
      kind: 'mission',
      id: this.id,
      missionId: this.id,
      title: this.title || ('Mission ' + shortDisplayId(this.id)),
      statusText: statusText,
      updatedAt: Math.max(this.updatedAt || 0, liveUpdatedAt),
      state: this.state || 'done',
    };
  }
}

class ActiveWorkStore {
  constructor() {
    this.threads = new Map();
    this.jobs = new Map();
    this.missions = new Map();
    this.threadMeta = new Map();
  }

  setEngineV2Enabled(enabled) {
    engineV2Enabled = !!enabled;
    this.render();
  }

  rememberThreads(entries) {
    if (!Array.isArray(entries) || entries.length === 0) return;
    let changed = false;
    entries.forEach(({ threadId, meta }) => {
      if (!threadId) return;
      const prev = this.threadMeta.get(threadId) || {};
      const next = { ...prev, ...meta };
      const nextKeys = Object.keys(next);
      const isSame = nextKeys.length === Object.keys(prev).length
        && nextKeys.every((key) => prev[key] === next[key]);
      if (isSame) return;
      this.threadMeta.set(threadId, next);
      changed = true;
    });
    if (changed) this.render();
  }

  rememberMissionThreads(mission) {
    if (!mission || !Array.isArray(mission.threads)) return;
    this.rememberThreads(mission.threads.map((thread) => ({
      threadId: thread.id,
      meta: {
        label: thread.goal || ('Thread ' + shortDisplayId(thread.id)),
        mission_id: mission.id,
        mission_name: mission.name,
      },
    })));
  }

  rememberJobs(jobs) {
    if (!Array.isArray(jobs)) return;
    jobs.forEach((job) => {
      if (!job || !job.id) return;
      this.jobs.set(job.id, JobActivityEntry.fromApi(job, this.jobs.get(job.id)));
    });
    this.render();
  }

  rememberMissions(missions) {
    if (!Array.isArray(missions)) return;
    missions.forEach((mission) => {
      if (!mission || !mission.id) return;
      this.missions.set(mission.id, MissionActivityEntry.fromApi(mission, this.missions.get(mission.id)));
    });
    this.render();
  }

  updateThread(threadId, patch) {
    if (!threadId) return;
    const prev = this.threads.get(threadId) || {};
    this.threads.set(threadId, {
      ...prev,
      ...patch,
      active: patch.active !== undefined ? patch.active : true,
      updatedAt: Date.now(),
    });
    const meta = this.threadMeta.get(threadId) || {};
    if (meta.mission_id) {
      const missionEntry = this.missions.get(meta.mission_id)
        || new MissionActivityEntry({
          id: meta.mission_id,
          title: meta.mission_name || ('Mission ' + shortDisplayId(meta.mission_id)),
          status: 'Active',
          state: 'running',
          statusText: MissionActivityEntry.formatStatus('Active'),
          updatedAt: Date.now(),
        });
      missionEntry.applyThreadPatch(meta, patch);
      this.missions.set(meta.mission_id, missionEntry);
    }
    this.render();
  }

  clearThread(threadId) {
    if (!threadId) return;
    this.threads.delete(threadId);
    this.render();
  }

  updateJob(jobId, patch) {
    if (!jobId) return;
    const prev = this.jobs.get(jobId)
      || new JobActivityEntry({
        id: jobId,
        title: patch.title || ('Job ' + shortDisplayId(jobId)),
        state: 'running',
        statusText: JobActivityEntry.formatStatus('running'),
        updatedAt: Date.now(),
      });
    prev.applyPatch(patch);
    this.jobs.set(jobId, prev);
    this.render();
  }

  getThreadProgress(threadId) {
    const entry = threadId ? this.threads.get(threadId) : null;
    return entry && entry.active ? entry.statusText : '';
  }

  isThreadBlocked(threadId) {
    const entry = threadId ? this.threads.get(threadId) : null;
    return !!(entry && entry.blockedReason);
  }

  getMissionProgress(missionId) {
    let newest = null;
    for (const [threadId, meta] of this.threadMeta.entries()) {
      if (meta.mission_id !== missionId) continue;
      const thread = this.threads.get(threadId);
      if (!thread || !thread.active) continue;
      if (!newest || thread.updatedAt > newest.updatedAt) {
        newest = thread;
      }
    }
    return newest ? newest.statusText : '';
  }

  getTabCounts() {
    let jobs = 0;

    for (const entry of this.jobs.values()) {
      if (entry && entry.isActive()) jobs += 1;
    }

    let missions = 0;
    for (const entry of this.missions.values()) {
      if (entry && entry.isActive()) missions += 1;
    }

    return {
      jobs: jobs,
      missions: missions,
    };
  }

  renderTabCounts() {
    const counts = this.getTabCounts();
    const chatButton = document.querySelector('.tab-bar button[data-tab="chat"]');
    if (chatButton) {
      chatButton.removeAttribute('data-active-count');
    }
    ['jobs', 'missions'].forEach((tabName) => {
      const button = document.querySelector('.tab-bar button[data-tab="' + tabName + '"]');
      if (!button) return;
      const count = counts[tabName] || 0;
      if (count > 0) {
        button.setAttribute('data-active-count', String(count));
      } else {
        button.removeAttribute('data-active-count');
      }
    });
  }

  getMissionLiveSnapshot(missionId) {
    let newestUpdatedAt = 0;
    let progress = '';
    for (const [threadId, meta] of this.threadMeta.entries()) {
      if (meta.mission_id !== missionId) continue;
      const thread = this.threads.get(threadId);
      if (!thread || !thread.active) continue;
      if ((thread.updatedAt || 0) >= newestUpdatedAt) {
        newestUpdatedAt = thread.updatedAt || 0;
        progress = thread.statusText || '';
      }
    }
    return { updatedAt: newestUpdatedAt, progress: progress };
  }

  getActiveMissionIds() {
    const ids = [];
    for (const [missionId, entry] of this.missions.entries()) {
      if (entry && entry.isActive()) ids.push(missionId);
    }
    return ids;
  }

  getActivityBarItems() {
    const items = [];
    for (const [missionId, entry] of this.missions.entries()) {
      if (!entry || !entry.isVisibleInBar()) continue;
      items.push(entry.toBarItem(this.getMissionLiveSnapshot(missionId)));
    }
    for (const [jobId, entry] of this.jobs.entries()) {
      if (!entry) continue;
      items.push(entry.toBarItem());
    }
    items.sort((a, b) => b.updatedAt - a.updatedAt);
    return items.slice(0, MAX_ACTIVITY_BAR_ITEMS);
  }

  render() {
    this.renderTabCounts();
    const strip = document.getElementById('active-work-strip');
    if (!strip) return;
    if (!engineV2Enabled) {
      strip.hidden = true;
      strip.innerHTML = '';
      scheduleMissionProgressViewsRefresh();
      return;
    }
    const items = this.getActivityBarItems();
    strip.hidden = false;
    strip.innerHTML = items.length === 0
      ? '<div class="active-work-empty">' + escapeHtml(ActivityEntry.t('activity.empty', 'No recent jobs or missions')) + '</div>'
      : items.map((item) => {
        const kindLabel = item.kind === 'job'
          ? ActivityEntry.t('activity.kind.job', 'Job')
          : ActivityEntry.t('activity.kind.mission', 'Mission');
        return '<button class="active-work-item" type="button"'
          + ' data-action="open-active-work"'
          + ' data-kind="' + escapeHtml(item.kind) + '"'
          + ' data-state="' + escapeHtml(item.state || 'done') + '"'
          + ' data-id="' + escapeHtml(item.id) + '"'
          + (item.missionId ? ' data-mission-id="' + escapeHtml(item.missionId) + '"' : '')
          + (item.updatedAt ? ' title="' + escapeHtml(relativeTime(new Date(item.updatedAt).toISOString())) + '"' : '')
          + '>'
          + '<span class="active-work-kind">' + escapeHtml(kindLabel) + '</span>'
          + '<span class="active-work-title">' + escapeHtml(item.title) + '</span>'
          + '<span class="active-work-status">' + escapeHtml(item.statusText) + '</span>'
          + '</button>';
      }).join('');

    scheduleMissionProgressViewsRefresh();
  }
}

const activeWorkStore = new ActiveWorkStore();

// --- Hash-based URL Navigation ---
//
// Encodes navigation state in window.location.hash so refreshing
// the page restores the current tab, thread, memory file, job detail, etc.
//
// Hash format: #/{tab}[/{detail}[/{subtab}]]
//   #/chat                     → chat tab, assistant thread
//   #/chat/{threadId}          → chat tab, specific thread
//   #/memory                   → memory tab, tree root
//   #/memory/{path/to/file}    → memory tab, specific file
//   #/jobs                     → jobs list
//   #/jobs/{jobId}             → job detail
//   #/routines                 → routines list
//   #/routines/{id}            → routine detail
//   #/settings/{subtab}        → settings tab with specific sub-tab
//   #/logs                     → logs tab

