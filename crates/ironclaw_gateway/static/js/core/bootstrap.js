// IronClaw Web Gateway - Client

// --- Theme Management (dark / light / system) ---
// Icon switching is handled by pure CSS via data-theme-mode on <html>.

function getSystemTheme() {
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

const VALID_THEME_MODES = { dark: true, light: true, system: true };

function getThemeMode() {
  const stored = localStorage.getItem('ironclaw-theme');
  return (stored && VALID_THEME_MODES[stored]) ? stored : 'system';
}

function resolveTheme(mode) {
  return mode === 'system' ? getSystemTheme() : mode;
}

function applyTheme(mode) {
  const resolved = resolveTheme(mode);
  document.documentElement.setAttribute('data-theme', resolved);
  document.documentElement.setAttribute('data-theme-mode', mode);
  const titleKeys = { dark: 'theme.tooltipDark', light: 'theme.tooltipLight', system: 'theme.tooltipSystem' };
  const btn = document.getElementById('theme-toggle');
  if (btn) btn.title = (typeof I18n !== 'undefined' && titleKeys[mode]) ? I18n.t(titleKeys[mode]) : ('Theme: ' + mode);
  const announce = document.getElementById('theme-announce');
  if (announce) announce.textContent = (typeof I18n !== 'undefined') ? I18n.t('theme.announce', { mode: mode }) : ('Theme: ' + mode);
}

function toggleTheme() {
  const cycle = { dark: 'light', light: 'system', system: 'dark' };
  const current = getThemeMode();
  const next = cycle[current] || 'dark';
  localStorage.setItem('ironclaw-theme', next);
  applyTheme(next);
}

// Apply theme immediately (FOUC prevention is done via inline script in <head>,
// but we call again here to ensure tooltip is set after DOM is ready).
applyTheme(getThemeMode());

// Delay enabling theme transition to avoid flash on initial load.
requestAnimationFrame(function() {
  requestAnimationFrame(function() {
    document.body.classList.add('theme-transition');
  });
});

// Listen for OS theme changes — only re-apply when in 'system' mode.
const mql = window.matchMedia('(prefers-color-scheme: light)');
const onSchemeChange = function() {
  if (getThemeMode() === 'system') {
    applyTheme('system');
  }
};
if (mql.addEventListener) {
  mql.addEventListener('change', onSchemeChange);
} else if (mql.addListener) {
  mql.addListener(onSchemeChange);
}

// Bind theme toggle buttons (CSP-compliant — no inline onclick).
document.getElementById('theme-toggle').addEventListener('click', toggleTheme);
document.getElementById('settings-theme-toggle')?.addEventListener('click', () => {
  toggleTheme();
  const btn = document.getElementById('settings-theme-toggle');
  if (btn) {
    const mode = localStorage.getItem('ironclaw-theme') || 'system';
    btn.textContent = I18n.t('theme.label', { mode: mode.charAt(0).toUpperCase() + mode.slice(1) });
  }
});

let token = '';
let oidcProxyAuth = false;
let eventSource = null;
let logEventSource = null;
let currentTab = 'chat';
let currentThreadId = null;
let currentThreadIsReadOnly = false;
const threadChannelHints = new Map();
let hasMore = false;
let oldestTimestamp = null;
let loadingOlder = false;
let sseHasConnectedBefore = false;
let jobEvents = new Map(); // job_id -> Array of events
let jobListRefreshTimer = null;
let pairingPollInterval = null;
let unreadThreads = new Map(); // thread_id -> unread count
let processingThreads = new Set(); // thread IDs with active agent work
let _loadThreadsTimer = null;
const JOB_EVENTS_CAP = 500;
const JOB_EVENTS_MAX_JOBS = 50;
const MAX_DOM_MESSAGES = 200;
const MEMORY_SEARCH_QUERY_MAX_LENGTH = 100;
let stagedImages = [];
// Non-image attachments staged for the next /api/chat/send submission.
// Shape matches SendMessageRequest::attachments: { mime_type, filename, data_base64 }.
let stagedAttachments = [];
// FileReader promises that have not yet resolved. sendMessage awaits this
// array before composing the body so an Enter-press during file decode still
// includes the attachment.
const pendingAttachmentReads = [];
// Reserved attachment budget for files accepted into FileReader but not yet
// materialized in `stagedAttachments`. This closes race windows across rapid
// repeated attach/drop actions before async reads complete.
let pendingAttachmentBytes = 0;
let pendingAttachmentCount = 0;
let authFlowPending = false;
// Tracks user messages sent but not yet persisted to DB (#2409).
// When loadHistory() clears the DOM, pending messages are re-injected
// so they don't vanish during the safety-pipeline processing window.
const _pendingUserMessages = new Map(); // threadId -> [{id, content, images, timestamp}]
const PENDING_MSG_TTL_MS = 60000; // discard after 60s
let _nextPendingId = 0;
let _ghostSuggestion = '';
let currentSettingsSubtab = 'inference';
let generatedImagesByThread = new Map();
const GENERATED_IMAGE_THREAD_CACHE_CAP = 20;
const GENERATED_IMAGES_PER_THREAD_CAP = 8;
let engineV2Enabled = false;
let engineModeApplied = false;
let currentMissionData = null;
let currentEngineThreadDetail = null;
let currentMissionList = [];
const missionDetailCache = new Map();
const missionDetailFetchInFlight = new Set();
const ACTIVE_MISSION_MAPPING_REFRESH_MS = 5000;
const MAX_ACTIVITY_BAR_ITEMS = 6;
let missionProgressRefreshScheduled = false;
let missionMappingRefreshTimer = null;
let missionMappingsLastRefreshedAt = 0;
let activityBarSnapshotInFlight = false;

