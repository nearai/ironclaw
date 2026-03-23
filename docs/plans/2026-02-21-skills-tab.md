# Skills Tab Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a Skills tab to the IronClaw web UI for browsing installed skills, searching ClawHub, and installing/removing skills.

**Architecture:** Frontend-only changes to three static files (HTML, CSS, JS). The REST API (`/api/skills/*`) already exists and needs no modification. Follows the existing Extensions tab pattern: card grid layout, `apiFetch()` helper, `showToast()` for feedback.

**Tech Stack:** Vanilla HTML/CSS/JS (no frameworks), existing design system (CSS variables, `ext-card` family classes).

---

### Task 1: Add Skills tab button and panel markup to index.html

**Files:**
- Modify: `src/channels/web/static/index.html:39-44` (tab bar) and `188-232` (before extensions panel)

**Step 1: Add the Skills tab button**

In `index.html`, inside the `.tab-bar` div, add a Skills button between Extensions and the spacer. Change lines 43-44 from:

```html
      <button data-tab="extensions">Extensions</button>
      <div class="spacer"></div>
```

to:

```html
      <button data-tab="extensions">Extensions</button>
      <button data-tab="skills">Skills</button>
      <div class="spacer"></div>
```

**Step 2: Add the Skills tab panel**

Add the Skills panel markup after the Extensions panel closing `</div>` (after line 232) and before the toasts div:

```html
    <!-- Skills Tab -->
    <div class="tab-panel" id="tab-skills">
      <div class="extensions-container">
        <div class="extensions-section">
          <h3>Search ClawHub</h3>
          <div class="skill-search-box">
            <input type="text" id="skill-search-input" placeholder="Search for skills...">
            <button onclick="searchClawHub()">Search</button>
          </div>
          <div class="extensions-list" id="skill-search-results"></div>
        </div>
        <div class="extensions-section">
          <h3>Installed Skills</h3>
          <div class="extensions-list" id="skills-list">
            <div class="empty-state">Loading skills...</div>
          </div>
        </div>
        <div class="extensions-section">
          <h3>Install Skill by URL</h3>
          <div class="ext-install-form">
            <input type="text" id="skill-install-name" placeholder="Skill name or slug">
            <input type="text" id="skill-install-url" placeholder="HTTPS URL to SKILL.md (optional)">
            <button onclick="installSkillFromForm()">Install</button>
          </div>
        </div>
      </div>
    </div>
```

**Step 3: Verify the HTML is well-formed**

Open the file and confirm the new panel is between the Extensions panel closing tag and `<div id="toasts">`.

**Step 4: Commit**

```bash
git add src/channels/web/static/index.html
git commit -m "feat(web): add Skills tab markup to index.html"
```

---

### Task 2: Add Skills CSS (trust badges, search box, fade-in animation)

**Files:**
- Modify: `src/channels/web/static/style.css` (append before the `@media` responsive block at line 2810)

**Step 1: Add skill-specific CSS**

Insert the following CSS before the `/* --- Activity toolbar --- */` comment (before line 2810):

```css
/* --- Skills tab --- */

.skill-search-box {
  display: flex;
  gap: 8px;
  align-items: center;
  margin-bottom: 12px;
}

.skill-search-box input {
  flex: 1;
  padding: 8px 12px;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  color: var(--text);
  font-size: 13px;
}

.skill-search-box input:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 3px rgba(52, 211, 153, 0.1);
}

.skill-search-box button {
  padding: 8px 20px;
  background: var(--accent);
  color: #09090b;
  border: none;
  border-radius: var(--radius);
  cursor: pointer;
  font-size: 13px;
  font-weight: 600;
  transition: background 0.2s, transform 0.2s;
}

.skill-search-box button:hover {
  background: var(--accent-hover);
  transform: translateY(-1px);
}

.skill-trust {
  font-size: 10px;
  padding: 2px 6px;
  border-radius: 8px;
  font-weight: 500;
  text-transform: uppercase;
  letter-spacing: 0.3px;
}

.skill-trust.trust-trusted {
  background: rgba(52, 211, 153, 0.15);
  color: var(--success);
}

.skill-trust.trust-installed {
  background: rgba(96, 165, 250, 0.15);
  color: #60a5fa;
}

.skill-version {
  font-size: 11px;
  color: var(--text-secondary);
  font-family: var(--font-mono);
}

@keyframes skillFadeIn {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: translateY(0); }
}

.skill-search-result {
  animation: skillFadeIn 0.3s ease-out both;
}
```

**Step 2: Commit**

```bash
git add src/channels/web/static/style.css
git commit -m "feat(web): add Skills tab CSS styles"
```

---

### Task 3: Wire Skills tab into switchTab() and keyboard shortcuts

**Files:**
- Modify: `src/channels/web/static/app.js:823-827` (switchTab function) and `2700-2704` (keyboard shortcuts)

**Step 1: Add skills tab loading to switchTab()**

In the `switchTab()` function, after line 827 (`if (tab === 'extensions') loadExtensions();`), add:

```javascript
  if (tab === 'skills') loadSkills();
```

**Step 2: Update keyboard shortcut tab array**

At line 2702, change:

```javascript
    const tabs = ['chat', 'memory', 'jobs', 'routines', 'extensions'];
```

to:

```javascript
    const tabs = ['chat', 'memory', 'jobs', 'routines', 'extensions', 'skills'];
```

And update the key range check at line 2700 from `'5'` to `'6'`:

```javascript
  if (mod && e.key >= '1' && e.key <= '6') {
```

**Step 3: Commit**

```bash
git add src/channels/web/static/app.js
git commit -m "feat(web): wire Skills tab into switchTab and keyboard shortcuts"
```

---

### Task 4: Implement loadSkills() -- render installed skills

**Files:**
- Modify: `src/channels/web/static/app.js` (add new section after the Extensions section, before keyboard shortcuts)

**Step 1: Add the loadSkills function**

Add this code block before the `// --- Keyboard shortcuts ---` comment (before line 2692):

```javascript
// --- Skills ---

function loadSkills() {
  var skillsList = document.getElementById('skills-list');
  apiFetch('/api/skills').then(function(data) {
    if (!data.skills || data.skills.length === 0) {
      skillsList.innerHTML = '<div class="empty-state">No skills installed</div>';
      return;
    }
    skillsList.innerHTML = '';
    for (var i = 0; i < data.skills.length; i++) {
      skillsList.appendChild(renderSkillCard(data.skills[i]));
    }
  }).catch(function(err) {
    skillsList.innerHTML = '<div class="empty-state">Failed to load skills: ' + escapeHtml(err.message) + '</div>';
  });
}

function renderSkillCard(skill) {
  var card = document.createElement('div');
  card.className = 'ext-card';

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
    kw.textContent = 'Activates on: ' + skill.keywords.join(', ');
    card.appendChild(kw);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  // Only show Remove for registry-installed skills, not user-placed trusted skills
  if (skill.trust.toLowerCase() !== 'trusted') {
    var removeBtn = document.createElement('button');
    removeBtn.className = 'btn-ext remove';
    removeBtn.textContent = 'Remove';
    removeBtn.addEventListener('click', function() { removeSkill(skill.name); });
    actions.appendChild(removeBtn);
  }

  card.appendChild(actions);
  return card;
}
```

**Step 2: Commit**

```bash
git add src/channels/web/static/app.js
git commit -m "feat(web): implement loadSkills and renderSkillCard"
```

---

### Task 5: Implement searchClawHub() -- search and render catalog results

**Files:**
- Modify: `src/channels/web/static/app.js` (add after `renderSkillCard`, before keyboard shortcuts)

**Step 1: Add search and catalog card rendering**

Add this code after the `renderSkillCard` function:

```javascript
function searchClawHub() {
  var input = document.getElementById('skill-search-input');
  var query = input.value.trim();
  if (!query) return;

  var resultsDiv = document.getElementById('skill-search-results');
  resultsDiv.innerHTML = '<div class="empty-state">Searching...</div>';

  apiFetch('/api/skills/search', {
    method: 'POST',
    body: { query: query },
  }).then(function(data) {
    resultsDiv.innerHTML = '';

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
      resultsDiv.innerHTML = '<div class="empty-state">No skills found for "' + escapeHtml(query) + '"</div>';
    }
  }).catch(function(err) {
    resultsDiv.innerHTML = '<div class="empty-state">Search failed: ' + escapeHtml(err.message) + '</div>';
  });
}

function renderCatalogSkillCard(entry, installedNames) {
  var card = document.createElement('div');
  card.className = 'ext-card ext-available skill-search-result';

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = entry.name || entry.slug;
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

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  var slug = entry.slug || entry.name;
  var isInstalled = installedNames[entry.name] || installedNames[slug];

  if (isInstalled) {
    var label = document.createElement('span');
    label.className = 'ext-active-label';
    label.textContent = 'Installed';
    actions.appendChild(label);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-ext install';
    installBtn.textContent = 'Install';
    installBtn.addEventListener('click', (function(s, btn) {
      return function() {
        if (!confirm('Install skill "' + s + '" from ClawHub?')) return;
        btn.disabled = true;
        btn.textContent = 'Installing...';
        installSkill(s, null, btn);
      };
    })(slug, installBtn));
    actions.appendChild(installBtn);
  }

  card.appendChild(actions);
  return card;
}

// Wire up Enter key on search input
document.getElementById('skill-search-input').addEventListener('keydown', function(e) {
  if (e.key === 'Enter') searchClawHub();
});
```

**Step 2: Commit**

```bash
git add src/channels/web/static/app.js
git commit -m "feat(web): implement ClawHub search with staggered card animation"
```

---

### Task 6: Implement installSkill() and removeSkill()

**Files:**
- Modify: `src/channels/web/static/app.js` (add after search functions, before keyboard shortcuts)

**Step 1: Add install and remove functions**

Add this code after the search event listener:

```javascript
function installSkill(nameOrSlug, url, btn) {
  var body = { name: nameOrSlug };
  if (url) body.url = url;

  apiFetch('/api/skills/install', {
    method: 'POST',
    headers: { 'X-Confirm-Action': 'true' },
    body: body,
  }).then(function(res) {
    if (res.success) {
      showToast('Installed skill "' + nameOrSlug + '"', 'success');
    } else {
      showToast('Install failed: ' + (res.message || 'unknown error'), 'error');
    }
    loadSkills();
    if (btn) { btn.disabled = false; btn.textContent = 'Install'; }
  }).catch(function(err) {
    showToast('Install failed: ' + err.message, 'error');
    if (btn) { btn.disabled = false; btn.textContent = 'Install'; }
  });
}

function removeSkill(name) {
  if (!confirm('Remove skill "' + name + '"?')) return;
  apiFetch('/api/skills/' + encodeURIComponent(name), {
    method: 'DELETE',
    headers: { 'X-Confirm-Action': 'true' },
  }).then(function(res) {
    if (res.success) {
      showToast('Removed skill "' + name + '"', 'success');
    } else {
      showToast('Remove failed: ' + (res.message || 'unknown error'), 'error');
    }
    loadSkills();
  }).catch(function(err) {
    showToast('Remove failed: ' + err.message, 'error');
  });
}

function installSkillFromForm() {
  var name = document.getElementById('skill-install-name').value.trim();
  if (!name) { showToast('Skill name is required', 'error'); return; }
  var url = document.getElementById('skill-install-url').value.trim() || null;
  if (url && !url.startsWith('https://')) {
    showToast('URL must use HTTPS', 'error');
    return;
  }
  if (!confirm('Install skill "' + name + '"?')) return;
  installSkill(name, url, null);
  document.getElementById('skill-install-name').value = '';
  document.getElementById('skill-install-url').value = '';
}
```

**Step 2: Commit**

```bash
git add src/channels/web/static/app.js
git commit -m "feat(web): implement installSkill, removeSkill, and form handler"
```

---

### Task 7: Fix apiFetch to merge extra headers properly

**Files:**
- Modify: `src/channels/web/static/app.js:86-98` (apiFetch function)

**Context:** The current `apiFetch` function sets `opts.headers` as an object and always overwrites with `Authorization`. When we pass `headers: { 'X-Confirm-Action': 'true' }` in options, the current code does `opts.headers = opts.headers || {}` which preserves our custom headers, then adds Authorization. However, `fetch()` expects headers as a `Headers` object or plain object -- the plain object approach works fine. Verify this works by reading the function carefully.

**Step 1: Verify apiFetch handles extra headers**

Read `app.js:86-98`. The current code:
```javascript
function apiFetch(path, options) {
  const opts = options || {};
  opts.headers = opts.headers || {};
  opts.headers['Authorization'] = 'Bearer ' + token;
  ...
}
```

This correctly merges: if we pass `{ headers: { 'X-Confirm-Action': 'true' } }`, it keeps our header and adds Authorization. **No change needed.** Move on.

**Step 2: Commit (skip -- no changes)**

---

### Task 8: Manual testing and final commit

**Step 1: Verify the HTML is valid**

Open `src/channels/web/static/index.html` and confirm:
- The Skills tab button appears in the tab bar
- The `tab-skills` panel has the correct structure
- No unclosed tags

**Step 2: Verify the JS doesn't have syntax errors**

Run a quick syntax check (if node is available):
```bash
node -c src/channels/web/static/app.js
```

**Step 3: Test the tab appears and loads**

Start the app and open the web gateway. Verify:
1. Skills tab appears in the tab bar between Extensions and the spacer
2. Clicking it shows the three sections
3. Installed skills load and display with trust badges and keywords
4. ClawHub search returns results with staggered animation
5. Install from search works (with confirm dialog)
6. Remove works for registry-installed skills
7. Install by URL form works
8. Cmd+6 keyboard shortcut switches to Skills tab

**Step 4: Final commit if any fixes were needed**

```bash
git add src/channels/web/static/index.html src/channels/web/static/app.js src/channels/web/static/style.css
git commit -m "feat(web): complete Skills tab with ClawHub search, install, and remove"
```
