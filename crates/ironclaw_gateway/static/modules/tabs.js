function switchTab(tab) {
  tab = normalizeTabForEngineMode(tab);
  currentTab = tab;
  // NOTE: this function takes a `tab` argument that may originate from
  // workspace-supplied `layout.tabs.default_tab`, so it must NOT be
  // refactored into a `querySelector('[data-tab="' + tab + '"]')`
  // shape. The current form does string equality on the
  // `getAttribute('data-tab')` value of every button (the loop below)
  // and on `p.id === 'tab-' + tab` for the panel — neither path
  // interpolates `tab` into a CSS selector, so a hostile id can't
  // alter the selector match. If a future change needs to look up a
  // single button by id directly, wrap `tab` in `CSS.escape()` first.
  document.querySelectorAll('.tab-bar button[data-tab]').forEach((b) => {
    b.classList.toggle('active', b.getAttribute('data-tab') === tab);
  });
  document.querySelectorAll('.tab-panel').forEach((p) => {
    p.classList.toggle('active', p.id === 'tab-' + tab);
  });
  applyAriaAttributes();

  if (tab === 'memory') {
    loadMemoryTree();
    // Auto-open README.md on first visit (no file selected yet)
    if (!currentMemoryPath) readMemoryFile('README.md');
  }
  if (tab === 'jobs') loadJobs();
  if (tab === 'projects') {
    loadProjectsOverview();
  } else if (crCurrentProjectId) {
    // Tear down project widgets and reset drill-in state when leaving
    // the Projects tab so widgets don't keep running in the background.
    crBackToOverview();
  }
  if (tab === 'routines') loadRoutines();
  if (tab === 'logs') { connectLogSSE(); applyLogFilters(); }
  else if (logEventSource) { logEventSource.close(); logEventSource = null; }
  if (tab === 'settings') {
    loadSettingsSubtab(currentSettingsSubtab);
  } else {
    stopPairingPoll();
  }
  updateTabIndicator();
  updateHash();
}

function updateTabIndicator() {
  const indicator = document.getElementById('tab-indicator');
  if (!indicator) return;
  const activeBtn = document.querySelector('.tab-bar button[data-tab].active');
  if (!activeBtn) {
    indicator.style.width = '0';
    return;
  }
  const bar = activeBtn.closest('.tab-bar');
  const barRect = bar.getBoundingClientRect();
  const btnRect = activeBtn.getBoundingClientRect();
  indicator.style.left = (btnRect.left - barRect.left) + 'px';
  indicator.style.width = btnRect.width + 'px';
}

window.addEventListener('resize', updateTabIndicator);
