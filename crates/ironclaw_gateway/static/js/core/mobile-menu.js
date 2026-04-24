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
    hamburger.setAttribute('aria-expanded', 'true');
    document.body.classList.add('mobile-menu-open');
  }

  function closeMenu() {
    menu.classList.remove('open');
    backdrop.classList.remove('open');
    menu.setAttribute('aria-hidden', 'true');
    hamburger.setAttribute('aria-expanded', 'false');
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
      // Skip buttons a feature-flag branch has hidden (data-v1-only/data-v2-only
      // or an inline style="display:none" set by the engine-mode switcher).
      if (src.offsetParent === null && getComputedStyle(src).display === 'none') return;
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

  const navObserver = new MutationObserver(rebuildNav);
  navObserver.observe(tabBar, {
    childList: true,
    subtree: true,
    characterData: true,
    attributes: true,
    attributeFilter: ['class', 'data-active-count', 'data-v1-only', 'data-v2-only', 'hidden', 'style'],
  });

  // --- thread list mirror ---
  //
  // Copy #thread-list content into #mobile-thread-list on every change; delegate
  // click events by looking up data-thread-id and routing through switchThread().

  function rebuildThreads() {
    mobileThreadList.innerHTML = threadList.innerHTML;
  }

  const threadObserver = new MutationObserver(rebuildThreads);
  threadObserver.observe(threadList, { childList: true, subtree: true });

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

  const dotObserver = new MutationObserver(syncDot);
  dotObserver.observe(sseDot, { attributes: true, attributeFilter: ['class'] });
  syncDot();

  // --- viewport breakpoint ---

  function applyViewport(matches) {
    if (matches) {
      relocateIntoFooter();
      rebuildNav();
      rebuildThreads();
    } else {
      if (isOpen()) closeMenu();
      restoreFromFooter();
    }
  }

  applyViewport(mql.matches);
  if (mql.addEventListener) mql.addEventListener('change', (e) => applyViewport(e.matches));
  else if (mql.addListener) mql.addListener((e) => applyViewport(e.matches));
})();
