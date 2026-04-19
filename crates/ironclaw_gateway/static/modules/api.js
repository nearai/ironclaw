// --- API helper ---

function apiFetch(path, options) {
  const opts = options || {};
  opts.headers = opts.headers || {};
  // In OIDC mode the reverse proxy provides auth; skip the Authorization header.
  console.log('[apiFetch]', path, 'token=' + (token ? token.substring(0,8) + '...' : '(empty)'), 'oidc=' + oidcProxyAuth);
  if (token && !oidcProxyAuth) {
    opts.headers['Authorization'] = 'Bearer ' + token;
  }
  if (opts.body && typeof opts.body === 'object') {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(opts.body);
  }
  return fetch(path, opts).then((res) => {
    if (!res.ok) {
      return res.text().then(function(body) {
        const err = new Error(body || (res.status + ' ' + res.statusText));
        err.status = res.status;
        throw err;
      });
    }
    if (res.status === 204) return null;
    return res.json();
  });
}
