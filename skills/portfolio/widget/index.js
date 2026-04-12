// Portfolio widget — renders projects/<id>/widgets/state.json as a
// stacked view: totals → positions table → top suggestions →
// pending intents. No build step, no framework.
//
// Reads the project ID from localStorage (`ironclaw.portfolio.projectId`)
// or falls back to the literal "portfolio". Refreshes every 30s.
//
// The widget container is `<div data-widget="portfolio">` — all CSS
// is scoped under that attribute (see skills/portfolio/widget/style.css).

(function () {
  var root = document.querySelector('[data-widget="portfolio"]');
  if (!root) return;

  var projectId =
    (typeof localStorage !== 'undefined' &&
      localStorage.getItem('ironclaw.portfolio.projectId')) ||
    'portfolio';

  var lastState = null;

  root.innerHTML =
    '<div class="pf-loading">Loading portfolio…</div>';

  async function fetchState() {
    var path = 'projects/' + projectId + '/widgets/state.json';
    try {
      var resp = await fetch(
        '/api/memory/read?path=' + encodeURIComponent(path),
        { credentials: 'same-origin' }
      );
      if (!resp.ok) throw new Error('status ' + resp.status);
      var body = await resp.json();
      if (typeof body === 'string') return JSON.parse(body);
      if (body && typeof body.content === 'string')
        return JSON.parse(body.content);
      return body;
    } catch (err) {
      return { __error: String(err) };
    }
  }

  function escapeHtml(str) {
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function fmtUsd(n) {
    if (n === undefined || n === null) return '-';
    return '$' + escapeHtml(n);
  }

  function fmtPct(n) {
    if (n === undefined || n === null) return '-';
    return (Number(n) * 100).toFixed(2) + '%';
  }

  function hasGains(state) {
    if (!state || state.__error) return false;
    var t = state.totals || {};
    var sug = state.top_suggestions || [];
    return (t.delta_vs_last_run_usd && parseFloat(t.delta_vs_last_run_usd) > 0) ||
           sug.length > 0;
  }

  function render(state) {
    lastState = state;
    if (!state) {
      root.innerHTML = '<div class="pf-empty">No portfolio data yet.</div>';
      return;
    }
    if (state.__error) {
      root.innerHTML =
        '<div class="pf-error">Failed to load portfolio: ' +
        escapeHtml(state.__error) +
        '</div>';
      return;
    }

    var t = state.totals || {};
    var positions = state.positions || [];
    var suggestions = state.top_suggestions || [];
    var pending = state.pending_intents || [];

    var positionsRows = positions
      .map(function (p) {
        return (
          '<tr>' +
          '<td>' + escapeHtml(p.protocol || '') + '</td>' +
          '<td>' + escapeHtml(p.chain || '') + '</td>' +
          '<td>' + escapeHtml(p.category || '') + '</td>' +
          '<td class="pf-num">' + fmtUsd(p.principal_usd) + '</td>' +
          '<td class="pf-num">' + fmtPct(p.net_apy) + '</td>' +
          '<td class="pf-num">' + (Number(p.risk_score) || 0) + '</td>' +
          '</tr>'
        );
      })
      .join('');

    var suggestionRows = suggestions
      .map(function (s, i) {
        return (
          '<li>' +
          '<strong>' + (i + 1) + '. ' + escapeHtml(s.strategy || '') + '</strong> — ' +
          escapeHtml(s.rationale || '') +
          ' <span class="pf-chip">+' + (Number(s.projected_delta_apy_bps) || 0) + ' bps</span>' +
          ' <span class="pf-chip">' + fmtUsd(s.projected_annual_gain_usd || '0') + '/yr</span>' +
          '</li>'
        );
      })
      .join('');

    var pendingRows = pending
      .map(function (p) {
        return (
          '<li>' +
          escapeHtml(p.id || '') + ' · ' + escapeHtml(p.status || '') +
          ' · ' + (Number(p.legs) || 0) + ' legs · ' +
          fmtUsd(p.total_cost_usd) +
          '</li>'
        );
      })
      .join('');

    var shareBtn = hasGains(state)
      ? '<button class="pf-share-btn" id="pf-share-btn">📤 Share gains</button>'
      : '';

    root.innerHTML =
      '<div class="pf-card">' +
      '<div class="pf-totals">' +
      '<div><span class="pf-label">Net value</span> <span class="pf-big">' + fmtUsd(t.net_value_usd) + '</span></div>' +
      '<div><span class="pf-label">Weighted APY</span> <span class="pf-big">' + fmtPct(t.realized_net_apy_7d) + '</span></div>' +
      '<div><span class="pf-label">Floor</span> <span>' + fmtPct(t.floor_apy) + '</span></div>' +
      (t.delta_vs_last_run_usd
        ? '<div><span class="pf-label">Δ</span> <span>' + t.delta_vs_last_run_usd + '</span></div>'
        : '') +
      shareBtn +
      '</div>' +
      (positions.length
        ? '<h3>Positions</h3>' +
          '<table class="pf-positions"><thead><tr>' +
          '<th>Protocol</th><th>Chain</th><th>Category</th>' +
          '<th class="pf-num">Principal</th><th class="pf-num">APY</th><th class="pf-num">Risk</th>' +
          '</tr></thead><tbody>' + positionsRows + '</tbody></table>'
        : '<p class="pf-empty">No positions.</p>') +
      (suggestions.length
        ? '<h3>Top suggestions</h3><ul class="pf-suggestions">' + suggestionRows + '</ul>'
        : '') +
      (pending.length
        ? '<h3>Pending intents</h3><ul class="pf-pending">' + pendingRows + '</ul>'
        : '') +
      (state.next_mission_run
        ? '<p class="pf-footer">Next run: ' + state.next_mission_run + '</p>'
        : '') +
      '</div>';

    var btn = document.getElementById('pf-share-btn');
    if (btn) {
      btn.addEventListener('click', function () {
        generateAndShare(state);
      });
    }
  }

  function generateAndShare(state) {
    var t = state.totals || {};
    var suggestions = state.top_suggestions || [];

    var totalGain = suggestions.reduce(function (sum, s) {
      return sum + parseFloat(s.projected_annual_gain_usd || '0');
    }, 0);
    var topDelta = suggestions.length > 0
      ? '+' + suggestions[0].projected_delta_apy_bps + ' bps'
      : '';
    var moves = suggestions.length;
    var apyStr = fmtPct(t.realized_net_apy_7d);
    var delta = t.delta_vs_last_run_usd || '';
    var headline = totalGain > 0
      ? 'Saving $' + totalGain.toFixed(0) + '/year'
      : (delta && parseFloat(delta) > 0 ? 'Up ' + delta : 'Portfolio optimized');

    var svg = buildCardSvg(headline, apyStr, topDelta, moves);
    svgToPng(svg, function (dataUrl) {
      if (window.IronClaw && window.IronClaw.api && window.IronClaw.api.share) {
        var shareText = headline + ' with my DeFi portfolio keeper';
        window.IronClaw.api.share({
          imageDataUrl: dataUrl,
          text: shareText,
          hashtags: 'DeFi,IronClaw,Crypto'
        });
      }
    });
  }

  function buildCardSvg(headline, apy, delta, moves) {
    var w = 600, h = 340;
    return (
      '<svg xmlns="http://www.w3.org/2000/svg" width="' + w + '" height="' + h + '" viewBox="0 0 ' + w + ' ' + h + '">' +
      '<defs>' +
      '<linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">' +
      '<stop offset="0%" stop-color="#0f0c29"/>' +
      '<stop offset="50%" stop-color="#302b63"/>' +
      '<stop offset="100%" stop-color="#24243e"/>' +
      '</linearGradient>' +
      '</defs>' +
      '<rect width="' + w + '" height="' + h + '" rx="16" fill="url(#bg)"/>' +
      '<text x="40" y="52" font-family="-apple-system,Segoe UI,Roboto,sans-serif" font-size="14" fill="#888" font-weight="500" letter-spacing="1">PORTFOLIO KEEPER</text>' +
      '<text x="40" y="110" font-family="-apple-system,Segoe UI,Roboto,sans-serif" font-size="36" fill="#fff" font-weight="700">' + escapeXml(headline) + '</text>' +
      '<line x1="40" y1="136" x2="560" y2="136" stroke="#444" stroke-width="1"/>' +
      '<text x="40" y="180" font-family="-apple-system,Segoe UI,Roboto,sans-serif" font-size="13" fill="#888" font-weight="500" letter-spacing="0.5">WEIGHTED APY</text>' +
      '<text x="40" y="210" font-family="-apple-system,Segoe UI,Roboto,sans-serif" font-size="28" fill="#4ade80" font-weight="600">' + escapeXml(apy) + '</text>' +
      (delta
        ? '<text x="240" y="180" font-family="-apple-system,Segoe UI,Roboto,sans-serif" font-size="13" fill="#888" font-weight="500" letter-spacing="0.5">TOP IMPROVEMENT</text>' +
          '<text x="240" y="210" font-family="-apple-system,Segoe UI,Roboto,sans-serif" font-size="28" fill="#60a5fa" font-weight="600">' + escapeXml(delta) + '</text>'
        : '') +
      (moves > 0
        ? '<text x="440" y="180" font-family="-apple-system,Segoe UI,Roboto,sans-serif" font-size="13" fill="#888" font-weight="500" letter-spacing="0.5">MOVES FOUND</text>' +
          '<text x="440" y="210" font-family="-apple-system,Segoe UI,Roboto,sans-serif" font-size="28" fill="#f59e0b" font-weight="600">' + moves + '</text>'
        : '') +
      '<text x="40" y="300" font-family="-apple-system,Segoe UI,Roboto,sans-serif" font-size="12" fill="#555">Powered by IronClaw · ironclaw.ai</text>' +
      '</svg>'
    );
  }

  function escapeXml(str) {
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function svgToPng(svgStr, cb) {
    var blob = new Blob([svgStr], { type: 'image/svg+xml;charset=utf-8' });
    var url = URL.createObjectURL(blob);
    var img = new Image();
    img.onload = function () {
      var canvas = document.createElement('canvas');
      canvas.width = img.naturalWidth * 2;
      canvas.height = img.naturalHeight * 2;
      var ctx = canvas.getContext('2d');
      ctx.scale(2, 2);
      ctx.drawImage(img, 0, 0);
      URL.revokeObjectURL(url);
      cb(canvas.toDataURL('image/png'));
    };
    img.onerror = function () {
      URL.revokeObjectURL(url);
      cb(null);
    };
    img.src = url;
  }

  async function refresh() {
    var state = await fetchState();
    render(state);
  }

  refresh();
  setInterval(refresh, 30000);
})();
