var root = document.createElement('section');
root.className = 'ita-console';
container.appendChild(root);

function itaEscape(value) {
  var div = document.createElement('div');
  div.textContent = value == null ? '' : String(value);
  return div.innerHTML;
}

function itaSlugify(value) {
  return String(value || 'intents-trading-agent')
    .toLowerCase()
    .replace(/[^a-z0-9-]/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '') || 'intents-trading-agent';
}

function itaPct(value) {
  var n = Number(value || 0);
  var sign = n > 0 ? '+' : '';
  return sign + n.toFixed(2) + '%';
}

function itaScore(value) {
  var n = Number(value || 0);
  return n.toFixed(2);
}

function itaStatusClass(status) {
  var s = String(status || 'unknown').toLowerCase();
  if (s === 'pass' || s === 'paper-built' || s === 'live-quote-built') return 'is-pass';
  if (s === 'warn' || s === 'awaiting-signature' || s === 'watch') return 'is-warn';
  if (s === 'fail' || s === 'failed' || s === 'blocked') return 'is-fail';
  return 'is-neutral';
}

function itaReadState(slug) {
  var path = 'projects/' + slug + '/widgets/state.json';
  return api.fetch('/api/memory/read?path=' + encodeURIComponent(path))
    .then(function(res) {
      if (!res.ok) throw new Error('state not found');
      return res.json();
    })
    .then(function(doc) {
      return JSON.parse(doc.content || '{}');
    });
}

function itaLoadProjectSlug() {
  return api.fetch('/api/engine/projects/' + encodeURIComponent(projectId))
    .then(function(res) {
      if (!res.ok) throw new Error('project not found');
      return res.json();
    })
    .then(function(data) {
      return itaSlugify(data && data.project && data.project.name);
    })
    .catch(function() {
      return 'intents-trading-agent';
    });
}

function itaCandidateRows(candidates) {
  if (!Array.isArray(candidates) || candidates.length === 0) {
    return '<div class="ita-empty-line">No ranked strategies yet.</div>';
  }
  return '<div class="ita-table" role="table" aria-label="Ranked strategy candidates">'
    + candidates.map(function(c) {
      return '<div class="ita-row" role="row">'
        + '<div class="ita-rank">#' + itaEscape(c.rank || '-') + '</div>'
        + '<div class="ita-strategy">'
        + '<strong>' + itaEscape(c.id || 'candidate') + '</strong>'
        + '<span>' + itaEscape(c.strategy_kind || 'strategy') + '</span>'
        + '</div>'
        + '<div class="ita-metric"><span>Score</span><strong>' + itaScore(c.selection_score) + '</strong></div>'
        + '<div class="ita-metric"><span>Return</span><strong>' + itaPct(c.total_return_pct) + '</strong></div>'
        + '<div class="ita-metric"><span>Alpha</span><strong>' + itaPct(c.alpha_vs_buy_hold_pct) + '</strong></div>'
        + '<div class="ita-metric"><span>DD</span><strong>' + itaPct(-Math.abs(Number(c.max_drawdown_pct || 0))) + '</strong></div>'
        + '<div class="ita-gate ' + (c.passes_basic_gate ? 'is-pass' : 'is-fail') + '">'
        + (c.passes_basic_gate ? 'Gate pass' : 'Gate fail')
        + '</div>'
        + '</div>';
    }).join('')
    + '</div>';
}

function itaRiskGates(gates) {
  if (!Array.isArray(gates) || gates.length === 0) {
    return '<div class="ita-empty-line">No risk gates recorded.</div>';
  }
  return '<div class="ita-gates">'
    + gates.map(function(gate) {
      return '<div class="ita-gate-row">'
        + '<span class="ita-gate-name">' + itaEscape(gate.name || 'gate') + '</span>'
        + '<span class="ita-pill ' + itaStatusClass(gate.status) + '">' + itaEscape(gate.status || 'unknown') + '</span>'
        + (gate.detail ? '<span class="ita-gate-detail">' + itaEscape(gate.detail) + '</span>' : '')
        + '</div>';
    }).join('')
    + '</div>';
}

function itaIntent(intent) {
  if (!intent) {
    return '<div class="ita-intent-empty">No unsigned intent built for this run.</div>';
  }
  var minOut = Array.isArray(intent.min_out) ? intent.min_out : [];
  return '<div class="ita-intent">'
    + '<div class="ita-intent-main">'
    + '<span class="ita-pill ' + itaStatusClass(intent.status) + '">' + itaEscape(intent.status || 'none') + '</span>'
    + '<strong>' + itaEscape(intent.route_label || 'NEAR Intents route') + '</strong>'
    + '<span>' + itaEscape(intent.quote_source || 'fixture') + ' quote source</span>'
    + '</div>'
    + '<div class="ita-intent-grid">'
    + '<div><span>Bundle</span><strong>' + itaEscape(intent.id || '-') + '</strong></div>'
    + '<div><span>Legs</span><strong>' + itaEscape(intent.legs || 0) + '</strong></div>'
    + '<div><span>Cost</span><strong>$' + itaEscape(intent.total_cost_usd || '0.00') + '</strong></div>'
    + '<div><span>Signer</span><strong>' + itaEscape(intent.signer_placeholder || '<signed-by-user>') + '</strong></div>'
    + '</div>'
    + (minOut.length ? '<div class="ita-minout">'
      + minOut.map(function(token) {
        return '<span>' + itaEscape(token.amount) + ' ' + itaEscape(token.symbol) + ' on ' + itaEscape(token.chain) + '</span>';
      }).join('')
      + '</div>' : '')
    + '</div>';
}

function itaMoney(value) {
  var n = Number(value || 0);
  return '$' + n.toFixed(n < 1 ? 4 : 2);
}

function itaPaidResearch(plan) {
  if (!plan) {
    return '<div class="ita-empty-line">No paid research plan recorded.</div>';
  }
  var sources = Array.isArray(plan.payable_sources) ? plan.payable_sources : [];
  var rails = Array.isArray(plan.payment_rails) ? plan.payment_rails : [];
  var routes = Array.isArray(plan.near_funding_routes) ? plan.near_funding_routes : [];
  var gates = Array.isArray(plan.policy_gates) ? plan.policy_gates : [];
  var wallet = plan.wallet_policy || null;
  return '<div class="ita-paid">'
    + '<div class="ita-paid-bar">'
    + '<div><span>Budget</span><strong>' + itaMoney(plan.budget_usd) + '</strong></div>'
    + '<div><span>Allocated</span><strong>' + itaMoney(plan.allocated_usd) + '</strong></div>'
    + '<div><span>Unspent</span><strong>' + itaMoney(plan.unspent_usd) + '</strong></div>'
    + '<div><span>Fetch</span><strong>' + (plan.ready_for_paid_fetch ? 'Ready' : 'Blocked') + '</strong></div>'
    + '</div>'
    + (plan.query ? '<div class="ita-paid-query">' + itaEscape(plan.query) + '</div>' : '')
    + '<div class="ita-paid-lanes">'
    + '<div>'
    + '<div class="ita-subtitle">Payable Sources</div>'
    + (sources.length ? sources.map(function(source) {
      return '<div class="ita-source-line">'
        + '<div><strong>' + itaEscape(source.title || source.id) + '</strong>'
        + '<span>' + itaEscape(source.author || 'Unknown author') + ' · ' + itaEscape(source.protocol || 'manual') + ' / ' + itaEscape(source.network || 'manual') + '</span></div>'
        + '<div class="ita-source-price">' + itaMoney(source.amount_usd) + '</div>'
        + (source.receipt_required ? '<span class="ita-pill is-warn">Receipt</span>' : '<span class="ita-pill is-pass">Free</span>')
        + '</div>';
    }).join('') : '<div class="ita-empty-line">No payable sources selected.</div>')
    + '</div>'
    + '<div>'
    + '<div class="ita-subtitle">Rails</div>'
    + (rails.length ? rails.map(function(rail) {
      return '<div class="ita-rail-line"><span>' + itaEscape(rail.protocol || 'manual') + '</span><strong>' + itaMoney(rail.allocated_usd) + '</strong></div>';
    }).join('') : '<div class="ita-empty-line">No payment rails.</div>')
    + (routes.length ? '<div class="ita-route-note">' + routes.map(function(route) {
      return itaEscape(route.via || 'near-intents') + ' -> ' + itaEscape(route.target_protocol || 'rail') + ' ' + itaMoney(route.amount_usd);
    }).join('<br>') + '</div>' : '')
    + '</div>'
    + '</div>'
    + (wallet ? '<div class="ita-wallet-line">'
      + '<div><span>Agent Wallet</span><strong>' + itaEscape(wallet.provider || 'Agent wallet') + ' · ' + itaEscape(wallet.network || 'base') + '</strong></div>'
      + '<div><span>Balance</span><strong>' + itaMoney(wallet.balance_usd) + '</strong></div>'
      + '<div><span>Articles</span><strong>' + itaEscape(wallet.max_articles_at_default_price || 0) + '</strong></div>'
      + '<span class="ita-pill ' + (wallet.safe_to_autopay ? 'is-pass' : 'is-warn') + '">' + (wallet.safe_to_autopay ? 'Policy OK' : 'Approval') + '</span>'
      + '</div>'
      + (Array.isArray(wallet.audit_urls) && wallet.audit_urls.length ? '<div class="ita-audit-links">'
        + wallet.audit_urls.map(function(url) {
          return '<a href="' + itaEscape(url) + '" target="_blank" rel="noreferrer">' + itaEscape(String(url).replace(/^https?:\/\//, '').replace(/\/$/, '')) + '</a>';
        }).join('')
        + '</div>' : '') : '')
    + (gates.length ? '<div class="ita-paid-gates">' + itaRiskGates(gates) + '</div>' : '')
    + '</div>';
}

function itaTrialPlan(plan) {
  if (!plan) {
    return '<div class="ita-empty-line">No nominal NEAR trial plan recorded.</div>';
  }
  var steps = Array.isArray(plan.setup_steps) ? plan.setup_steps : [];
  var gates = Array.isArray(plan.risk_gates) ? plan.risk_gates : [];
  return '<div class="ita-trial">'
    + '<div class="ita-trial-bar">'
    + '<div><span>Budget</span><strong>' + itaEscape(plan.nominal_near || 0) + ' NEAR</strong></div>'
    + '<div><span>Trade cap</span><strong>' + itaEscape(plan.max_trade_near || 0) + ' NEAR</strong></div>'
    + '<div><span>USD est.</span><strong>' + itaMoney(plan.max_trade_usd) + '</strong></div>'
    + '<div><span>Quote</span><strong>' + (plan.safe_to_quote ? 'Ready' : 'Blocked') + '</strong></div>'
    + '</div>'
    + '<div class="ita-trial-meta">'
    + '<span class="ita-pill ' + itaStatusClass(plan.safe_to_quote ? 'pass' : 'blocked') + '">' + itaEscape(plan.mode || 'paper') + '</span>'
    + (plan.recommended_strategy_id ? '<strong>' + itaEscape(plan.recommended_strategy_id) + '</strong>' : '<strong>No strategy selected</strong>')
    + '<span>' + itaEscape(plan.pair || 'NEAR/USDC') + '</span>'
    + '</div>'
    + (steps.length ? '<div class="ita-trial-steps">'
      + steps.slice(0, 5).map(function(step) {
        return '<div><span>' + itaEscape(step.order || '') + '</span><strong>' + itaEscape(step.name || 'Step') + '</strong><small>' + itaEscape(step.status || 'manual') + '</small></div>';
      }).join('')
      + '</div>' : '')
    + (gates.length ? '<div class="ita-trial-gates">' + itaRiskGates(gates.slice(0, 5)) + '</div>' : '')
    + (plan.next_action ? '<div class="ita-trial-next">' + itaEscape(plan.next_action) + '</div>' : '')
    + '</div>';
}

function itaWritePrompt(kind, state) {
  if (api && api.navigate) api.navigate('chat');
  var input = document.getElementById('chat-input');
  if (!input) return;
  var pair = state && state.pair ? state.pair : 'the watched pair';
  input.value = kind === 'trial'
    ? 'For the Intents Trading Agent, prepare a nominal NEAR trial for ' + pair + ': use paper mode first, cap the trial wallet, show the strategy menu, run backtest_suite before any live quote, and keep all NEAR Intents payloads unsigned.'
    : kind === 'paid'
    ? 'For the Intents Trading Agent, build a paid research plan for ' + pair + ' first: discover MPP/x402/NEAR payable sources, enforce the source budget, require receipts before use, then run backtest_suite and risk gates before any unsigned NEAR intent.'
    : kind === 'quote'
    ? 'For the Intents Trading Agent, request a live NEAR Intents quote for ' + pair + ' only if all current risk gates still pass. Keep it unsigned.'
    : 'For the Intents Trading Agent, run the paper NEAR Intents workflow for ' + pair + ': research, backtest_suite, risk gates, and unsigned intent preview.';
  input.focus();
  if (typeof autoGrow === 'function') autoGrow(input);
}

function itaRender(state) {
  var statusClass = itaStatusClass(state.stance);
  root.innerHTML = '<div class="ita-head">'
    + '<div>'
    + '<div class="ita-kicker">NEAR Intents Trading Agent</div>'
    + '<h3>' + itaEscape(state.pair || 'Watchlist') + '</h3>'
    + '</div>'
    + '<div class="ita-head-actions">'
    + '<span class="ita-pill ' + statusClass + '">' + itaEscape(state.stance || 'watch') + '</span>'
    + '<button type="button" data-action="ita-refresh">Refresh</button>'
    + '</div>'
    + '</div>'
    + '<div class="ita-summary">'
    + '<div><span>Mode</span><strong>' + itaEscape(state.mode || 'paper') + '</strong></div>'
    + '<div><span>Confidence</span><strong>' + (state.confidence == null ? '-' : Math.round(Number(state.confidence) * 100) + '%') + '</strong></div>'
    + '<div><span>Sources</span><strong>' + itaEscape(state.paid_research ? state.paid_research.selected_count : state.source_count || 0) + '</strong></div>'
    + '<div><span>Updated</span><strong>' + itaEscape(state.generated_at || '-') + '</strong></div>'
    + '</div>'
    + '<div class="ita-body">'
    + '<section><div class="ita-section-title">Nominal NEAR Trial</div>' + itaTrialPlan(state.trial_plan) + '</section>'
    + '<section><div class="ita-section-title">Paid Research</div>' + itaPaidResearch(state.paid_research) + '</section>'
    + '<section><div class="ita-section-title">Strategy Suite</div>' + itaCandidateRows(state.top_candidates) + '</section>'
    + '<section><div class="ita-section-title">Risk Gates</div>' + itaRiskGates(state.risk_gates) + '</section>'
    + '<section><div class="ita-section-title">Unsigned Intent</div>' + itaIntent(state.intent) + '</section>'
    + '</div>'
    + (state.next_action ? '<div class="ita-next">' + itaEscape(state.next_action) + '</div>' : '')
    + '<div class="ita-actions">'
    + '<button type="button" data-action="ita-trial">Prepare NEAR Trial</button>'
    + '<button type="button" data-action="ita-paid">Prepare Paid Research</button>'
    + '<button type="button" data-action="ita-paper">Prepare Paper Run</button>'
    + '<button type="button" data-action="ita-quote">Prepare Live Quote Request</button>'
    + '</div>';

  root.querySelector('[data-action="ita-refresh"]').addEventListener('click', itaLoad);
  root.querySelector('[data-action="ita-trial"]').addEventListener('click', function() {
    itaWritePrompt('trial', state);
  });
  root.querySelector('[data-action="ita-paid"]').addEventListener('click', function() {
    itaWritePrompt('paid', state);
  });
  root.querySelector('[data-action="ita-paper"]').addEventListener('click', function() {
    itaWritePrompt('paper', state);
  });
  root.querySelector('[data-action="ita-quote"]').addEventListener('click', function() {
    itaWritePrompt('quote', state);
  });
}

function itaRenderEmpty() {
  root.innerHTML = '<div class="ita-empty">'
    + '<div class="ita-kicker">NEAR Intents Trading Agent</div>'
    + '<h3>No console state yet</h3>'
    + '<p>Run the paper workflow to produce ranked strategies, risk gates, and an unsigned intent preview.</p>'
    + '<button type="button" data-action="ita-paper-empty">Prepare Paper Run</button>'
    + '</div>';
  root.querySelector('[data-action="ita-paper-empty"]').addEventListener('click', function() {
    itaWritePrompt('paper', { pair: 'the configured watchlist' });
  });
}

function itaRenderLoading() {
  root.innerHTML = '<div class="ita-loading">Loading NEAR Intents console...</div>';
}

function itaLoad() {
  itaRenderLoading();
  itaLoadProjectSlug()
    .then(itaReadState)
    .then(itaRender)
    .catch(itaRenderEmpty);
}

itaLoad();
