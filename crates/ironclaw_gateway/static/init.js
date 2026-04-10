// Early initialization — runs before app.js
// Add any pre-app setup here (feature flags, env detection, etc.)

// Theme: prevent FOUC by applying saved theme before first paint.
(function() {
  var stored = localStorage.getItem('ironclaw-theme');
  var mode = (stored === 'dark' || stored === 'light' || stored === 'system') ? stored : 'system';
  var resolved = mode;
  if (mode === 'system') {
    resolved = window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  }
  document.documentElement.setAttribute('data-theme', resolved);
  document.documentElement.setAttribute('data-theme-mode', mode);
})();

// Debug mode
(function() {
  var params = new URLSearchParams(window.location.search);
  if (params.get('debug') === 'true') {
    sessionStorage.setItem('ironclaw_debug', 'true');
    params.delete('debug');
    var u = window.location.pathname + (params.toString() ? '?' + params.toString() : '') + window.location.hash;
    window.history.replaceState({}, '', u);
  }
  window.isDebugMode = sessionStorage.getItem('ironclaw_debug') === 'true';
})();
