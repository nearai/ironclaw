// --- Integrations (first-class surface) ---
//
// One place to see and manage what the agent is connected to. Three data
// sources merge here:
//   1. Real installed extensions (`/api/extensions`) — actual connection state.
//   2. The extension registry (`/api/extensions/registry`) — installable
//      channels/tools/MCP servers, reusing the same install flow as Discover.
//   3. MOCK: handoff state from the marketing-site onboarding
//      (`?integrations=gmail,slack` → sessionStorage, see landing.js) plus the
//      curated NUX_DATA.integrationCatalog. Mock-connected entries render with
//      a "Connected" badge even though no real install happened — prototype
//      glue so the onboarding demo flows end-to-end.

let _integrationsLoaded = false;

const INTEGRATION_KIND_LABELS = {
  wasm_channel: 'Channel',
  channel: 'Channel',
  channel_relay: 'Channel',
  wasm_tool: 'Tool',
  tool: 'Tool',
  native: 'Tool',
  mcp_server: 'MCP server',
};

function integrationCatalogEntry(id) {
  const catalog = (typeof NUX_DATA !== 'undefined' && NUX_DATA.integrationCatalog) || [];
  return catalog.find((entry) => entry.id === id) || null;
}

function loadIntegrations(force) {
  const availableGrid = document.getElementById('integrations-available-grid');
  if (!availableGrid) return;
  if (_integrationsLoaded && !force) return;
  availableGrid.innerHTML = renderCardsSkeleton(6);

  Promise.all([
    apiFetch('/api/extensions').catch(() => ({ extensions: [] })),
    apiFetch('/api/extensions/registry').catch(() => ({ entries: [] })),
  ]).then(([extData, registryData]) => {
    _integrationsLoaded = true;
    renderIntegrations(extData.extensions || [], registryData.entries || []);
  });
}

function integrationConnectionState(ext) {
  if (!ext) return null;
  const state = ext.onboarding_state || ext.activation_status;
  if (state === 'active' || state === 'ready' || ext.active) return 'connected';
  if (state === 'pairing' || state === 'pairing_required') return 'pairing';
  return 'setup';
}

function renderIntegrations(extensions, registryEntries) {
  const connectedSection = document.getElementById('integrations-connected-section');
  const connectedGrid = document.getElementById('integrations-connected-grid');
  const availableGrid = document.getElementById('integrations-available-grid');
  if (!connectedGrid || !availableGrid) return;

  const installedByName = new Map();
  extensions
    .filter((ext) => ext.kind !== 'skill')
    .forEach((ext) => installedByName.set(ext.name, ext));

  const mockConnected = typeof getHandoffConnectedIntegrations === 'function'
    ? getHandoffConnectedIntegrations()
    : [];

  const connectedItems = [];
  const availableItems = [];
  const seen = new Set();

  // Registry entries (channels, tools, MCP servers — not skills).
  registryEntries
    .filter((entry) => entry.kind !== 'skill')
    .forEach((entry) => {
      seen.add(entry.name);
      const installed = installedByName.get(entry.name) || null;
      const state = integrationConnectionState(installed);
      const catalog = integrationCatalogEntry(entry.name);
      const item = {
        id: entry.name,
        label: entry.display_name || (catalog && catalog.label) || entry.name,
        kind: entry.kind,
        blurb: entry.description || (catalog && catalog.blurb) || '',
        glyph: catalog && catalog.glyph,
        icon: catalog && catalog.icon,
        lucideIcon: entry.lucideIcon,
        installed,
        state,
        mock: false,
      };
      if (state === 'connected') connectedItems.push(item);
      else availableItems.push(item);
    });

  // Installed-but-off-registry extensions (custom MCP servers etc.).
  extensions
    .filter((ext) => ext.kind !== 'skill' && !seen.has(ext.name))
    .forEach((ext) => {
      seen.add(ext.name);
      const state = integrationConnectionState(ext);
      const item = {
        id: ext.name,
        label: ext.display_name || ext.name,
        kind: ext.kind,
        blurb: ext.description || '',
        installed: ext,
        state,
        mock: false,
      };
      if (state === 'connected') connectedItems.push(item);
      else availableItems.push(item);
    });

  // Curated catalog entries with no registry/installed match: shown as
  // "ask the agent to connect" cards, or — when the onboarding handoff says
  // they were connected — as MOCK connected entries.
  const catalog = (typeof NUX_DATA !== 'undefined' && NUX_DATA.integrationCatalog) || [];
  catalog.forEach((entry) => {
    if (seen.has(entry.id)) return;
    const isMockConnected = mockConnected.indexOf(entry.id) !== -1;
    const item = {
      id: entry.id,
      label: entry.label,
      kind: null,
      blurb: entry.blurb,
      glyph: entry.glyph,
      icon: entry.icon,
      installed: null,
      state: isMockConnected ? 'connected' : null,
      mock: isMockConnected,
    };
    if (isMockConnected) connectedItems.push(item);
    else availableItems.push(item);
  });

  // Handoff ids that matched registry entries: mark those connected too.
  mockConnected.forEach((id) => {
    const inAvailable = availableItems.findIndex((item) => item.id === id);
    if (inAvailable !== -1) {
      const item = availableItems.splice(inAvailable, 1)[0];
      item.state = 'connected';
      item.mock = true;
      connectedItems.push(item);
    }
  });

  connectedGrid.innerHTML = '';
  availableGrid.innerHTML = '';

  connectedItems.forEach((item) => connectedGrid.appendChild(renderIntegrationCard(item)));
  availableItems.forEach((item) => availableGrid.appendChild(renderIntegrationCard(item)));

  if (connectedSection) {
    connectedSection.style.display = connectedItems.length > 0 ? '' : 'none';
  }
  if (availableItems.length === 0) {
    availableGrid.innerHTML = '<div class="integrations-empty">'
      + '<div class="integrations-empty-title">' + escapeHtml(I18n.t('integrations.emptyTitle')) + '</div>'
      + '<div class="integrations-empty-hint">' + escapeHtml(I18n.t('integrations.emptyHint')) + '</div>'
      + '<button type="button" class="btn-secondary" data-action="integrations-ask">'
      + escapeHtml(I18n.t('integrations.askAgentGeneric')) + '</button>'
      + '</div>';
    availableGrid.querySelector('[data-action="integrations-ask"]')?.addEventListener('click', () => {
      switchTab('chat');
      prefillChatPrompt('Connect a new tool for me — ');
    });
  }
}

function renderIntegrationCard(item) {
  const card = document.createElement('div');
  card.className = 'integration-card' + (item.state === 'connected' ? ' connected' : '');

  const header = document.createElement('div');
  header.className = 'integration-card-header';

  if (item.icon) {
    // Real provider app mark (committed asset under /icons/integrations/),
    // full-square artwork rounded by CSS.
    const icon = document.createElement('img');
    icon.className = 'integration-icon';
    icon.src = item.icon;
    icon.alt = '';
    icon.setAttribute('aria-hidden', 'true');
    icon.loading = 'lazy';
    header.appendChild(icon);
  } else if (item.lucideIcon) {
    // Non-brand entries (HTTP, headless browser, custom MCP servers)
    // carry a purposeful lucide mark instead of a letter chip.
    const glyph = document.createElement('span');
    glyph.className = 'integration-glyph';
    glyph.setAttribute('aria-hidden', 'true');
    glyph.innerHTML = lucideGlyphSvg(item.lucideIcon, 16);
    header.appendChild(glyph);
  } else if (item.glyph) {
    const glyph = document.createElement('span');
    glyph.className = 'integration-glyph';
    glyph.setAttribute('aria-hidden', 'true');
    glyph.textContent = item.glyph;
    header.appendChild(glyph);
  } else {
    const fallback = document.createElement('span');
    fallback.className = 'integration-glyph';
    fallback.setAttribute('aria-hidden', 'true');
    fallback.innerHTML = lucideGlyphSvg('sparkles', 16);
    header.appendChild(fallback);
  }

  const name = document.createElement('span');
  name.className = 'integration-name';
  name.textContent = item.label;
  header.appendChild(name);

  // Action lives top-right (unified card anatomy); the kind tag moves
  // under the description as part of the tag list.
  const action = document.createElement('span');
  action.className = 'integration-card-action';
  if (item.state === 'connected') {
    const badge = document.createElement('span');
    badge.className = 'integration-badge connected';
    badge.textContent = '\u2713 ' + I18n.t('integrations.connectedBadge');
    if (item.mock) {
      // Make the prototype nature visible on hover without cluttering the UI.
      badge.title = I18n.t('integrations.mockConnectedHint');
    }
    action.appendChild(badge);
    // Demo-only: let the walkthrough disconnect a mock-connected tool so the
    // "manage your connections" beat is interactive end-to-end.
    if (window.__IRONCLAW_DEMO__) {
      const disconnectBtn = document.createElement('button');
      disconnectBtn.type = 'button';
      disconnectBtn.className = 'discover-btn secondary integration-disconnect';
      disconnectBtn.textContent = I18n.t('integrations.disconnect');
      disconnectBtn.addEventListener('click', () => {
        disconnectBtn.disabled = true;
        apiFetch('/api/extensions/uninstall', {
          method: 'POST',
          body: { name: item.id },
        }).then(() => {
          showToast(I18n.t('integrations.disconnected', { name: item.label }), 'success');
          loadIntegrations(true);
        }).catch(() => {
          disconnectBtn.disabled = false;
        });
      });
      action.appendChild(disconnectBtn);
    }
  } else if (item.state === 'pairing' || item.state === 'setup') {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'discover-btn secondary';
    btn.textContent = item.state === 'pairing'
      ? I18n.t('nux.enterPairingCode')
      : I18n.t('nux.finishSetup');
    btn.addEventListener('click', () => showConfigureModal(item.id));
    action.appendChild(btn);
  } else if (item.kind) {
    // Real registry entry: reuse the canonical install flow.
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'discover-btn primary';
    btn.textContent = I18n.t('integrations.connect');
    btn.addEventListener('click', () => installIntegration(item, btn));
    action.appendChild(btn);
  } else {
    // No registry entry — the agent can wire it up from chat.
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'discover-btn secondary';
    btn.textContent = I18n.t('integrations.askAgent');
    btn.addEventListener('click', () => {
      switchTab('chat');
      prefillChatPrompt('Connect ' + item.label + ' for me');
    });
    action.appendChild(btn);
  }
  header.appendChild(action);

  card.appendChild(header);

  if (item.blurb) {
    const desc = document.createElement('div');
    desc.className = 'integration-desc';
    desc.textContent = item.blurb;
    card.appendChild(desc);
  }

  // Tag list under the description (supports several capability tags).
  const tags = document.createElement('div');
  tags.className = 'card-tags';
  if (item.kind && INTEGRATION_KIND_LABELS[item.kind]) {
    const kindTag = document.createElement('span');
    kindTag.className = 'card-tag';
    kindTag.textContent = INTEGRATION_KIND_LABELS[item.kind];
    tags.appendChild(kindTag);
  }
  (item.tags || []).forEach((t) => {
    const tag = document.createElement('span');
    tag.className = 'card-tag';
    tag.textContent = t;
    tags.appendChild(tag);
  });
  if (tags.children.length > 0) card.appendChild(tags);

  return card;
}

function installIntegration(item, btn) {
  btn.disabled = true;
  btn.textContent = I18n.t('extensions.installing');
  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: item.id, kind: item.kind },
  }).then((res) => {
    if (res.success) {
      showToast(I18n.t('extensions.installedSuccess', { name: item.label }), 'success');
      if (res.auth_url) {
        showToast(I18n.t('extensions.openingAuth', { name: item.label }), 'info');
        openOAuthUrl(res.auth_url);
      } else if (item.kind === 'wasm_channel') {
        showConfigureModal(item.id);
      }
    } else {
      showToast(I18n.t('extensions.installFailed', { message: res.message || 'unknown error' }), 'error');
    }
    loadIntegrations(true);
  }).catch((err) => {
    showToast(I18n.t('extensions.installFailed', { message: err.message }), 'error');
    loadIntegrations(true);
  });
}
