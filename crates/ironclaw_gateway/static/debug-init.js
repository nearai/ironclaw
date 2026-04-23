// Debug mode bootstrap — runs early so `window.isDebugMode` is set before
// `app.js` calls `connectSSE()`. Theme bootstrap stays in `theme-init.js`.
(function () {
  var params = new URLSearchParams(window.location.search);
  if (params.get('debug') === 'true') {
    sessionStorage.setItem('ironclaw_debug', 'true');
    params.delete('debug');
    var u = window.location.pathname
      + (params.toString() ? '?' + params.toString() : '')
      + window.location.hash;
    window.history.replaceState({}, '', u);
  }
  window.isDebugMode = sessionStorage.getItem('ironclaw_debug') === 'true';
})();
