// IronClaw Web Gateway - Client

// --- Theme Management (dark / light / system) ---
// Icon switching is handled by pure CSS via data-theme-mode on <html>.

function getSystemTheme() {
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

const VALID_THEME_MODES = { dark: true, light: true, system: true };

function getThemeMode() {
  const stored = localStorage.getItem('ironclaw-theme');
  return (stored && VALID_THEME_MODES[stored]) ? stored : 'system';
}

function resolveTheme(mode) {
  return mode === 'system' ? getSystemTheme() : mode;
}

function applyTheme(mode) {
  const resolved = resolveTheme(mode);
  document.documentElement.setAttribute('data-theme', resolved);
  document.documentElement.setAttribute('data-theme-mode', mode);
  const titleKeys = { dark: 'theme.tooltipDark', light: 'theme.tooltipLight', system: 'theme.tooltipSystem' };
  const btn = document.getElementById('theme-toggle');
  if (btn) btn.title = (typeof I18n !== 'undefined' && titleKeys[mode]) ? I18n.t(titleKeys[mode]) : ('Theme: ' + mode);
  const announce = document.getElementById('theme-announce');
  if (announce) announce.textContent = (typeof I18n !== 'undefined') ? I18n.t('theme.announce', { mode: mode }) : ('Theme: ' + mode);
}

function toggleTheme() {
  const cycle = { dark: 'light', light: 'system', system: 'dark' };
  const current = getThemeMode();
  const next = cycle[current] || 'dark';
  localStorage.setItem('ironclaw-theme', next);
  applyTheme(next);
}

// Apply theme immediately (FOUC prevention is done via inline script in <head>,
// but we call again here to ensure tooltip is set after DOM is ready).
applyTheme(getThemeMode());

// Delay enabling theme transition to avoid flash on initial load.
requestAnimationFrame(function() {
  requestAnimationFrame(function() {
    document.body.classList.add('theme-transition');
  });
});

// Listen for OS theme changes — only re-apply when in 'system' mode.
const mql = window.matchMedia('(prefers-color-scheme: light)');
const onSchemeChange = function() {
  if (getThemeMode() === 'system') {
    applyTheme('system');
  }
};
if (mql.addEventListener) {
  mql.addEventListener('change', onSchemeChange);
} else if (mql.addListener) {
  mql.addListener(onSchemeChange);
}

// Bind theme toggle buttons (CSP-compliant — no inline onclick).
document.getElementById('theme-toggle').addEventListener('click', toggleTheme);
document.getElementById('settings-theme-toggle')?.addEventListener('click', () => {
  toggleTheme();
  const btn = document.getElementById('settings-theme-toggle');
  if (btn) {
    const mode = localStorage.getItem('ironclaw-theme') || 'system';
    btn.textContent = I18n.t('theme.label', { mode: mode.charAt(0).toUpperCase() + mode.slice(1) });
  }
});
