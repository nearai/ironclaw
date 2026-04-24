// Mobile hamburger drawer — replaces the horizontally-scrolling tab bar and
// the .expanded-mobile thread-sidebar overlay at viewports ≤768px. The
// desktop DOM stays authoritative: nav buttons, thread items, and auxiliary
// controls (theme/user/docs/language/tee-shield/restart) are cloned or
// relocated into the drawer so every existing click binding keeps working
// without duplication.

(function () {
  const MOBILE_QUERY = '(max-width: 768px)';
  const mql = window.matchMedia(MOBILE_QUERY);

  const hamburger = document.getElementById('hamburger-btn');
  const menu = document.getElementById('mobile-menu');
  const backdrop = document.getElementById('mobile-menu-backdrop');
  const nav = document.getElementById('mobile-menu-nav');
  const footer = document.getElementById('mobile-menu-footer');
  const mobileThreadList = document.getElementById('mobile-thread-list');
  const mobileNewThreadBtn = document.getElementById('mobile-new-thread-btn');
  const mobileDot = document.getElementById('mobile-sse-dot');
  const tabBar = document.querySelector('.tab-bar');
  const threadList = document.getElementById('thread-list');
  const sseDot = document.getElementById('sse-dot');

  // All pieces are in index.html; bail defensively if any future edit removes one.
  if (!hamburger || !menu || !backdrop || !nav || !footer
    || !mobileThreadList || !mobileNewThreadBtn || !mobileDot
    || !tabBar || !threadList || !sseDot) {
    return;
  }

  // --- open/close ---

  function openMenu() {
    menu.classList.add('open');
    backdrop.classList.add('open');
    menu.setAttribute('aria-hidden', 'false');
    menu.removeAttribute('inert');
    hamburger.setAttribute('aria-expanded', 'true');
    hamburger.setAttribute('aria-label', 'Close menu');
    document.body.classList.add('mobile-menu-open');
  }

  function closeMenu() {
    menu.classList.remove('open');
    backdrop.classList.remove('open');
    menu.setAttribute('aria-hidden', 'true');
    menu.setAttribute('inert', '');
    hamburger.setAttribute('aria-expanded', 'false');
    hamburger.setAttribute('aria-label', 'Open menu');
    document.body.classList.remove('mobile-menu-open');
  }

  function isOpen() {
    return menu.classList.contains('open');
  }

  hamburger.addEventListener('click', () => {
    if (isOpen()) closeMenu(); else openMenu();
  });
  backdrop.addEventListener('click', closeMenu);
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && isOpen()) closeMenu();
  });
  // Back-button / programmatic hash change should dismiss an open drawer.
  window.addEventListener('hashchange', () => { if (isOpen()) closeMenu(); });

  // --- nav button mirror ---
  //
  // Clone each .tab-bar button[data-tab] into the drawer; cloned clicks
  // dispatch .click() on the source button so switchTab() and all other
  // bindings run through their existing path.

  function rebuildNav() {
    nav.textContent = '';
    const sources = tabBar.querySelectorAll('button[data-tab]');
    sources.forEach((src) => {
      // Engine-mode switcher hides v1-only / v2-only tabs via inline
      // `style.display = 'none'` (see surfaces/projects.js). Read the
      // inline style directly — avoids forcing style/layout recalc from
      // getComputedStyle() inside an observer callback.
      if (src.style.display === 'none') return;
      const tab = src.getAttribute('data-tab');
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'mobile-nav-item' + (src.classList.contains('active') ? ' active' : '');
      btn.setAttribute('data-tab', tab);
      btn.textContent = (src.textContent || tab).trim();
      const badge = src.getAttribute('data-active-count');
      if (badge && badge !== '0') btn.setAttribute('data-active-count', badge);
      btn.addEventListener('click', () => {
        src.click();
        closeMenu();
      });
      nav.appendChild(btn);
    });
  }

  // --- thread list mirror ---
  //
  // Deep-clone each thread item into the mobile list, then strip `id`
  // attributes from the clones to avoid duplicates in the document.
  // (Thread items today carry only `data-thread-id`, not DOM ids, but a
  // future change to history.js's renderer could introduce one and
  // silently break `getElementById` elsewhere — defensive strip locks
  // the invariant in.) Clicks on the mobile clones delegate back through
  // `data-thread-id` → switchThread().

  function rebuildThreads() {
    mobileThreadList.textContent = '';
    for (const item of threadList.children) {
      const clone = item.cloneNode(true);
      if (clone.id) clone.removeAttribute('id');
      clone.querySelectorAll('[id]').forEach((el) => el.removeAttribute('id'));
      mobileThreadList.appendChild(clone);
    }
  }

  mobileThreadList.addEventListener('click', (e) => {
    const item = e.target.closest('[data-thread-id]');
    if (!item) return;
    const id = item.getAttribute('data-thread-id');
    if (id && typeof switchThread === 'function') {
      switchThread(id);
      closeMenu();
    }
  });

  mobileNewThreadBtn.addEventListener('click', () => {
    const src = document.getElementById('thread-new-btn');
    if (src) src.click();
    closeMenu();
  });

  // --- auxiliary control relocation ---
  //
  // On mobile, move docs/language/theme/tee-shield/user/restart from the
  // (now-hidden) tab-bar into the drawer footer. On resize back to desktop,
  // return them to their original positions so nothing desktop-side breaks.

  const RELOCATE_SELECTORS = [
    '.docs-link-btn',
    '.language-switcher',
    '#theme-toggle',
    '#tee-shield',
    '#user-account',
    '#restart-btn',
  ];

  const relocated = []; // { el, parent, next }

  function relocateIntoFooter() {
    if (relocated.length > 0) return; // already moved
    RELOCATE_SELECTORS.forEach((sel) => {
      const el = tabBar.querySelector(sel);
      if (!el) return;
      relocated.push({ el, parent: el.parentNode, next: el.nextSibling });
      footer.appendChild(el);
    });
  }

  function restoreFromFooter() {
    while (relocated.length > 0) {
      const { el, parent, next } = relocated.pop();
      if (next && next.parentNode === parent) parent.insertBefore(el, next);
      else parent.appendChild(el);
    }
  }

  // --- sse dot mirror ---

  function syncDot() {
    mobileDot.classList.toggle('disconnected', sseDot.classList.contains('disconnected'));
  }

  // --- observers (connected only while mobile is active) ---
  //
  // On desktop the drawer is never visible (display: none via CSS), so
  // running MutationObservers for it is pure waste — every tab-indicator
  // style mutation, every thread-list update, every SSE connect/disconnect
  // would still trigger rebuild work nothing consumes. Observers attach
  // in applyViewport(true) and detach in applyViewport(false).

  const navObserver = new MutationObserver(rebuildNav);
  const threadObserver = new MutationObserver(rebuildThreads);
  const dotObserver = new MutationObserver(syncDot);
  let observing = false;

  function startObservers() {
    if (observing) return;
    navObserver.observe(tabBar, {
      childList: true,
      subtree: true,
      characterData: true,
      attributes: true,
      attributeFilter: ['class', 'data-active-count', 'data-v1-only', 'data-v2-only', 'hidden', 'style'],
    });
    threadObserver.observe(threadList, { childList: true, subtree: true });
    dotObserver.observe(sseDot, { attributes: true, attributeFilter: ['class'] });
    observing = true;
  }

  function stopObservers() {
    if (!observing) return;
    navObserver.disconnect();
    threadObserver.disconnect();
    dotObserver.disconnect();
    observing = false;
  }

  // --- viewport breakpoint ---

  function applyViewport(matches) {
    if (matches) {
      relocateIntoFooter();
      rebuildNav();
      rebuildThreads();
      syncDot();
      startObservers();
    } else {
      if (isOpen()) closeMenu();
      stopObservers();
      restoreFromFooter();
    }
  }

  applyViewport(mql.matches);
  if (mql.addEventListener) mql.addEventListener('change', (e) => applyViewport(e.matches));
  else if (mql.addListener) mql.addListener((e) => applyViewport(e.matches));
})();
