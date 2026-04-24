// --- Command Palette (Cmd+K) ---

let _paletteOpen = false;
let _paletteResults = [];
let _paletteSelected = -1;
let _palettePreviouslyFocused = null;

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
  _palettePreviouslyFocused = document.activeElement;
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
  if (_palettePreviouslyFocused && typeof _palettePreviouslyFocused.focus === 'function') {
    _palettePreviouslyFocused.focus();
  }
  _palettePreviouslyFocused = null;
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
      category: 'Threads',
      icon: '\u{1F4AC}',
      title: t.title,
      desc: '',
      badge: t.channel !== 'gateway' ? t.channel : '',
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
    updatePaletteAriaActive();
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
    item.setAttribute('tabindex', '-1');
    item.setAttribute('aria-selected', i === _paletteSelected ? 'true' : 'false');
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

    if (r.badge) {
      var badge = document.createElement('span');
      badge.className = 'command-palette-item-badge';
      badge.textContent = r.badge;
      item.appendChild(badge);
    }

    container.appendChild(item);
  }

  updatePaletteAriaActive();
}

function updatePaletteHighlight() {
  var items = document.querySelectorAll('.command-palette-item');
  var active = document.activeElement;
  var activeIsItem = active && active.classList && active.classList.contains('command-palette-item');
  for (var i = 0; i < items.length; i++) {
    var selected = parseInt(items[i].dataset.index, 10) === _paletteSelected;
    items[i].classList.toggle('selected', selected);
    items[i].setAttribute('aria-selected', selected ? 'true' : 'false');
  }
  updatePaletteAriaActive();
  var sel = document.getElementById('palette-item-' + _paletteSelected);
  if (sel) {
    sel.scrollIntoView({ block: 'nearest' });
    if (activeIsItem && active !== sel) sel.focus();
  }
}

function updatePaletteAriaActive() {
  var input = document.getElementById('palette-input');
  if (_paletteSelected >= 0) {
    input.setAttribute('aria-activedescendant', 'palette-item-' + _paletteSelected);
  } else {
    input.removeAttribute('aria-activedescendant');
  }
}

function getPaletteFocusableElements() {
  var input = document.getElementById('palette-input');
  var items = Array.prototype.slice.call(document.querySelectorAll('#palette-results .command-palette-item'));
  return [input].concat(items).filter(Boolean);
}

function focusPaletteElement(index) {
  var input = document.getElementById('palette-input');
  if (index <= 0) {
    if (input) input.focus();
    return;
  }

  var itemIndex = index - 1;
  if (itemIndex < 0 || itemIndex >= _paletteResults.length) return;
  _paletteSelected = itemIndex;
  updatePaletteHighlight();
  var item = document.getElementById('palette-item-' + itemIndex);
  if (item) item.focus();
}

function cyclePaletteFocus(direction) {
  var focusable = getPaletteFocusableElements();
  if (focusable.length === 0) return;
  var currentIndex = focusable.indexOf(document.activeElement);
  if (currentIndex < 0) currentIndex = 0;
  var nextIndex = (currentIndex + direction + focusable.length) % focusable.length;
  focusPaletteElement(nextIndex);
}

function executePaletteSelection() {
  if (_paletteSelected < 0 || _paletteSelected >= _paletteResults.length) return;
  var result = _paletteResults[_paletteSelected];
  closeCommandPalette();
  if (typeof result.action === 'function') result.action();
}

function handlePaletteKeydown(e) {
  if (!_paletteOpen) return;

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
    e.preventDefault();
    cyclePaletteFocus(e.shiftKey ? -1 : 1);
  } else if (e.key === 'Escape') {
    e.preventDefault();
    closeCommandPalette();
  }
}

// Wire up palette events
(function() {
  var input = document.getElementById('palette-input');
  var overlay = document.getElementById('command-palette-overlay');
  var results = document.getElementById('palette-results');

  input.addEventListener('input', function() {
    searchPalette(input.value);
  });

  overlay.addEventListener('keydown', handlePaletteKeydown);

  results.addEventListener('click', function(e) {
    var item = e.target.closest('.command-palette-item');
    if (!item) return;
    var idx = parseInt(item.dataset.index, 10);
    if (isNaN(idx)) return;
    _paletteSelected = idx;
    executePaletteSelection();
  });

  results.addEventListener('mouseover', function(e) {
    var item = e.target.closest('.command-palette-item');
    if (!item) return;
    var idx = parseInt(item.dataset.index, 10);
    if (isNaN(idx) || idx === _paletteSelected) return;
    _paletteSelected = idx;
    updatePaletteHighlight();
  });

  results.addEventListener('focusin', function(e) {
    var item = e.target.closest('.command-palette-item');
    if (!item) return;
    var idx = parseInt(item.dataset.index, 10);
    if (isNaN(idx)) return;
    _paletteSelected = idx;
    updatePaletteHighlight();
  });

  overlay.addEventListener('click', function(e) {
    if (e.target === overlay) closeCommandPalette();
  });
})();
