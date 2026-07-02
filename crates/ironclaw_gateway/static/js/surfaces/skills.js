function installWasmExtension() {
  var name = document.getElementById('wasm-install-name').value.trim();
  if (!name) {
    showToast(I18n.t('extensions.nameRequired'), 'error');
    return;
  }
  var url = document.getElementById('wasm-install-url').value.trim();
  if (!url) {
    showToast(I18n.t('extensions.urlRequired'), 'error');
    return;
  }

  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: name, url: url, kind: 'wasm_tool' },
  }).then(function(res) {
    if (res.success) {
      showToast(I18n.t('extensions.installedName', { name: name }), 'success');
      document.getElementById('wasm-install-name').value = '';
      document.getElementById('wasm-install-url').value = '';
      loadExtensions();
    } else {
      showToast(I18n.t('extensions.installFailed', { message: res.message || 'unknown error' }), 'error');
    }
  }).catch(function(err) {
    showToast(I18n.t('extensions.installFailed', { message: err.message }), 'error');
  });
}

function addMcpServer() {
  var name = document.getElementById('mcp-install-name').value.trim();
  if (!name) {
    showToast(I18n.t('mcp.serverNameRequired'), 'error');
    return;
  }
  var url = document.getElementById('mcp-install-url').value.trim();
  if (!url) {
    showToast(I18n.t('mcp.urlRequired'), 'error');
    return;
  }

  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: name, url: url, kind: 'mcp_server' },
  }).then(function(res) {
    if (res.success) {
      showToast(I18n.t('mcp.added', { name: name }), 'success');
      document.getElementById('mcp-install-name').value = '';
      document.getElementById('mcp-install-url').value = '';
      loadMcpServers();
    } else {
      showToast(I18n.t('mcp.addFailed', { message: res.message || 'unknown error' }), 'error');
    }
  }).catch(function(err) {
    showToast(I18n.t('mcp.addFailed', { message: err.message }), 'error');
  });
}

// --- Skills ---

// Trust is optional on the wire (older gateways / registry entries may omit
// it) — normalize instead of crashing on `undefined.toLowerCase()`.
function skillTrustLabel(skill) {
  return String((skill && skill.trust) || 'Installed');
}

// Each skill carries a purposeful lucide mark (`icon` on the wire) —
// distinct per skill, sparkle only as the fallback.
function skillGlyphSvg(skill) {
  return lucideGlyphSvg((skill && skill.icon) || 'sparkles', 15);
}

function loadSkills() {
  var skillsList = document.getElementById('skills-list');
  skillsList.innerHTML = renderCardsSkeleton(3);
  apiFetch('/api/skills').then(function(data) {
    var skills = (data && data.skills) || [];
    setSlashSkillEntries(skills);
    var count = document.getElementById('skills-installed-count');
    if (count) count.textContent = skills.length > 0 ? String(skills.length) : '';
    if (skills.length === 0) {
      skillsList.innerHTML = '<div class="skills-empty">'
        + '<div class="skills-empty-title">' + escapeHtml(I18n.t('skills.noInstalled')) + '</div>'
        + '<div class="skills-empty-hint">' + escapeHtml(I18n.t('skills.noInstalledHint')) + '</div>'
        + '</div>';
      return;
    }
    skillsList.innerHTML = '';
    for (var i = 0; i < skills.length; i++) {
      skillsList.appendChild(renderSkillRow(skills[i]));
    }
  }).catch(function(err) {
    skillsList.innerHTML = '<div class="empty-state">' + I18n.t('skills.loadFailed', {message: escapeHtml(err.message)}) + '</div>';
  });
  // Populate discovery with the featured shelf (empty query).
  runClawHubSearch('');
}

// One row per skill: glyph, name + badges, description, hover actions.
function renderSkillRow(skill) {
  var trustLabel = skillTrustLabel(skill);
  var trusted = trustLabel.toLowerCase() === 'trusted';

  var row = document.createElement('div');
  row.className = 'skill-row';

  var glyph = document.createElement('span');
  glyph.className = 'skill-row-glyph';
  glyph.innerHTML = skillGlyphSvg(skill);
  row.appendChild(glyph);

  var main = document.createElement('div');
  main.className = 'skill-row-main';

  var head = document.createElement('div');
  head.className = 'skill-row-head';
  var name = document.createElement('span');
  name.className = 'skill-row-name';
  name.textContent = skill.name;
  head.appendChild(name);
  if (skill.version) {
    var version = document.createElement('span');
    version.className = 'skill-version';
    version.textContent = 'v' + skill.version;
    head.appendChild(version);
  }
  var trust = document.createElement('span');
  trust.className = 'skill-trust ' + (trusted ? 'trust-trusted' : 'trust-installed');
  trust.textContent = trustLabel;
  head.appendChild(trust);
  main.appendChild(head);

  if (skill.description) {
    var desc = document.createElement('div');
    desc.className = 'skill-row-desc';
    desc.textContent = skill.description;
    main.appendChild(desc);
  }

  var metaParts = [];
  if (skill.keywords && skill.keywords.length > 0) {
    metaParts.push(I18n.t('skills.activatesOn') + ': ' + skill.keywords.join(', '));
  }
  if (skill.usage_hint) metaParts.push(skill.usage_hint);
  if (skill.install_source_url) metaParts.push(I18n.t('skills.installedFrom', { url: skill.install_source_url }));
  if (metaParts.length > 0) {
    var meta = document.createElement('div');
    meta.className = 'skill-row-meta';
    meta.textContent = metaParts.join('  \u00b7  ');
    main.appendChild(meta);
  }

  row.appendChild(main);

  var actions = document.createElement('div');
  actions.className = 'skill-row-actions';
  // User-placed trusted skills are managed on disk, not removable here.
  if (!trusted) {
    var removeBtn = document.createElement('button');
    removeBtn.className = 'skill-row-remove';
    removeBtn.textContent = I18n.t('skills.remove');
    removeBtn.addEventListener('click', function() { removeSkill(skill.name); });
    actions.appendChild(removeBtn);
  }
  row.appendChild(actions);
  return row;
}

function searchClawHub() {
  var input = document.getElementById('skill-search-input');
  runClawHubSearch(input ? input.value.trim() : '');
}

// Shared by the toolbar search and the default featured shelf (query '').
function runClawHubSearch(query) {
  var resultsDiv = document.getElementById('skill-search-results');
  var title = document.getElementById('skills-results-title');
  if (!resultsDiv) return;
  if (title) {
    title.textContent = query
      ? I18n.t('skills.resultsFor', { query: query })
      : I18n.t('skills.featured');
  }
  resultsDiv.innerHTML = '<div class="empty-state">' + I18n.t('skills.searching') + '</div>';

  apiFetch('/api/skills/search', {
    method: 'POST',
    body: { query: query },
  }).then(function(data) {
    resultsDiv.innerHTML = '';

    // Show registry error as a warning banner if present
    if (data.catalog_error) {
      var warning = document.createElement('div');
      warning.className = 'skills-registry-warning';
      warning.textContent = I18n.t('skills.registryError', {message: data.catalog_error});
      resultsDiv.appendChild(warning);
    }

    if (data.catalog && data.catalog.length > 0) {
      var installedNames = {};
      if (data.installed) {
        for (var j = 0; j < data.installed.length; j++) {
          installedNames[data.installed[j].name] = true;
        }
      }
      for (var i = 0; i < data.catalog.length; i++) {
        resultsDiv.appendChild(renderCatalogSkillRow(data.catalog[i], installedNames));
      }
    }

    // Matching installed skills surface inline for search queries.
    if (query && data.installed && data.installed.length > 0) {
      for (var k = 0; k < data.installed.length; k++) {
        resultsDiv.appendChild(renderSkillRow(data.installed[k]));
      }
    }

    if (resultsDiv.children.length === 0) {
      resultsDiv.innerHTML = '<div class="skills-empty">'
        + '<div class="skills-empty-title">' + escapeHtml(I18n.t('skills.noResults', {query: query})) + '</div>'
        + '<div class="skills-empty-hint">' + escapeHtml(I18n.t('skills.noResultsHint')) + '</div>'
        + '</div>';
    }
  }).catch(function(err) {
    resultsDiv.innerHTML = '<div class="empty-state">' + I18n.t('skills.searchFailed', {message: escapeHtml(err.message)}) + '</div>';
  });
}

function renderCatalogSkillRow(entry, installedNames) {
  var row = document.createElement('div');
  row.className = 'skill-row skill-row-catalog';

  var glyph = document.createElement('span');
  glyph.className = 'skill-row-glyph';
  glyph.innerHTML = skillGlyphSvg(entry);
  row.appendChild(glyph);

  var main = document.createElement('div');
  main.className = 'skill-row-main';

  var head = document.createElement('div');
  head.className = 'skill-row-head';
  var name = document.createElement('a');
  name.className = 'skill-row-name skill-row-link';
  name.textContent = entry.name || entry.slug;
  name.href = 'https://clawhub.ai/skills/' + encodeURIComponent(entry.slug);
  name.target = '_blank';
  name.rel = 'noopener noreferrer';
  name.title = I18n.t('skills.viewOnClawHub');
  head.appendChild(name);
  if (entry.version) {
    var version = document.createElement('span');
    version.className = 'skill-version';
    version.textContent = 'v' + entry.version;
    head.appendChild(version);
  }
  main.appendChild(head);

  if (entry.description) {
    var desc = document.createElement('div');
    desc.className = 'skill-row-desc';
    desc.textContent = entry.description;
    main.appendChild(desc);
  }

  var metaParts = [];
  if (entry.owner) metaParts.push('by ' + entry.owner);
  if (entry.stars != null) metaParts.push('\u2605 ' + formatCompactNumber(entry.stars));
  if (entry.downloads != null) metaParts.push(formatCompactNumber(entry.downloads) + ' installs');
  if (entry.updatedAt) {
    var ago = formatTimeAgo(entry.updatedAt);
    if (ago) metaParts.push('updated ' + ago);
  }
  if (metaParts.length > 0) {
    var meta = document.createElement('div');
    meta.className = 'skill-row-meta';
    meta.textContent = metaParts.join('  \u00b7  ');
    main.appendChild(meta);
  }

  row.appendChild(main);

  var actions = document.createElement('div');
  actions.className = 'skill-row-actions skill-row-actions-static';

  var slug = entry.slug || entry.name;
  var slugSuffix = slug.indexOf('/') >= 0 ? slug.split('/').pop() : slug;
  var isInstalled = entry.installed || installedNames[entry.name] || installedNames[slug] || installedNames[slugSuffix];

  if (isInstalled) {
    var label = document.createElement('span');
    label.className = 'skill-row-installed';
    label.innerHTML = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12"/></svg>'
      + escapeHtml(I18n.t('status.installed'));
    actions.appendChild(label);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-primary skill-row-install';
    installBtn.textContent = I18n.t('extensions.install');
    installBtn.addEventListener('click', (function(displayName, slugValue, btn) {
      return function() {
        btn.disabled = true;
        btn.textContent = I18n.t('extensions.installing');
        installSkill(displayName, null, btn, slugValue);
      };
    })(entry.name || slug, slug, installBtn));
    actions.appendChild(installBtn);
  }

  row.appendChild(actions);
  return row;
}

function formatCompactNumber(n) {
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
  return '' + n;
}

function formatTimeAgo(epochMs) {
  var now = Date.now();
  var diff = now - epochMs;
  if (diff < 0) return null;
  var minutes = Math.floor(diff / 60000);
  if (minutes < 60) return minutes <= 1 ? 'just now' : minutes + 'm ago';
  var hours = Math.floor(minutes / 60);
  if (hours < 24) return hours + 'h ago';
  var days = Math.floor(hours / 24);
  if (days < 30) return days + 'd ago';
  var months = Math.floor(days / 30);
  if (months < 12) return months + 'mo ago';
  return Math.floor(months / 12) + 'y ago';
}

function installSkill(name, url, btn, slug) {
  var body = { name: name };
  if (slug) body.slug = slug;
  if (url) body.url = url;

  apiFetch('/api/skills/install', {
    method: 'POST',
    headers: { 'X-Confirm-Action': 'true' },
    body: body,
  }).then(function(res) {
    if (res.success) {
      showToast(I18n.t('skills.installedSuccess', {name: name}), 'success');
      if (btn && btn.parentNode) {
        var label = document.createElement('span');
        label.className = 'skill-row-installed';
        label.textContent = I18n.t('status.installed');
        btn.parentNode.innerHTML = '';
        btn.parentNode.appendChild(label);
      }
    } else {
      showToast(I18n.t('extensions.installFailed', { message: res.message || 'unknown error' }), 'error');
    }
    loadSkills();
    if (btn && !res.success) { btn.disabled = false; btn.textContent = I18n.t('extensions.install'); }
  }).catch(function(err) {
    showToast(I18n.t('extensions.installFailed', { message: err.message }), 'error');
    if (btn) { btn.disabled = false; btn.textContent = I18n.t('extensions.install'); }
  });
}

function removeSkill(name) {
  showConfirmModal(I18n.t('skills.confirmRemove', { name: name }), '', function() {
    apiFetch('/api/skills/' + encodeURIComponent(name), {
      method: 'DELETE',
      headers: { 'X-Confirm-Action': 'true' },
    }).then(function(res) {
      if (res.success) {
        showToast(I18n.t('skills.removed', { name: name }), 'success');
      } else {
        showToast(I18n.t('skills.removeFailed', { message: res.message || 'unknown error' }), 'error');
      }
      loadSkills();
    }).catch(function(err) {
      showToast(I18n.t('skills.removeFailed', { message: err.message }), 'error');
    });
  }, I18n.t('common.remove'), 'btn-danger');
}

function installSkillFromForm() {
  var name = document.getElementById('skill-install-name').value.trim();
  if (!name) { showToast(I18n.t('skills.nameRequired'), 'error'); return; }
  var url = document.getElementById('skill-install-url').value.trim() || null;
  if (url && !url.startsWith('https://')) {
    showToast(I18n.t('skills.httpsRequired'), 'error');
    return;
  }
  installSkill(name, url, null);
  document.getElementById('skill-install-name').value = '';
  document.getElementById('skill-install-url').value = '';
  closeSkillUrlModal();
}

// --- Install-from-URL modal ---

function openSkillUrlModal() {
  var overlay = document.getElementById('skills-url-overlay');
  if (!overlay) return;
  overlay.style.display = 'flex';
  var name = document.getElementById('skill-install-name');
  if (name) name.focus();
}

function closeSkillUrlModal() {
  var overlay = document.getElementById('skills-url-overlay');
  if (overlay) overlay.style.display = 'none';
}

document.getElementById('skill-install-url-open')?.addEventListener('click', openSkillUrlModal);
document.getElementById('skills-url-close')?.addEventListener('click', closeSkillUrlModal);
document.getElementById('skills-url-cancel')?.addEventListener('click', closeSkillUrlModal);
document.getElementById('skills-url-overlay')?.addEventListener('click', function(e) {
  if (e.target === this) closeSkillUrlModal();
});
document.addEventListener('keydown', function(e) {
  if (e.key === 'Escape') closeSkillUrlModal();
});

// Wire up Enter key on search input
document.getElementById('skill-search-input').addEventListener('keydown', function(e) {
  if (e.key === 'Enter') searchClawHub();
});

// --- Tool Permissions ---
