function loadToolsPermissions() {
  var container = document.getElementById('tools-permissions-list');
  if (!container) return;
  container.innerHTML = '<div class="empty-state">' + I18n.t('common.loading') + '</div>';
  apiFetch('/api/settings/tools').then(function(data) {
    if (!data.tools || data.tools.length === 0) {
      container.innerHTML = '<div class="empty-state">' + I18n.t('tools.noTools') + '</div>';
      return;
    }
    container.innerHTML = '';
    for (var i = 0; i < data.tools.length; i++) {
      container.appendChild(renderToolPermissionRow(data.tools[i]));
    }
  }).catch(function(err) {
    container.innerHTML = '<div class="empty-state">' + I18n.t('common.loadFailed') + ': ' + escapeHtml(err.message) + '</div>';
  });
}

function renderToolPermissionRow(tool) {
  var row = document.createElement('div');
  row.className = 'tool-permission-row';
  row.dataset.toolName = tool.name;

  // Left: name + description
  var info = document.createElement('div');
  info.className = 'tool-permission-info';

  var name = document.createElement('span');
  name.className = 'tool-permission-name';
  name.textContent = tool.name;

  var desc = document.createElement('span');
  desc.className = 'tool-permission-desc';
  desc.textContent = tool.description;

  info.appendChild(name);
  info.appendChild(desc);

  // Right: lock icon or toggle + default badge
  var controls = document.createElement('div');
  controls.className = 'tool-permission-controls';

  if (tool.locked) {
    var lock = document.createElement('span');
    lock.className = 'tool-lock-icon';
    lock.title = I18n.t('tools.lockedTooltip');
    lock.textContent = '\uD83D\uDD12';
    controls.appendChild(lock);
  } else {
    var toggle = document.createElement('div');
    toggle.className = 'tool-permission-toggle';

    var states = [
      { value: 'always_allow', label: I18n.t('tools.alwaysAllow') },
      { value: 'ask_each_time', label: I18n.t('tools.askEachTime') },
      { value: 'disabled', label: I18n.t('tools.disabled') },
    ];

    for (var j = 0; j < states.length; j++) {
      (function(state) {
        var btn = document.createElement('button');
        btn.textContent = state.label;
        btn.dataset.state = state.value;
        btn.setAttribute('aria-pressed', tool.current_state === state.value);
        if (tool.current_state === state.value) btn.classList.add('active');
        btn.addEventListener('click', function() {
          setToolPermission(tool.name, state.value, row);
        });
        toggle.appendChild(btn);
      })(states[j]);
    }

    controls.appendChild(toggle);
  }

  if (tool.current_state === tool.default_state) {
    var badge = document.createElement('span');
    badge.className = 'tool-default-badge';
    badge.textContent = I18n.t('tools.defaultBadge');
    controls.appendChild(badge);
  }

  row.appendChild(info);
  row.appendChild(controls);
  return row;
}

function setToolPermission(toolName, newState, rowEl) {
  apiFetch('/api/settings/tools/' + encodeURIComponent(toolName), {
    method: 'PUT',
    body: { state: newState },
  }).then(function(updated) {
    // Re-render just this row in-place.
    var newRow = renderToolPermissionRow(updated);
    if (rowEl && rowEl.parentNode) {
      rowEl.parentNode.replaceChild(newRow, rowEl);
    }
  }).catch(function(err) {
    showToast(I18n.t('tools.saveFailed', { message: err.message }), 'error');
  });
}

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

// --- Settings Tab ---

document.querySelectorAll('.settings-subtab').forEach(function(btn) {
  btn.addEventListener('click', function() {
    switchSettingsSubtab(btn.getAttribute('data-settings-subtab'));
  });
});
