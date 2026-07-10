// --- Discover (unified marketplace) ---
//
// One search & browse surface for everything the agent can plug into:
// channels, tools, MCP servers (extension registry) and skills (installed +
// ClawHub catalog). Replaces four separate Settings subtabs as the *front
// door* for discovery — the Settings views remain for management.
//
// "Ask the agent to build it" is a first-class result: anything missing from
// the registry is one click away from a chat prompt that has the agent build
// a new connector on the fly.

let _discoverCategory = 'all';
let _discoverQuery = '';
let _discoverItems = [];
let _discoverLoaded = false;
let _discoverSkillSearchTimer = null;
let _discoverSkillResults = [];

const DISCOVER_KIND_LABELS = {
  wasm_channel: 'Channel',
  channel: 'Channel',
  channel_relay: 'Channel',
  wasm_tool: 'Tool',
  tool: 'Tool',
  native: 'Tool',
  mcp_server: 'MCP server',
  skill: 'Skill',
};

function loadDiscover(force) {
  const grid = document.getElementById('discover-grid');
  if (!grid) return;
  if (_discoverLoaded && !force) {
    renderDiscoverGrid();
    return;
  }
  grid.innerHTML = renderCardsSkeleton(6);

  Promise.all([
    apiFetch('/api/extensions').catch(() => ({ extensions: [] })),
    apiFetch('/api/extensions/registry').catch(() => ({ entries: [] })),
    apiFetch('/api/skills').catch(() => ({ skills: [] })),
  ]).then(([extData, registryData, skillsData]) => {
    const installedByName = new Map();
    (extData.extensions || []).forEach((ext) => installedByName.set(ext.name, ext));

    const items = [];
    const seen = new Set();

    (registryData.entries || []).forEach((entry) => {
      const installed = installedByName.get(entry.name) || null;
      seen.add(entry.name);
      items.push({
        name: entry.name,
        displayName: entry.display_name || entry.name,
        kind: entry.kind,
        description: entry.description || '',
        keywords: entry.keywords || [],
        version: entry.version || null,
        installed,
      });
    });

    // Installed extensions that are not in the registry (custom MCP servers,
    // hand-installed tools) still belong in the catalog view.
    (extData.extensions || []).forEach((ext) => {
      if (seen.has(ext.name)) return;
      items.push({
        name: ext.name,
        displayName: ext.display_name || ext.name,
        kind: ext.kind,
        description: ext.description || '',
        keywords: [],
        version: ext.version || null,
        installed: ext,
      });
    });

    (skillsData.skills || []).forEach((skill) => {
      items.push({
        name: skill.name,
        displayName: skill.name,
        kind: 'skill',
        description: skill.description || '',
        keywords: skill.keywords || [],
        version: skill.version || null,
        installed: skill,
      });
    });

    _discoverItems = items;
    _discoverLoaded = true;
    renderDiscoverGrid();
  });
}

function discoverCategoryKinds() {
  const categories = (typeof NUX_DATA !== 'undefined' && NUX_DATA.discoverCategories) || [];
  const cat = categories.find((c) => c.id === _discoverCategory);
  return cat ? cat.kinds : null;
}

function discoverItemMatches(item, kinds, query) {
  if (kinds && kinds.indexOf(item.kind) === -1) return false;
  if (!query) return true;
  const haystack = (item.displayName + ' ' + item.name + ' ' + item.description + ' '
    + (item.keywords || []).join(' ')).toLowerCase();
  return haystack.indexOf(query) !== -1;
}

function renderDiscoverCategories() {
  const bar = document.getElementById('discover-categories');
  if (!bar || typeof NUX_DATA === 'undefined') return;
  bar.innerHTML = '';
  (NUX_DATA.discoverCategories || []).forEach((cat) => {
    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'discover-category' + (cat.id === _discoverCategory ? ' active' : '');
    chip.textContent = cat.label;
    chip.addEventListener('click', () => {
      _discoverCategory = cat.id;
      renderDiscoverCategories();
      renderDiscoverGrid();
    });
    bar.appendChild(chip);
  });
}

function renderDiscoverGrid() {
  const grid = document.getElementById('discover-grid');
  if (!grid) return;
  const kinds = discoverCategoryKinds();
  const query = _discoverQuery.trim().toLowerCase();

  const matches = _discoverItems.filter((item) => discoverItemMatches(item, kinds, query));

  // ClawHub catalog results ride along when searching skills (or everything).
  const includeSkills = !kinds || kinds.indexOf('skill') !== -1;
  const localNames = new Set(matches.map((item) => item.name));
  const remoteSkills = (includeSkills && query) ? _discoverSkillResults.filter((entry) => {
    const slug = entry.slug || entry.name || '';
    const suffix = slug.indexOf('/') >= 0 ? slug.split('/').pop() : slug;
    return !localNames.has(entry.name) && !localNames.has(slug) && !localNames.has(suffix);
  }) : [];

  grid.innerHTML = '';

  if (matches.length === 0 && remoteSkills.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'discover-empty';
    empty.textContent = query
      ? I18n.t('discover.noResults', { query: _discoverQuery.trim() })
      : I18n.t('discover.empty');
    grid.appendChild(empty);
  }

  matches.forEach((item) => grid.appendChild(renderDiscoverCard(item)));
  remoteSkills.forEach((entry) => grid.appendChild(renderDiscoverCatalogSkillCard(entry)));

  // Always present: the agent can build what the catalog lacks.
  grid.appendChild(renderDiscoverBuildCard(query));
}

function discoverStatusForItem(item) {
  if (!item.installed) return null;
  if (item.kind === 'skill') return 'active';
  const ext = item.installed;
  const state = ext.onboarding_state || ext.activation_status;
  if (state === 'active' || state === 'ready' || ext.active) return 'active';
  if (state === 'pairing' || state === 'pairing_required') return 'pairing';
  return 'setup';
}

// Icon resolution shared with Integrations: brand app mark → lucide mark →
// per-skill icon → sparkles.
function discoverCardIcon(item) {
  const catalog = typeof integrationCatalogEntry === 'function'
    ? integrationCatalogEntry(item.name)
    : null;
  if (catalog && catalog.icon) {
    return '<img class="card-app-icon" src="' + escapeHtml(catalog.icon) + '" alt="" aria-hidden="true">';
  }
  const lucide = item.lucideIcon
    || (item.kind === 'skill' && item.installed && item.installed.icon)
    || item.icon;
  return '<span class="card-glyph" aria-hidden="true">' + lucideGlyphSvg(lucide || 'sparkles', 16) + '</span>';
}

// Unified card anatomy (Discover + Integrations): icon, title(+version),
// ACTION in the top-right, description, then a tag list underneath.
function renderDiscoverCard(item) {
  const card = document.createElement('div');
  card.className = 'discover-card';

  const header = document.createElement('div');
  header.className = 'discover-card-header';
  header.innerHTML = discoverCardIcon(item);

  const name = document.createElement('span');
  name.className = 'discover-card-name';
  name.textContent = item.displayName;
  header.appendChild(name);

  if (item.version) {
    const version = document.createElement('span');
    version.className = 'card-version';
    version.textContent = 'v' + item.version;
    header.appendChild(version);
  }

  // Action sits top-right where the kind tag used to be.
  const action = document.createElement('span');
  action.className = 'discover-card-action';
  const status = discoverStatusForItem(item);
  if (status === 'active') {
    const label = document.createElement('span');
    label.className = 'discover-status active';
    label.textContent = item.kind === 'skill' ? I18n.t('status.installed') : I18n.t('ext.active');
    action.appendChild(label);
  } else if (status === 'pairing' || status === 'setup') {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'discover-btn secondary';
    btn.textContent = status === 'pairing'
      ? I18n.t('nux.enterPairingCode')
      : I18n.t('nux.finishSetup');
    btn.addEventListener('click', () => showConfigureModal(item.name));
    action.appendChild(btn);
  } else {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'discover-btn primary';
    btn.textContent = I18n.t('extensions.install');
    btn.addEventListener('click', () => installDiscoverItem(item, btn));
    action.appendChild(btn);
  }
  header.appendChild(action);

  card.appendChild(header);

  if (item.description) {
    const desc = document.createElement('div');
    desc.className = 'discover-card-desc';
    desc.textContent = item.description;
    card.appendChild(desc);
  }

  // Tags under the description — the kind plus any capability keywords
  // (entries exposing several skills/tools list them all here).
  const tags = document.createElement('div');
  tags.className = 'card-tags';
  const kindTag = document.createElement('span');
  kindTag.className = 'card-tag kind-' + item.kind;
  kindTag.textContent = DISCOVER_KIND_LABELS[item.kind] || item.kind;
  tags.appendChild(kindTag);
  (item.keywords || []).slice(0, 4).forEach((kw) => {
    const tag = document.createElement('span');
    tag.className = 'card-tag';
    tag.textContent = kw;
    tags.appendChild(tag);
  });
  card.appendChild(tags);

  return card;
}

function installDiscoverItem(item, btn) {
  btn.disabled = true;
  btn.textContent = I18n.t('extensions.installing');
  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: item.name, kind: item.kind },
  }).then((res) => {
    if (res.success) {
      showToast(I18n.t('extensions.installedSuccess', { name: item.displayName }), 'success');
      if (res.auth_url) {
        showToast(I18n.t('extensions.openingAuth', { name: item.displayName }), 'info');
        openOAuthUrl(res.auth_url);
      } else if (item.kind === 'wasm_channel') {
        showConfigureModal(item.name);
      }
    } else {
      showToast(I18n.t('extensions.installFailed', { message: res.message || 'unknown error' }), 'error');
    }
    loadDiscover(true);
  }).catch((err) => {
    showToast(I18n.t('extensions.installFailed', { message: err.message }), 'error');
    loadDiscover(true);
  });
}

function renderDiscoverCatalogSkillCard(entry) {
  const card = document.createElement('div');
  card.className = 'discover-card';

  const header = document.createElement('div');
  header.className = 'discover-card-header';
  header.innerHTML = '<span class="card-glyph" aria-hidden="true">' + lucideGlyphSvg(entry.icon || 'sparkles', 16) + '</span>';

  const name = document.createElement('span');
  name.className = 'discover-card-name';
  name.textContent = entry.name || entry.slug;
  header.appendChild(name);

  if (entry.version) {
    const version = document.createElement('span');
    version.className = 'card-version';
    version.textContent = 'v' + entry.version;
    header.appendChild(version);
  }

  // Install action top-right, matching the unified card anatomy.
  const action = document.createElement('span');
  action.className = 'discover-card-action';
  const slug = entry.slug || entry.name;
  const installBtn = document.createElement('button');
  installBtn.type = 'button';
  installBtn.className = 'discover-btn primary';
  installBtn.textContent = I18n.t('extensions.install');
  installBtn.addEventListener('click', () => {
    installBtn.disabled = true;
    installBtn.textContent = I18n.t('extensions.installing');
    // Reuses the skills surface installer (toast + installed-state handling);
    // refresh once the install request actually settles.
    installSkill(entry.name || slug, null, installBtn, slug)
      .then(() => loadDiscover(true));
  });
  action.appendChild(installBtn);
  header.appendChild(action);

  card.appendChild(header);

  if (entry.description) {
    const desc = document.createElement('div');
    desc.className = 'discover-card-desc';
    desc.textContent = entry.description;
    card.appendChild(desc);
  }

  const tags = document.createElement('div');
  tags.className = 'card-tags';
  tags.innerHTML = '<span class="card-tag kind-skill">' + escapeHtml(DISCOVER_KIND_LABELS.skill) + '</span>'
    + '<span class="card-tag">ClawHub</span>';
  card.appendChild(tags);

  return card;
}

function renderDiscoverBuildCard(query) {
  const card = document.createElement('button');
  card.type = 'button';
  card.className = 'discover-card discover-build-card';

  const header = document.createElement('div');
  header.className = 'discover-card-header';
  const name = document.createElement('span');
  name.className = 'discover-card-name';
  name.textContent = I18n.t('discover.buildTitle');
  header.appendChild(name);
  card.appendChild(header);

  const desc = document.createElement('div');
  desc.className = 'discover-card-desc';
  desc.textContent = I18n.t('discover.buildDesc');
  card.appendChild(desc);

  card.addEventListener('click', () => {
    switchTab('chat');
    prefillChatPrompt(query
      ? I18n.t('discover.buildPrompt', { query: query })
      : I18n.t('discover.buildPromptBlank'));
  });

  return card;
}

// --- Wiring ---

(function wireDiscoverUI() {
  const search = document.getElementById('discover-search-input');
  if (search) {
    search.addEventListener('input', () => {
      _discoverQuery = search.value;
      renderDiscoverGrid();
      // Remote ClawHub search is debounced — local results filter instantly.
      if (_discoverSkillSearchTimer) clearTimeout(_discoverSkillSearchTimer);
      const query = _discoverQuery.trim();
      if (!query) {
        _discoverSkillResults = [];
        return;
      }
      _discoverSkillSearchTimer = setTimeout(() => {
        apiFetch('/api/skills/search', {
          method: 'POST',
          body: { query },
        }).then((data) => {
          if (_discoverQuery.trim() !== query) return; // stale response
          _discoverSkillResults = (data && data.catalog) || [];
          renderDiscoverGrid();
        }).catch(() => {});
      }, 350);
    });
  }
  renderDiscoverCategories();
})();
