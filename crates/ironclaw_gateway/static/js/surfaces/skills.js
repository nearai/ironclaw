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

function normalizeMcpServerName(raw) {
  return raw.toLowerCase()
    .replace(/[^a-z0-9_-]/g, '_')
    .replace(/_+/g, '_')
    .replace(/^_|_$/g, '');
}

function addMcpServer() {
  var rawName = document.getElementById('mcp-install-name').value.trim();
  if (!rawName) {
    showToast(I18n.t('mcp.serverNameRequired'), 'error');
    return;
  }
  var name = normalizeMcpServerName(rawName);
  if (!name) {
    showToast('Server name contains no valid characters', 'error');
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

function loadSkills() {
  var skillsList = document.getElementById('skills-list');
  skillsList.innerHTML = renderCardsSkeleton(3);
  apiFetch('/api/skills').then(function(data) {
    if (!data.skills || data.skills.length === 0) {
      skillsList.innerHTML = '<div class="empty-state">' + I18n.t('skills.noInstalled') + '</div>';
      return;
    }
    skillsList.innerHTML = '';
    for (var i = 0; i < data.skills.length; i++) {
      skillsList.appendChild(renderSkillCard(data.skills[i]));
    }
  }).catch(function(err) {
    skillsList.innerHTML = '<div class="empty-state">' + I18n.t('skills.loadFailed', {message: escapeHtml(err.message)}) + '</div>';
  });
}

function renderSkillCard(skill) {
  var card = document.createElement('div');
  card.className = 'ext-card state-active';

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = skill.name;
  header.appendChild(name);

  var trust = document.createElement('span');
  var trustClass = skill.trust.toLowerCase() === 'trusted' ? 'trust-trusted' : 'trust-installed';
  trust.className = 'skill-trust ' + trustClass;
  trust.textContent = skill.trust;
  header.appendChild(trust);

  var version = document.createElement('span');
  version.className = 'skill-version';
  version.textContent = 'v' + skill.version;
  header.appendChild(version);

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = skill.description;
  card.appendChild(desc);

  if (skill.keywords && skill.keywords.length > 0) {
    var kw = document.createElement('div');
    kw.className = 'ext-keywords';
    kw.textContent = I18n.t('skills.activatesOn') + ': ' + skill.keywords.join(', ');
    card.appendChild(kw);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  // Only show Remove for registry-installed skills, not user-placed trusted skills
  if (skill.trust.toLowerCase() !== 'trusted') {
    var removeBtn = document.createElement('button');
    removeBtn.className = 'btn-ext remove';
    removeBtn.textContent = I18n.t('skills.remove');
    removeBtn.addEventListener('click', function() { removeSkill(skill.name); });
    actions.appendChild(removeBtn);
  }

  card.appendChild(actions);
  return card;
}

function searchClawHub() {
  var input = document.getElementById('skill-search-input');
  var query = input.value.trim();
  if (!query) return;

  var resultsDiv = document.getElementById('skill-search-results');
  resultsDiv.innerHTML = '<div class="empty-state">' + I18n.t('skills.searching') + '</div>';

  apiFetch('/api/skills/search', {
    method: 'POST',
    body: { query: query },
  }).then(function(data) {
    resultsDiv.innerHTML = '';

    // Show registry error as a warning banner if present
    if (data.catalog_error) {
      var warning = document.createElement('div');
      warning.className = 'empty-state';
      warning.style.color = '#f0ad4e';
      warning.style.borderLeft = '3px solid #f0ad4e';
      warning.style.paddingLeft = '12px';
      warning.style.marginBottom = '16px';
      warning.textContent = I18n.t('skills.registryError', {message: data.catalog_error});
      resultsDiv.appendChild(warning);
    }

    // Show catalog results
    if (data.catalog && data.catalog.length > 0) {
      // Build a set of installed skill names for quick lookup
      var installedNames = {};
      if (data.installed) {
        for (var j = 0; j < data.installed.length; j++) {
          installedNames[data.installed[j].name] = true;
        }
      }

      for (var i = 0; i < data.catalog.length; i++) {
        var card = renderCatalogSkillCard(data.catalog[i], installedNames);
        card.style.animationDelay = (i * 0.06) + 's';
        resultsDiv.appendChild(card);
      }
    }

    // Show matching installed skills too
    if (data.installed && data.installed.length > 0) {
      for (var k = 0; k < data.installed.length; k++) {
        var installedCard = renderSkillCard(data.installed[k]);
        installedCard.style.animationDelay = ((data.catalog ? data.catalog.length : 0) + k) * 0.06 + 's';
        installedCard.classList.add('skill-search-result');
        resultsDiv.appendChild(installedCard);
      }
    }

    if (resultsDiv.children.length === 0) {
      resultsDiv.innerHTML = '<div class="empty-state">' + I18n.t('skills.noResults', {query: escapeHtml(query)}) + '</div>';
    }
  }).catch(function(err) {
    resultsDiv.innerHTML = '<div class="empty-state">' + I18n.t('skills.searchFailed', {message: escapeHtml(err.message)}) + '</div>';
  });
}

function renderCatalogSkillCard(entry, installedNames) {
  var card = document.createElement('div');
  card.className = 'ext-card ext-available skill-search-result';

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('a');
  name.className = 'ext-name';
  name.textContent = entry.name || entry.slug;
  name.href = 'https://clawhub.ai/skills/' + encodeURIComponent(entry.slug);
  name.target = '_blank';
  name.rel = 'noopener noreferrer';
  name.style.textDecoration = 'none';
  name.style.color = 'inherit';
  name.title = I18n.t('skills.viewOnClawHub');
  header.appendChild(name);

  if (entry.version) {
    var version = document.createElement('span');
    version.className = 'skill-version';
    version.textContent = 'v' + entry.version;
    header.appendChild(version);
  }

  card.appendChild(header);

  if (entry.description) {
    var desc = document.createElement('div');
    desc.className = 'ext-desc';
    desc.textContent = entry.description;
    card.appendChild(desc);
  }

  // Metadata row: owner, stars, downloads, recency
  var meta = document.createElement('div');
  meta.className = 'ext-meta';
  meta.style.fontSize = '11px';
  meta.style.color = '#888';
  meta.style.marginTop = '6px';

  function addMetaSep() {
    if (meta.children.length > 0) {
      meta.appendChild(document.createTextNode(' \u00b7 '));
    }
  }

  if (entry.owner) {
    var ownerSpan = document.createElement('span');
    ownerSpan.textContent = 'by ' + entry.owner;
    meta.appendChild(ownerSpan);
  }

  if (entry.stars != null) {
    addMetaSep();
    var starsSpan = document.createElement('span');
    starsSpan.textContent = entry.stars + ' stars';
    meta.appendChild(starsSpan);
  }

  if (entry.downloads != null) {
    addMetaSep();
    var dlSpan = document.createElement('span');
    dlSpan.textContent = formatCompactNumber(entry.downloads) + ' downloads';
    meta.appendChild(dlSpan);
  }

  if (entry.updatedAt) {
    var ago = formatTimeAgo(entry.updatedAt);
    if (ago) {
      addMetaSep();
      var updatedSpan = document.createElement('span');
      updatedSpan.textContent = 'updated ' + ago;
      meta.appendChild(updatedSpan);
    }
  }

  if (meta.children.length > 0) {
    card.appendChild(meta);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  var slug = entry.slug || entry.name;
  var slugSuffix = slug.indexOf('/') >= 0 ? slug.split('/').pop() : slug;
  var isInstalled = entry.installed || installedNames[entry.name] || installedNames[slug] || installedNames[slugSuffix];

  if (isInstalled) {
    var label = document.createElement('span');
    label.className = 'ext-active-label';
    label.textContent = I18n.t('status.installed');
    actions.appendChild(label);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-ext install';
    installBtn.textContent = I18n.t('extensions.install');
    installBtn.addEventListener('click', (function(displayName, slugValue, btn) {
      return function() {
        if (!confirm(I18n.t('skills.confirmInstallHub', { name: displayName }))) return;
        btn.disabled = true;
        btn.textContent = I18n.t('extensions.installing');
        installSkill(displayName, null, btn, slugValue);
      };
    })(entry.name || slug, slug, installBtn));
    actions.appendChild(installBtn);
  }

  card.appendChild(actions);
  return card;
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
        label.className = 'ext-active-label';
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
  if (!confirm(I18n.t('skills.confirmInstall', { name: name }))) return;
  installSkill(name, url, null);
  document.getElementById('skill-install-name').value = '';
  document.getElementById('skill-install-url').value = '';
}

// Wire up Enter key on search input
document.getElementById('skill-search-input').addEventListener('keydown', function(e) {
  if (e.key === 'Enter') searchClawHub();
});

// --- Tool Permissions ---

