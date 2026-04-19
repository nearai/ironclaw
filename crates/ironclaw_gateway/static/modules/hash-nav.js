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

/** Suppress hash-change handling while we're programmatically updating. */
let _suppressHashChange = false;

/** Update the URL hash to reflect current navigation state. */
function updateHash() {
  if (_suppressHashChange) return;
  var parts = [currentTab];

  switch (currentTab) {
    case 'chat':
      if (currentThreadId && currentThreadId !== assistantThreadId) {
        parts.push(currentThreadId);
      }
      break;
    case 'memory':
      if (typeof currentMemoryPath === 'string' && currentMemoryPath) {
        parts.push(currentMemoryPath);
      }
      break;
    case 'jobs':
      if (typeof currentJobId !== 'undefined' && currentJobId) {
        parts.push(currentJobId);
      }
      break;
    case 'routines':
      if (typeof currentRoutineId !== 'undefined' && currentRoutineId) {
        parts.push(currentRoutineId);
      }
      break;
    case 'settings':
      if (currentSettingsSubtab && currentSettingsSubtab !== 'inference') {
        parts.push(currentSettingsSubtab);
      }
      break;
  }

  var hash = '#/' + parts.join('/');
  if (window.location.hash !== hash) {
    window.history.replaceState(null, '', hash);
  }
}

/** Parse the current URL hash into navigation state. */
function parseHash() {
  var hash = window.location.hash || '';
  if (!hash.startsWith('#/')) return null;
  var parts = hash.substring(2).split('/');
  return {
    tab: parts[0] || 'chat',
    detail: parts.slice(1).join('/') || null,
  };
}

function normalizeTabForEngineMode(tab) {
  if (engineV2Enabled && tab === 'routines') {
    return 'missions';
  }
  return tab;
}

function applyEngineModeUi() {
  var routinesTab = document.querySelector('.tab-bar [data-tab-role="routines"]');
  var routinesPanel = document.getElementById('tab-routines');
  if (routinesTab) {
    routinesTab.style.display = engineV2Enabled ? 'none' : '';
  }
  if (routinesPanel && engineV2Enabled && currentTab !== 'routines') {
    routinesPanel.classList.remove('active');
  }
  if (engineV2Enabled && currentTab === 'routines') {
    switchTab('missions');
  }
}

/**
 * Restore navigation state from the URL hash.
 * Called once after authentication and on hashchange events.
 */
function restoreFromHash() {
  var state = parseHash();
  if (!state) return;

  // Suppress hash updates while restoring — switchTab/readMemoryFile/etc.
  // each call updateHash(), which would overwrite the full hash before
  // the detail part is restored.
  _suppressHashChange = true;

  // Switch tab
  if (state.tab && state.tab !== currentTab) {
    switchTab(normalizeTabForEngineMode(state.tab));
  }

  // Restore detail state within the tab
  if (state.detail) {
    switch (state.tab) {
      case 'chat':
        // Defer thread switch until threads are loaded
        window._pendingThreadRestore = state.detail;
        break;
      case 'memory':
        readMemoryFile(state.detail);
        break;
      case 'jobs':
        openJobDetail(state.detail);
        break;
      case 'routines':
        if (engineV2Enabled) {
          switchTab('missions');
        } else {
          openRoutineDetail(state.detail);
        }
        break;
      case 'settings':
        switchSettingsSubtab(state.detail);
        break;
    }
  }

  _suppressHashChange = false;
}

window.addEventListener('hashchange', function() {
  if (_suppressHashChange) return;
  restoreFromHash();
});
