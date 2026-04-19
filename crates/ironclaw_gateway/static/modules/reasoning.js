// --- Streaming Debounce State ---
let _streamBuffer = '';
let _streamDebounceTimer = null;
const STREAM_DEBOUNCE_MS = 50;

// --- Connection Status Banner State ---
let _connectionLostTimer = null;
let _reconnectAttempts = 0;
let _lastSseEventId = null;
// Timestamp of the most recent SSE disconnect (tab hide or onerror). Cleared
// on successful reconnect. Used to decide whether to reload chat history on
// reconnect — brief disconnects (<SSE_RELOAD_THRESHOLD_MS) preserve DOM and
// rely on SSE catch-up + the "Done without response" safety net (#2079);
// longer ones reload to catch missed events.
let _sseDisconnectedAt = null;
const SSE_RELOAD_THRESHOLD_MS = 10000;

// --- Turn Response Tracking State ---
// Safety net for lost SSE response events (see #2079): tracks whether we
// received a `response` event for the current turn so that a "Done" status
// arriving without one can trigger a history reload.
const DONE_WITHOUT_RESPONSE_TIMEOUT_MS = 1500;
// Single-thread tracking is intentional: background thread events are already
// filtered out by `isCurrentThread`, so only the active thread's turn state
// matters here. Per-thread state is unnecessary.
let _turnResponseReceived = false;
let _doneWithoutResponseTimer = null;

// Clean up connection-level timers and buffers.
// Called before creating a new connection, on tab hide, and on page unload
// to prevent leaked intervals/timeouts from accumulating across reconnects.
// Note: _doneWithoutResponseTimer is intentionally NOT cleared here — it is a
// turn-level concern managed by the onopen and response handlers (#2079).
function cleanupConnectionState() {
  if (_streamDebounceTimer) { clearInterval(_streamDebounceTimer); _streamDebounceTimer = null; }
  _streamBuffer = '';
  if (_connectionLostTimer) { clearTimeout(_connectionLostTimer); _connectionLostTimer = null; }
  if (jobListRefreshTimer) { clearTimeout(jobListRefreshTimer); jobListRefreshTimer = null; }
  if (_loadThreadsTimer) { clearTimeout(_loadThreadsTimer); _loadThreadsTimer = null; }
  if (missionMappingRefreshTimer) { clearTimeout(missionMappingRefreshTimer); missionMappingRefreshTimer = null; }
  missionProgressRefreshScheduled = false;
  if (gatewayStatusInterval) { clearInterval(gatewayStatusInterval); gatewayStatusInterval = null; }
}

// --- Send Cooldown State ---
let _sendCooldown = false;
let _recentLocalPairingApprovals = new Map();

// --- Slash Commands ---

const SLASH_COMMANDS = [
  { cmd: '/status',     desc: 'Show all jobs, or /status <id> for one job' },
  { cmd: '/list',       desc: 'List all jobs' },
  { cmd: '/cancel',     desc: '/cancel <job-id> — cancel a running job' },
  { cmd: '/undo',       desc: 'Revert the last turn' },
  { cmd: '/redo',       desc: 'Re-apply an undone turn' },
  { cmd: '/compact',    desc: 'Compress the context window' },
  { cmd: '/clear',      desc: 'Clear thread and start fresh' },
  { cmd: '/interrupt',  desc: 'Stop the current turn' },
  { cmd: '/heartbeat',  desc: 'Trigger manual heartbeat check' },
  { cmd: '/summarize',  desc: 'Summarize the current thread' },
  { cmd: '/suggest',    desc: 'Suggest next steps' },
  { cmd: '/help',       desc: 'Show help' },
  { cmd: '/version',    desc: 'Show version info' },
  { cmd: '/tools',      desc: 'List available tools' },
  { cmd: '/skills',     desc: 'List installed skills' },
  { cmd: '/model',      desc: 'Show or switch the LLM model' },
  { cmd: '/thread new', desc: 'Create a new conversation thread' },
];

let _slashSelected = -1;
let _slashMatches = [];

// --- Tool Activity State ---
// Chat uses a reusable controller so the same entry and rendering helpers can
// be shared with history, jobs, and future activity surfaces.
let _chatToolActivity = createToolActivityController({ containerId: 'chat-messages' });
