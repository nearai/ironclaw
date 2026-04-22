// --- Command Palette (Cmd+K) ---

let _paletteOpen = false;
let _paletteResults = [];
let _paletteSelected = -1;

const PALETTE_TABS = [
  { id: 'chat',     label: 'Chat',     icon: '\u{1F4AC}' },
  { id: 'memory',   label: 'Memory',   icon: '\u{1F9E0}' },
  { id: 'jobs',     label: 'Jobs',     icon: '\u{1F4CB}' },
  { id: 'missions', label: 'Missions', icon: '\u{1F3AF}' },
  { id: 'routines', label: 'Routines', icon: '\u{1F504}' },
  { id: 'settings', label: 'Settings', icon: '\u{2699}\uFE0F' },
  { id: 'logs',     label: 'Logs',     icon: '\u{1F4DC}' },
];

const PALETTE_ACTIONS = [
  { id: 'new-thread',      label: 'New Thread',      icon: '\u{2795}',      action: function() { createNewThread(); } },
  { id: 'toggle-theme',    label: 'Toggle Theme',    icon: '\u{1F3A8}',     action: function() { toggleTheme(); } },
  { id: 'show-shortcuts',  label: 'Show Shortcuts',  icon: '\u{2328}\uFE0F', action: function() { toggleShortcutsOverlay(); } },
];

function openCommandPalette() {
  _paletteOpen = true;
  _paletteSelected = -1;
  var overlay = document.getElementById('command-palette-overlay');
  var input = document.getElementById('palette-input');
  overlay.style.display = 'flex';
  input.value = '';
  input.focus();
  searchPalette('');
  _refreshPaletteThreads();
}

function _refreshPaletteThreads() {
  apiFetch('/api/chat/threads').then(function(data) {
    var threads = (data && data.threads) || [];
    _cachedThreads = threads.map(function(t) {
      return { id: t.id, title: threadTitle(t), channel: t.channel || 'gateway', updated_at: t.updated_at };
    });
    if (_paletteOpen) {
      var input = document.getElementById('palette-input');
      searchPalette(input ? input.value : '');
    }
  }).catch(function() { /* palette still works with stale cache */ });
}

function closeCommandPalette() {
  _paletteOpen = false;
  _paletteSelected = -1;
  _paletteResults = [];
  var overlay = document.getElementById('command-palette-overlay');
  overlay.style.display = 'none';
}

function searchPalette(query) {
  var q = query.toLowerCase().trim();
  var results = [];

  // Threads
  var threadMatches = _cachedThreads.filter(function(t) {
    return (t.title || '').toLowerCase().indexOf(q) !== -1 || (t.id || '').toLowerCase().indexOf(q) !== -1;
  });
  if (q === '') threadMatches = threadMatches.slice(0, 5);
  else threadMatches = threadMatches.slice(0, 10);
  for (var i = 0; i < threadMatches.length; i++) {
    var t = threadMatches[i];
    results.push({
      category: 'Threads', icon: '\u{1F4AC}', title: t.title, desc: t.channel !== 'gateway' ? t.channel : '',
      action: (function(id) { return function() { switchThread(id); switchTab('chat'); }; })(t.id)
    });
  }

  // Commands
  var cmdMatches = SLASH_COMMANDS.filter(function(c) {
    return c.cmd.toLowerCase().indexOf(q) !== -1 || c.desc.toLowerCase().indexOf(q) !== -1;
  });
  if (q === '') cmdMatches = cmdMatches.slice(0, 5);
  else cmdMatches = cmdMatches.slice(0, 10);
  for (var i = 0; i < cmdMatches.length; i++) {
    var c = cmdMatches[i];
    results.push({
      category: 'Commands', icon: '\u{2F}', title: c.cmd, desc: c.desc,
      action: (function(cmd) { return function() {
        switchTab('chat');
        var chatInput = document.getElementById('chat-input');
        if (chatInput) { chatInput.value = cmd + ' '; chatInput.focus(); }
      }; })(c.cmd)
    });
  }

  // Tabs
  var tabMatches = PALETTE_TABS.filter(function(tab) {
    return tab.label.toLowerCase().indexOf(q) !== -1 || tab.id.toLowerCase().indexOf(q) !== -1;
  });
  for (var i = 0; i < tabMatches.length; i++) {
    var tab = tabMatches[i];
    results.push({
      category: 'Tabs', icon: tab.icon, title: tab.label,
      desc: tab.id === currentTab ? 'Current' : '',
      action: (function(id) { return function() { switchTab(id); }; })(tab.id)
    });
  }

  // Quick Actions
  var actionMatches = PALETTE_ACTIONS.filter(function(a) {
    return a.label.toLowerCase().indexOf(q) !== -1;
  });
  for (var i = 0; i < actionMatches.length; i++) {
    var a = actionMatches[i];
    results.push({
      category: 'Actions', icon: a.icon, title: a.label, desc: '', action: a.action
    });
  }

  _paletteResults = results;
  _paletteSelected = results.length > 0 ? 0 : -1;
  renderPaletteResults();
}

function renderPaletteResults() {
  var container = document.getElementById('palette-results');
  container.innerHTML = '';

  if (_paletteResults.length === 0) {
    var empty = document.createElement('div');
    empty.className = 'command-palette-empty';
    empty.textContent = I18n.t('palette.noResults');
    container.appendChild(empty);
    return;
  }

  var lastCategory = '';
  for (var i = 0; i < _paletteResults.length; i++) {
    var r = _paletteResults[i];

    if (r.category !== lastCategory) {
      lastCategory = r.category;
      var groupLabel = document.createElement('div');
      groupLabel.className = 'command-palette-group-label';
      groupLabel.setAttribute('role', 'presentation');
      groupLabel.textContent = r.category;
      container.appendChild(groupLabel);
    }

    var item = document.createElement('div');
    item.className = 'command-palette-item' + (i === _paletteSelected ? ' selected' : '');
    item.setAttribute('role', 'option');
    item.id = 'palette-item-' + i;
    item.dataset.index = String(i);

    var icon = document.createElement('span');
    icon.className = 'command-palette-item-icon';
    icon.textContent = r.icon;
    item.appendChild(icon);

    var textWrap = document.createElement('span');
    textWrap.className = 'command-palette-item-text';

    var title = document.createElement('div');
    title.className = 'command-palette-item-title';
    title.textContent = r.title;
    textWrap.appendChild(title);

    if (r.desc) {
      var desc = document.createElement('div');
      desc.className = 'command-palette-item-desc';
      desc.textContent = r.desc;
      textWrap.appendChild(desc);
    }

    item.appendChild(textWrap);

    if (r.category === 'Threads' && r.desc) {
      var badge = document.createElement('span');
      badge.className = 'command-palette-item-badge';
      badge.textContent = r.desc;
      item.appendChild(badge);
    }

    (function(idx) {
      item.addEventListener('click', function() {
        _paletteSelected = idx;
        executePaletteSelection();
      });
      item.addEventListener('mouseenter', function() {
        _paletteSelected = idx;
        updatePaletteHighlight();
      });
    })(i);

    container.appendChild(item);
  }

  updatePaletteAriaActive();
}

function updatePaletteHighlight() {
  var items = document.querySelectorAll('.command-palette-item');
  for (var i = 0; i < items.length; i++) {
    items[i].classList.toggle('selected', parseInt(items[i].dataset.index) === _paletteSelected);
  }
  updatePaletteAriaActive();
  var sel = document.getElementById('palette-item-' + _paletteSelected);
  if (sel) sel.scrollIntoView({ block: 'nearest' });
}

function updatePaletteAriaActive() {
  var input = document.getElementById('palette-input');
  if (_paletteSelected >= 0) {
    input.setAttribute('aria-activedescendant', 'palette-item-' + _paletteSelected);
  } else {
    input.removeAttribute('aria-activedescendant');
  }
}

function executePaletteSelection() {
  if (_paletteSelected < 0 || _paletteSelected >= _paletteResults.length) return;
  var result = _paletteResults[_paletteSelected];
  closeCommandPalette();
  if (typeof result.action === 'function') result.action();
}

// Wire up palette events
(function() {
  var input = document.getElementById('palette-input');
  var overlay = document.getElementById('command-palette-overlay');

  input.addEventListener('input', function() {
    searchPalette(input.value);
  });

  input.addEventListener('keydown', function(e) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (_paletteResults.length > 0) {
        _paletteSelected = (_paletteSelected + 1) % _paletteResults.length;
        updatePaletteHighlight();
      }
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (_paletteResults.length > 0) {
        _paletteSelected = (_paletteSelected - 1 + _paletteResults.length) % _paletteResults.length;
        updatePaletteHighlight();
      }
    } else if (e.key === 'Enter') {
      e.preventDefault();
      executePaletteSelection();
    } else if (e.key === 'Tab') {
      e.preventDefault(); // focus trap
    }
  });

  overlay.addEventListener('click', function(e) {
    if (e.target === overlay) closeCommandPalette();
  });
})();
