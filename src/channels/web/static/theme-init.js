// Prevent FOUC: apply saved theme before first paint.
// This script must be loaded synchronously in <head> (no defer/async).
(function() {
  const mode = localStorage.getItem('ironclaw-theme') || 'system';
  let resolved = mode;
  if (mode === 'system') {
    resolved = window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  }
  document.documentElement.setAttribute('data-theme', resolved);
  document.documentElement.setAttribute('data-theme-mode', mode);
})();
