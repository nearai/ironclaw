let memorySearchTimeout = null;
let currentMemoryPath = null;
let currentMemoryContent = null;
// Tree state: nested nodes persisted across renders
// { name, path, is_dir, children: [] | null, expanded: bool, loaded: bool }
let memoryTreeState = null;

document.getElementById('memory-search').addEventListener('input', (e) => {
  clearTimeout(memorySearchTimeout);
  const query = e.target.value.trim();
  if (!query) {
    loadMemoryTree();
    return;
  }
  memorySearchTimeout = setTimeout(() => searchMemory(query), 300);
});

function loadMemoryTree() {
  // Only load top-level on first load (or refresh)
  apiFetch('/api/memory/list?path=').then((data) => {
    memoryTreeState = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    renderTree();
  }).catch(() => {});
}

function renderTree() {
  const container = document.getElementById('memory-tree');
  container.innerHTML = '';
  if (!memoryTreeState || memoryTreeState.length === 0) {
    container.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No files in workspace</div>';
    return;
  }
  renderNodes(memoryTreeState, container, 0);
}

// Small inline file-type icons (lucide-style strokes) for the tree.
function memoryFileIconSvg(node) {
  const stroke = 'fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"';
  if (node.is_dir) {
    return node.expanded
      ? '<svg width="14" height="14" viewBox="0 0 24 24" ' + stroke + '><path d="m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2"/></svg>'
      : '<svg width="14" height="14" viewBox="0 0 24 24" ' + stroke + '><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>';
  }
  const name = String(node.name || '').toLowerCase();
  if (name.endsWith('.md')) {
    return '<svg width="14" height="14" viewBox="0 0 24 24" ' + stroke + '><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/></svg>';
  }
  if (name.endsWith('.csv') || name.endsWith('.tsv')) {
    return '<svg width="14" height="14" viewBox="0 0 24 24" ' + stroke + '><path d="M12 3v18"/><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/></svg>';
  }
  if (name.endsWith('.json') || name.endsWith('.js') || name.endsWith('.toml') || name.endsWith('.yaml') || name.endsWith('.yml')) {
    return '<svg width="14" height="14" viewBox="0 0 24 24" ' + stroke + '><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>';
  }
  return '<svg width="14" height="14" viewBox="0 0 24 24" ' + stroke + '><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/></svg>';
}

function renderNodes(nodes, container, depth) {
  for (const node of nodes) {
    const row = document.createElement('div');
    row.className = 'tree-row'
      + (!node.is_dir && node.path === currentMemoryPath ? ' active' : '');
    row.style.paddingLeft = (depth * 14 + 8) + 'px';
    row.tabIndex = 0;
    row.setAttribute('role', 'treeitem');

    if (node.is_dir) {
      row.setAttribute('aria-expanded', node.expanded ? 'true' : 'false');
      const arrow = document.createElement('span');
      arrow.className = 'expand-arrow' + (node.expanded ? ' expanded' : '');
      arrow.textContent = '\u25B8';
      row.appendChild(arrow);
    } else {
      const spacer = document.createElement('span');
      spacer.className = 'expand-arrow-spacer';
      row.appendChild(spacer);
    }

    const icon = document.createElement('span');
    icon.className = 'tree-icon' + (node.is_dir ? ' dir' : '');
    icon.innerHTML = memoryFileIconSvg(node);
    row.appendChild(icon);

    const label = document.createElement('span');
    label.className = 'tree-label ' + (node.is_dir ? 'dir' : 'file');
    label.textContent = node.name;
    row.appendChild(label);

    if (node.is_dir) {
      row.addEventListener('click', () => toggleExpand(node));
      row.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleExpand(node); }
      });
    } else {
      row.addEventListener('click', () => readMemoryFile(node.path));
      row.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); readMemoryFile(node.path); }
      });
    }

    container.appendChild(row);

    if (node.is_dir && node.expanded && node.children) {
      const childContainer = document.createElement('div');
      childContainer.className = 'tree-children';
      renderNodes(node.children, childContainer, depth + 1);
      container.appendChild(childContainer);
    }
  }
}

function toggleExpand(node) {
  if (node.expanded) {
    node.expanded = false;
    renderTree();
    return;
  }

  if (node.loaded) {
    node.expanded = true;
    renderTree();
    return;
  }

  // Lazy-load children
  apiFetch('/api/memory/list?path=' + encodeURIComponent(node.path)).then((data) => {
    node.children = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    node.loaded = true;
    node.expanded = true;
    renderTree();
  }).catch(() => {});
}

function readMemoryFile(path) {
  currentMemoryPath = path;
  updateHash();
  document.getElementById('memory-breadcrumb-path').innerHTML = buildBreadcrumb(path);

  // Exit edit mode if active
  cancelMemoryEdit();

  apiFetch('/api/memory/read?path=' + encodeURIComponent(path)).then((data) => {
    currentMemoryContent = data.content;
    renderMemoryViewer(path, data.content);
    syncMemoryFilebar();
    renderTree(); // refresh active-row highlight
  }).catch((err) => {
    currentMemoryContent = null;
    document.getElementById('memory-viewer').innerHTML = '<div class="memory-welcome"><div class="memory-welcome-title">' + escapeHtml(err.message) + '</div></div>';
    syncMemoryFilebar();
  });
}

function renderMemoryViewer(path, content) {
  const viewer = document.getElementById('memory-viewer');
  if (path.endsWith('.md')) {
    viewer.innerHTML = '<div class="memory-rendered">' + renderMarkdown(content) + '</div>';
    viewer.classList.add('rendered');
  } else {
    viewer.textContent = content;
    viewer.classList.remove('rendered');
  }
}

// --- Edit / preview mode ---
//
// View mode:   viewer + [Edit]
// Edit mode:   textarea + [Save] [Cancel]; .md files additionally get an
//              Edit/Preview segmented toggle (preview renders the draft).
let _memoryEditing = false;
let _memoryPreviewingDraft = false;

function syncMemoryFilebar() {
  const isMd = !!currentMemoryPath && currentMemoryPath.endsWith('.md');
  // Content is null when the read failed — no Edit button for a file we
  // couldn't load (startMemoryEdit would be a dead click).
  document.getElementById('memory-edit-btn').style.display =
    !_memoryEditing && currentMemoryPath && currentMemoryContent != null ? '' : 'none';
  document.getElementById('memory-save-btn').style.display = _memoryEditing ? '' : 'none';
  document.getElementById('memory-cancel-btn').style.display = _memoryEditing ? '' : 'none';
  const toggle = document.getElementById('memory-mode-toggle');
  toggle.style.display = _memoryEditing && isMd ? '' : 'none';
  if (_memoryEditing && isMd) {
    document.getElementById('memory-mode-edit').classList.toggle('active', !_memoryPreviewingDraft);
    document.getElementById('memory-mode-preview').classList.toggle('active', _memoryPreviewingDraft);
  }
  document.getElementById('memory-viewer').style.display =
    _memoryEditing && !_memoryPreviewingDraft ? 'none' : '';
  document.getElementById('memory-editor').style.display =
    _memoryEditing && !_memoryPreviewingDraft ? 'flex' : 'none';
}

function startMemoryEdit() {
  if (!currentMemoryPath || currentMemoryContent === null) return;
  _memoryEditing = true;
  _memoryPreviewingDraft = false;
  const textarea = document.getElementById('memory-edit-textarea');
  textarea.value = currentMemoryContent;
  syncMemoryFilebar();
  textarea.focus();
}

function cancelMemoryEdit() {
  _memoryEditing = false;
  _memoryPreviewingDraft = false;
  if (currentMemoryPath && currentMemoryContent !== null) {
    renderMemoryViewer(currentMemoryPath, currentMemoryContent);
  }
  syncMemoryFilebar();
}

function saveMemoryEdit() {
  if (!currentMemoryPath) return;
  const textarea = document.getElementById('memory-edit-textarea');
  const content = textarea.value;
  apiFetch('/api/memory/write', {
    method: 'POST',
    body: { path: currentMemoryPath, content: content },
  }).then(() => {
    showToast(I18n.t('memory.savedPath', { path: currentMemoryPath }), 'success');
    _memoryEditing = false;
    _memoryPreviewingDraft = false;
    readMemoryFile(currentMemoryPath);
  }).catch((err) => {
    showToast(I18n.t('memory.saveFailed', { message: err.message }), 'error');
  });
}

// Draft preview toggle (edit mode, .md only).
document.getElementById('memory-mode-preview')?.addEventListener('click', () => {
  if (!_memoryEditing) return;
  _memoryPreviewingDraft = true;
  renderMemoryViewer(currentMemoryPath, document.getElementById('memory-edit-textarea').value);
  syncMemoryFilebar();
});
document.getElementById('memory-mode-edit')?.addEventListener('click', () => {
  if (!_memoryEditing) return;
  _memoryPreviewingDraft = false;
  syncMemoryFilebar();
});

function buildBreadcrumb(path) {
  const parts = path.split('/');
  let html = '<a data-action="breadcrumb-root" href="#">workspace</a>';
  let current = '';
  for (const part of parts) {
    current += (current ? '/' : '') + part;
    html += ' / <a data-action="breadcrumb-file" data-path="' + escapeHtml(current) + '" href="#">' + escapeHtml(part) + '</a>';
  }
  return html;
}

function searchMemory(query) {
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) return;

  apiFetch('/api/memory/search', {
    method: 'POST',
    body: { query: normalizedQuery, limit: 20 },
  }).then((data) => {
    const tree = document.getElementById('memory-tree');
    tree.innerHTML = '';
    if (data.results.length === 0) {
      tree.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No results</div>';
      return;
    }
    for (const result of data.results) {
      const item = document.createElement('div');
      item.className = 'search-result';
      const snippet = snippetAround(result.content, normalizedQuery, 120);
      item.innerHTML = '<div class="path">' + escapeHtml(result.path) + '</div>'
        + '<div class="snippet">' + highlightQuery(snippet, normalizedQuery) + '</div>';
      item.addEventListener('click', () => readMemoryFile(result.path));
      tree.appendChild(item);
    }
  }).catch(() => {});
}

function normalizeSearchQuery(query) {
  return (typeof query === 'string' ? query : '').slice(0, MEMORY_SEARCH_QUERY_MAX_LENGTH);
}

function snippetAround(text, query, len) {
  const normalizedQuery = normalizeSearchQuery(query);
  const lower = text.toLowerCase();
  const idx = lower.indexOf(normalizedQuery.toLowerCase());
  if (idx < 0) return text.substring(0, len);
  const start = Math.max(0, idx - Math.floor(len / 2));
  const end = Math.min(text.length, start + len);
  let s = text.substring(start, end);
  if (start > 0) s = '...' + s;
  if (end < text.length) s = s + '...';
  return s;
}

function highlightQuery(text, query) {
  if (!query) return escapeHtml(text);
  const escaped = escapeHtml(text);
  const normalizedQuery = normalizeSearchQuery(query);
  const queryEscaped = normalizedQuery.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const re = new RegExp('(' + queryEscaped + ')', 'gi');
  return escaped.replace(re, '<mark>$1</mark>');
}
// --- Logs ---

