(function() {
  'use strict';

  var DEFAULT_TIMEOUT_MS = 10000;

  function text(label, params) {
    var values = params || {};
    var rendered = '';
    if (label && label.key && window.I18n && typeof window.I18n.t === 'function') {
      rendered = window.I18n.t(label.key, values);
      if (rendered && rendered !== label.key) return rendered;
    }
    rendered = (label && (label.fallback || label.key)) || '';
    Object.keys(values).forEach(function(key) {
      rendered = rendered.replace(new RegExp('\\{' + key + '\\}', 'g'), function() {
        return values[key];
      });
    });
    return rendered;
  }

  function findAll(selectors) {
    return (selectors || []).map(function(selector) {
      return document.querySelector(selector);
    }).filter(Boolean);
  }

  function setPending(config, isPending) {
    findAll(config.controls).forEach(function(el) {
      el.disabled = isPending;
    });

    var submit = document.querySelector(config.submit);
    if (!submit) return;
    submit.disabled = isPending;
    submit.setAttribute('aria-busy', isPending ? 'true' : 'false');
    if (config.loadingClass) submit.classList.toggle(config.loadingClass, isPending);
    submit.textContent = isPending
      ? text(config.pendingLabel, {})
      : text(config.idleLabel, {});
  }

  function normalizedTimeout(timeoutMs) {
    var override = Number(window.__IRONCLAW_USER_CREATE_TIMEOUT_MS);
    if (override > 0) return override;
    var value = Number(timeoutMs);
    return value > 0 ? value : DEFAULT_TIMEOUT_MS;
  }

  function errorMessage(err) {
    var message = err && err.message ? String(err.message) : 'Unknown error';
    message = message.replace(/[\u0000-\u001f\u007f]+/g, ' ').trim();
    return message || 'Unknown error';
  }

  function request(apiFetchFn, path, options, timeoutMs) {
    if (typeof apiFetchFn !== 'function') {
      return Promise.reject(new Error('API unavailable'));
    }

    if (typeof AbortController === 'undefined') {
      return apiFetchFn(path, options);
    }

    var controller = new AbortController();
    var opts = {};
    Object.keys(options || {}).forEach(function(key) {
      opts[key] = options[key];
    });
    opts.signal = controller.signal;

    var timedOut = false;
    var timer = setTimeout(function() {
      timedOut = true;
      controller.abort();
    }, normalizedTimeout(timeoutMs));

    return apiFetchFn(path, opts).catch(function(err) {
      if (timedOut || (err && err.name === 'AbortError')) {
        throw new Error(text({
          key: 'users.createTimedOut',
          fallback: 'Create user request timed out. Please try again.',
        }, {}));
      }
      throw err;
    }).finally(function() {
      clearTimeout(timer);
    });
  }

  window.IronClawUserCreateForm = {
    DEFAULT_TIMEOUT_MS: DEFAULT_TIMEOUT_MS,
    errorMessage: errorMessage,
    request: request,
    setPending: setPending,
    text: text,
  };
})();
