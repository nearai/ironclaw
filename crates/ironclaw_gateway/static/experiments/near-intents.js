(function () {
  'use strict';

  var COINGECKO_IDS = {
    'NEAR-USD': 'near',
    'BTC-USD': 'bitcoin',
    'ETH-USD': 'ethereum',
    'SOL-USD': 'solana'
  };

  var ASSET_BY_PAIR = {
    'NEAR-USD': { base: 'nep141:wrap.near', baseDecimals: 24, quote: 'nep141:usdt.tether-token.near', quoteDecimals: 6 },
    'BTC-USD': { base: 'nep141:btc.omft.near', baseDecimals: 8, quote: 'nep141:usdt.tether-token.near', quoteDecimals: 6 },
    'ETH-USD': { base: 'nep141:eth.omft.near', baseDecimals: 18, quote: 'nep141:usdt.tether-token.near', quoteDecimals: 6 },
    'SOL-USD': { base: 'nep141:sol.omft.near', baseDecimals: 9, quote: 'nep141:usdt.tether-token.near', quoteDecimals: 6 }
  };

  var INTENTS_CONTRACT = 'intents.near';
  var NEAR_RPC = {
    mainnet: 'https://rpc.mainnet.near.org',
    testnet: 'https://rpc.testnet.near.org'
  };
  var SOLVER_RELAY_URL = 'https://solver-relay-v2.chaindefuser.com/rpc';
  var ONE_CLICK_URL = 'https://1click.chaindefuser.com/v0';

  var STRATEGIES = [
    {
      id: 'momentum',
      name: 'Momentum',
      badge: 'SMA + RSI',
      copy: 'Trend entry when fast SMA leads slow SMA with RSI guardrails.',
      prompt: 'NEAR spot momentum using SMA confirmation, RSI guardrail, 1.0% take profit, 0.6% stop, and small paper sizing.'
    },
    {
      id: 'mean_reversion',
      name: 'Mean Reversion',
      badge: 'Bands',
      copy: 'Bollinger lower-band entries with mean exit and hard stop.',
      prompt: 'Mean reversion on NEAR: buy lower Bollinger Band with RSI below 38, exit near the mean, cap losses quickly.'
    },
    {
      id: 'breakout',
      name: 'Breakout',
      badge: 'Channel',
      copy: 'Trades fresh range expansion after a quiet channel.',
      prompt: 'Breakout strategy that buys a 24 hour channel break, exits on channel failure, with realistic fees and slippage.'
    },
    {
      id: 'grid',
      name: 'Spot Grid',
      badge: 'Range',
      copy: 'Rebalances around a moving center to harvest oscillation.',
      prompt: 'Spot grid for NEAR with tight levels, small order size, and automatic recentering when the range drifts.'
    },
    {
      id: 'dca',
      name: 'DCA Rebalance',
      badge: 'Steady',
      copy: 'Buys on cadence and exits only after cumulative profit target.',
      prompt: 'DCA into NEAR with a small recurring buy, then take profit when the basket is materially above cost.'
    },
    {
      id: 'hl_momentum',
      name: 'HL Momentum Watch',
      badge: 'Perp style',
      copy: 'Hyperliquid-style momentum watcher constrained to spot intents.',
      prompt: 'Hyperliquid-style momentum watch for NEAR, but produce spot-only NEAR Intents drafts with no leverage.'
    }
  ];

  var state = {
    selectedStrategy: 'momentum',
    candles: [],
    result: null,
    latestIntent: null,
    latestLivePlan: null,
    latestSignedIntent: null,
    isStale: true,
    nearConnectorModule: null,
    wallet: {
      connector: null,
      instance: null,
      accountId: '',
      accounts: [],
      network: 'mainnet'
    },
    supportedTokens: [],
    dataSource: 'No data',
    lastRunAt: null
  };

  var els = {
    dataSourceLabel: document.getElementById('dataSourceLabel'),
    runStateLabel: document.getElementById('runStateLabel'),
    walletPill: document.getElementById('walletPill'),
    connectWalletBtn: document.getElementById('connectWalletBtn'),
    heroRunBtn: document.getElementById('heroRunBtn'),
    heroConnectBtn: document.getElementById('heroConnectBtn'),
    strategyPrompt: document.getElementById('strategyPrompt'),
    strategyDeck: document.getElementById('strategyDeck'),
    pairSelect: document.getElementById('pairSelect'),
    lookbackSelect: document.getElementById('lookbackSelect'),
    balanceInput: document.getElementById('balanceInput'),
    liveAmountUsdInput: document.getElementById('liveAmountUsdInput'),
    accountInput: document.getElementById('accountInput'),
    maxTradeInput: document.getElementById('maxTradeInput'),
    riskPctInput: document.getElementById('riskPctInput'),
    feeBpsInput: document.getElementById('feeBpsInput'),
    slippageInput: document.getElementById('slippageInput'),
    baseAssetInput: document.getElementById('baseAssetInput'),
    quoteAssetInput: document.getElementById('quoteAssetInput'),
    assetIdList: document.getElementById('assetIdList'),
    baseDecimalsInput: document.getElementById('baseDecimalsInput'),
    quoteDecimalsInput: document.getElementById('quoteDecimalsInput'),
    networkSelect: document.getElementById('networkSelect'),
    sideOverrideSelect: document.getElementById('sideOverrideSelect'),
    solverApiKeyInput: document.getElementById('solverApiKeyInput'),
    oneClickApiKeyInput: document.getElementById('oneClickApiKeyInput'),
    walletStatusLabel: document.getElementById('walletStatusLabel'),
    walletAccountLabel: document.getElementById('walletAccountLabel'),
    walletBalanceLabel: document.getElementById('walletBalanceLabel'),
    liveModeLabel: document.getElementById('liveModeLabel'),
    liveExecutionOutput: document.getElementById('liveExecutionOutput'),
    liveStepBacktest: document.getElementById('liveStepBacktest'),
    liveStepQuote: document.getElementById('liveStepQuote'),
    liveStepSign: document.getElementById('liveStepSign'),
    liveStepPublish: document.getElementById('liveStepPublish'),
    runLiveBtn: document.getElementById('runLiveBtn'),
    dryQuoteBtn: document.getElementById('dryQuoteBtn'),
    fetchBalancesBtn: document.getElementById('fetchBalancesBtn'),
    topRunBtn: document.getElementById('topRunBtn'),
    runBacktestBtn: document.getElementById('runBacktestBtn'),
    buildIntentBtn: document.getElementById('buildIntentBtn'),
    copyIntentBtn: document.getElementById('copyIntentBtn'),
    copySignedBtn: document.getElementById('copySignedBtn'),
    describeStatus: document.getElementById('describeStatus'),
    validateStatus: document.getElementById('validateStatus'),
    backtestStatus: document.getElementById('backtestStatus'),
    intentStatus: document.getElementById('intentStatus'),
    returnMetric: document.getElementById('returnMetric'),
    sharpeMetric: document.getElementById('sharpeMetric'),
    drawdownMetric: document.getElementById('drawdownMetric'),
    winRateMetric: document.getElementById('winRateMetric'),
    profitMetric: document.getElementById('profitMetric'),
    signalMetric: document.getElementById('signalMetric'),
    chartTitle: document.getElementById('chartTitle'),
    chartCanvas: document.getElementById('chartCanvas'),
    tradeCountLabel: document.getElementById('tradeCountLabel'),
    tradeTableBody: document.getElementById('tradeTableBody'),
    tapeSummary: document.getElementById('tapeSummary'),
    eventTape: document.getElementById('eventTape'),
    signalTitle: document.getElementById('signalTitle'),
    signalSummary: document.getElementById('signalSummary'),
    validationList: document.getElementById('validationList'),
    strategyCode: document.getElementById('strategyCode'),
    intentOutput: document.getElementById('intentOutput')
  };

  function strategyById(id) {
    return STRATEGIES.find(function (strategy) {
      return strategy.id === id;
    }) || STRATEGIES[0];
  }

  function params() {
    var pairAssets = ASSET_BY_PAIR[els.pairSelect.value] || ASSET_BY_PAIR['NEAR-USD'];
    return {
      pair: els.pairSelect.value,
      lookbackDays: Number(els.lookbackSelect.value),
      initialCash: Math.max(10, Number(els.balanceInput.value || 0)),
      maxTradeUsd: Math.max(0.01, Number(els.maxTradeInput.value || 0)),
      liveAmountUsd: Math.max(0.01, Number(els.liveAmountUsdInput.value || els.maxTradeInput.value || 0)),
      riskPct: Math.max(0.001, Number(els.riskPctInput.value || 0) / 100),
      feeBps: Math.max(0, Number(els.feeBpsInput.value || 0)),
      slippageBps: Math.max(0, Number(els.slippageInput.value || 0)),
      accountId: els.accountInput.value.trim() || '<viewer-provided-account.near>',
      baseAsset: (els.baseAssetInput.value || pairAssets.base).trim(),
      quoteAsset: (els.quoteAssetInput.value || pairAssets.quote).trim(),
      baseDecimals: clamp(Math.round(Number(els.baseDecimalsInput.value || pairAssets.baseDecimals)), 0, 30),
      quoteDecimals: clamp(Math.round(Number(els.quoteDecimalsInput.value || pairAssets.quoteDecimals)), 0, 30),
      network: els.networkSelect.value || 'mainnet',
      sideOverride: els.sideOverrideSelect.value || 'auto',
      solverApiKey: els.solverApiKeyInput.value.trim(),
      oneClickApiKey: els.oneClickApiKeyInput.value.trim(),
      prompt: els.strategyPrompt.value.trim(),
      strategy: strategyById(state.selectedStrategy)
    };
  }

  function clamp(value, min, max) {
    return Math.max(min, Math.min(max, value));
  }

  function average(values) {
    if (!values.length) return 0;
    return values.reduce(function (sum, value) {
      return sum + value;
    }, 0) / values.length;
  }

  function stddev(values) {
    if (values.length < 2) return 0;
    var mean = average(values);
    var variance = values.reduce(function (sum, value) {
      return sum + Math.pow(value - mean, 2);
    }, 0) / (values.length - 1);
    return Math.sqrt(Math.max(0, variance));
  }

  function fmtUsd(value) {
    if (!Number.isFinite(value)) return '-';
    var decimals = Math.abs(value) < 10 ? 4 : 2;
    return '$' + value.toLocaleString(undefined, {
      minimumFractionDigits: decimals,
      maximumFractionDigits: decimals
    });
  }

  function fmtPct(value) {
    if (!Number.isFinite(value)) return '-';
    var sign = value > 0 ? '+' : '';
    return sign + (value * 100).toFixed(2) + '%';
  }

  function fmtNum(value, decimals) {
    if (!Number.isFinite(value)) return '-';
    return value.toFixed(decimals);
  }

  function fmtDate(timestamp) {
    var date = new Date(timestamp);
    return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) +
      ' ' + date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  }

  function hashText(text) {
    var hash = 2166136261;
    for (var i = 0; i < text.length; i += 1) {
      hash ^= text.charCodeAt(i);
      hash = Math.imul(hash, 16777619);
    }
    return (hash >>> 0).toString(16);
  }

  function shortAccount(accountId) {
    if (!accountId) return '-';
    if (accountId.length <= 22) return accountId;
    return accountId.slice(0, 10) + '...' + accountId.slice(-8);
  }

  function effectiveSide(result) {
    var input = result ? result.input : params();
    if (input.sideOverride && input.sideOverride !== 'auto') return input.sideOverride;
    return result && result.latestSignal ? result.latestSignal.side : 'hold';
  }

  function priceForAsset(assetId, fallbackPrice) {
    var token = state.supportedTokens.find(function (candidate) {
      return candidate.assetId === assetId;
    });
    if (token && Number.isFinite(Number(token.price)) && Number(token.price) > 0) {
      return Number(token.price);
    }
    if (assetId.indexOf('usdc') >= 0 || assetId.indexOf('usdt') >= 0 || assetId.indexOf('usd') >= 0) {
      return 1;
    }
    return Number.isFinite(fallbackPrice) && fallbackPrice > 0 ? fallbackPrice : 1;
  }

  function decimalToAtomic(value, decimals) {
    var safeDecimals = clamp(Math.round(Number(decimals) || 0), 0, 30);
    var numeric = Number(value);
    if (!Number.isFinite(numeric) || numeric <= 0) return '0';
    var fixed = numeric.toFixed(Math.min(safeDecimals, 24));
    var parts = fixed.split('.');
    var whole = parts[0] || '0';
    var frac = (parts[1] || '').padEnd(safeDecimals, '0').slice(0, safeDecimals);
    var base = BigInt(10) ** BigInt(safeDecimals);
    return (BigInt(whole) * base + BigInt(frac || '0')).toString();
  }

  function atomicToDecimalText(value, decimals) {
    var safeDecimals = clamp(Math.round(Number(decimals) || 0), 0, 30);
    var raw = String(value || '0').replace(/^-/, '');
    if (!/^\d+$/.test(raw)) return String(value || '0');
    if (safeDecimals === 0) return raw;
    var padded = raw.padStart(safeDecimals + 1, '0');
    var whole = padded.slice(0, -safeDecimals) || '0';
    var frac = padded.slice(-safeDecimals).replace(/0+$/, '');
    return frac ? whole + '.' + frac : whole;
  }

  function bytesToBase64(bytes) {
    var binary = '';
    bytes.forEach(function (byte) {
      binary += String.fromCharCode(byte);
    });
    return btoa(binary);
  }

  function hexToBytes(hex) {
    var clean = String(hex || '').replace(/^0x/, '').replace(/[^a-fA-F0-9]/g, '');
    var bytes = new Uint8Array(clean.length / 2);
    for (var i = 0; i < bytes.length; i += 1) {
      bytes[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
    }
    return bytes;
  }

  function jsonToBase64(value) {
    var text = JSON.stringify(value || {});
    var bytes = new TextEncoder().encode(text);
    return bytesToBase64(bytes);
  }

  function decodeNearViewResult(result) {
    var bytes = new Uint8Array(result && result.result ? result.result : []);
    var text = new TextDecoder().decode(bytes);
    try {
      return JSON.parse(text);
    } catch (err) {
      return text;
    }
  }

  function normalizeWalletSignature(signed) {
    var publicKey = signed.publicKey || signed.public_key || signed.publicKeyString || '';
    var signature = signed.signature || '';
    if (publicKey && typeof publicKey !== 'string' && publicKey.toString) {
      publicKey = publicKey.toString();
    }
    if (signature && typeof signature !== 'string' && signature.toString) {
      signature = signature.toString();
    }
    return {
      publicKey: String(publicKey || ''),
      signature: String(signature || '')
    };
  }

  function setRunState(text) {
    els.runStateLabel.textContent = text;
  }

  function setPhase(activePhase, tones) {
    var toneMap = tones || {};
    Array.prototype.forEach.call(document.querySelectorAll('.pipeline-step'), function (step) {
      var phase = step.getAttribute('data-phase');
      step.classList.toggle('is-active', phase === activePhase);
      step.classList.toggle('is-warn', toneMap[phase] === 'warn');
      step.classList.toggle('is-error', toneMap[phase] === 'error');
    });
  }

  function setLiveMode(text) {
    els.liveModeLabel.textContent = text;
  }

  function setLiveStep(activeStep, tone) {
    var steps = {
      backtest: els.liveStepBacktest,
      quote: els.liveStepQuote,
      sign: els.liveStepSign,
      publish: els.liveStepPublish
    };
    Object.keys(steps).forEach(function (key) {
      var el = steps[key];
      el.classList.toggle('is-active', key === activeStep);
      el.classList.toggle('is-warn', key === activeStep && tone === 'warn');
      el.classList.toggle('is-error', key === activeStep && tone === 'error');
    });
  }

  function writeLiveOutput(value) {
    els.liveExecutionOutput.textContent = typeof value === 'string' ? value : JSON.stringify(value, null, 2);
  }

  function updateWalletUi() {
    var accountId = state.wallet.accountId;
    els.walletPill.textContent = accountId ? shortAccount(accountId) : 'Wallet offline';
    els.walletPill.classList.toggle('is-connected', Boolean(accountId));
    els.walletStatusLabel.textContent = accountId ? 'Connected' : 'Not connected';
    els.walletAccountLabel.textContent = accountId ? accountId : '-';
    els.connectWalletBtn.textContent = accountId ? 'Connected' : 'Connect';
    els.heroConnectBtn.textContent = accountId ? 'Wallet connected' : 'Connect wallet';
    if (accountId) els.accountInput.value = accountId;
  }

  function renderStrategies() {
    els.strategyDeck.innerHTML = '';
    STRATEGIES.forEach(function (strategy) {
      var button = document.createElement('button');
      button.type = 'button';
      button.className = 'strategy-option';
      button.setAttribute('role', 'radio');
      button.setAttribute('aria-checked', strategy.id === state.selectedStrategy ? 'true' : 'false');
      button.innerHTML = '<div><strong>' + strategy.name + '</strong><span>' +
        strategy.copy + '</span></div><em>' + strategy.badge + '</em>';
      button.addEventListener('click', function () {
        state.selectedStrategy = strategy.id;
        els.strategyPrompt.value = strategy.prompt;
        renderStrategies();
        markStale();
      });
      els.strategyDeck.appendChild(button);
    });
  }

  function updateAssetDefaults() {
    var defaults = ASSET_BY_PAIR[els.pairSelect.value] || ASSET_BY_PAIR['NEAR-USD'];
    els.baseAssetInput.value = defaults.base;
    els.quoteAssetInput.value = defaults.quote;
    els.baseDecimalsInput.value = defaults.baseDecimals;
    els.quoteDecimalsInput.value = defaults.quoteDecimals;
    applyKnownTokenMetadata(defaults.base, defaults.quote);
  }

  function applyKnownTokenMetadata(baseAsset, quoteAsset) {
    if (!state.supportedTokens.length) return;
    var base = state.supportedTokens.find(function (token) { return token.assetId === baseAsset; });
    var quote = state.supportedTokens.find(function (token) { return token.assetId === quoteAsset; });
    if (base && Number.isFinite(Number(base.decimals))) els.baseDecimalsInput.value = Number(base.decimals);
    if (quote && Number.isFinite(Number(quote.decimals))) els.quoteDecimalsInput.value = Number(quote.decimals);
  }

  async function loadSupportedTokens() {
    try {
      var res = await fetch(ONE_CLICK_URL + '/tokens', { headers: { Accept: 'application/json' } });
      if (!res.ok) throw new Error('1Click tokens ' + res.status);
      var tokens = await res.json();
      if (!Array.isArray(tokens)) return;
      state.supportedTokens = tokens;
      els.assetIdList.innerHTML = '';
      tokens.slice(0, 450).forEach(function (token) {
        if (!token.assetId) return;
        var option = document.createElement('option');
        option.value = token.assetId;
        option.label = [token.symbol, token.blockchain, token.decimals + ' decimals'].filter(Boolean).join(' · ');
        els.assetIdList.appendChild(option);
      });
      applyKnownTokenMetadata(els.baseAssetInput.value, els.quoteAssetInput.value);
    } catch (err) {
      writeLiveOutput('Token catalog unavailable from 1Click for now. You can still paste any NEAR Intents asset ID manually.');
    }
  }

  async function nearRpcView(methodName, args) {
    var network = els.networkSelect.value || 'mainnet';
    var rpcUrl = NEAR_RPC[network] || NEAR_RPC.mainnet;
    var res = await fetch(rpcUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 'ironclaw-' + Date.now(),
        method: 'query',
        params: {
          request_type: 'call_function',
          finality: 'final',
          account_id: INTENTS_CONTRACT,
          method_name: methodName,
          args_base64: jsonToBase64(args || {})
        }
      })
    });
    if (!res.ok) throw new Error('NEAR RPC ' + res.status);
    var body = await res.json();
    if (body.error) throw new Error(body.error.message || 'NEAR RPC error');
    return decodeNearViewResult(body.result);
  }

  async function fetchCurrentSalt() {
    var salt = await nearRpcView('current_salt', {});
    var saltText = String(salt || '').replace(/^"|"$/g, '');
    var bytes = hexToBytes(saltText);
    if (bytes.length !== 4) throw new Error('Verifier returned invalid current_salt');
    return bytes;
  }

  async function fetchBalances() {
    var accountId = state.wallet.accountId || els.accountInput.value.trim();
    if (!accountId) {
      els.walletBalanceLabel.textContent = 'Connect first';
      writeLiveOutput('Connect a NEAR wallet or enter a NEAR account to query Intents balances.');
      return null;
    }
    try {
      els.walletBalanceLabel.textContent = 'Querying...';
      var input = params();
      var balances = await nearRpcView('mt_batch_balance_of', {
        account_id: accountId,
        token_ids: [input.baseAsset, input.quoteAsset]
      });
      var baseBalance = balances && balances[0] ? balances[0] : '0';
      var quoteBalance = balances && balances[1] ? balances[1] : '0';
      els.walletBalanceLabel.textContent = atomicToDecimalText(baseBalance, input.baseDecimals) + ' base / ' +
        atomicToDecimalText(quoteBalance, input.quoteDecimals) + ' quote';
      writeLiveOutput({
        account_id: accountId,
        verifier_contract: INTENTS_CONTRACT,
        balances: [
          { asset_id: input.baseAsset, raw: baseBalance, display: atomicToDecimalText(baseBalance, input.baseDecimals) },
          { asset_id: input.quoteAsset, raw: quoteBalance, display: atomicToDecimalText(quoteBalance, input.quoteDecimals) }
        ]
      });
      return balances;
    } catch (err) {
      els.walletBalanceLabel.textContent = 'Balance query failed';
      writeLiveOutput('Balance query failed: ' + (err.message || String(err)));
      return null;
    }
  }

  async function loadNearConnector() {
    if (!state.nearConnectorModule) {
      state.nearConnectorModule = await import('https://esm.sh/@hot-labs/near-connect@0.11');
    }
    return state.nearConnectorModule;
  }

  async function connectWallet() {
    var network = els.networkSelect.value || 'mainnet';
    els.connectWalletBtn.disabled = true;
    els.heroConnectBtn.disabled = true;
    setLiveMode('Connecting wallet');
    writeLiveOutput('Opening NEAR wallet connector for ' + network + '.');
    try {
      var mod = await loadNearConnector();
      if (!state.wallet.connector || state.wallet.network !== network) {
        state.wallet.connector = new mod.NearConnector({ network: network });
      }
      state.wallet.network = network;
      state.wallet.instance = await state.wallet.connector.connect();
      state.wallet.accounts = await state.wallet.instance.getAccounts();
      if (!state.wallet.accounts || !state.wallet.accounts.length) throw new Error('No NEAR account returned by wallet');
      state.wallet.accountId = state.wallet.accounts[0].accountId;
      updateWalletUi();
      setLiveMode('Wallet connected');
      writeLiveOutput('Wallet connected: ' + state.wallet.accountId + '\nRun a backtest, then click Run live once to quote and sign.');
      fetchBalances();
      return state.wallet.accountId;
    } catch (err) {
      setLiveMode('Wallet failed');
      writeLiveOutput('Wallet connection failed: ' + (err.message || String(err)));
      throw err;
    } finally {
      els.connectWalletBtn.disabled = false;
      els.heroConnectBtn.disabled = false;
    }
  }

  function buildLivePlan(result) {
    if (!result) throw new Error('Run a strategy before building a live plan');
    var input = result.input;
    var side = effectiveSide(result);
    if (side === 'hold') {
      return {
        side: 'hold',
        reason: result.latestSignal.reason,
        account_id: state.wallet.accountId || input.accountId,
        strategy: input.strategy.name
      };
    }
    var liveUsd = Math.min(input.liveAmountUsd, input.maxTradeUsd, input.initialCash * input.riskPct);
    var basePrice = priceForAsset(input.baseAsset, result.latestPrice);
    var quotePrice = priceForAsset(input.quoteAsset, 1);
    var amountInDecimal;
    var amountOutEstimateDecimal;
    var assetIn;
    var assetOut;
    var decimalsIn;
    var decimalsOut;
    if (side === 'buy') {
      assetIn = input.quoteAsset;
      assetOut = input.baseAsset;
      decimalsIn = input.quoteDecimals;
      decimalsOut = input.baseDecimals;
      amountInDecimal = liveUsd / quotePrice;
      amountOutEstimateDecimal = (liveUsd / basePrice) * (1 - input.slippageBps / 10000);
    } else {
      assetIn = input.baseAsset;
      assetOut = input.quoteAsset;
      decimalsIn = input.baseDecimals;
      decimalsOut = input.quoteDecimals;
      amountInDecimal = liveUsd / basePrice;
      amountOutEstimateDecimal = (liveUsd / quotePrice) * (1 - input.slippageBps / 10000);
    }
    return {
      side: side,
      side_source: input.sideOverride === 'auto' ? 'latest_strategy_signal' : 'manual_override',
      account_id: state.wallet.accountId || input.accountId,
      strategy: input.strategy.name,
      reason: result.latestSignal.reason,
      confidence: result.latestSignal.confidence,
      live_cap_usd: Number(liveUsd.toFixed(6)),
      asset_in: assetIn,
      asset_out: assetOut,
      decimals_in: decimalsIn,
      decimals_out: decimalsOut,
      amount_in_decimal: Number(amountInDecimal.toFixed(decimalsIn > 8 ? 8 : decimalsIn)),
      amount_out_estimate_decimal: Number(amountOutEstimateDecimal.toFixed(decimalsOut > 8 ? 8 : decimalsOut)),
      amount_in: decimalToAtomic(amountInDecimal, decimalsIn),
      min_amount_out_estimate: decimalToAtomic(amountOutEstimateDecimal, decimalsOut),
      slippage_bps: input.slippageBps,
      pair: input.pair,
      latest_price_usd: result.latestPrice
    };
  }

  async function requestSolverQuote(plan) {
    var apiKey = els.solverApiKeyInput.value.trim();
    if (!apiKey) throw new Error('Solver relay JWT required for low-level quote/publish');
    var request = {
      id: 'ironclaw-' + Date.now(),
      jsonrpc: '2.0',
      method: 'quote',
      params: [
        {
          defuse_asset_identifier_in: plan.asset_in,
          defuse_asset_identifier_out: plan.asset_out,
          exact_amount_in: plan.amount_in,
          min_deadline_ms: 60000
        }
      ]
    };
    var res = await fetch(SOLVER_RELAY_URL, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-API-Key': apiKey
      },
      body: JSON.stringify(request)
    });
    if (!res.ok) throw new Error('Solver relay quote ' + res.status);
    var body = await res.json();
    if (body.error) throw new Error(body.error.message || 'Solver relay quote error');
    var quotes = Array.isArray(body.result) ? body.result : [];
    if (!quotes.length) throw new Error('No solver quotes returned');
    quotes.sort(function (a, b) {
      var aOut = BigInt(a.amount_out || '0');
      var bOut = BigInt(b.amount_out || '0');
      if (aOut === bOut) return 0;
      return aOut > bOut ? -1 : 1;
    });
    return {
      request: request,
      selected: quotes[0],
      quotes: quotes
    };
  }

  async function requestOneClickQuote(plan, dry) {
    var input = params();
    var accountId = state.wallet.accountId || input.accountId;
    if (!accountId || accountId.indexOf('<') === 0) throw new Error('Connect wallet or enter a recipient account first');
    var headers = {
      'Content-Type': 'application/json',
      Accept: 'application/json'
    };
    if (input.oneClickApiKey) headers.Authorization = 'Bearer ' + input.oneClickApiKey;
    var request = {
      dry: dry !== false,
      swapType: 'EXACT_INPUT',
      slippageTolerance: input.slippageBps,
      originAsset: plan.asset_in,
      depositType: 'INTENTS',
      destinationAsset: plan.asset_out,
      amount: plan.amount_in,
      recipient: accountId,
      recipientType: 'INTENTS',
      refundTo: accountId,
      refundType: 'INTENTS',
      deadline: new Date(Date.now() + 5 * 60 * 1000).toISOString()
    };
    var res = await fetch(ONE_CLICK_URL + '/quote', {
      method: 'POST',
      headers: headers,
      body: JSON.stringify(request)
    });
    var body = await res.json().catch(function () { return {}; });
    if (!res.ok) throw new Error(body.message || body.error || '1Click quote ' + res.status);
    return {
      request: request,
      response: body
    };
  }

  async function signLiveIntent(plan, quote) {
    if (!state.wallet.instance || !state.wallet.accountId) throw new Error('Connect a NEAR wallet before signing');
    var salt = await fetchCurrentSalt();
    var nonceBytes = new Uint8Array(32);
    nonceBytes.set(salt, 0);
    crypto.getRandomValues(nonceBytes.subarray(4));
    var amountOut = quote && quote.selected && quote.selected.amount_out
      ? quote.selected.amount_out
      : plan.min_amount_out_estimate;
    var message = {
      signer_id: state.wallet.accountId,
      deadline: new Date(Date.now() + 2 * 60 * 1000).toISOString(),
      intents: [
        {
          intent: 'token_diff',
          diff: {},
          memo: 'ironclaw-intents-strategy:' + plan.strategy,
          referral: 'ironclaw.near'
        }
      ]
    };
    message.intents[0].diff[plan.asset_in] = '-' + plan.amount_in;
    message.intents[0].diff[plan.asset_out] = amountOut;
    var payloadMessage = JSON.stringify(message);
    var signed = await state.wallet.instance.signMessage({
      message: payloadMessage,
      recipient: INTENTS_CONTRACT,
      nonce: nonceBytes
    });
    var normalized = normalizeWalletSignature(signed);
    if (!normalized.signature || !normalized.publicKey) throw new Error('Wallet did not return a NEP-413 signature/public key');
    return {
      standard: 'nep413',
      payload: {
        recipient: INTENTS_CONTRACT,
        nonce: bytesToBase64(nonceBytes),
        message: payloadMessage
      },
      public_key: normalized.publicKey,
      signature: normalized.signature
    };
  }

  async function publishSignedIntent(signedData, quote) {
    var apiKey = els.solverApiKeyInput.value.trim();
    if (!apiKey) throw new Error('Solver relay JWT required to publish');
    if (!quote || !quote.selected || !quote.selected.quote_hash) throw new Error('Solver quote hash required to publish');
    var request = {
      id: 'ironclaw-' + Date.now(),
      jsonrpc: '2.0',
      method: 'publish_intent',
      params: [
        {
          quote_hashes: [quote.selected.quote_hash],
          signed_data: signedData
        }
      ]
    };
    var res = await fetch(SOLVER_RELAY_URL, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-API-Key': apiKey
      },
      body: JSON.stringify(request)
    });
    if (!res.ok) throw new Error('Solver relay publish ' + res.status);
    var body = await res.json();
    if (body.error) throw new Error(body.error.message || 'Solver relay publish error');
    return {
      request: request,
      response: body
    };
  }

  function validateRun(input) {
    var messages = [];
    var minWarmup = input.strategy.id === 'momentum' ? 48 : input.strategy.id === 'mean_reversion' ? 24 : 18;
    if (input.prompt.length < 24) {
      messages.push({ tone: 'warn', text: 'Strategy prompt is short; template defaults will carry most behavior.' });
    } else {
      messages.push({ tone: 'ok', text: 'Prompt captured for strategy metadata and run hash.' });
    }
    if (input.lookbackDays * 24 < minWarmup + 10) {
      messages.push({ tone: 'warn', text: 'Lookback barely covers indicator warmup; metrics may be noisy.' });
    } else {
      messages.push({ tone: 'ok', text: 'Lookback covers indicator warmup.' });
    }
    if (input.maxTradeUsd > input.initialCash * 0.5) {
      messages.push({ tone: 'warn', text: 'Max trade is large versus paper balance.' });
    } else {
      messages.push({ tone: 'ok', text: 'Sizing is bounded by cash, risk percent, and max trade.' });
    }
    if (input.feeBps + input.slippageBps <= 0) {
      messages.push({ tone: 'warn', text: 'Fees and slippage are zero; fills may look too optimistic.' });
    } else {
      messages.push({ tone: 'ok', text: 'Costs are included in every paper fill.' });
    }
    if (!input.baseAsset || !input.quoteAsset) {
      messages.push({ tone: 'error', text: 'Base and quote asset IDs are required for live NEAR Intents execution.' });
    } else {
      messages.push({ tone: 'ok', text: 'Live path can quote, sign NEP-413, and publish through the solver relay when a JWT is provided.' });
    }
    if (input.network !== 'mainnet') {
      messages.push({ tone: 'warn', text: 'NEAR Intents verifier is mainnet-first; testnet wallet mode is only for connector testing.' });
    }
    return messages;
  }

  function renderValidation(messages) {
    els.validationList.innerHTML = '';
    messages.forEach(function (message) {
      var item = document.createElement('li');
      item.className = message.tone === 'ok' ? '' : message.tone;
      item.textContent = message.text;
      els.validationList.appendChild(item);
    });
  }

  function closeSeries(candles) {
    return candles.map(function (candle) {
      return candle.close;
    });
  }

  function smaAt(values, index, period) {
    if (index + 1 < period) return null;
    return average(values.slice(index + 1 - period, index + 1));
  }

  function stdevAt(values, index, period) {
    if (index + 1 < period) return null;
    return stddev(values.slice(index + 1 - period, index + 1));
  }

  function rsiAt(values, index, period) {
    if (index < period) return null;
    var gains = [];
    var losses = [];
    for (var i = index - period + 1; i <= index; i += 1) {
      var delta = values[i] - values[i - 1];
      if (delta >= 0) gains.push(delta);
      else losses.push(Math.abs(delta));
    }
    var avgGain = average(gains);
    var avgLoss = average(losses);
    if (avgLoss === 0) return 100;
    var rs = avgGain / avgLoss;
    return 100 - (100 / (1 + rs));
  }

  function atrAt(candles, index, period) {
    if (index < period) return null;
    var ranges = [];
    for (var i = index - period + 1; i <= index; i += 1) {
      var prevClose = candles[i - 1] ? candles[i - 1].close : candles[i].close;
      var highLow = candles[i].high - candles[i].low;
      var highClose = Math.abs(candles[i].high - prevClose);
      var lowClose = Math.abs(candles[i].low - prevClose);
      ranges.push(Math.max(highLow, highClose, lowClose));
    }
    return average(ranges);
  }

  function signal(strategyId, candles, index, simState) {
    var values = closeSeries(candles);
    var candle = candles[index];
    var price = candle.close;
    var hasPosition = simState.qty > 0.00000001;
    var avgEntry = simState.qty > 0 ? simState.entryCost / simState.qty : 0;

    if (strategyId === 'momentum') {
      var fast = smaAt(values, index, 12);
      var slow = smaAt(values, index, 48);
      var rsi = rsiAt(values, index, 14);
      if (!fast || !slow || rsi === null) return { side: 'hold', confidence: 0.12, reason: 'warming up indicators' };
      if (hasPosition) {
        var pnl = avgEntry > 0 ? (price - avgEntry) / avgEntry : 0;
        if (pnl >= 0.01) return { side: 'sell', confidence: 0.77, reason: 'take profit threshold reached' };
        if (pnl <= -0.006) return { side: 'sell', confidence: 0.82, reason: 'stop loss threshold reached' };
        if (fast < slow || rsi > 82) return { side: 'sell', confidence: 0.64, reason: 'trend guard turned defensive' };
      }
      if (!hasPosition && fast > slow && rsi < 74) {
        return { side: 'buy', confidence: clamp(0.48 + ((fast - slow) / slow) * 18, 0.5, 0.91), reason: 'fast SMA leads slow SMA with RSI below overbought' };
      }
      return { side: 'hold', confidence: 0.34, reason: 'momentum is not actionable' };
    }

    if (strategyId === 'mean_reversion') {
      var mean = smaAt(values, index, 20);
      var sigma = stdevAt(values, index, 20);
      var bandRsi = rsiAt(values, index, 14);
      if (!mean || !sigma || bandRsi === null) return { side: 'hold', confidence: 0.12, reason: 'warming up bands' };
      var lower = mean - 2 * sigma;
      if (hasPosition) {
        var mrPnl = avgEntry > 0 ? (price - avgEntry) / avgEntry : 0;
        if (price >= mean || mrPnl >= 0.012) return { side: 'sell', confidence: 0.73, reason: 'price reverted toward mean' };
        if (mrPnl <= -0.008) return { side: 'sell', confidence: 0.79, reason: 'mean reversion stop triggered' };
      }
      if (!hasPosition && price < lower && bandRsi < 42) {
        return { side: 'buy', confidence: clamp((mean - price) / Math.max(sigma, 0.00001) / 3, 0.52, 0.9), reason: 'price below lower band with weak RSI' };
      }
      return { side: 'hold', confidence: 0.31, reason: 'price remains inside range' };
    }

    if (strategyId === 'breakout') {
      if (index < 28) return { side: 'hold', confidence: 0.12, reason: 'warming up channel' };
      var channel = candles.slice(index - 24, index);
      var high = Math.max.apply(null, channel.map(function (c) { return c.high; }));
      var low = Math.min.apply(null, channel.map(function (c) { return c.low; }));
      var atr = atrAt(candles, index, 14) || price * 0.01;
      if (hasPosition) {
        if (price < low || price < avgEntry - atr * 1.7) return { side: 'sell', confidence: 0.76, reason: 'channel failed or ATR stop hit' };
      }
      if (!hasPosition && price > high) {
        return { side: 'buy', confidence: 0.78, reason: 'price cleared 24 hour channel high' };
      }
      return { side: 'hold', confidence: 0.33, reason: 'no channel break' };
    }

    if (strategyId === 'grid') {
      if (!simState.gridCenter) simState.gridCenter = price;
      var spacing = 0.008;
      var drift = Math.abs(price - simState.gridCenter) / price;
      if (drift > spacing * 3) simState.gridCenter = price;
      if (price <= simState.gridCenter * (1 - spacing) && simState.cash > 1) {
        simState.gridCenter = price;
        return { side: 'buy', confidence: 0.58, reason: 'price moved into lower grid level' };
      }
      if (hasPosition && price >= avgEntry * (1 + spacing * 1.4)) {
        simState.gridCenter = price;
        return { side: 'sell', confidence: 0.62, reason: 'grid take profit level reached' };
      }
      return { side: 'hold', confidence: 0.4, reason: 'grid remains centered' };
    }

    if (strategyId === 'dca') {
      if (hasPosition) {
        var dcaPnl = avgEntry > 0 ? (price - avgEntry) / avgEntry : 0;
        if (dcaPnl >= 0.045) return { side: 'sell', confidence: 0.74, reason: 'DCA basket reached profit target' };
      }
      if (index % 24 === 0 && simState.cash > 1) {
        return { side: 'buy', confidence: 0.5, reason: 'scheduled DCA interval' };
      }
      return { side: 'hold', confidence: 0.35, reason: 'waiting for next DCA interval' };
    }

    if (index < 24) return { side: 'hold', confidence: 0.12, reason: 'warming up momentum window' };
    var ret12 = (price - candles[index - 12].close) / candles[index - 12].close;
    var ret24 = (price - candles[index - 24].close) / candles[index - 24].close;
    if (hasPosition && ret12 < -0.012) return { side: 'sell', confidence: 0.74, reason: '12 hour momentum flipped negative' };
    if (!hasPosition && ret12 > 0.012 && ret24 > 0.018) return { side: 'buy', confidence: clamp(0.5 + ret24 * 5, 0.52, 0.86), reason: '12 hour and 24 hour momentum align' };
    return { side: 'hold', confidence: 0.34, reason: 'momentum watch is neutral' };
  }

  async function fetchCoingeckoCandles(pair, days) {
    var id = COINGECKO_IDS[pair];
    if (!id) throw new Error('Unsupported market');
    var url = 'https://api.coingecko.com/api/v3/coins/' + encodeURIComponent(id) +
      '/market_chart?vs_currency=usd&days=' + encodeURIComponent(days) + '&interval=hourly';
    var res = await fetch(url, { headers: { Accept: 'application/json' } });
    if (!res.ok) throw new Error('CoinGecko ' + res.status);
    var body = await res.json();
    if (!body.prices || body.prices.length < 24) throw new Error('Too few prices');
    return body.prices.map(function (row, index, rows) {
      var close = Number(row[1]);
      var prev = rows[index - 1] ? Number(rows[index - 1][1]) : close;
      var high = Math.max(close, prev) * 1.002;
      var low = Math.min(close, prev) * 0.998;
      return {
        time: Number(row[0]),
        open: prev,
        high: high,
        low: low,
        close: close,
        volume: body.total_volumes && body.total_volumes[index] ? Number(body.total_volumes[index][1]) : 0
      };
    });
  }

  function syntheticCandles(pair, days) {
    var count = Math.max(120, Number(days) * 24);
    var bases = {
      'NEAR-USD': 1.28,
      'BTC-USD': 94500,
      'ETH-USD': 1820,
      'SOL-USD': 126
    };
    var base = bases[pair] || 1;
    var now = Date.now();
    var seed = pair.split('').reduce(function (sum, char) {
      return sum + char.charCodeAt(0);
    }, 0);
    var candles = [];
    for (var i = 0; i < count; i += 1) {
      var trend = (i - count / 2) * 0.00018;
      var cycle = Math.sin((i + seed) / 13) * 0.025;
      var micro = Math.cos((i + seed) / 5.5) * 0.009;
      var close = base * (1 + trend + cycle + micro);
      var open = i > 0 ? candles[i - 1].close : close * 0.997;
      candles.push({
        time: now - (count - i) * 60 * 60 * 1000,
        open: open,
        high: Math.max(open, close) * 1.004,
        low: Math.min(open, close) * 0.996,
        close: close,
        volume: 0
      });
    }
    return candles;
  }

  async function loadCandles(input) {
    try {
      var candles = await fetchCoingeckoCandles(input.pair, input.lookbackDays);
      state.dataSource = 'CoinGecko hourly';
      return candles;
    } catch (err) {
      state.dataSource = 'Sample fallback';
      return syntheticCandles(input.pair, input.lookbackDays);
    }
  }

  function executeBuy(sim, candle, input, reason) {
    var spend = Math.min(sim.cash, input.maxTradeUsd, input.initialCash * input.riskPct);
    if (spend < 0.01) return null;
    var executionPrice = candle.close * (1 + input.slippageBps / 10000);
    var fee = spend * input.feeBps / 10000;
    var netSpend = Math.max(0, spend - fee);
    var qty = netSpend / executionPrice;
    sim.cash -= spend;
    sim.qty += qty;
    sim.entryCost += netSpend;
    sim.fees += fee;
    return {
      time: candle.time,
      side: 'BUY',
      price: executionPrice,
      qty: qty,
      notional: spend,
      fee: fee,
      pnl: null,
      reason: reason
    };
  }

  function executeSell(sim, candle, input, reason) {
    if (sim.qty <= 0) return null;
    var sellQty = sim.qty;
    var executionPrice = candle.close * (1 - input.slippageBps / 10000);
    var gross = sellQty * executionPrice;
    var fee = gross * input.feeBps / 10000;
    var proceeds = gross - fee;
    var costBasis = sim.entryCost;
    var pnl = proceeds - costBasis;
    sim.cash += proceeds;
    sim.qty = 0;
    sim.entryCost = 0;
    sim.fees += fee;
    return {
      time: candle.time,
      side: 'SELL',
      price: executionPrice,
      qty: sellQty,
      notional: proceeds,
      fee: fee,
      pnl: pnl,
      reason: reason
    };
  }

  function runSimulation(candles, input) {
    var sim = {
      cash: input.initialCash,
      qty: 0,
      entryCost: 0,
      fees: 0,
      gridCenter: 0
    };
    var equity = [];
    var trades = [];
    var events = [];
    var lastSignal = { side: 'hold', confidence: 0, reason: 'not run' };

    for (var i = 0; i < candles.length; i += 1) {
      var candle = candles[i];
      var currentSignal = signal(input.strategy.id, candles, i, sim);
      var trade = null;
      if (currentSignal.side === 'buy') {
        trade = executeBuy(sim, candle, input, currentSignal.reason);
      } else if (currentSignal.side === 'sell') {
        trade = executeSell(sim, candle, input, currentSignal.reason);
      }
      if (trade) {
        trades.push(trade);
        events.push(trade.side + ' at ' + fmtUsd(trade.price) + ': ' + trade.reason);
      }
      lastSignal = currentSignal;
      equity.push({
        time: candle.time,
        price: candle.close,
        value: sim.cash + sim.qty * candle.close,
        cash: sim.cash,
        qty: sim.qty
      });
    }

    var latest = candles[candles.length - 1];
    lastSignal = signal(input.strategy.id, candles, candles.length - 1, sim);
    var metrics = calculateMetrics(equity, trades, input, sim.fees);
    var status = sim.qty > 0 ? 'holding spot exposure' : 'flat';
    if (!events.length) events.push('No fills. Latest signal: ' + lastSignal.side.toUpperCase() + ' - ' + lastSignal.reason + '.');
    events.unshift(input.strategy.name + ' completed on ' + candles.length + ' hourly candles; account is ' + status + '.');

    return {
      input: input,
      candles: candles,
      equity: equity,
      trades: trades,
      events: events.slice(-24),
      metrics: metrics,
      latestSignal: lastSignal,
      latestPrice: latest ? latest.close : null,
      finalCash: sim.cash,
      finalQty: sim.qty,
      fees: sim.fees
    };
  }

  function calculateMetrics(equity, trades, input, totalFees) {
    if (equity.length < 2) {
      return emptyMetrics(totalFees);
    }
    var values = equity.map(function (point) { return point.value; });
    var returns = [];
    for (var i = 1; i < values.length; i += 1) {
      returns.push(values[i - 1] > 0 ? (values[i] - values[i - 1]) / values[i - 1] : 0);
    }
    var totalReturn = input.initialCash > 0 ? (values[values.length - 1] - input.initialCash) / input.initialCash : 0;
    var days = Math.max(1, (equity[equity.length - 1].time - equity[0].time) / (24 * 60 * 60 * 1000));
    var years = days / 365;
    var cagr = years > 0 && values[values.length - 1] > 0 ? Math.pow(values[values.length - 1] / input.initialCash, 1 / years) - 1 : 0;
    var mean = average(returns);
    var vol = stddev(returns);
    var downside = returns.filter(function (r) { return r < 0; });
    var downsideVol = Math.sqrt(average(downside.map(function (r) { return r * r; })));
    var sharpe = vol > 0 ? mean / vol * Math.sqrt(365 * 24) : 0;
    var sortino = downsideVol > 0 ? mean / downsideVol * Math.sqrt(365 * 24) : 0;
    var drawdown = maxDrawdown(values);
    var calmar = drawdown < 0 ? cagr / Math.abs(drawdown) : 0;
    var sellTrades = trades.filter(function (trade) {
      return trade.side === 'SELL' && Number.isFinite(trade.pnl);
    });
    var wins = sellTrades.filter(function (trade) { return trade.pnl > 0; });
    var losses = sellTrades.filter(function (trade) { return trade.pnl < 0; });
    var grossProfit = wins.reduce(function (sum, trade) { return sum + trade.pnl; }, 0);
    var grossLoss = Math.abs(losses.reduce(function (sum, trade) { return sum + trade.pnl; }, 0));
    var profitFactor = grossLoss > 0 ? grossProfit / grossLoss : grossProfit > 0 ? 100 : 0;
    var winRate = sellTrades.length ? wins.length / sellTrades.length : 0;
    var expectancy = sellTrades.length
      ? sellTrades.reduce(function (sum, trade) { return sum + trade.pnl; }, 0) / sellTrades.length
      : 0;

    return {
      totalReturn: totalReturn,
      cagr: cagr,
      sharpe: sharpe,
      sortino: sortino,
      calmar: calmar,
      maxDrawdown: drawdown,
      winRate: winRate,
      profitFactor: profitFactor,
      expectancy: expectancy,
      totalTrades: trades.length,
      closedTrades: sellTrades.length,
      totalFees: totalFees,
      endingValue: values[values.length - 1]
    };
  }

  function emptyMetrics(totalFees) {
    return {
      totalReturn: 0,
      cagr: 0,
      sharpe: 0,
      sortino: 0,
      calmar: 0,
      maxDrawdown: 0,
      winRate: 0,
      profitFactor: 0,
      expectancy: 0,
      totalTrades: 0,
      closedTrades: 0,
      totalFees: totalFees || 0,
      endingValue: 0
    };
  }

  function maxDrawdown(values) {
    var peak = values[0] || 0;
    var drawdown = 0;
    values.forEach(function (value) {
      peak = Math.max(peak, value);
      if (peak > 0) drawdown = Math.min(drawdown, (value - peak) / peak);
    });
    return drawdown;
  }

  function buildStrategyCode(input) {
    var asset = input.pair.replace('-USD', '');
    var interval = '1h';
    if (input.strategy.id === 'momentum') {
      return [
        'from vibetrading import vibe, get_spot_price, get_futures_ohlcv, buy, sell, my_spot_balance',
        '',
        'ASSET = "' + asset + '"',
        'SMA_FAST = 12',
        'SMA_SLOW = 48',
        'RSI_PERIOD = 14',
        'TP_PCT = 0.010',
        'SL_PCT = 0.006',
        '',
        '@vibe(interval="' + interval + '")',
        'def near_intents_momentum():',
        '    price = get_spot_price(ASSET)',
        '    ohlcv = get_futures_ohlcv(ASSET, "' + interval + '", 72)',
        '    fast = ohlcv["close"].rolling(SMA_FAST).mean().iloc[-1]',
        '    slow = ohlcv["close"].rolling(SMA_SLOW).mean().iloc[-1]',
        '    rsi = compute_rsi(ohlcv["close"], RSI_PERIOD).iloc[-1]',
        '    if fast > slow and rsi < 74:',
        '        buy(ASSET, sized_usd_order(), price, order_type="market")',
        '    elif trend_guard_failed():',
        '        sell(ASSET, current_position(), price, order_type="market")'
      ].join('\n');
    }
    if (input.strategy.id === 'mean_reversion') {
      return [
        'from vibetrading import vibe, get_spot_price, get_futures_ohlcv, buy, sell',
        '',
        'ASSET = "' + asset + '"',
        'BB_PERIOD = 20',
        'BB_STD = 2.0',
        'RSI_ENTRY = 42',
        '',
        '@vibe(interval="' + interval + '")',
        'def near_intents_mean_reversion():',
        '    price = get_spot_price(ASSET)',
        '    bars = get_futures_ohlcv(ASSET, "' + interval + '", 40)',
        '    middle = bars["close"].rolling(BB_PERIOD).mean().iloc[-1]',
        '    sigma = bars["close"].rolling(BB_PERIOD).std().iloc[-1]',
        '    lower = middle - BB_STD * sigma',
        '    rsi = compute_rsi(bars["close"], 14).iloc[-1]',
        '    if price < lower and rsi < RSI_ENTRY:',
        '        buy(ASSET, sized_usd_order(), price, order_type="market")',
        '    elif price >= middle or stop_loss_hit():',
        '        sell(ASSET, current_position(), price, order_type="market")'
      ].join('\n');
    }
    if (input.strategy.id === 'grid') {
      return [
        'from vibetrading import vibe, get_spot_price, buy, sell, my_spot_balance',
        '',
        'ASSET = "' + asset + '"',
        'GRID_SPACING_PCT = 0.008',
        '',
        '@vibe(interval="' + interval + '")',
        'def near_intents_spot_grid():',
        '    price = get_spot_price(ASSET)',
        '    center = load_grid_center(ASSET) or price',
        '    if price <= center * (1 - GRID_SPACING_PCT):',
        '        buy(ASSET, sized_usd_order(), price, order_type="market")',
        '        save_grid_center(ASSET, price)',
        '    elif price >= avg_entry(ASSET) * (1 + GRID_SPACING_PCT * 1.4):',
        '        sell(ASSET, current_position(), price, order_type="market")',
        '        save_grid_center(ASSET, price)'
      ].join('\n');
    }
    if (input.strategy.id === 'dca') {
      return [
        'from vibetrading import vibe, get_spot_price, buy, sell',
        '',
        'ASSET = "' + asset + '"',
        'TAKE_PROFIT = 0.045',
        '',
        '@vibe(interval="' + interval + '")',
        'def near_intents_dca_rebalance():',
        '    price = get_spot_price(ASSET)',
        '    if basket_return(ASSET) >= TAKE_PROFIT:',
        '        sell(ASSET, current_position(), price, order_type="market")',
        '    elif cadence_due("daily"):',
        '        buy(ASSET, sized_usd_order(), price, order_type="market")'
      ].join('\n');
    }
    if (input.strategy.id === 'breakout') {
      return [
        'from vibetrading import vibe, get_spot_price, get_futures_ohlcv, buy, sell',
        '',
        'ASSET = "' + asset + '"',
        'CHANNEL = 24',
        '',
        '@vibe(interval="' + interval + '")',
        'def near_intents_breakout():',
        '    price = get_spot_price(ASSET)',
        '    bars = get_futures_ohlcv(ASSET, "' + interval + '", CHANNEL + 6)',
        '    channel_high = bars["high"].iloc[-CHANNEL:-1].max()',
        '    channel_low = bars["low"].iloc[-CHANNEL:-1].min()',
        '    if price > channel_high:',
        '        buy(ASSET, sized_usd_order(), price, order_type="market")',
        '    elif price < channel_low or atr_stop_hit():',
        '        sell(ASSET, current_position(), price, order_type="market")'
      ].join('\n');
    }
    return [
      'from vibetrading import vibe, get_spot_price, get_futures_ohlcv, buy, sell',
      '',
      'ASSET = "' + asset + '"',
      '',
      '@vibe(interval="' + interval + '")',
      'def near_intents_hl_momentum_watch():',
      '    price = get_spot_price(ASSET)',
      '    bars = get_futures_ohlcv(ASSET, "' + interval + '", 30)',
      '    ret_12h = bars["close"].iloc[-1] / bars["close"].iloc[-12] - 1',
      '    ret_24h = bars["close"].iloc[-1] / bars["close"].iloc[-24] - 1',
      '    if ret_12h > 0.012 and ret_24h > 0.018:',
      '        buy(ASSET, sized_usd_order(), price, order_type="market")',
      '    elif ret_12h < -0.012:',
      '        sell(ASSET, current_position(), price, order_type="market")'
    ].join('\n');
  }

  function buildIntentDraft(result) {
    if (!result) return null;
    var input = result.input;
    var side = effectiveSide(result);
    var token = input.baseAsset;
    var quote = input.quoteAsset;
    var maxUsd = Math.min(input.liveAmountUsd, input.maxTradeUsd, input.initialCash * input.riskPct);
    var quoteRequest = null;
    if (side === 'buy') {
      quoteRequest = {
        sell: { asset_id: quote, amount_usd_estimate: Number(maxUsd.toFixed(6)) },
        buy: { asset_id: token, min_amount_usd: Number((maxUsd * (1 - input.slippageBps / 10000)).toFixed(6)) },
        route_preference: 'best_solver_quote',
        solver_method: 'quote'
      };
    } else if (side === 'sell') {
      quoteRequest = {
        sell: { asset_id: token, amount_usd_estimate: Number(maxUsd.toFixed(6)) },
        buy: { asset_id: quote, min_amount_usd: Number((maxUsd * (1 - input.slippageBps / 10000)).toFixed(6)) },
        route_preference: 'best_solver_quote',
        solver_method: 'quote'
      };
    }
    var runHash = hashText(JSON.stringify({
      strategy: input.strategy.id,
      pair: input.pair,
      prompt: input.prompt,
      metrics: result.metrics,
      latest: result.latestSignal
    }));
    return {
      schema: 'near-intents-vibetrading-lab/1',
      mode: 'paper_plus_live_execution',
      run_hash: runHash,
      created_at: new Date().toISOString(),
      source: 'VibeTrading-inspired client backtest',
      execution_boundary: 'manual_wallet_signature_required',
      account_id: input.accountId,
      market: {
        pair: input.pair.replace('-', '/'),
        base_asset_id: token,
        quote_asset_id: quote,
        data_source: state.dataSource,
        last_price_usd: result.latestPrice ? Number(result.latestPrice.toFixed(8)) : null
      },
      strategy: {
        id: input.strategy.id,
        name: input.strategy.name,
        prompt: input.prompt,
        generated_code_hash: hashText(buildStrategyCode(input)),
        latest_signal: side,
        confidence: Number(result.latestSignal.confidence.toFixed(4)),
        reason: result.latestSignal.reason
      },
      backtest: {
        lookback_days: input.lookbackDays,
        starting_cash_usd: input.initialCash,
        ending_value_usd: Number(result.metrics.endingValue.toFixed(6)),
        total_return: Number(result.metrics.totalReturn.toFixed(6)),
        sharpe: Number(result.metrics.sharpe.toFixed(4)),
        max_drawdown: Number(result.metrics.maxDrawdown.toFixed(6)),
        win_rate: Number(result.metrics.winRate.toFixed(4)),
        profit_factor: Number(result.metrics.profitFactor.toFixed(4)),
        total_trades: result.metrics.totalTrades,
        total_fees_usd: Number(result.metrics.totalFees.toFixed(6))
      },
      risk: {
        max_trade_usd: Number(input.maxTradeUsd.toFixed(6)),
        live_cap_usd: Number(input.liveAmountUsd.toFixed(6)),
        risk_per_signal_pct: Number((input.riskPct * 100).toFixed(4)),
        fee_bps: input.feeBps,
        slippage_bps: input.slippageBps,
        funding_asset_id: quote,
        base_decimals: input.baseDecimals,
        quote_decimals: input.quoteDecimals,
        no_auto_signing: true,
        publish_requires_solver_jwt: true
      },
      draft_intent: {
        venue: 'near_intents',
        verifier_contract: INTENTS_CONTRACT,
        deposit_model: 'direct_near_intents_balance',
        solver_quote_required: side !== 'hold',
        decision: side,
        side_source: input.sideOverride === 'auto' ? 'latest_strategy_signal' : 'manual_override',
        quote_request: quoteRequest,
        hold_reason: side === 'hold' ? result.latestSignal.reason : null,
        expires_in_seconds: 120
      }
    };
  }

  function renderMetrics(result) {
    var metrics = result.metrics;
    els.returnMetric.textContent = fmtPct(metrics.totalReturn);
    els.returnMetric.className = metrics.totalReturn >= 0 ? 'good' : 'bad';
    els.sharpeMetric.textContent = fmtNum(metrics.sharpe, 2);
    els.drawdownMetric.textContent = fmtPct(metrics.maxDrawdown);
    els.drawdownMetric.className = metrics.maxDrawdown < -0.08 ? 'bad' : '';
    els.winRateMetric.textContent = metrics.closedTrades ? (metrics.winRate * 100).toFixed(2) + '%' : '-';
    els.profitMetric.textContent = metrics.closedTrades ? fmtNum(metrics.profitFactor, 2) : '-';
    els.signalMetric.textContent = result.latestSignal.side.toUpperCase();
    els.signalMetric.className = 'signal-' + result.latestSignal.side;
    els.chartTitle.textContent = result.input.strategy.name + ' on ' + result.input.pair.replace('-', ' / ');
    els.dataSourceLabel.textContent = state.dataSource;
  }

  function renderSignal(result) {
    var signalText = result.latestSignal.side.toUpperCase();
    els.signalTitle.textContent = signalText;
    els.signalTitle.className = 'signal-' + result.latestSignal.side;
    els.signalSummary.textContent = result.latestSignal.reason + '. Confidence ' +
      Math.round(result.latestSignal.confidence * 100) + '%. Ending value ' +
      fmtUsd(result.metrics.endingValue) + ' after ' + result.trades.length + ' paper fills.';
  }

  function renderTrades(trades) {
    els.tradeTableBody.innerHTML = '';
    els.tradeCountLabel.textContent = trades.length + (trades.length === 1 ? ' fill' : ' fills');
    if (!trades.length) {
      var empty = document.createElement('tr');
      empty.className = 'empty-row';
      empty.innerHTML = '<td colspan="5">No fills in this run.</td>';
      els.tradeTableBody.appendChild(empty);
      return;
    }
    trades.slice(-12).reverse().forEach(function (trade) {
      var row = document.createElement('tr');
      row.innerHTML = '<td>' + fmtDate(trade.time) + '</td>' +
        '<td class="' + (trade.side === 'BUY' ? 'signal-buy' : 'signal-sell') + '">' + trade.side + '</td>' +
        '<td>' + fmtUsd(trade.price) + '</td>' +
        '<td>' + fmtNum(trade.qty, 6) + '</td>' +
        '<td class="' + (trade.pnl >= 0 ? 'signal-buy' : 'signal-sell') + '">' +
        (trade.pnl === null ? '-' : fmtUsd(trade.pnl)) + '</td>';
      els.tradeTableBody.appendChild(row);
    });
  }

  function renderTape(events) {
    els.eventTape.innerHTML = '';
    els.tapeSummary.textContent = events.length + ' events';
    events.slice(-14).reverse().forEach(function (event) {
      var item = document.createElement('li');
      item.textContent = event;
      els.eventTape.appendChild(item);
    });
  }

  function renderIntent(result) {
    var draft = buildIntentDraft(result);
    state.latestIntent = draft;
    els.intentOutput.textContent = JSON.stringify(draft, null, 2);
    els.intentStatus.textContent = draft.draft_intent.decision.toUpperCase();
  }

  function renderCode(input) {
    els.strategyCode.textContent = buildStrategyCode(input);
  }

  function drawChart(result) {
    var canvas = els.chartCanvas;
    var ctx = canvas.getContext('2d');
    var width = canvas.width;
    var height = canvas.height;
    var pad = { left: 54, right: 28, top: 26, bottom: 42 };
    ctx.clearRect(0, 0, width, height);
    ctx.fillStyle = '#080908';
    ctx.fillRect(0, 0, width, height);

    if (!result || !result.equity.length) {
      ctx.fillStyle = '#9da59f';
      ctx.font = '28px system-ui';
      ctx.fillText('Run backtest to render price, equity, and fills', pad.left, height / 2);
      return;
    }

    var equity = result.equity;
    var values = equity.map(function (point) { return point.value; });
    var prices = equity.map(function (point) { return point.price; });
    var minValue = Math.min.apply(null, values);
    var maxValue = Math.max.apply(null, values);
    var minPrice = Math.min.apply(null, prices);
    var maxPrice = Math.max.apply(null, prices);
    var plotW = width - pad.left - pad.right;
    var plotH = height - pad.top - pad.bottom;

    function x(index) {
      return pad.left + (equity.length <= 1 ? 0 : (index / (equity.length - 1)) * plotW);
    }

    function yNorm(value, min, max) {
      var span = Math.max(max - min, Math.abs(max) * 0.001, 0.000001);
      return pad.top + plotH - ((value - min) / span) * plotH;
    }

    ctx.strokeStyle = 'rgba(246, 244, 237, 0.08)';
    ctx.lineWidth = 1;
    for (var g = 0; g <= 5; g += 1) {
      var gy = pad.top + (plotH / 5) * g;
      ctx.beginPath();
      ctx.moveTo(pad.left, gy);
      ctx.lineTo(width - pad.right, gy);
      ctx.stroke();
    }

    ctx.fillStyle = '#6f7771';
    ctx.font = '18px system-ui';
    ctx.fillText(fmtUsd(maxValue), 10, pad.top + 10);
    ctx.fillText(fmtUsd(minValue), 10, height - pad.bottom);

    ctx.beginPath();
    prices.forEach(function (price, index) {
      var px = x(index);
      var py = yNorm(price, minPrice, maxPrice);
      if (index === 0) ctx.moveTo(px, py);
      else ctx.lineTo(px, py);
    });
    ctx.strokeStyle = '#82f6b8';
    ctx.lineWidth = 2.5;
    ctx.stroke();

    ctx.beginPath();
    values.forEach(function (value, index) {
      var px = x(index);
      var py = yNorm(value, minValue, maxValue);
      if (index === 0) ctx.moveTo(px, py);
      else ctx.lineTo(px, py);
    });
    ctx.strokeStyle = '#73d5ff';
    ctx.lineWidth = 2.5;
    ctx.stroke();

    result.trades.forEach(function (trade) {
      var idx = equity.findIndex(function (point) {
        return point.time >= trade.time;
      });
      if (idx < 0) return;
      var tx = x(idx);
      var ty = yNorm(equity[idx].price, minPrice, maxPrice);
      ctx.beginPath();
      ctx.arc(tx, ty, 6, 0, Math.PI * 2);
      ctx.fillStyle = trade.side === 'BUY' ? '#82f6b8' : '#ff766f';
      ctx.fill();
      ctx.strokeStyle = '#080908';
      ctx.lineWidth = 2;
      ctx.stroke();
    });
  }

  function resetMetrics() {
    [els.returnMetric, els.sharpeMetric, els.drawdownMetric, els.winRateMetric, els.profitMetric, els.signalMetric].forEach(function (el) {
      el.textContent = '-';
      el.className = '';
    });
    els.chartTitle.textContent = 'Awaiting run';
    els.signalTitle.textContent = 'No run yet';
    els.signalTitle.className = '';
    els.signalSummary.textContent = 'Select a strategy and run a paper backtest.';
    els.tradeCountLabel.textContent = '0 fills';
    els.tradeTableBody.innerHTML = '<tr class="empty-row"><td colspan="5">Run a strategy to populate fills.</td></tr>';
    els.tapeSummary.textContent = 'Idle';
    els.eventTape.innerHTML = '';
    els.intentOutput.textContent = 'Run a strategy to prepare a NEAR Intents draft.';
    els.strategyCode.textContent = 'Run a strategy to generate code.';
    setLiveMode('Dry run ready');
    setLiveStep('backtest');
    updateWalletUi();
    drawChart(null);
  }

  function markStale() {
    state.isStale = true;
    setRunState('Ready');
    setPhase('describe');
    setLiveMode('Strategy edited');
    setLiveStep('backtest');
    els.describeStatus.textContent = 'Edited';
    els.validateStatus.textContent = 'Pending';
    els.backtestStatus.textContent = 'Pending';
    els.intentStatus.textContent = 'Unsigned';
  }

  async function ensureBacktest() {
    if (state.result && !state.isStale) return state.result;
    await runBacktest();
    if (!state.result) throw new Error('Backtest did not produce a result');
    return state.result;
  }

  function renderLivePlan(plan) {
    state.latestLivePlan = plan;
    if (plan.side === 'hold') {
      writeLiveOutput({
        status: 'hold',
        reason: plan.reason,
        next_step: 'No live order will be built unless you override the side.'
      });
      return;
    }
    writeLiveOutput({
      status: 'ready_to_quote',
      side: plan.side,
      source: plan.side_source,
      strategy: plan.strategy,
      reason: plan.reason,
      confidence: Number(plan.confidence.toFixed(4)),
      live_cap_usd: plan.live_cap_usd,
      quote_request: {
        defuse_asset_identifier_in: plan.asset_in,
        defuse_asset_identifier_out: plan.asset_out,
        exact_amount_in: plan.amount_in,
        human_amount_in: plan.amount_in_decimal
      }
    });
  }

  async function runDryQuote() {
    setLiveStep('quote');
    setLiveMode('Preparing quote');
    els.dryQuoteBtn.disabled = true;
    try {
      var result = await ensureBacktest();
      var plan = buildLivePlan(result);
      renderLivePlan(plan);
      if (plan.side === 'hold') {
        setLiveMode('Hold signal');
        return;
      }
      var dryQuote = await requestOneClickQuote(plan, true);
      setLiveMode('1Click dry quote ready');
      writeLiveOutput({
        mode: 'one_click_dry_quote',
        plan: plan,
        quote: dryQuote,
        note: 'This validates a deposited Intents-balance swap path. Use Run live once with a solver relay JWT to sign/publish token_diff.'
      });
    } catch (err) {
      setLiveStep('quote', 'error');
      setLiveMode('Quote failed');
      writeLiveOutput('Dry quote failed: ' + (err.message || String(err)));
    } finally {
      els.dryQuoteBtn.disabled = false;
    }
  }

  async function runLiveOnce() {
    els.runLiveBtn.disabled = true;
    setLiveMode('Running live once');
    setLiveStep('backtest');
    try {
      if (!state.wallet.accountId) {
        await connectWallet();
      }
      var result = await ensureBacktest();
      var plan = buildLivePlan(result);
      renderLivePlan(plan);
      if (plan.side === 'hold') {
        setLiveMode('Hold signal');
        return;
      }

      var solverQuote = null;
      setLiveStep('quote');
      if (els.solverApiKeyInput.value.trim()) {
        solverQuote = await requestSolverQuote(plan);
        writeLiveOutput({
          mode: 'solver_quote_ready',
          plan: plan,
          selected_quote: solverQuote.selected,
          quote_count: solverQuote.quotes.length
        });
      } else {
        var dryQuote = await requestOneClickQuote(plan, true);
        setLiveStep('quote', 'warn');
        writeLiveOutput({
          mode: 'dry_quote_only',
          plan: plan,
          one_click_quote: dryQuote,
          blocked_before_signature: 'Paste a solver relay JWT to request a low-level quote_hash and publish a signed token_diff intent.'
        });
        setLiveMode('JWT needed to publish');
        return;
      }

      setLiveStep('sign');
      setLiveMode('Wallet signature required');
      var signedIntent = await signLiveIntent(plan, solverQuote);
      state.latestSignedIntent = signedIntent;
      state.latestIntent = {
        plan: plan,
        quote: solverQuote.selected,
        signed_data: signedIntent
      };
      els.intentOutput.textContent = JSON.stringify(state.latestIntent, null, 2);
      els.intentStatus.textContent = 'SIGNED';
      writeLiveOutput({
        mode: 'signed_nep413',
        signed_data: signedIntent,
        quote_hash: solverQuote.selected.quote_hash,
        next_step: 'Publishing through solver relay.'
      });

      setLiveStep('publish');
      setLiveMode('Publishing intent');
      var published = await publishSignedIntent(signedIntent, solverQuote);
      state.latestIntent.publish = published;
      els.intentOutput.textContent = JSON.stringify(state.latestIntent, null, 2);
      setLiveMode('Intent submitted');
      writeLiveOutput({
        mode: 'published',
        plan: plan,
        quote: solverQuote.selected,
        publish: published
      });
      fetchBalances();
    } catch (err) {
      setLiveStep('publish', 'error');
      setLiveMode('Live run failed');
      writeLiveOutput('Live run failed: ' + (err.message || String(err)));
    } finally {
      els.runLiveBtn.disabled = false;
    }
  }

  async function runBacktest() {
    var input = params();
    var validations = validateRun(input);
    renderValidation(validations);
    setRunState('Loading data');
    setPhase('validate');
    els.validateStatus.textContent = validations.some(function (item) { return item.tone === 'warn'; }) ? 'Warnings' : 'Valid';
    els.describeStatus.textContent = 'Captured';
    els.runBacktestBtn.disabled = true;
    els.topRunBtn.disabled = true;
    els.heroRunBtn.disabled = true;
    els.buildIntentBtn.disabled = true;

    try {
      var candles = await loadCandles(input);
      state.candles = candles;
      els.dataSourceLabel.textContent = state.dataSource;
      setRunState('Backtesting');
      setPhase('backtest');
      els.backtestStatus.textContent = candles.length + ' candles';
      await new Promise(function (resolve) { setTimeout(resolve, 70); });
      var result = runSimulation(candles, input);
      state.result = result;
      state.isStale = false;
      state.lastRunAt = new Date();
      renderMetrics(result);
      renderSignal(result);
      renderTrades(result.trades);
      renderTape(result.events);
      renderCode(input);
      renderIntent(result);
      drawChart(result);
      setPhase('intent', state.dataSource === 'Sample fallback' ? { backtest: 'warn' } : {});
      setRunState(state.dataSource === 'Sample fallback' ? 'Sample run' : 'Live data run');
      setLiveMode('Strategy ready');
      setLiveStep('quote');
    } catch (err) {
      setRunState('Run failed');
      setPhase('backtest', { backtest: 'error' });
      setLiveStep('backtest', 'error');
      els.backtestStatus.textContent = 'Failed';
      els.signalTitle.textContent = 'Run failed';
      els.signalSummary.textContent = err.message || String(err);
    } finally {
      els.runBacktestBtn.disabled = false;
      els.topRunBtn.disabled = false;
      els.heroRunBtn.disabled = false;
      els.buildIntentBtn.disabled = false;
    }
  }

  function buildIntentFromCurrent() {
    if (!state.result || state.isStale) {
      runBacktest();
      return;
    }
    renderIntent(state.result);
    setPhase('intent');
    setRunState('Intent drafted');
  }

  function copyIntent() {
    var text = els.intentOutput.textContent || '';
    if (text.indexOf('{') !== 0) return;
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

  function copySigned() {
    if (!state.latestSignedIntent) {
      els.copySignedBtn.textContent = 'No signature';
      setTimeout(function () {
        els.copySignedBtn.textContent = 'Copy signed';
      }, 1200);
      return;
    }
    navigator.clipboard.writeText(JSON.stringify(state.latestSignedIntent, null, 2)).then(function () {
      els.copySignedBtn.textContent = 'Copied';
      setTimeout(function () {
        els.copySignedBtn.textContent = 'Copy signed';
      }, 1200);
    }).catch(function () {
      els.copySignedBtn.textContent = 'Copy failed';
      setTimeout(function () {
        els.copySignedBtn.textContent = 'Copy signed';
      }, 1200);
    });
  }

  function bind() {
    renderStrategies();
    resetMetrics();
    renderValidation(validateRun(params()));
    els.runBacktestBtn.addEventListener('click', runBacktest);
    els.topRunBtn.addEventListener('click', runBacktest);
    els.heroRunBtn.addEventListener('click', function () {
      document.getElementById('studio').scrollIntoView({ behavior: 'smooth', block: 'start' });
      runBacktest();
    });
    els.connectWalletBtn.addEventListener('click', connectWallet);
    els.heroConnectBtn.addEventListener('click', connectWallet);
    els.runLiveBtn.addEventListener('click', runLiveOnce);
    els.dryQuoteBtn.addEventListener('click', runDryQuote);
    els.fetchBalancesBtn.addEventListener('click', fetchBalances);
    els.buildIntentBtn.addEventListener('click', buildIntentFromCurrent);
    els.copyIntentBtn.addEventListener('click', copyIntent);
    els.copySignedBtn.addEventListener('click', copySigned);
    els.pairSelect.addEventListener('change', function () {
      updateAssetDefaults();
      markStale();
    });
    [els.strategyPrompt, els.lookbackSelect, els.balanceInput, els.liveAmountUsdInput, els.accountInput, els.maxTradeInput, els.riskPctInput, els.feeBpsInput, els.slippageInput, els.baseAssetInput, els.quoteAssetInput, els.baseDecimalsInput, els.quoteDecimalsInput, els.networkSelect, els.sideOverrideSelect].forEach(function (el) {
      el.addEventListener('input', markStale);
      el.addEventListener('change', markStale);
    });
    [els.baseAssetInput, els.quoteAssetInput].forEach(function (el) {
      el.addEventListener('change', function () {
        applyKnownTokenMetadata(els.baseAssetInput.value, els.quoteAssetInput.value);
      });
    });
    window.addEventListener('resize', function () {
      drawChart(state.result);
    });
    updateWalletUi();
    loadSupportedTokens();
    setTimeout(runBacktest, 250);
  }

  bind();
}());
