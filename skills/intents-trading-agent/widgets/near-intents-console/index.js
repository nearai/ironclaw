var root = document.createElement('section');
root.className = 'ita-console';
container.appendChild(root);

var itaLastState = null;

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

function itaMoney(value) {
  var n = Number(value || 0);
  return '$' + n.toFixed(n < 1 ? 4 : 2);
}

function itaNumber(value, fallback) {
  if (value === '') return fallback;
  var n = Number(value);
  return Number.isFinite(n) ? n : fallback;
}

function itaStatusClass(status) {
  var s = String(status || 'unknown').toLowerCase();
  if (s === 'pass' || s === 'paper-built' || s === 'live-quote-built' || s === 'ready') return 'is-pass';
  if (s === 'warn' || s === 'awaiting-signature' || s === 'watch' || s === 'manual') return 'is-warn';
  if (s === 'fail' || s === 'failed' || s === 'blocked') return 'is-fail';
  return 'is-neutral';
}

function itaStoreKey() {
  return 'ironclaw.intentsTradingAgent.ticket';
}

function itaReadPrefs() {
  if (typeof localStorage === 'undefined') return {};
  try {
    return JSON.parse(localStorage.getItem(itaStoreKey()) || '{}') || {};
  } catch (_) {
    return {};
  }
}

function itaSavePrefs(prefs) {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(itaStoreKey(), JSON.stringify(prefs || {}));
}

function itaTrialDefaults(state) {
  var plan = state && state.trial_plan ? state.trial_plan : {};
  var prefs = itaReadPrefs();
  return {
    pair: prefs.pair || plan.pair || (state && state.pair) || 'NEAR/USDC',
    mode: prefs.mode || plan.mode || (state && state.mode) || 'paper',
    nearAccount: prefs.nearAccount || '',
    nominalNear: itaNumber(prefs.nominalNear, itaNumber(plan.nominal_near, 0.25)),
    maxTradeNear: itaNumber(prefs.maxTradeNear, itaNumber(plan.max_trade_near, 0.05)),
    assumedNearUsd: itaNumber(prefs.assumedNearUsd, itaNumber(plan.assumed_near_usd, 3.0)),
    maxSlippageBps: itaNumber(prefs.maxSlippageBps, 50),
    selectedStrategyId: prefs.selectedStrategyId || plan.selected_strategy_id || plan.recommended_strategy_id || itaFirstStrategyId(state),
    fundingPath: prefs.fundingPath || 'managed'
  };
}

function itaPersistPref(name, value) {
  var prefs = itaReadPrefs();
  prefs[name] = value;
  itaSavePrefs(prefs);
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

function itaAllStrategies(state) {
  var seen = {};
  var out = [];
  var top = Array.isArray(state && state.top_candidates) ? state.top_candidates : [];
  var menu = state && state.trial_plan && Array.isArray(state.trial_plan.strategy_menu)
    ? state.trial_plan.strategy_menu
    : [];

  top.forEach(function(candidate) {
    if (!candidate || seen[candidate.id]) return;
    seen[candidate.id] = true;
    out.push({
      id: candidate.id,
      kind: candidate.strategy_kind || 'strategy',
      rank: candidate.rank,
      score: candidate.selection_score,
      returnPct: candidate.total_return_pct,
      alphaPct: candidate.alpha_vs_buy_hold_pct,
      drawdownPct: candidate.max_drawdown_pct,
      trades: candidate.trades,
      passes: !!candidate.passes_basic_gate,
      note: 'Backtested candidate'
    });
  });

  menu.forEach(function(strategy) {
    if (!strategy || seen[strategy.id]) return;
    seen[strategy.id] = true;
    out.push({
      id: strategy.id,
      kind: strategy.kind || 'strategy',
      rank: null,
      score: null,
      returnPct: null,
      alphaPct: null,
      drawdownPct: null,
      trades: null,
      passes: null,
      note: strategy.note || '',
      sizePct: strategy.position_size_pct,
      stopLossBps: strategy.stop_loss_bps
    });
  });

  return out;
}

function itaFirstStrategyId(state) {
  var strategies = itaAllStrategies(state || {});
  return strategies.length ? strategies[0].id : 'sma_cross_fast';
}

function itaWorkflow(state) {
  var plan = state && state.trial_plan;
  var candidates = Array.isArray(state && state.top_candidates) ? state.top_candidates : [];
  var passing = candidates.some(function(candidate) { return candidate.passes_basic_gate; });
  var intent = state && state.intent;
  var paperBuilt = !!intent && (intent.status === 'paper-built' || intent.status === 'live-quote-built');
  var quoteBuilt = !!intent && intent.status === 'live-quote-built';
  return [
    { label: 'Ticket', status: plan ? 'pass' : 'warn', detail: plan ? 'Trial plan ready' : 'Needs plan' },
    { label: 'Backtest', status: passing ? 'pass' : candidates.length ? 'fail' : 'warn', detail: passing ? 'Passing strategy' : 'Run suite' },
    { label: 'Paper', status: paperBuilt ? 'pass' : 'warn', detail: paperBuilt ? 'Fixture built' : 'Build fixture' },
    { label: 'Quote', status: quoteBuilt ? 'pass' : plan && plan.safe_to_quote ? 'ready' : 'blocked', detail: quoteBuilt ? 'Live quote built' : plan && plan.safe_to_quote ? 'Ready' : 'Blocked' }
  ];
}

function itaPhaseRail(state) {
  return '<div class="ita-phases" aria-label="Trial workflow">'
    + itaWorkflow(state).map(function(phase, index) {
      return '<div class="ita-phase ' + itaStatusClass(phase.status) + '">'
        + '<span>' + (index + 1) + '</span>'
        + '<strong>' + itaEscape(phase.label) + '</strong>'
        + '<small>' + itaEscape(phase.detail) + '</small>'
        + '</div>';
    }).join('')
    + '</div>';
}

function itaTicket(state, prefs) {
  var modeOptions = ['paper', 'quote', 'execution'];
  var fundingOptions = [
    { id: 'managed', label: '1Click / Deposit API' },
    { id: 'direct', label: 'Direct Verifier' }
  ];

  return '<section class="ita-ticket">'
    + '<div class="ita-section-head"><div><div class="ita-section-title">Trial Ticket</div><p>Nominal NEAR sizing, route mode, and funding path.</p></div></div>'
    + '<div class="ita-ticket-grid">'
    + itaField('Pair', 'pair', prefs.pair, 'text', 'NEAR/USDC')
    + itaField('NEAR account', 'nearAccount', prefs.nearAccount, 'text', 'small-test.near')
    + itaField('Budget NEAR', 'nominalNear', prefs.nominalNear, 'number', '0.25', '0.01', '0.01')
    + itaField('Trade cap NEAR', 'maxTradeNear', prefs.maxTradeNear, 'number', '0.05', '0.01', '0.01')
    + itaField('NEAR USD', 'assumedNearUsd', prefs.assumedNearUsd, 'number', '3.00', '0.01', '0.01')
    + itaField('Slippage bps', 'maxSlippageBps', prefs.maxSlippageBps, 'number', '50', '1', '1')
    + '</div>'
    + '<div class="ita-segments" role="group" aria-label="Mode">'
    + modeOptions.map(function(mode) {
      return '<button type="button" class="' + (prefs.mode === mode ? 'is-selected' : '') + '" data-pref-button="mode" data-value="' + itaEscape(mode) + '">' + itaEscape(mode) + '</button>';
    }).join('')
    + '</div>'
    + '<div class="ita-segments" role="group" aria-label="Funding path">'
    + fundingOptions.map(function(option) {
      return '<button type="button" class="' + (prefs.fundingPath === option.id ? 'is-selected' : '') + '" data-pref-button="fundingPath" data-value="' + itaEscape(option.id) + '">' + itaEscape(option.label) + '</button>';
    }).join('')
    + '</div>'
    + '</section>';
}

function itaField(label, name, value, type, placeholder, step, min) {
  return '<label class="ita-field">'
    + '<span>' + itaEscape(label) + '</span>'
    + '<input data-pref="' + itaEscape(name) + '" type="' + itaEscape(type) + '" value="' + itaEscape(value == null ? '' : value) + '" placeholder="' + itaEscape(placeholder || '') + '"'
    + (step ? ' step="' + itaEscape(step) + '"' : '')
    + (min ? ' min="' + itaEscape(min) + '"' : '')
    + '>'
    + '</label>';
}

function itaStrategyLab(state, prefs) {
  var strategies = itaAllStrategies(state);
  if (!strategies.length) {
    strategies = [{
      id: prefs.selectedStrategyId || 'sma_cross_fast',
      kind: 'sma-cross',
      rank: null,
      score: null,
      returnPct: null,
      alphaPct: null,
      drawdownPct: null,
      trades: null,
      passes: null,
      note: 'Default strategy menu loads after plan_near_intents_trial.'
    }];
  }

  return '<section class="ita-strategy-lab">'
    + '<div class="ita-section-head"><div><div class="ita-section-title">Strategy Lab</div><p>Select a strategy before paper or quote mode.</p></div>'
    + '<button type="button" data-run="backtest">Run Backtest Suite</button></div>'
    + '<div class="ita-strategy-list">'
    + strategies.map(function(strategy) {
      var selected = prefs.selectedStrategyId === strategy.id;
      var gate = strategy.passes == null ? 'manual' : strategy.passes ? 'pass' : 'fail';
      return '<button type="button" class="ita-strategy-row ' + (selected ? 'is-selected ' : '') + itaStatusClass(gate) + '" data-strategy="' + itaEscape(strategy.id) + '">'
        + '<span class="ita-rank">' + (strategy.rank ? '#' + itaEscape(strategy.rank) : '-') + '</span>'
        + '<span class="ita-strategy-name"><strong>' + itaEscape(strategy.id) + '</strong><small>' + itaEscape(strategy.kind) + '</small></span>'
        + '<span><small>Score</small><strong>' + (strategy.score == null ? '-' : itaScore(strategy.score)) + '</strong></span>'
        + '<span><small>Return</small><strong>' + (strategy.returnPct == null ? '-' : itaPct(strategy.returnPct)) + '</strong></span>'
        + '<span><small>Alpha</small><strong>' + (strategy.alphaPct == null ? '-' : itaPct(strategy.alphaPct)) + '</strong></span>'
        + '<span><small>DD</small><strong>' + (strategy.drawdownPct == null ? '-' : itaPct(-Math.abs(Number(strategy.drawdownPct || 0)))) + '</strong></span>'
        + '<span class="ita-pill ' + itaStatusClass(gate) + '">' + itaEscape(gate) + '</span>'
        + (strategy.note ? '<em>' + itaEscape(strategy.note) + '</em>' : '')
        + '</button>';
    }).join('')
    + '</div>'
    + '</section>';
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

function itaTrialPlan(plan) {
  if (!plan) {
    return '<div class="ita-empty-line">No nominal NEAR trial plan recorded.</div>';
  }
  return '<div class="ita-trial">'
    + '<div class="ita-trial-bar">'
    + '<div><span>Budget</span><strong>' + itaEscape(plan.nominal_near || 0) + ' NEAR</strong></div>'
    + '<div><span>Trade cap</span><strong>' + itaEscape(plan.max_trade_near || 0) + ' NEAR</strong></div>'
    + '<div><span>USD est.</span><strong>' + itaMoney(plan.max_trade_usd) + '</strong></div>'
    + '<div><span>Quote</span><strong>' + (plan.safe_to_quote ? 'Ready' : 'Blocked') + '</strong></div>'
    + '</div>'
    + (plan.next_action ? '<div class="ita-trial-next">' + itaEscape(plan.next_action) + '</div>' : '')
    + '</div>';
}

function itaSetupPanel(state) {
  var plan = state && state.trial_plan;
  var steps = plan && Array.isArray(plan.setup_steps) ? plan.setup_steps : [];
  var gates = []
    .concat(Array.isArray(state && state.risk_gates) ? state.risk_gates : [])
    .concat(plan && Array.isArray(plan.risk_gates) ? plan.risk_gates : []);
  var warnings = plan && Array.isArray(plan.warnings) ? plan.warnings : [];

  return '<section class="ita-setup">'
    + '<div class="ita-section-head"><div><div class="ita-section-title">Funding And Gates</div><p>Wallet setup, quote blockers, and signing boundary.</p></div></div>'
    + (steps.length ? '<div class="ita-steps">'
      + steps.map(function(step) {
        return '<div><span>' + itaEscape(step.order || '') + '</span><strong>' + itaEscape(step.name || 'Step') + '</strong><small>' + itaEscape(step.status || 'manual') + '</small><p>' + itaEscape(step.detail || '') + '</p></div>';
      }).join('')
      + '</div>' : '<div class="ita-empty-line">No setup steps recorded.</div>')
    + '<div class="ita-section-title ita-tight-title">Risk Gates</div>'
    + itaRiskGates(gates)
    + (warnings.length ? '<div class="ita-warnings">'
      + warnings.map(function(warning) { return '<span>' + itaEscape(warning) + '</span>'; }).join('')
      + '</div>' : '')
    + '</section>';
}

function itaRunPanel(state, prefs) {
  var plan = state && state.trial_plan;
  var quoteReady = !!(plan && plan.safe_to_quote);
  var hasPlan = !!plan;
  var intent = state && state.intent;
  var hasPaperIntent = !!intent && (intent.status === 'paper-built' || intent.status === 'live-quote-built');

  return '<section class="ita-run-panel">'
    + '<div class="ita-section-head"><div><div class="ita-section-title">Paper And Quote</div><p>Build fixture first, then request an unsigned live quote.</p></div></div>'
    + '<div class="ita-run-grid">'
    + '<div><span>Selected</span><strong>' + itaEscape(prefs.selectedStrategyId || '-') + '</strong></div>'
    + '<div><span>Mode</span><strong>' + itaEscape(prefs.mode) + '</strong></div>'
    + '<div><span>Funding</span><strong>' + itaEscape(prefs.fundingPath === 'direct' ? 'Direct Verifier' : '1Click / Deposit API') + '</strong></div>'
    + '<div><span>Live quote</span><strong>' + (quoteReady ? 'Ready' : 'Blocked') + '</strong></div>'
    + '</div>'
    + itaTrialPlan(plan)
    + itaIntent(intent)
    + '<div class="ita-run-actions">'
    + '<button type="button" data-run="plan">Plan Trial</button>'
    + '<button type="button" data-run="paper" ' + (hasPlan ? '' : 'disabled') + '>Build Paper Intent</button>'
    + '<button type="button" data-run="quote" ' + (quoteReady && hasPaperIntent ? '' : 'disabled') + '>Request Live Quote</button>'
    + '<button type="button" data-run="paid">Paid Research</button>'
    + '</div>'
    + '</section>';
}

function itaPaidResearch(plan) {
  if (!plan) {
    return '<div class="ita-empty-line">No paid research plan recorded.</div>';
  }
  var sources = Array.isArray(plan.payable_sources) ? plan.payable_sources : [];
  var rails = Array.isArray(plan.payment_rails) ? plan.payment_rails : [];
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
        + '<span>' + itaEscape(source.author || 'Unknown author') + ' / ' + itaEscape(source.protocol || 'manual') + ' / ' + itaEscape(source.network || 'manual') + '</span></div>'
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
    + (wallet ? '<div class="ita-wallet-line">'
      + '<div><span>Agent Wallet</span><strong>' + itaEscape(wallet.provider || 'Agent wallet') + ' / ' + itaEscape(wallet.network || 'base') + '</strong></div>'
      + '<div><span>Balance</span><strong>' + itaMoney(wallet.balance_usd) + '</strong></div>'
      + '<span class="ita-pill ' + (wallet.safe_to_autopay ? 'is-pass' : 'is-warn') + '">' + (wallet.safe_to_autopay ? 'Policy OK' : 'Approval') + '</span>'
      + '</div>' : '')
    + '</div>'
    + '</div>'
    + '</div>';
}

function itaPaidPanel(state) {
  return '<section class="ita-paid-panel">'
    + '<div class="ita-section-head"><div><div class="ita-section-title">Research Budget</div><p>Paid-source spend and receipt readiness.</p></div>'
    + '<button type="button" data-run="paid">Prepare Paid Research</button></div>'
    + itaPaidResearch(state && state.paid_research)
    + '</section>';
}

function itaCleanPayload(payload) {
  var out = {};
  Object.keys(payload).forEach(function(key) {
    var value = payload[key];
    if (value === undefined || value === null || value === '') return;
    out[key] = value;
  });
  return out;
}

function itaTrialPayload(state, prefs, mode) {
  return itaCleanPayload({
    action: 'plan_near_intents_trial',
    near_account_id: prefs.nearAccount,
    mode: mode || prefs.mode || 'paper',
    pair: prefs.pair || 'NEAR/USDC',
    nominal_near: itaNumber(prefs.nominalNear, 0.25),
    max_trade_near: itaNumber(prefs.maxTradeNear, 0.05),
    assumed_near_usd: itaNumber(prefs.assumedNearUsd, 3.0),
    max_slippage_bps: Math.round(itaNumber(prefs.maxSlippageBps, 50)),
    selected_strategy_id: prefs.selectedStrategyId
  });
}

function itaWritePrompt(kind, state) {
  if (api && api.navigate) api.navigate('chat');
  var input = document.getElementById('chat-input');
  if (!input) return;

  var prefs = itaTrialDefaults(state || {});
  var pair = prefs.pair || 'NEAR/USDC';
  var payloadMode = kind === 'quote' ? 'quote' : kind === 'paper' ? 'paper' : prefs.mode || 'paper';
  var funding = prefs.fundingPath === 'direct'
    ? 'direct Verifier funding: use wNEAR/nep141:wrap.near for direct deposits and keep signing in the wallet'
    : 'managed 1Click or Deposit/Withdrawal Service funding: let the service route/wrap supported assets where applicable';
  var trialPayload = itaTrialPayload(state, prefs, payloadMode);
  var text;

  if (kind === 'plan') {
    text = 'Build and persist the nominal NEAR Intents trial frontend state.\n\nRun portfolio.plan_near_intents_trial with:\n```json\n'
      + JSON.stringify(trialPayload, null, 2)
      + '\n```\nFunding path selected in the UI: ' + funding
      + '\n\nThen run portfolio.format_intents_widget with the trial_plan, latest backtest_suite if available, and write the result to projects/intents-trading-agent/widgets/state.json.';
  } else if (kind === 'backtest') {
    text = 'Run a fresh backtest suite for the Intents Trading Agent on ' + pair + '. Include the default strategy menu plus the selected strategy `'
      + (prefs.selectedStrategyId || 'sma_cross_fast')
      + '`, rank by risk-adjusted return, reject lookahead bias, then refresh projects/intents-trading-agent/widgets/state.json with portfolio.format_intents_widget.';
  } else if (kind === 'paper') {
    text = 'Build the paper NEAR Intents run for ' + pair + '. Use the latest near-intents-trial-plan/1, selected strategy `'
      + (prefs.selectedStrategyId || 'sma_cross_fast')
      + '`, and solver=fixture. Keep the output unsigned, persist the intent preview, and refresh the widget state.';
  } else if (kind === 'quote') {
    text = 'Request an unsigned live NEAR Intents quote for ' + pair + ' only if all current trial and strategy gates still pass.\n\nFirst rerun portfolio.plan_near_intents_trial in quote mode with:\n```json\n'
      + JSON.stringify(trialPayload, null, 2)
      + '\n```\nThen build_intent with solver=near-intents. Do not sign, submit, or increase the trade size automatically. Funding path: ' + funding + '. Refresh the widget state with the unsigned quote preview.';
  } else {
    text = 'Prepare paid research for ' + pair + ' before any live quote. Use free DripStack catalog/title routes first, require explicit user confirmation before paid bodies, require payment receipts, enforce a small spend cap, then refresh the Intents Trading Agent widget.';
  }

  input.value = text;
  input.focus();
  input.dispatchEvent(new Event('input', { bubbles: true }));
  if (typeof autoGrow === 'function') autoGrow(input);
}

function itaWire(state) {
  root.querySelectorAll('[data-pref]').forEach(function(input) {
    input.addEventListener('input', function() {
      var name = input.getAttribute('data-pref');
      var value = input.type === 'number' && input.value !== '' ? Number(input.value) : input.value;
      itaPersistPref(name, value);
    });
  });

  root.querySelectorAll('[data-pref-button]').forEach(function(button) {
    button.addEventListener('click', function() {
      itaPersistPref(button.getAttribute('data-pref-button'), button.getAttribute('data-value'));
      itaRender(state);
    });
  });

  root.querySelectorAll('[data-strategy]').forEach(function(button) {
    button.addEventListener('click', function() {
      itaPersistPref('selectedStrategyId', button.getAttribute('data-strategy'));
      itaRender(state);
    });
  });

  root.querySelectorAll('[data-run]').forEach(function(button) {
    button.addEventListener('click', function() {
      if (button.disabled) return;
      itaWritePrompt(button.getAttribute('data-run'), state);
    });
  });
}

function itaRender(state) {
  itaLastState = state;
  var prefs = itaTrialDefaults(state || {});
  var statusClass = itaStatusClass(state && state.stance);
  var generatedAt = state && state.generated_at ? state.generated_at : '-';
  var sourceCount = state && state.paid_research ? state.paid_research.selected_count : state && state.source_count || 0;

  root.innerHTML = '<div class="ita-head">'
    + '<div>'
    + '<div class="ita-kicker">NEAR Intents Trading Agent</div>'
    + '<h3>' + itaEscape(prefs.pair || state.pair || 'Watchlist') + '</h3>'
    + '</div>'
    + '<div class="ita-head-actions">'
    + '<span class="ita-pill ' + statusClass + '">' + itaEscape(state && state.stance || 'watch') + '</span>'
    + '<button type="button" data-action="refresh">Refresh</button>'
    + '</div>'
    + '</div>'
    + '<div class="ita-summary">'
    + '<div><span>Mode</span><strong>' + itaEscape(prefs.mode) + '</strong></div>'
    + '<div><span>Confidence</span><strong>' + (state && state.confidence == null ? '-' : Math.round(Number(state && state.confidence || 0) * 100) + '%') + '</strong></div>'
    + '<div><span>Sources</span><strong>' + itaEscape(sourceCount) + '</strong></div>'
    + '<div><span>Updated</span><strong>' + itaEscape(generatedAt) + '</strong></div>'
    + '</div>'
    + itaPhaseRail(state || {})
    + '<div class="ita-workspace">'
    + '<div class="ita-primary">'
    + itaTicket(state || {}, prefs)
    + itaStrategyLab(state || {}, prefs)
    + itaRunPanel(state || {}, prefs)
    + '</div>'
    + '<div class="ita-secondary">'
    + itaSetupPanel(state || {})
    + itaPaidPanel(state || {})
    + '</div>'
    + '</div>'
    + (state && state.next_action ? '<div class="ita-next">' + itaEscape(state.next_action) + '</div>' : '');

  var refresh = root.querySelector('[data-action="refresh"]');
  if (refresh) refresh.addEventListener('click', itaLoad);
  itaWire(state || {});
}

function itaRenderEmpty() {
  root.innerHTML = '<div class="ita-empty">'
    + '<div class="ita-kicker">NEAR Intents Trading Agent</div>'
    + '<h3>No console state yet</h3>'
    + '<p>Start with a nominal NEAR trial ticket and fixture-only paper run.</p>'
    + '<button type="button" data-run="plan">Plan Trial</button>'
    + '</div>';
  root.querySelector('[data-run="plan"]').addEventListener('click', function() {
    itaWritePrompt('plan', { pair: 'NEAR/USDC', mode: 'paper' });
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
