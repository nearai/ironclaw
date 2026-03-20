// Prevent FOUC: apply saved theme before first paint.
// This script must be loaded synchronously in <head> (no defer/async).
(function() {
  var mode = localStorage.getItem('ironclaw-theme') || 'system';
  var resolved = mode;
  if (mode === 'system') {
    resolved = window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  }
  document.documentElement.setAttribute('data-theme', resolved);
  document.documentElement.setAttribute('data-theme-mode', mode);
})();
