function rememberSseEventId(event) {
  if (!event || !event.lastEventId) return;
  _lastSseEventId = event.lastEventId;
  window.__e2e = window.__e2e || {};
  window.__e2e.lastSseEventId = event.lastEventId;
}

function connectSSE(lastEventIdOverride) {
  if (eventSource) eventSource.close();
  cleanupConnectionState();

  // In OIDC mode the reverse proxy provides auth; no query token needed.
  let chatSseUrl = (token && !oidcProxyAuth)
    ? '/api/chat/events?token=' + encodeURIComponent(token)
    : '/api/chat/events';
  if (window.isDebugMode) {
    chatSseUrl += (chatSseUrl.includes('?') ? '&' : '?') + 'debug=true';
  }
  const lastEventId = lastEventIdOverride || _lastSseEventId;
  if (lastEventId) {
    chatSseUrl += (chatSseUrl.includes('?') ? '&' : '?')
      + 'last_event_id=' + encodeURIComponent(lastEventId);
  }
  eventSource = new EventSource(chatSseUrl);

  // Notify the debug panel (when loaded) so it can attach listeners to
  // the freshly created EventSource. Hooked in lazily so the panel can
  // be absent in non-admin builds without breaking SSE setup.
  if (typeof window.onDebugSSEConnect === 'function') {
    window.onDebugSSEConnect(eventSource);
  }

  const addTrackedEventListener = (eventType, handler) => {
    eventSource.addEventListener(eventType, (event) => {
      rememberSseEventId(event);
      handler(event);
    });
  };

  eventSource.onopen = () => {
    document.getElementById('sse-dot').classList.remove('disconnected');
    var statusEl = document.getElementById('sse-status');
    if (statusEl) statusEl.textContent = I18n.t('status.connected');
    _reconnectAttempts = 0;
    // Clear stale turn-tracking state from before the disconnect
    _turnResponseReceived = false;
    if (_doneWithoutResponseTimer) {
      clearTimeout(_doneWithoutResponseTimer);
      _doneWithoutResponseTimer = null;
    }

    // Dismiss connection-lost banner and show reconnected flash
    if (_connectionLostTimer) {
      clearTimeout(_connectionLostTimer);
      _connectionLostTimer = null;
    }
    const lostBanner = document.getElementById('connection-banner');
    if (lostBanner) {
      lostBanner.textContent = I18n.t('connection.reconnected');
      lostBanner.className = 'connection-banner connection-banner-success';
      setTimeout(() => { lostBanner.remove(); }, 2000);
    }

    // If we were restarting, close the modal and reset button now that server is back
    if (isRestarting) {
      const loaderEl = document.getElementById('restart-loader');
      if (loaderEl) loaderEl.style.display = 'none';
      const restartBtn = document.getElementById('restart-btn');
      const restartIcon = document.getElementById('restart-icon');
      if (restartBtn) restartBtn.disabled = false;
      if (restartIcon) restartIcon.classList.remove('spinning');
      isRestarting = false;
    }

    if (sseHasConnectedBefore && currentThreadId) {
      finalizeActivityGroup();
      // Only reload full history if disconnected beyond the threshold. Brief
      // reconnects (tab visibility change, transient network blip) rely on
      // SSE catch-up and the "Done without response" safety net (#2079).
      // Full re-render loses scroll position and disrupts the user.
      const disconnectMs = _sseDisconnectedAt ? Date.now() - _sseDisconnectedAt : 0;
      if (disconnectMs > SSE_RELOAD_THRESHOLD_MS) {
        loadHistory();
      }
    }
    _sseDisconnectedAt = null;
    // Clear stale processing state — agents may have finished during disconnect.
    // Refresh sidebar so stale spinners are removed immediately.
    processingThreads.clear();
    debouncedLoadThreads();
    sseHasConnectedBefore = true;
  };

  eventSource.onerror = () => {
    _sseDisconnectedAt = _sseDisconnectedAt || Date.now();
    _reconnectAttempts++;
    document.getElementById('sse-dot').classList.add('disconnected');
    var statusEl2 = document.getElementById('sse-status');
    if (statusEl2) statusEl2.textContent = I18n.t('status.reconnecting');

    // Update existing banner with attempt count
    const existingBanner = document.getElementById('connection-banner');
    if (existingBanner && existingBanner.classList.contains('connection-banner-warning')) {
      existingBanner.textContent = I18n.t('connection.reconnecting', { count: _reconnectAttempts });
    }

    // Start connection-lost banner timer (3s delay)
    if (!_connectionLostTimer && !existingBanner) {
      _connectionLostTimer = setTimeout(() => {
        _connectionLostTimer = null;
        // Only show if still disconnected
        const dot = document.getElementById('sse-dot');
        if (dot?.classList.contains('disconnected')) {
          showConnectionBanner(I18n.t('connection.reconnecting', { count: _reconnectAttempts }), 'warning');
        }
      }, 3000);
    }
  };

  // Forward all SSE events to registered widget handlers.
  // Wraps addEventListener to intercept every named event and dispatch
  // to widget subscribers before the built-in handler runs.
  // Must run before any addTrackedEventListener calls so the wrapper is in place.
  //
  // NOTE: Only NAMED events (those dispatched via `addEventListener('foo', …)`
  // by the gateway, see `SseEvent` in `src/channels/web/types.rs`) are
  // forwarded. The generic `eventSource.onmessage` handler is intentionally
  // NOT wrapped because the IronClaw gateway never emits SSE frames without
  // an `event:` field — every frame carries a typed name (`response`,
  // `tool_started`, `gate_required`, etc.). Widget authors should subscribe
  // to those typed events via `IronClaw.api.on('<event_type>', handler)`
  // rather than relying on the generic message channel; if a widget needs
  // an untyped stream it must open its own `EventSource`.
  var _origAddEventListener = eventSource.addEventListener.bind(eventSource);
  eventSource.addEventListener = function(type, listener, opts) {
    _origAddEventListener(type, function(e) {
      // Dispatch to widget handlers
      if (IronClaw.api && e.data) {
        try {
          var parsed = JSON.parse(e.data);
          IronClaw.api._dispatch(type, parsed);
        } catch (parseErr) {
          console.warn('[IronClaw] SSE parse error for event', type, parseErr);
        }
      }
      // Call original handler
      listener(e);
    }, opts);
  };

  addTrackedEventListener('response', (e) => {
    const data = JSON.parse(e.data);
    if (data.thread_id) activeWorkStore.clearThread(data.thread_id);
    if (!isCurrentThread(data.thread_id)) {
      if (data.thread_id) {
        unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
        debouncedLoadThreads();
      }
      return;
    }
    // Flush any remaining streaming buffer
    if (_streamDebounceTimer) {
      clearInterval(_streamDebounceTimer);
      _streamDebounceTimer = null;
    }
    if (_streamBuffer) {
      appendToLastAssistant(_streamBuffer);
      _streamBuffer = '';
    }
    // Remove streaming attribute from active assistant message
    const streamingMsg = document.querySelector('.message.assistant[data-streaming="true"]');
    if (streamingMsg) streamingMsg.removeAttribute('data-streaming');

    _turnResponseReceived = true;
    if (_doneWithoutResponseTimer) {
      clearTimeout(_doneWithoutResponseTimer);
      _doneWithoutResponseTimer = null;
    }
    finalizeActivityGroup();
    addMessage('assistant', data.content);
    pruneOldMessages();
    enableChatInput();
    // Refresh thread list so new titles appear after first message
    loadThreads();

    // Turn complete — remove oldest pending entry for this thread (#2409).
    // FIFO is safe here because the agent loop processes one turn at a time
    // per thread, so the oldest pending entry is the one that just completed.
    const pending = _pendingUserMessages.get(data.thread_id);
    if (pending) {
      pending.shift();
      if (pending.length === 0) _pendingUserMessages.delete(data.thread_id);
    }

    // Show restart modal if the response indicates restart was initiated
    if (data.content && data.content.toLowerCase().includes('restart initiated')) {
      setTimeout(() => tryShowRestartModal(), 500);
    }
  });

  addTrackedEventListener('thinking', (e) => {
    const data = JSON.parse(e.data);
    if (data.thread_id) {
      activeWorkStore.updateThread(data.thread_id, {
        statusText: data.message || ActivityEntry.t('activity.thinking', 'Thinking'),
      });
    }
    if (!isCurrentThread(data.thread_id)) {
      if (data.thread_id) {
        processingThreads.add(data.thread_id);
        debouncedLoadThreads();
      }
      return;
    }
    clearSuggestionChips();
    showActivityThinking(data.message);
  });

  addTrackedEventListener('suggestions', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    if (data.suggestions && data.suggestions.length > 0) {
      showSuggestionChips(data.suggestions);
    }
  });

  addTrackedEventListener('skill_activated', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    const names = Array.isArray(data.skill_names) ? data.skill_names : [];
    const feedback = Array.isArray(data.feedback) ? data.feedback : [];
    if (names.length === 0 && feedback.length === 0) return;
    addSkillActivationCard(names, feedback);
  });

  addTrackedEventListener('tool_started', (e) => {
    const data = JSON.parse(e.data);
    if (data.thread_id) {
      activeWorkStore.updateThread(data.thread_id, {
        statusText: ActivityEntry.t('activity.usingTool', 'Using {name}', {
          name: data.name,
        }),
      });
    }
    if (!isCurrentThread(data.thread_id)) {
      if (data.thread_id) {
        processingThreads.add(data.thread_id);
        debouncedLoadThreads();
      }
      return;
    }
    addToolCard(data);
  });

  addTrackedEventListener('tool_completed', (e) => {
    const data = JSON.parse(e.data);
    if (data.thread_id) {
      activeWorkStore.updateThread(data.thread_id, {
        statusText: data.success
          ? ActivityEntry.t('activity.finishedTool', 'Finished {name}', { name: data.name })
          : ActivityEntry.t('activity.failedTool', 'Failed {name}', { name: data.name }),
      });
    }
    if (!isCurrentThread(data.thread_id)) return;
    completeToolCard(data);

    // Show restart modal only when the restart tool succeeds
    if (data.name.toLowerCase() === 'restart' && data.success) {
      setTimeout(() => tryShowRestartModal(), 500);
    }
  });

  addTrackedEventListener('tool_result', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    setToolCardOutput(data);
  });

  addTrackedEventListener('stream_chunk', (e) => {
    const data = JSON.parse(e.data);
    if (data.thread_id) {
      activeWorkStore.updateThread(data.thread_id, {
        statusText: ActivityEntry.t('activity.streamingResponse', 'Streaming response'),
      });
    }
    if (!isCurrentThread(data.thread_id)) {
      if (data.thread_id) {
        processingThreads.add(data.thread_id);
        debouncedLoadThreads();
      }
      return;
    }
    finalizeActivityGroup();

    // Mark the active assistant message as streaming
    const container = document.getElementById('chat-messages');
    let lastAssistant = container.querySelector('.message.assistant:last-of-type');
    if (!lastAssistant) {
      addMessage('assistant', '');
      lastAssistant = container.querySelector('.message.assistant:last-of-type');
    }
    if (lastAssistant) lastAssistant.setAttribute('data-streaming', 'true');

    // Mark turn as having received content so the Done safety net
    // does not trigger a spurious loadHistory() for streaming responses.
    _turnResponseReceived = true;

    // Accumulate chunks and debounce rendering at 50ms intervals
    _streamBuffer += data.content;
    // Force flush when buffer exceeds 10K chars to prevent memory buildup
    if (_streamBuffer.length > 10000) {
      appendToLastAssistant(_streamBuffer);
      _streamBuffer = '';
    }
    if (!_streamDebounceTimer) {
      _streamDebounceTimer = setInterval(() => {
        if (_streamBuffer) {
          appendToLastAssistant(_streamBuffer);
          _streamBuffer = '';
        }
      }, STREAM_DEBOUNCE_MS);
    }
  });

  addTrackedEventListener('status', (e) => {
    const data = JSON.parse(e.data);
    if (data.thread_id) {
      const isBlockedStatus = activeWorkStore.isThreadBlocked(data.thread_id);
      if (data.message === 'Done' || data.message === 'Interrupted'
          || data.message === 'Rejected' || data.message === 'Tool call denied.') {
        activeWorkStore.clearThread(data.thread_id);
      } else if (data.message === 'Awaiting approval') {
        activeWorkStore.updateThread(data.thread_id, {
          statusText: ActivityEntry.t('activity.waitingApproval', 'Waiting for approval'),
          blockedReason: 'approval',
        });
      } else if (isBlockedStatus) {
        // Keep the user-visible blocked state until the gate resolves. Generic
        // step/status updates from the runner are less informative here.
      } else if (data.message) {
        activeWorkStore.updateThread(data.thread_id, {
          statusText: data.message,
          blockedReason: null,
        });
      }
    }
    if (!isCurrentThread(data.thread_id)) {
      if (data.thread_id) {
        if (data.message === 'Done' || data.message === 'Awaiting approval'
            || data.message === 'Interrupted' || data.message === 'Rejected'
            || data.message === 'Tool call denied.') {
          processingThreads.delete(data.thread_id);
        }
        debouncedLoadThreads();
      }
      return;
    }
    // "Done" and "Awaiting approval" are terminal signals from the agent:
    // the agentic loop finished, so re-enable input as a safety net in case
    // the response SSE event is empty or lost.
    // Status text is not displayed — inline activity cards handle visual feedback.
    if (data.message === 'Done' || data.message === 'Awaiting approval') {
      finalizeActivityGroup();
      enableChatInput();
      // Safety net (#2079): if "Done" arrives but we never received a
      // `response` event for this turn, the message may have been lost
      // (broadcast lag, proxy buffering, brief SSE disconnect). Reload
      // history after a short delay so the user sees the answer.
      if (!_turnResponseReceived && data.message === 'Done') {
        if (!_doneWithoutResponseTimer) {
          _doneWithoutResponseTimer = setTimeout(() => {
            _doneWithoutResponseTimer = null;
            if (currentThreadId) loadHistory();
          }, DONE_WITHOUT_RESPONSE_TIMEOUT_MS);
        }
      }
      _turnResponseReceived = false;
    }
  });

  addTrackedEventListener('job_started', (e) => {
    const data = JSON.parse(e.data);
    activeWorkStore.updateJob(data.job_id, {
      title: data.title,
      statusText: ActivityEntry.t('activity.starting', 'Starting'),
      state: 'running',
    });
    showJobCard(data);
  });

  addTrackedEventListener('approval_needed', (e) => {
    const data = JSON.parse(e.data);
    const hasThread = !!data.thread_id;
    const forCurrentThread = !hasThread || isCurrentThread(data.thread_id);
    if (data.thread_id) {
      activeWorkStore.updateThread(data.thread_id, {
        statusText: ActivityEntry.t('activity.waitingApproval', 'Waiting for approval'),
        blockedReason: 'approval',
      });
    }

    if (forCurrentThread) {
      showApproval(data);
    } else {
      // Keep thread list fresh when approval is requested in a background thread.
      unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
      debouncedLoadThreads();
    }

    // Track legacy approval events in the cross-thread sidebar tray too.
    aqAddGate(data);

    // Extension setup flows can surface approvals from any settings subtab.
    if (currentTab === 'settings') refreshCurrentSettingsTab();
  });

  addTrackedEventListener('onboarding_state', (e) => {
    const data = JSON.parse(e.data);
    handleOnboardingState(data);
  });

  addTrackedEventListener('gate_required', (e) => {
    const data = JSON.parse(e.data);
    if (data.thread_id) {
      const isCredentialGate = data.gate_name === 'credential' || data.gate_name === 'auth';
      activeWorkStore.updateThread(data.thread_id, {
        statusText: isCredentialGate
          ? ActivityEntry.t('activity.waitingAuth', 'Waiting for auth')
          : ActivityEntry.t('activity.waitingApproval', 'Waiting for approval'),
        blockedReason: isCredentialGate ? 'auth' : 'approval',
      });
    }
    handleGateRequired(data);
    // Track approval-type gates in the sidebar tray for cross-thread visibility.
    const resume = parseGateResumeKind(data.resume_kind);
    if (!resume || resume.type === 'approval') {
      aqAddGate({
        request_id: data.request_id,
        tool_name: data.tool_name,
        description: data.description,
        parameters: data.parameters,
        thread_id: data.thread_id || currentThreadId,
        allow_always: !(resume && resume.type === 'approval' && resume.allow_always === false),
      });
    }
  });

  addTrackedEventListener('gate_resolved', (e) => {
    const data = JSON.parse(e.data);
    if (data.thread_id) {
      activeWorkStore.updateThread(data.thread_id, {
        statusText: ActivityEntry.t('activity.resuming', 'Resuming'),
        blockedReason: null,
      });
    }
    handleGateResolved(data);
    aqResolveGate(data.request_id, data.resolution === 'denied' ? 'deny' : 'approve');
  });

  addTrackedEventListener('extension_status', (e) => {
    if (currentTab === 'settings') refreshCurrentSettingsTab();
  });

  addTrackedEventListener('image_generated', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    rememberGeneratedImage(data.thread_id, data.event_id, data.data_url, data.path);
    addGeneratedImage(data.data_url, data.path, data.event_id);
  });

  addTrackedEventListener('error', (e) => {
    if (e.data) {
      const data = JSON.parse(e.data);
      if (data.thread_id) activeWorkStore.clearThread(data.thread_id);
      if (!isCurrentThread(data.thread_id)) return;
      finalizeActivityGroup();
      addMessage('system', 'Error: ' + data.message);
      enableChatInput();
    }
  });

  // Job event listeners (activity stream for all sandbox jobs)
  const jobEventTypes = [
    'job_message', 'job_tool_use', 'job_tool_result',
    'job_status', 'job_result'
  ];
  for (const evtType of jobEventTypes) {
    addTrackedEventListener(evtType, (e) => {
      const data = JSON.parse(e.data);
      const jobId = data.job_id;
      if (!jobId) return;
      if (evtType === 'job_message') {
        activeWorkStore.updateJob(jobId, {
          statusText: (data.role ? data.role + ': ' : '') + (data.content || ActivityEntry.t('activity.working', 'Working')),
        });
      } else if (evtType === 'job_tool_use') {
        activeWorkStore.updateJob(jobId, {
          statusText: ActivityEntry.t('activity.runningTool', 'Running {name}', {
            name: data.tool_name || ActivityEntry.t('activity.tool', 'tool'),
          }),
        });
      } else if (evtType === 'job_tool_result') {
        activeWorkStore.updateJob(jobId, {
          statusText: ActivityEntry.t('activity.finishedTool', 'Finished {name}', {
            name: data.tool_name || ActivityEntry.t('activity.tool', 'tool'),
          }),
        });
      } else if (evtType === 'job_status') {
        activeWorkStore.updateJob(jobId, {
          statusText: data.message || JobActivityEntry.formatStatus('running'),
        });
      } else if (evtType === 'job_result') {
        activeWorkStore.updateJob(jobId, {
          active: false,
          state: JobActivityEntry.normalizeState(data.status),
          statusText: JobActivityEntry.formatStatus(data.status),
        });
      }
      // Move jobId to end of Map insertion order (LRU: most-recent last).
      // delete+set keeps the Map ordered by last-access time so that
      // keys().next() always yields the least-recently-used entry in O(1).
      const existing = jobEvents.get(jobId);
      if (existing) jobEvents.delete(jobId);
      const events = existing || [];
      jobEvents.set(jobId, events);
      events.push({ type: evtType, data: data, ts: Date.now() });
      // Cap per-job events to prevent memory leak
      while (events.length > JOB_EVENTS_CAP) events.shift();
      // Cap total tracked jobs — evict the least-recently-used entry (O(1)).
      // Skip currentJobId so the user's actively-viewed job detail panel
      // doesn't go empty when many other jobs fire events.
      if (jobEvents.size > JOB_EVENTS_MAX_JOBS) {
        let evicted = false;
        for (const k of jobEvents.keys()) {
          if (k !== currentJobId) {
            jobEvents.delete(k);
            evicted = true;
            break;
          }
        }
        // Fallback: if every entry is currentJobId (impossible in practice),
        // evict the first key to maintain the cap.
        if (!evicted) {
          jobEvents.delete(jobEvents.keys().next().value);
        }
      }
      // If the Activity tab is currently visible for this job, refresh it
      refreshActivityTab(jobId);
      // Auto-refresh job list when on jobs tab (debounced)
      if ((evtType === 'job_result' || evtType === 'job_status') && currentTab === 'jobs' && !currentJobId) {
        clearTimeout(jobListRefreshTimer);
        jobListRefreshTimer = setTimeout(loadJobs, 200);
      }
      // Clean up finished job events after a viewing window
      if (evtType === 'job_result') {
        setTimeout(() => jobEvents.delete(jobId), 60000);
      }
    });
  }

  // Plan progress checklist
  addTrackedEventListener('plan_update', (e) => {
    const data = JSON.parse(e.data);
    if (data.thread_id && !isCurrentThread(data.thread_id)) return;
    renderPlanChecklist(data);
  });
}

// Check if an SSE event belongs to the currently viewed thread.
// Events without a thread_id are dropped (prevents notification leaking).
