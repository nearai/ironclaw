use super::*;

impl RunDeliveryEventHandler {
    pub fn new(
        services: RunDeliveryServices,
        adapter_id: impl Into<String>,
        installation_id: impl Into<String>,
    ) -> Self {
        Self {
            services,
            adapter_id: adapter_id.into(),
            installation_id: installation_id.into(),
            current_target_resolver: None,
            ledger: Mutex::new(DeliveryLedger::default()),
        }
    }

    pub fn with_current_target_resolver(
        mut self,
        resolver: Arc<dyn CurrentDeliveryTargetResolver>,
    ) -> Self {
        self.current_target_resolver = Some(resolver);
        self
    }

    /// Reconcile an accepted external user message after the inbound workflow
    /// has durably finished binding its source route.
    ///
    /// Lifecycle events may be published by the turn coordinator before the
    /// product workflow returns its admission acknowledgement. The router is
    /// intentionally enqueue-only, so that event can race the final binding
    /// commit. This post-admission replay re-opens canonical run state through
    /// the same event path; it never sends provider traffic inline.
    pub async fn reconcile_accepted_user_message(
        &self,
        router: &RunDeliveryEventRouter,
        envelope: &ProductInboundEnvelope,
        ack: &ProductInboundAck,
    ) -> Result<(), ProductWorkflowError> {
        let Some(submitted_run_id) = accepted_user_message_run_id(envelope, ack) else {
            return Ok(());
        };
        let binding = self
            .services
            .binding_service
            .lookup_binding(crate::ResolveBindingRequest::from_envelope(envelope))
            .await?;
        let scope = ironclaw_turns::TurnScope::new_with_owner(
            binding.tenant_id,
            binding.agent_id,
            binding.project_id,
            binding.thread_id,
            binding.subject_user_id,
        );
        let state = self
            .services
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope,
                run_id: submitted_run_id,
            })
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("post-admission run state is unavailable: {error}"),
            })?;
        router
            .publish(TurnLifecycleEvent::from_run_state(
                &state,
                reconciliation_event_kind(state.status),
                None,
            ))
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("failed to enqueue post-admission run delivery: {error}"),
            })
    }

    pub(super) async fn handle_event(
        &self,
        event: &TurnLifecycleEvent,
    ) -> Result<DeliveryEventDisposition, RunDeliveryError> {
        let state = self
            .services
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: event.scope.clone(),
                run_id: event.run_id,
            })
            .await?;
        let Some(context) = state.product_context.as_ref() else {
            return Ok(DeliveryEventDisposition::without_source(
                DeliveryEventOutcome::Irrelevant,
            ));
        };
        let required_source_adapter = context
            .adapter
            .as_ref()
            .map(|adapter| adapter.as_str().to_string());
        let disposition = |outcome, source_cleanup_settled| {
            DeliveryEventDisposition::for_source(
                outcome,
                required_source_adapter.clone(),
                source_cleanup_settled,
            )
        };
        // External inbound runs use their sealed source route for every live
        // notification. A WebUI run has no channel adapter, but its completed
        // answer may still carry an explicit, host-sealed external target.
        // Let that one terminal case proceed to the durable target lookup;
        // without a run target it will fail closed below. Triggered runs remain
        // owned by the triggered-delivery driver.
        let handles_live_origin = context.origin == TurnOriginKind::Inbound
            || (context.origin == TurnOriginKind::WebUi && state.status == TurnStatus::Completed);
        if !handles_live_origin {
            return Ok(disposition(DeliveryEventOutcome::Irrelevant, false));
        }
        let is_source_handler = required_source_adapter
            .as_deref()
            .is_some_and(|source_adapter| source_adapter == self.adapter_id);
        let Some(actor) = state.actor.clone() else {
            return Ok(disposition(DeliveryEventOutcome::Deferred, false));
        };
        let terminal = matches!(
            state.status,
            TurnStatus::Completed | TurnStatus::Failed | TurnStatus::Cancelled
        );
        if terminal {
            self.purge_delivered_run(state.run_id);
        }
        if matches!(state.status, TurnStatus::Failed | TurnStatus::Cancelled) {
            if is_source_handler {
                self.retract_pending_messages(&state).await?;
            }
            return Ok(disposition(
                if is_source_handler {
                    DeliveryEventOutcome::Settled
                } else {
                    DeliveryEventOutcome::Irrelevant
                },
                is_source_handler,
            ));
        }
        if state.status != TurnStatus::Completed && !is_source_handler {
            return Ok(disposition(DeliveryEventOutcome::Irrelevant, false));
        }

        let mut source_cleanup_settled = false;
        let delivery_target = if state.status == TurnStatus::Completed {
            match self
                .services
                .outbound_store
                .load_run_final_reply_target(RunFinalReplyTargetRequest {
                    run_id: state.run_id,
                    scope: state.scope.clone(),
                    actor: actor.clone(),
                })
                .await?
                .map(|record| record.destination)
            {
                Some(RunFinalReplyDestination::WebApp) => {
                    if is_source_handler {
                        self.retract_pending_messages(&state).await?;
                        source_cleanup_settled = true;
                    }
                    return Ok(disposition(
                        if is_source_handler {
                            DeliveryEventOutcome::Settled
                        } else {
                            DeliveryEventOutcome::Irrelevant
                        },
                        source_cleanup_settled,
                    ));
                }
                Some(RunFinalReplyDestination::External {
                    reply_target_binding_ref,
                }) => {
                    if is_source_handler {
                        self.retract_pending_messages(&state).await?;
                        source_cleanup_settled = true;
                    }
                    reply_target_binding_ref
                }
                None if is_source_handler => state.reply_target_binding_ref.clone(),
                None => {
                    return Ok(disposition(
                        DeliveryEventOutcome::Irrelevant,
                        source_cleanup_settled,
                    ));
                }
            }
        } else {
            state.reply_target_binding_ref.clone()
        };
        let Some((stage, access)) = notification_claim(event, &state) else {
            return Ok(disposition(
                DeliveryEventOutcome::Irrelevant,
                source_cleanup_settled,
            ));
        };
        // The durable at-most-once identity for this notification must be keyed
        // off the frozen cursor of the event being processed — already loaded
        // and validated by the drain (`handoff.event_cursor` == `event.cursor`)
        // — not the re-fetched live `state.event_cursor`. They coincide only
        // while Completed is terminal; a future post-Completed cursor bump would
        // otherwise drift the Final projection ref to a new `delivery_id`,
        // defeating the CAS dedup and double-sending the final reply.
        let projection_epoch = stage.projection_epoch(event.cursor.0);
        let key = DeliveryEventKey {
            run_id: state.run_id,
            stage,
        };
        let Some(claim) = self.claim(key) else {
            return Ok(disposition(
                DeliveryEventOutcome::Settled,
                source_cleanup_settled || is_source_handler,
            ));
        };

        // Establish the sealed source-route authority before constructing an
        // auth prompt. Prompt construction can mint or supersede a provider
        // OAuth flow, so it must not happen for an uncommitted or revoked
        // reply route.
        let uses_run_scoped_target = delivery_target != state.reply_target_binding_ref;
        let source_target = if uses_run_scoped_target {
            let Some(resolver) = self.current_target_resolver.as_ref() else {
                return Ok(disposition(
                    DeliveryEventOutcome::Irrelevant,
                    source_cleanup_settled,
                ));
            };
            match resolver
                .resolve_current_target(&state.scope, &actor, &delivery_target)
                .await
            {
                Ok(Some(target)) if target.extension_id == self.services.extension_id => None,
                Ok(Some(_)) => {
                    return Ok(disposition(
                        DeliveryEventOutcome::Irrelevant,
                        source_cleanup_settled,
                    ));
                }
                // Let the outbound policy persist its durable Rejected outcome
                // for a removed target. The same current-authority check runs
                // again immediately before egress.
                Ok(None) | Err(ProductWorkflowError::BindingAccessDenied) => None,
                Err(error) => return Err(RunDeliveryError::Workflow(error)),
            }
        } else if state.status == TurnStatus::Completed {
            // Completed replies have no auth-prompt construction side effect.
            // Defer current membership/pairing validation to the outbound
            // policy so denial is persisted as a terminal Rejected attempt.
            None
        } else {
            match self
                .resolve_target(&state, &actor, &delivery_target, access)
                .await
            {
                Ok(Some(target)) => Some(target),
                Ok(None) => {
                    return Ok(disposition(
                        DeliveryEventOutcome::Irrelevant,
                        source_cleanup_settled,
                    ));
                }
                Err(
                    error @ (ProductWorkflowError::BindingAccessDenied
                    | ProductWorkflowError::BindingRequired { .. }
                    | ProductWorkflowError::UnknownInstallation),
                ) => {
                    tracing::debug!(
                        target = "ironclaw::reborn::run_delivery",
                        run_id = %state.run_id,
                        %error,
                        "deferred lifecycle delivery because the stored source route is not currently authorized"
                    );
                    // Submitted/blocked events can race the inbound workflow's
                    // source-route commit. Their post-admission replay must be
                    // allowed to retry.
                    return Ok(disposition(
                        DeliveryEventOutcome::Deferred,
                        source_cleanup_settled,
                    ));
                }
                Err(error) => return Err(RunDeliveryError::Workflow(error)),
            }
        };
        let Some(notification) = self.notification(event, &state, &actor).await? else {
            return Ok(disposition(
                DeliveryEventOutcome::Deferred,
                source_cleanup_settled,
            ));
        };

        let cleanup_intent = notification.intent;
        let delivered = match self
            .deliver_policy_notification(
                &state,
                &actor,
                &delivery_target,
                source_target.as_ref(),
                notification,
                &projection_epoch,
            )
            .await
        {
            Ok(delivered) => delivered,
            Err(RunDeliveryError::DeliveryFailed { .. }) => {
                // The coordinator has already recorded a durable terminal
                // provider outcome. Retrying this handoff cannot improve it.
                if is_source_handler && !source_cleanup_settled {
                    self.retract_pending_messages(&state).await?;
                    source_cleanup_settled = true;
                }
                claim.complete(terminal);
                return Ok(disposition(
                    DeliveryEventOutcome::Settled,
                    source_cleanup_settled,
                ));
            }
            Err(error) => return Err(error),
        };

        self.update_cleanup(cleanup_intent, &delivered, &state)
            .await?;
        source_cleanup_settled |= is_source_handler;
        claim.complete(terminal);
        Ok(disposition(
            DeliveryEventOutcome::Settled,
            source_cleanup_settled,
        ))
    }

    fn claim(&self, key: DeliveryEventKey) -> Option<DeliveryClaim<'_>> {
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if ledger.delivered.contains(&key) || !ledger.active.insert(key.clone()) {
            return None;
        }
        drop(ledger);
        Some(DeliveryClaim {
            ledger: &self.ledger,
            key,
            complete: false,
        })
    }

    fn purge_delivered_run(&self, run_id: TurnRunId) {
        self.ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .delivered
            .retain(|key| key.run_id != run_id);
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn delivered_claim_count_for_test(&self) -> usize {
        self.ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .delivered
            .len()
    }

    async fn notification(
        &self,
        event: &TurnLifecycleEvent,
        state: &TurnRunState,
        actor: &TurnActor,
    ) -> Result<Option<EventNotification>, RunDeliveryError> {
        let notification = match state.status {
            TurnStatus::Completed => {
                let Some(text) = self.read_final_text(state).await? else {
                    tracing::warn!(run_id = %state.run_id, "completed run has no finalized assistant message");
                    return Ok(None);
                };
                EventNotification {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    intent: DeliveryIntent::FinalReply,
                    access: StoredProductReplyTargetAccess::OrdinaryReply,
                    part: OutboundPart::Text(text),
                    gate_ref: None,
                    require_direct_message_target: false,
                }
            }
            TurnStatus::BlockedApproval => {
                let Some(gate_ref) = state.gate_ref.as_ref() else {
                    return Ok(None);
                };
                let context = match &self.services.approval_context {
                    Some(source) => {
                        source
                            .approval_prompt_context(gate_ref, &actor.user_id, &state.scope)
                            .await
                    }
                    None => None,
                };
                let direct = source_surface_is_direct(state);
                let view =
                    prompts::approval_gate_prompt_view(state.run_id, gate_ref, context.as_ref());
                EventNotification {
                    event_kind: RunNotificationEventKind::ApprovalNeeded,
                    intent: DeliveryIntent::GatePrompt,
                    access: StoredProductReplyTargetAccess::AuthorityBearingPrompt,
                    part: OutboundPart::Text(prompts::gate_prompt_text(&view, direct)),
                    gate_ref: Some(gate_ref.as_str().to_string()),
                    require_direct_message_target: false,
                }
            }
            TurnStatus::BlockedAuth => {
                let Some(gate_ref) = state.gate_ref.as_ref() else {
                    return Ok(None);
                };
                let direct = source_surface_is_direct(state);
                let access = auth_prompt_target_access(direct);
                let view = match &self.services.blocked_auth_prompts {
                    Some(source) => Some(
                        source
                            .auth_prompt_for_blocked_run(BlockedAuthPromptRequest {
                                fallback_owner_user_id: &actor.user_id,
                                scope: &state.scope,
                                run_id: state.run_id,
                                gate_ref: gate_ref.as_str(),
                                invocation_id: None,
                                body: "Authenticate to continue this run.".to_string(),
                                credential_requirements: &state.credential_requirements,
                            })
                            .await?,
                    ),
                    None => None,
                };
                let unavailable_message = prompts::unserviceable_auth_prompt_message(view.as_ref());
                let Some(mut view) = view.filter(prompts::auth_prompt_is_serviceable) else {
                    cancel_auth_blocked_run(
                        self.services.turn_coordinator.as_ref(),
                        self.services.auth_flow_cancel.as_deref(),
                        &state.scope,
                        actor.clone(),
                        state.run_id,
                        Some(gate_ref.as_str()),
                    )
                    .await?;
                    return Ok(Some(EventNotification {
                        event_kind: RunNotificationEventKind::AuthRequired,
                        intent: DeliveryIntent::RunFailureNotice,
                        access: StoredProductReplyTargetAccess::OrdinaryReply,
                        part: OutboundPart::Text(unavailable_message.to_string()),
                        gate_ref: None,
                        require_direct_message_target: false,
                    }));
                };
                view.body = prompts::actionable_auth_prompt_body(&view);
                if !direct {
                    view.authorization_url = None;
                    view.pairing = None;
                    if view.challenge_kind == Some(AuthPromptChallengeKind::Pairing) {
                        view.body = prompts::PAIRING_PRIVATE_SETUP_MESSAGE.to_string();
                    } else if view.challenge_kind == Some(AuthPromptChallengeKind::OAuthUrl) {
                        view.body = prompts::OAUTH_PRIVATE_SETUP_MESSAGE.to_string();
                    }
                }
                let require_direct_message_target =
                    view.authorization_url.is_some() || view.pairing.is_some();
                EventNotification {
                    event_kind: RunNotificationEventKind::AuthRequired,
                    intent: DeliveryIntent::AuthPrompt,
                    access,
                    part: OutboundPart::AuthPrompt {
                        view: Box::new(view),
                        direct_message: direct,
                    },
                    gate_ref: Some(gate_ref.as_str().to_string()),
                    require_direct_message_target,
                }
            }
            _ if matches!(
                event.kind,
                TurnEventKind::Submitted | TurnEventKind::Resumed
            ) =>
            {
                EventNotification {
                    event_kind: RunNotificationEventKind::ProgressUpdate,
                    intent: DeliveryIntent::RunProgress,
                    access: StoredProductReplyTargetAccess::OrdinaryReply,
                    part: OutboundPart::Text(prompts::WORKING_MESSAGE.to_string()),
                    gate_ref: None,
                    require_direct_message_target: false,
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(notification))
    }

    async fn resolve_target(
        &self,
        state: &TurnRunState,
        actor: &TurnActor,
        reply_target_binding_ref: &ReplyTargetBindingRef,
        access: StoredProductReplyTargetAccess,
    ) -> Result<Option<ResolvedStoredProductReplyTarget>, ProductWorkflowError> {
        let target = self
            .services
            .binding_service
            .resolve_stored_reply_target(ResolveStoredProductReplyTargetRequest {
                scope: state.scope.clone(),
                actor: actor.clone(),
                reply_target_binding_ref: reply_target_binding_ref.clone(),
                access,
            })
            .await?;
        if target.adapter_id.as_str() != self.adapter_id
            || target.installation_id.as_str() != self.installation_id
        {
            return Ok(None);
        }
        Ok(Some(target))
    }

    async fn deliver_policy_notification(
        &self,
        state: &TurnRunState,
        actor: &TurnActor,
        delivery_target: &ReplyTargetBindingRef,
        source_target: Option<&ResolvedStoredProductReplyTarget>,
        notification: EventNotification,
        projection_epoch: &str,
    ) -> Result<Vec<DeliveredChannelMessage>, RunDeliveryError> {
        let uses_run_scoped_target = delivery_target != &state.reply_target_binding_ref;
        let authority = if uses_run_scoped_target {
            let resolver =
                self.current_target_resolver
                    .as_ref()
                    .ok_or(RunDeliveryError::Workflow(
                        ProductWorkflowError::BindingAccessDenied,
                    ))?;
            LiveReplyTargetAuthority::Current {
                resolver: Arc::clone(resolver),
                scope: state.scope.clone(),
                actor: actor.clone(),
                expected_target: delivery_target.clone(),
                expected_extension_id: self.services.extension_id.clone(),
            }
        } else {
            LiveReplyTargetAuthority::Source(StoredReplyTargetAuthority {
                binding_service: Arc::clone(&self.services.binding_service),
                scope: state.scope.clone(),
                actor: actor.clone(),
                expected_target: delivery_target.clone(),
                expected_adapter_id: self.adapter_id.clone(),
                expected_installation_id: self.installation_id.clone(),
                access: notification.access,
            })
        };
        let projection_policy = AllowNoProjectionAccess;
        let outbound_policy = OutboundPolicyService::new(
            self.services.outbound_store.as_ref(),
            &projection_policy,
            &authority,
        );
        // The lifecycle cursor gives each committed notification epoch a
        // stable projection identity. The handler ledger suppresses replay
        // within this router lifetime; durable cross-process at-most-once
        // behavior remains the outbound coordinator/store's responsibility.
        let projection_ref = ProjectionUpdateRef::new(format!(
            "{}:{projection_epoch}",
            prompts::run_notification_projection_id(state.run_id, notification.event_kind),
        ))
        .map_err(|reason| RunDeliveryError::InvalidProjectionRef { reason })?;
        let outcome = self
            .services
            .coordinator
            .deliver(
                &outbound_policy,
                self.services.communication_preferences.as_ref(),
                &authority,
                CoordinatedDeliveryRequest {
                    intent: notification.intent,
                    delivery: PrepareCommunicationDeliveryRequest {
                        resolution_request: CommunicationDeliveryResolutionRequest {
                            scope: state.scope.clone(),
                            actor: actor.clone(),
                            modality: CommunicationModality::Text,
                            intent: CommunicationDeliveryIntent::RunNotification(
                                RunNotificationContext {
                                    event_kind: notification.event_kind,
                                    origin: if delivery_target == &state.reply_target_binding_ref {
                                        RunNotificationOrigin::LiveSourceRoute {
                                            source_route: SourceRouteContext {
                                                reply_target_binding_ref: delivery_target.clone(),
                                            },
                                        }
                                    } else {
                                        RunNotificationOrigin::RunScopedTarget {
                                            target: delivery_target.clone(),
                                        }
                                    },
                                },
                            ),
                        },
                        turn_run_id: Some(state.run_id),
                        projection_ref,
                        attempted_at: Utc::now(),
                    },
                    parts: vec![notification.part],
                    thread_anchor: None,
                    require_direct_message_target: notification.require_direct_message_target,
                    extension_id: &self.services.extension_id,
                },
            )
            .await?;
        let delivered = match outcome {
            CoordinatedDeliveryOutcome::Failed { failure_kind, .. } => {
                return Err(RunDeliveryError::DeliveryFailed { failure_kind });
            }
            outcome => delivered_messages_from_outcome(&outcome),
        };
        if let Some(gate_ref) = notification.gate_ref.as_deref() {
            record_gate_route_if_needed(
                self.services.route_store.as_ref(),
                state.run_id,
                &state.scope.tenant_id,
                &actor.user_id,
                gate_ref,
                &state.scope,
                &delivered,
                source_target.map(|target| &target.external_conversation_ref),
            )
            .await;
        }
        Ok(delivered)
    }

    async fn read_final_text(
        &self,
        state: &TurnRunState,
    ) -> Result<Option<String>, RunDeliveryError> {
        let Some(agent_id) = state.scope.agent_id.clone() else {
            return Ok(None);
        };
        let thread_scope = ThreadScope {
            tenant_id: state.scope.tenant_id.clone(),
            agent_id,
            project_id: state.scope.project_id.clone(),
            owner_user_id: state.scope.explicit_owner_user_id().cloned(),
            mission_id: None,
        };
        Ok(self
            .services
            .thread_service
            .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                scope: thread_scope,
                thread_id: state.scope.thread_id.clone(),
                turn_run_id: state.run_id.to_string(),
            })
            .await?
            .and_then(|message| message.content))
    }

    async fn update_cleanup(
        &self,
        intent: DeliveryIntent,
        delivered: &[DeliveredChannelMessage],
        state: &TurnRunState,
    ) -> Result<(), RunDeliveryError> {
        if is_retractable_notification(intent) && delivered.is_empty() {
            return Ok(());
        }
        let request = self.cleanup_request(state)?;
        let previous = self
            .services
            .outbound_store
            .load_run_delivery_cleanup(request.clone())
            .await?;
        if is_retractable_notification(intent) {
            for message in delivered {
                self.services
                    .outbound_store
                    .put_run_delivery_cleanup(
                        RunDeliveryCleanupRecord::new(
                            state.scope.clone(),
                            state.run_id,
                            request.adapter.clone(),
                            message.reply_target_binding_ref.clone(),
                            message.conversation.conversation_fingerprint(),
                            message.vendor_message_ref.clone(),
                        )
                        .map_err(|reason| {
                            RunDeliveryError::InvalidProjectionRef {
                                reason: reason.to_string(),
                            }
                        })?,
                    )
                    .await?;
            }
        }
        for record in previous {
            self.retract_if_current(state, record).await?;
        }
        Ok(())
    }

    async fn retract_pending_messages(&self, state: &TurnRunState) -> Result<(), RunDeliveryError> {
        let records = self
            .services
            .outbound_store
            .load_run_delivery_cleanup(self.cleanup_request(state)?)
            .await?;
        for record in records {
            self.retract_if_current(state, record).await?;
        }
        Ok(())
    }

    fn cleanup_request(
        &self,
        state: &TurnRunState,
    ) -> Result<RunDeliveryCleanupRequest, RunDeliveryError> {
        Ok(RunDeliveryCleanupRequest {
            scope: state.scope.clone(),
            run_id: state.run_id,
            adapter: RunOriginAdapter::new(self.adapter_id.clone()).map_err(|error| {
                RunDeliveryError::InvalidProjectionRef {
                    reason: error.to_string(),
                }
            })?,
        })
    }

    async fn retract_if_current(
        &self,
        state: &TurnRunState,
        record: RunDeliveryCleanupRecord,
    ) -> Result<(), RunDeliveryError> {
        let Some(actor) = state.actor.as_ref() else {
            return Ok(());
        };
        if record.reply_target_binding_ref != state.reply_target_binding_ref {
            self.services
                .outbound_store
                .complete_run_delivery_cleanup(&record)
                .await?;
            return Ok(());
        }
        let target = match self
            .resolve_target(
                state,
                actor,
                &state.reply_target_binding_ref,
                StoredProductReplyTargetAccess::OrdinaryReply,
            )
            .await
        {
            Ok(target) => target,
            Err(
                ProductWorkflowError::BindingAccessDenied
                | ProductWorkflowError::BindingRequired { .. }
                | ProductWorkflowError::UnknownInstallation,
            ) => None,
            Err(error) => return Err(RunDeliveryError::Workflow(error)),
        };
        let Some(target) = target else {
            self.services
                .outbound_store
                .complete_run_delivery_cleanup(&record)
                .await?;
            return Ok(());
        };
        if target.external_conversation_ref.conversation_fingerprint()
            != record.conversation_fingerprint
        {
            self.services
                .outbound_store
                .complete_run_delivery_cleanup(&record)
                .await?;
            return Ok(());
        }
        let message = DeliveredChannelMessage {
            reply_target_binding_ref: record.reply_target_binding_ref.clone(),
            conversation: target.external_conversation_ref,
            vendor_message_ref: record.vendor_message_ref.clone(),
        };
        let cleanup_delivered = self
            .services
            .retract_message(state.scope.clone(), Some(state.run_id), message)
            .await?;
        if !cleanup_delivered {
            return Err(RunDeliveryError::CleanupNotDelivered);
        }
        self.services
            .outbound_store
            .complete_run_delivery_cleanup(&record)
            .await?;
        Ok(())
    }
}
