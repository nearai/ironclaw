(function () {
  'use strict';

  var strategies = [
    {
      id: 'sma_cross',
      name: 'SMA Cross',
      copy: 'Trend-following spot strategy using 12h and 48h moving averages.'
    },
    {
      id: 'mean_reversion',
      name: 'Mean Reversion',
      copy: 'Range strategy using a 20h z-score and volatility-aware exits.'
    },
    {
      id: 'breakout',
      name: 'Breakout',
      copy: 'Momentum strategy that reacts to 24h channel breaks.'
    },
    {
      id: 'hl_momentum_watch',
      name: 'HL Momentum Watch',
      copy: 'Hyperliquid-style momentum watcher with spot-only NEAR Intents output.'
    }
  ];

  var state = {
    candles: [],
    pair: 'NEAR-USD',
    source: 'Waiting',
    selectedStrategy: 'sma_cross',
    result: null,
    lastIntent: null
  };

  var els = {
    runLiveBtn: document.getElementById('runLiveBtn'),
    pairSelect: document.getElementById('pairSelect'),
    lookbackSelect: document.getElementById('lookbackSelect'),
    strategyList: document.getElementById('strategyList'),
    accountInput: document.getElementById('accountInput'),
    budgetInput: document.getElementById('budgetInput'),
    tradeCapInput: document.getElementById('tradeCapInput'),
    slippageInput: document.getElementById('slippageInput'),
    sourceLabel: document.getElementById('sourceLabel'),
    priceCaption: document.getElementById('priceCaption'),
    priceLabel: document.getElementById('priceLabel'),
    signalLabel: document.getElementById('signalLabel'),
    strategyTitle: document.getElementById('strategyTitle'),
    statusPill: document.getElementById('statusPill'),
    priceCanvas: document.getElementById('priceCanvas'),
    returnMetric: document.getElementById('returnMetric'),
    drawdownMetric: document.getElementById('drawdownMetric'),
    tradesMetric: document.getElementById('tradesMetric'),
    confidenceMetric: document.getElementById('confidenceMetric'),
    signalNote: document.getElementById('signalNote'),
    buildIntentBtn: document.getElementById('buildIntentBtn'),
    copyIntentBtn: document.getElementById('copyIntentBtn'),
    intentOutput: document.getElementById('intentOutput')
  };

  function fmtUsd(n) {
    if (!Number.isFinite(n)) return '-';
    return '$' + n.toLocaleString(undefined, {
      minimumFractionDigits: n < 10 ? 4 : 2,
      maximumFractionDigits: n < 10 ? 4 : 2
    });
  }

  function fmtPct(n) {
    if (!Number.isFinite(n)) return '-';
    var sign = n > 0 ? '+' : '';
    return sign + (n * 100).toFixed(2) + '%';
  }

  function clamp(n, min, max) {
    return Math.max(min, Math.min(max, n));
  }

  function average(values) {
    if (!values.length) return 0;
    return values.reduce(function (sum, value) { return sum + value; }, 0) / values.length;
  }

  function stdev(values) {
    if (values.length < 2) return 0;
    var mean = average(values);
    var variance = average(values.map(function (value) {
      return Math.pow(value - mean, 2);
    }));
    return Math.sqrt(variance);
  }

  function movingAverage(candles, end, period) {
    if (end + 1 < period) return null;
    var slice = candles.slice(end + 1 - period, end + 1).map(function (c) { return c.close; });
    return average(slice);
  }

  function setStatus(text, tone) {
    els.statusPill.textContent = text;
    els.statusPill.className = 'status-pill' + (tone ? ' ' + tone : '');
  }

  function selectedStrategy() {
    return strategies.find(function (strategy) {
      return strategy.id === state.selectedStrategy;
    }) || strategies[0];
  }

  function renderStrategies() {
    els.strategyList.innerHTML = '';
    strategies.forEach(function (strategy) {
      var button = document.createElement('button');
      button.type = 'button';
      button.setAttribute('role', 'radio');
      button.setAttribute('aria-checked', strategy.id === state.selectedStrategy ? 'true' : 'false');
      button.innerHTML = '<strong>' + strategy.name + '</strong><span>' + strategy.copy + '</span>';
      button.addEventListener('click', function () {
        state.selectedStrategy = strategy.id;
        renderStrategies();
        els.strategyTitle.textContent = strategy.name;
        if (state.candles.length) runStrategy();
      });
      els.strategyList.appendChild(button);
    });
  }

  async function fetchCoinbaseCandles(pair, days) {
    var end = Math.floor(Date.now() / 1000);
    var start = end - Number(days) * 24 * 60 * 60;
    var url = 'https://api.exchange.coinbase.com/products/' + encodeURIComponent(pair) +
      '/candles?granularity=3600&start=' + new Date(start * 1000).toISOString() +
      '&end=' + new Date(end * 1000).toISOString();
    var res = await fetch(url, { headers: { Accept: 'application/json' } });
    if (!res.ok) throw new Error('Coinbase candles failed: ' + res.status);
    var rows = await res.json();
    if (!Array.isArray(rows) || rows.length < 24) throw new Error('Coinbase returned too few candles');
    return rows.map(function (row) {
      return {
        time: row[0] * 1000,
        low: Number(row[1]),
        high: Number(row[2]),
        open: Number(row[3]),
        close: Number(row[4]),
        volume: Number(row[5])
      };
    }).sort(function (a, b) { return a.time - b.time; });
  }

  async function fetchCoingeckoNear(days) {
    var url = 'https://api.coingecko.com/api/v3/coins/near/market_chart?vs_currency=usd&days=' +
      encodeURIComponent(days) + '&interval=hourly';
    var res = await fetch(url, { headers: { Accept: 'application/json' } });
    if (!res.ok) throw new Error('CoinGecko failed: ' + res.status);
    var body = await res.json();
    if (!body.prices || body.prices.length < 24) throw new Error('CoinGecko returned too few prices');
    return body.prices.map(function (row) {
      var price = Number(row[1]);
      return {
        time: Number(row[0]),
        low: price,
        high: price,
        open: price,
        close: price,
        volume: 0
      };
    });
  }

  async function fetchCandles() {
    var pair = els.pairSelect.value;
    var days = els.lookbackSelect.value;
    state.pair = pair;
    try {
      state.source = 'Coinbase hourly';
      return await fetchCoinbaseCandles(pair, days);
    } catch (err) {
      if (pair !== 'NEAR-USD') throw err;
      state.source = 'CoinGecko hourly';
      return fetchCoingeckoNear(days);
    }
  }

  function syntheticFallbackCandles(pair, days) {
    var count = Number(days) * 24;
    var baseByPair = {
      'NEAR-USD': 1.28,
      'BTC-USD': 94500,
      'ETH-USD': 1820,
      'SOL-USD': 126
    };
    var base = baseByPair[pair] || 1;
    var candles = [];
    var now = Date.now();
    var drift = pair === 'BTC-USD' ? 0.00008 : 0.00018;

    for (var i = 0; i < count; i += 1) {
      var wave = Math.sin(i / 9) * 0.016 + Math.cos(i / 31) * 0.026;
      var shock = Math.sin(i / 3.7) * 0.005;
      var trend = (i - count / 2) * drift;
      var close = base * (1 + trend + wave + shock);
      candles.push({
        time: now - (count - i) * 60 * 60 * 1000,
        low: close * 0.994,
        high: close * 1.006,
        open: close * (1 - shock / 2),
        close: close,
        volume: 0
      });
    }

    state.source = 'Sample fallback';
    return candles;
  }

  function signalAt(candles, index, strategyId) {
    var candle = candles[index];
    if (!candle) return { side: 'hold', score: 0, reason: 'No candle' };

    if (strategyId === 'sma_cross') {
      var fast = movingAverage(candles, index, 12);
      var slow = movingAverage(candles, index, 48);
      if (!fast || !slow) return { side: 'hold', score: 0, reason: 'Waiting for SMA warmup' };
      var spread = (fast - slow) / slow;
      if (spread > 0.006) return { side: 'buy', score: clamp(spread * 900, 0.15, 0.92), reason: 'Fast average is above slow average.' };
      if (spread < -0.006) return { side: 'sell', score: clamp(Math.abs(spread) * 900, 0.15, 0.92), reason: 'Fast average is below slow average.' };
      return { side: 'hold', score: 0.34, reason: 'Averages are compressed.' };
    }

    if (strategyId === 'mean_reversion') {
      if (index < 20) return { side: 'hold', score: 0, reason: 'Waiting for range warmup' };
      var window = candles.slice(index - 19, index + 1).map(function (c) { return c.close; });
      var mean = average(window);
      var sigma = stdev(window);
      var z = sigma > 0 ? (candle.close - mean) / sigma : 0;
      if (z < -1.15) return { side: 'buy', score: clamp(Math.abs(z) / 2.8, 0.22, 0.88), reason: 'Price is below its local range.' };
      if (z > 1.15) return { side: 'sell', score: clamp(Math.abs(z) / 2.8, 0.22, 0.88), reason: 'Price is above its local range.' };
      return { side: 'hold', score: 0.36, reason: 'Price is near fair range.' };
    }

    if (strategyId === 'breakout') {
      if (index < 24) return { side: 'hold', score: 0, reason: 'Waiting for channel warmup' };
      var channel = candles.slice(index - 24, index);
      var high = Math.max.apply(null, channel.map(function (c) { return c.high; }));
      var low = Math.min.apply(null, channel.map(function (c) { return c.low; }));
      if (candle.close > high) return { side: 'buy', score: 0.78, reason: 'Price cleared the 24h channel high.' };
      if (candle.close < low) return { side: 'sell', score: 0.78, reason: 'Price lost the 24h channel low.' };
      return { side: 'hold', score: 0.31, reason: 'Price remains inside the channel.' };
    }

    if (index < 12) return { side: 'hold', score: 0, reason: 'Waiting for momentum warmup' };
    var prior = candles[index - 12].close;
    var momentum = prior > 0 ? (candle.close - prior) / prior : 0;
    if (momentum > 0.025) return { side: 'buy', score: clamp(momentum * 10, 0.24, 0.86), reason: '12h momentum is positive; paper-only spot route.' };
    if (momentum < -0.025) return { side: 'sell', score: clamp(Math.abs(momentum) * 10, 0.24, 0.86), reason: '12h momentum is negative; paper-only de-risk route.' };
    return { side: 'hold', score: 0.35, reason: 'Momentum is not decisive.' };
  }

  function backtest(candles, strategyId) {
    var equity = 1;
    var peak = 1;
    var maxDrawdown = 0;
    var position = 0;
    var trades = 0;
    var curve = [];

    for (var i = 1; i < candles.length; i += 1) {
      var signal = signalAt(candles, i - 1, strategyId);
      var nextPosition = signal.side === 'buy' ? 1 : signal.side === 'sell' ? 0 : position;
      if (nextPosition !== position) trades += 1;
      position = nextPosition;
      var ret = candles[i - 1].close > 0 ? (candles[i].close - candles[i - 1].close) / candles[i - 1].close : 0;
      equity *= 1 + position * ret;
      peak = Math.max(peak, equity);
      maxDrawdown = Math.max(maxDrawdown, peak > 0 ? (peak - equity) / peak : 0);
      curve.push(equity);
    }

    var latest = signalAt(candles, candles.length - 1, strategyId);
    var totalReturn = equity - 1;
    var confidence = clamp(0.45 + latest.score * 0.42 + Math.min(Math.max(totalReturn, -0.15), 0.25), 0.05, 0.93);
    return {
      latest: latest,
      totalReturn: totalReturn,
      maxDrawdown: maxDrawdown,
      trades: trades,
      confidence: confidence,
      curve: curve
    };
  }

  function drawChart() {
    var canvas = els.priceCanvas;
    var ctx = canvas.getContext('2d');
    var width = canvas.width;
    var height = canvas.height;
    var candles = state.candles;
    ctx.clearRect(0, 0, width, height);
    ctx.fillStyle = '#080a0c';
    ctx.fillRect(0, 0, width, height);

    if (!candles.length) {
      ctx.fillStyle = '#9aa69c';
      ctx.font = '28px system-ui';
      ctx.fillText('Run live strategy to draw recent candles', 44, height / 2);
      return;
    }

    var closes = candles.map(function (c) { return c.close; });
    var min = Math.min.apply(null, closes);
    var max = Math.max.apply(null, closes);
    var pad = Math.max((max - min) * 0.12, max * 0.01);
    var lo = min - pad;
    var hi = max + pad;

    function x(i) {
      return candles.length <= 1 ? 0 : (i / (candles.length - 1)) * width;
    }

    function y(price) {
      return height - ((price - lo) / (hi - lo)) * height;
    }

    ctx.strokeStyle = 'rgba(243, 246, 240, 0.08)';
    ctx.lineWidth = 1;
    for (var g = 1; g < 5; g += 1) {
      ctx.beginPath();
      ctx.moveTo(0, (height / 5) * g);
      ctx.lineTo(width, (height / 5) * g);
      ctx.stroke();
    }

    var gradient = ctx.createLinearGradient(0, 0, 0, height);
    gradient.addColorStop(0, 'rgba(110, 240, 176, 0.32)');
    gradient.addColorStop(1, 'rgba(110, 240, 176, 0)');
    ctx.beginPath();
    closes.forEach(function (close, i) {
      if (i === 0) ctx.moveTo(x(i), y(close));
      else ctx.lineTo(x(i), y(close));
    });
    ctx.lineTo(width, height);
    ctx.lineTo(0, height);
    ctx.closePath();
    ctx.fillStyle = gradient;
    ctx.fill();

    ctx.beginPath();
    closes.forEach(function (close, i) {
      if (i === 0) ctx.moveTo(x(i), y(close));
      else ctx.lineTo(x(i), y(close));
    });
    ctx.strokeStyle = '#6ef0b0';
    ctx.lineWidth = 3;
    ctx.stroke();

    if (state.result && state.result.curve.length > 2) {
      var curve = state.result.curve;
      var cMin = Math.min.apply(null, curve);
      var cMax = Math.max.apply(null, curve);
      ctx.beginPath();
      curve.forEach(function (point, i) {
        var px = (i / (curve.length - 1)) * width;
        var py = height - ((point - cMin) / Math.max(cMax - cMin, 0.0001)) * height;
        if (i === 0) ctx.moveTo(px, py);
        else ctx.lineTo(px, py);
      });
      ctx.strokeStyle = 'rgba(244, 196, 107, 0.72)';
      ctx.lineWidth = 2;
      ctx.stroke();
    }
  }

  function updateMetrics() {
    var last = state.candles[state.candles.length - 1];
    els.sourceLabel.textContent = state.source;
    els.priceCaption.textContent = 'Last ' + state.pair.replace('-USD', '');
    els.priceLabel.textContent = last ? fmtUsd(last.close) : '-';

    if (!state.result) {
      els.signalLabel.textContent = 'Idle';
      els.returnMetric.textContent = '-';
      els.drawdownMetric.textContent = '-';
      els.tradesMetric.textContent = '-';
      els.confidenceMetric.textContent = '-';
      return;
    }

    var latest = state.result.latest;
    els.signalLabel.textContent = latest.side.toUpperCase();
    els.returnMetric.textContent = fmtPct(state.result.totalReturn);
    els.drawdownMetric.textContent = fmtPct(-state.result.maxDrawdown);
    els.tradesMetric.textContent = String(state.result.trades);
    els.confidenceMetric.textContent = Math.round(state.result.confidence * 100) + '%';
    els.signalNote.textContent = latest.reason + ' Latest paper decision: ' + latest.side.toUpperCase() + ' with ' + Math.round(state.result.confidence * 100) + '% confidence.';

    if (latest.side === 'buy') setStatus('Paper buy', '');
    else if (latest.side === 'sell') setStatus('Paper sell', 'warn');
    else setStatus('Hold', 'warn');
  }

  async function runLive() {
    setStatus('Loading candles', 'warn');
    els.runLiveBtn.disabled = true;
    try {
      state.candles = await fetchCandles();
      runStrategy();
    } catch (err) {
      state.candles = syntheticFallbackCandles(els.pairSelect.value, els.lookbackSelect.value);
      runStrategy();
      setStatus('Sample data', 'warn');
      els.signalNote.textContent = 'Live data failed, so this run is using deterministic sample candles. The page is still safe; no funds or signatures are touched.';
    } finally {
      els.runLiveBtn.disabled = false;
    }
  }

  function runStrategy() {
    var strategy = selectedStrategy();
    els.strategyTitle.textContent = strategy.name;
    state.result = backtest(state.candles, strategy.id);
    updateMetrics();
    drawChart();
    buildIntent();
  }

  function tokenForPair(pair) {
    if (pair.indexOf('NEAR') === 0) return 'wrap.near';
    if (pair.indexOf('BTC') === 0) return 'btc.omft.near';
    if (pair.indexOf('ETH') === 0) return 'eth.omft.near';
    if (pair.indexOf('SOL') === 0) return 'sol.omft.near';
    return 'wrap.near';
  }

  function buildIntent() {
    var last = state.candles[state.candles.length - 1];
    var strategy = selectedStrategy();
    var latest = state.result ? state.result.latest : { side: 'hold', reason: 'No signal yet' };
    var budgetUsd = Number(els.budgetInput.value || 0);
    var tradeCapUsd = Number(els.tradeCapInput.value || 0);
    var slippageBps = Number(els.slippageInput.value || 0);
    var safeTradeUsd = Math.max(0, Math.min(budgetUsd, tradeCapUsd));
    var side = latest.side;
    var account = els.accountInput.value.trim() || '<viewer-provided-account.near>';
    var minOutUsd = safeTradeUsd * (1 - slippageBps / 10000);
    var quoteRequest = side === 'hold' ? null : {
      sell: side === 'buy'
        ? { token: 'usdc.near', amount_usd_estimate: Number(safeTradeUsd.toFixed(6)) }
        : { token: tokenForPair(state.pair), amount_usd_estimate: Number(safeTradeUsd.toFixed(6)) },
      buy: side === 'buy'
        ? { token: tokenForPair(state.pair), min_amount_usd: Number(minOutUsd.toFixed(6)) }
        : { token: 'usdc.near', min_amount_usd: Number(minOutUsd.toFixed(6)) },
      slippage_bps: slippageBps
    };

    state.lastIntent = {
      schema: 'near-intents-public-experiment/1',
      mode: 'paper',
      public_demo: true,
      execution_boundary: 'unsigned-only',
      created_at: new Date().toISOString(),
      account_id: account,
      market: {
        pair: state.pair.replace('-', '/'),
        source: state.source,
        last_price_usd: last ? Number(last.close.toFixed(8)) : null
      },
      strategy: {
        id: strategy.id,
        name: strategy.name,
        decision: side,
        confidence: state.result ? Number(state.result.confidence.toFixed(4)) : null,
        note: latest.reason
      },
      risk: {
        nominal_budget_usd: budgetUsd,
        max_trade_usd: safeTradeUsd,
        slippage_bps: slippageBps,
        funding_asset_hint: 'any_near_intents_supported_deposit_asset',
        no_auto_signing: true,
        no_broadcast: true
      },
      draft_intent: {
        venue: 'near_intents',
        deposit_model: 'direct_near_intents_balance_or_any_supported_asset',
        solver_quote_required: true,
        decision: side,
        quote_request: quoteRequest,
        hold_reason: side === 'hold' ? latest.reason : null,
        expires_in_seconds: 120
      }
    };
    els.intentOutput.textContent = JSON.stringify(state.lastIntent, null, 2);
  }

  function copyIntent() {
    var text = els.intentOutput.textContent || '';
    if (!text || text.indexOf('{') !== 0) return;
    navigator.clipboard.writeText(text).then(function () {
      els.copyIntentBtn.textContent = 'Copied';
      setTimeout(function () {
        els.copyIntentBtn.textContent = 'Copy JSON';
      }, 1200);
    }).catch(function () {
      els.copyIntentBtn.textContent = 'Copy failed';
      setTimeout(function () {
        els.copyIntentBtn.textContent = 'Copy JSON';
      }, 1200);
    });
  }

  function bind() {
    renderStrategies();
    drawChart();
    els.runLiveBtn.addEventListener('click', runLive);
    els.pairSelect.addEventListener('change', runLive);
    els.lookbackSelect.addEventListener('change', runLive);
    els.buildIntentBtn.addEventListener('click', buildIntent);
    els.copyIntentBtn.addEventListener('click', copyIntent);
    [els.accountInput, els.budgetInput, els.tradeCapInput, els.slippageInput].forEach(function (input) {
      input.addEventListener('input', buildIntent);
    });
    window.addEventListener('resize', drawChart);
    setTimeout(runLive, 250);
  }

  bind();
}());
