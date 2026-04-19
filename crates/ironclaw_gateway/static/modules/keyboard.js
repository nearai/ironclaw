// --- Keyboard shortcuts ---

document.addEventListener('keydown', (e) => {
  const mod = e.metaKey || e.ctrlKey;
  const tag = (e.target.tagName || '').toLowerCase();
  const inInput = tag === 'input' || tag === 'textarea';

  // Mod+1-5: switch tabs
  if (mod && e.key >= '1' && e.key <= '5') {
    e.preventDefault();
    const tabs = engineV2
      ? ['chat', 'memory', 'projects', 'settings', 'jobs']
      : ['chat', 'memory', 'routines', 'settings', 'jobs'];
    const idx = parseInt(e.key) - 1;
    if (tabs[idx]) switchTab(tabs[idx]);
    return;
  }

  // Mod+K: focus chat input or memory search
  if (mod && e.key === 'k') {
    e.preventDefault();
    if (currentTab === 'memory') {
      document.getElementById('memory-search').focus();
    } else {
      document.getElementById('chat-input').focus();
    }
    return;
  }

  // Mod+N: new thread
  if (mod && e.key === 'n' && currentTab === 'chat') {
    e.preventDefault();
    createNewThread();
    return;
  }

  // Mod+/: toggle shortcuts overlay
  if (mod && e.key === '/') {
    e.preventDefault();
    toggleShortcutsOverlay();
    return;
  }

  // Escape: close modals, autocomplete, job detail, or blur input
  if (e.key === 'Escape') {
    const acEl = document.getElementById('slash-autocomplete');
    if (acEl && acEl.style.display !== 'none') {
      hideSlashAutocomplete();
      return;
    }
    // Close shortcuts overlay if open
    const shortcutsOverlay = document.getElementById('shortcuts-overlay');
    if (shortcutsOverlay?.style.display === 'flex') {
      shortcutsOverlay.style.display = 'none';
      return;
    }
    closeModals();
    if (currentJobId) {
      closeJobDetail();
    } else if (inInput) {
      e.target.blur();
    }
    return;
  }
});
