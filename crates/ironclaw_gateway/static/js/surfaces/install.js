function ironhubInstallPanel() {
  return document.getElementById('tab-install');
}

function renderIronhubInstallState(html) {
  var panel = ironhubInstallPanel();
  if (panel) panel.innerHTML = '<div class="ext-card ironhub-install-card">' + html + '</div>';
}

function ironhubInstallCancel() {
  switchTab('chat');
}

function renderIronhubInstallError(message) {
  renderIronhubInstallState(
    '<h2>' + escapeHtml(I18n.t('ironhub.install.unverifiedTitle')) + '</h2>' +
    '<p class="ironhub-install-error">' + escapeHtml(message) + '</p>' +
    '<button class="btn-ext" onclick="ironhubInstallCancel()">' +
    escapeHtml(I18n.t('ironhub.install.close')) + '</button>'
  );
}

function renderIronhubConfirm(signed, info) {
  var tool = signed && signed.slug ? signed.slug : '';
  var kind = info && info.kind ? info.kind : 'tool';
  var version = info && info.version ? info.version : '';
  var description = info && info.description ? info.description : '';
  var release = info && info.release_tag ? info.release_tag : '';
  var provenance = info && info.provenance ? info.provenance : '';
  var trustLabel = info && info.trust_label ? info.trust_label : '';
  var isCommunityUnverified = provenance === 'new';
  var rows = '';
  rows += '<div class="ironhub-install-row"><span>' +
    escapeHtml(I18n.t('ironhub.install.name')) + '</span><strong>' +
    escapeHtml(tool) + '</strong></div>';
  rows += '<div class="ironhub-install-row"><span>' +
    escapeHtml(I18n.t('ironhub.install.kind')) + '</span><span>' +
    escapeHtml(kind) + '</span></div>';
  if (version) {
    rows += '<div class="ironhub-install-row"><span>' +
      escapeHtml(I18n.t('ironhub.install.version')) + '</span><span>' +
      escapeHtml(version) + '</span></div>';
  }
  if (release) {
    rows += '<div class="ironhub-install-row"><span>' +
      escapeHtml(I18n.t('ironhub.install.release')) + '</span><span>' +
      escapeHtml(release) + '</span></div>';
  }
  if (trustLabel) {
    rows += '<div class="ironhub-install-row"><span>' +
      escapeHtml(I18n.t('ironhub.install.trustLabel')) + '</span><span>' +
      escapeHtml(trustLabel) + '</span></div>';
  }
  if (description) {
    rows += '<p class="ironhub-install-desc">' + escapeHtml(description) + '</p>';
  }

  var warning = '';
  var ackCheckbox = '';
  var confirmDisabled = '';
  if (isCommunityUnverified) {
    warning = '<p class="ironhub-install-error">' +
      escapeHtml(I18n.t('ironhub.install.communityWarning')) + '</p>';
    ackCheckbox =
      '<label class="ironhub-install-row"><input type="checkbox" ' +
      'id="ironhub-install-ack-cb" onchange="ironhubAckChanged()" /> ' +
      escapeHtml(I18n.t('ironhub.install.ackUnverified')) + '</label>';
    confirmDisabled = ' disabled';
  }

  renderIronhubInstallState(
    '<h2>' + escapeHtml(I18n.t('ironhub.install.confirmTitle')) + '</h2>' +
    '<p class="ironhub-install-source">' +
    escapeHtml(I18n.t('ironhub.install.fromHub')) + '</p>' +
    rows +
    warning +
    ackCheckbox +
    '<div class="ironhub-install-actions">' +
    '<button class="btn-ext install" id="ironhub-install-confirm-btn"' + confirmDisabled + '>' +
    escapeHtml(I18n.t('ironhub.install.confirm')) + '</button>' +
    '<button class="btn-ext" onclick="ironhubInstallCancel()">' +
    escapeHtml(I18n.t('ironhub.install.cancel')) + '</button>' +
    '</div>'
  );
  var btn = document.getElementById('ironhub-install-confirm-btn');
  if (btn) {
    btn.addEventListener('click', function() {
      ironhubInstallConfirm(signed, isCommunityUnverified);
    });
  }
}

function ironhubAckChanged() {
  var cb = document.getElementById('ironhub-install-ack-cb');
  var btn = document.getElementById('ironhub-install-confirm-btn');
  if (cb && btn) {
    btn.disabled = !cb.checked;
  }
}

function ironhubInstallConfirm(signed, requireAck) {
  var btn = document.getElementById('ironhub-install-confirm-btn');
  if (btn) {
    btn.disabled = true;
    btn.textContent = I18n.t('ironhub.install.installing');
  }
  var body = {
    slug: signed.slug,
    version: signed.version,
    uid: signed.uid,
    aid: signed.aid,
    ts: parseInt(signed.ts, 10),
    nonce: signed.nonce,
    sig: signed.sig,
  };
  if (requireAck) {
    body.acknowledge_unverified = true;
  }
  apiFetch('/api/ironhub/install', {
    method: 'POST',
    body: body,
  }).then(function(res) {
    var name = res && res.name ? res.name : signed.slug;
    showToast(I18n.t('ironhub.install.success', { name: name }), 'success');
    renderIronhubInstallState(
      '<h2>' + escapeHtml(I18n.t('ironhub.install.doneTitle')) + '</h2>' +
      '<p>' + escapeHtml(I18n.t('ironhub.install.success', { name: name })) + '</p>' +
      '<button class="btn-ext" onclick="ironhubInstallCancel()">' +
      escapeHtml(I18n.t('ironhub.install.close')) + '</button>'
    );
  }).catch(function(err) {
    var msg = err && err.message ? err.message : 'unknown error';
    showToast(I18n.t('ironhub.install.failed', { message: msg }), 'error');
    if (btn) {
      btn.disabled = false;
      btn.textContent = I18n.t('ironhub.install.confirm');
    }
  });
}

function startIronhubInstall(params) {
  renderIronhubInstallState(
    '<h2>' + escapeHtml(I18n.t('ironhub.install.verifyingTitle')) + '</h2>' +
    '<p>' + escapeHtml(I18n.t('ironhub.install.verifying')) + '</p>'
  );

  var slug = params && params.slug;
  var version = params && params.version;
  var uid = params && params.uid;
  var aid = params && params.aid;
  var ts = params && params.ts;
  var nonce = params && params.nonce;
  var sig = params && params.sig;

  if (!slug || !version || !uid || !aid || !ts || !nonce || !sig) {
    renderIronhubInstallError(I18n.t('ironhub.install.missingParams'));
    return;
  }

  var signed = {
    slug: slug,
    version: version,
    uid: uid,
    aid: aid,
    ts: parseInt(ts, 10),
    nonce: nonce,
    sig: sig,
  };

  apiFetch('/api/ironhub/verify-intent', {
    method: 'POST',
    body: signed,
  }).then(function(res) {
    if (!res || !res.valid) {
      var reason = res && res.reason ? res.reason : I18n.t('ironhub.install.unverified');
      renderIronhubInstallError(reason);
      return;
    }
    return apiFetch('/api/ironhub/info?name=' + encodeURIComponent(slug))
      .then(function(info) {
        renderIronhubConfirm(signed, info);
      })
      .catch(function() {
        renderIronhubConfirm(signed, null);
      });
  }).catch(function(err) {
    var msg = err && err.message ? err.message : I18n.t('ironhub.install.unverified');
    renderIronhubInstallError(msg);
  });
}
