use super::*;

impl ChannelPairingService {
    pub fn new(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
        descriptor: ExtensionAccountSetupDescriptor,
        dependencies: ChannelPairingServiceDependencies,
    ) -> Self {
        Self {
            completion_owner_id: uuid::Uuid::new_v4(),
            #[cfg(any(test, feature = "test-support"))]
            completion_lease_duration: Duration::seconds(PAIRING_COMPLETION_LEASE_SECONDS),
            #[cfg(any(test, feature = "test-support"))]
            completion_renewal_interval: std::time::Duration::from_secs(
                PAIRING_COMPLETION_RENEWAL_SECONDS,
            ),
            tenant_id,
            agent_id,
            project_id,
            extension_id: descriptor.extension_id,
            connection_notices: descriptor.connection_notices,
            connection_requirement: descriptor.connection_requirement,
            deep_link_template: descriptor.pairing_deep_link_template,
            inbound_code_prefixes: descriptor.pairing_inbound_code_prefixes,
            store: dependencies.store,
            installation: dependencies.installation,
            template_values: dependencies.template_values,
            identity: dependencies.identity,
            continuation: dependencies.continuation,
            conversation_actor_pairings: dependencies.conversation_actor_pairings,
            direct_targets: dependencies.direct_targets,
        }
    }

    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    /// Test-only view of the continuation dispatcher composition wired in.
    /// Mirrors the production dispatch in `dispatch_pairing_completion_with`;
    /// lets composition tests pin (by `Arc::ptr_eq`) that pairing completions
    /// run the SAME lifecycle-wrapped dispatcher product-auth uses, so
    /// readiness reconciliation cannot be silently skipped for paired
    /// channels. Ships zero bytes in production builds.
    #[cfg(any(test, feature = "test-support"))]
    pub fn continuation_dispatcher_for_test(&self) -> Arc<dyn ProductAuthContinuationDispatcher> {
        Arc::clone(&self.continuation)
    }

    pub fn connection_notices(&self) -> &ChannelConnectionNoticePolicy {
        &self.connection_notices
    }

    pub fn connection_requirement(&self) -> &ChannelConnectionRequirement {
        &self.connection_requirement
    }

    fn completion_lease_duration(&self) -> Duration {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.completion_lease_duration
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            Duration::seconds(PAIRING_COMPLETION_LEASE_SECONDS)
        }
    }

    fn completion_renewal_interval(&self) -> std::time::Duration {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.completion_renewal_interval
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            std::time::Duration::from_secs(PAIRING_COMPLETION_RENEWAL_SECONDS)
        }
    }

    async fn resolve_deep_link(
        &self,
        code: &ChannelPairingCode,
    ) -> Result<Option<String>, ChannelPairingError> {
        let Some(template) = &self.deep_link_template else {
            return Ok(None);
        };
        let mut link = template.clone();
        link = link.replace("{code}", code.as_str());
        for handle in template_handles(template) {
            let Some(value) = self
                .template_values
                .template_value(&handle)
                .await
                .map_err(store_unavailable)?
            else {
                return Ok(None);
            };
            link = link.replace(&format!("{{{handle}}}"), &value);
        }
        // A template placeholder without a configured value means setup is
        // incomplete — presenting a broken link would strand the user, so the
        // issue falls back to code-only presentation.
        if link.contains('{') {
            return Ok(None);
        }
        Ok(Some(link))
    }

    async fn issue_for_record(
        &self,
        record: &ChannelPairingRecord,
    ) -> Result<ChannelPairingIssue, ChannelPairingError> {
        Ok(ChannelPairingIssue {
            code: record.code.clone(),
            deep_link: self.resolve_deep_link(&record.code).await?,
            expires_at: record.expires_at,
        })
    }

    /// Mint (or rotate) the caller's pairing code. Fails closed when the
    /// channel is not installed for the caller — no code is ever minted first.
    pub async fn issue_or_rotate(
        &self,
        caller: &UserId,
    ) -> Result<ChannelPairingIssue, ChannelPairingError> {
        self.finish_pending_unpairs_for_user(caller).await?;
        let installation_id = self
            .installation
            .current_installation(caller)
            .await
            .map_err(store_unavailable)?
            .ok_or(ChannelPairingError::NotConfigured)?;
        let now = Utc::now();
        let record = ChannelPairingRecord {
            code: mint_pairing_code()?,
            user_id: caller.clone(),
            installation_id,
            created_at: now,
            expires_at: now + Duration::minutes(PAIRING_TTL_MINUTES),
            consumed_at: None,
        };
        let stored = record.clone();
        self.store
            .update_snapshot(move |mut snapshot| {
                // Rotation: at most one live code per user.
                snapshot.pairings.retain(|existing| {
                    existing.user_id != stored.user_id || existing.consumed_at.is_some()
                });
                snapshot.pairings.push(stored.clone());
                (snapshot, ())
            })
            .await?;
        self.issue_for_record(&record).await
    }

    pub async fn status_for(
        &self,
        caller: &UserId,
    ) -> Result<ChannelPairingStatus, ChannelPairingError> {
        let installation_id = self
            .installation
            .current_installation(caller)
            .await
            .map_err(store_unavailable)?;
        let connected = match &installation_id {
            Some(_installation_id) => self
                .direct_targets
                .is_connected(&self.extension_id, caller)
                .await
                .map_err(store_unavailable)?,
            None => false,
        };
        let pending = match (&installation_id, connected) {
            (Some(installation_id), false) => {
                let snapshot = self.store.read_snapshot().await?;
                let record = snapshot
                    .pairings
                    .iter()
                    .find(|record| {
                        &record.user_id == caller
                            && record.is_live(Utc::now())
                            && &record.installation_id == installation_id
                    })
                    .cloned();
                match record {
                    Some(record) => Some(self.issue_for_record(&record).await?),
                    None => None,
                }
            }
            _ => None,
        };
        Ok(ChannelPairingStatus { connected, pending })
    }

    /// Materialize the current pairing challenge without rotating a still-live
    /// code. Projection and channel-delivery replays therefore observe the
    /// same durable challenge as the WebUI pairing panel.
    pub async fn pending_or_issue(
        &self,
        caller: &UserId,
    ) -> Result<Option<ChannelPairingIssue>, ChannelPairingError> {
        let status = self.status_for(caller).await?;
        if status.connected {
            return Ok(None);
        }
        match status.pending {
            Some(issue) => Ok(Some(issue)),
            None => self.issue_or_rotate(caller).await.map(Some),
        }
    }

    /// Consume a code arriving over the verified webhook from a direct
    /// conversation.
    ///
    /// Ordering is claim-first: the code is atomically consumed (single
    /// winner) BEFORE any identity/target side effect, so two concurrent
    /// consumers of one code can never both bind. Completion (peer target +
    /// continuation dispatch) is idempotently repairable: a sender already
    /// bound to the code's user re-runs the completion effects — including on
    /// an already-consumed code — so a consume that failed after the claim is
    /// recovered by re-sending a code instead of stranding the blocked run.
    pub async fn consume(
        &self,
        authenticated_installation_id: &AdapterInstallationId,
        raw_code: &str,
        actor_kind: &str,
        external_actor_id: &str,
        conversation_space_id: Option<&str>,
        conversation_id: &str,
    ) -> Result<ChannelPairingConsumeOutcome, ChannelPairingError> {
        let Ok(code) = ChannelPairingCode::new(raw_code) else {
            return Ok(ChannelPairingConsumeOutcome::ExpiredOrUnknown);
        };
        let snapshot = self.store.read_snapshot().await?;
        let Some(record) = snapshot
            .pairings
            .iter()
            .find(|record| record.code == code)
            .cloned()
        else {
            return Ok(ChannelPairingConsumeOutcome::ExpiredOrUnknown);
        };
        if &record.installation_id != authenticated_installation_id {
            return Ok(ChannelPairingConsumeOutcome::ExpiredOrUnknown);
        }
        self.finish_pending_unpairs_for_user(&record.user_id)
            .await?;
        match self
            .bound_user_for(&record.installation_id, external_actor_id)
            .await?
        {
            Some(existing) if existing == record.user_id => {
                // Repair path: burn the code if it is still live (whoever
                // wins — the sender is already bound), then re-run completion.
                let _already_burned = self.claim(&code).await?;
                self.complete_pairing(
                    &record,
                    actor_kind,
                    external_actor_id,
                    conversation_space_id,
                    conversation_id,
                )
                .await?;
                return Ok(ChannelPairingConsumeOutcome::AlreadyPairedSameUser {
                    user_id: existing,
                });
            }
            Some(_other) => {
                if !record.is_live(Utc::now()) {
                    return Ok(ChannelPairingConsumeOutcome::ExpiredOrUnknown);
                }
                // Refusal keeps the live code intact for its owner.
                return Ok(ChannelPairingConsumeOutcome::AlreadyBoundToOtherUser);
            }
            None => {}
        }
        // Single-consumer claim BEFORE identity/target writes: exactly one
        // concurrent consumer of a live code proceeds past this point.
        let Some(record) = self.claim(&code).await? else {
            return Ok(ChannelPairingConsumeOutcome::ExpiredOrUnknown);
        };
        match self
            .identity
            .bind_user(
                &self.extension_id,
                &record.installation_id,
                external_actor_id,
                record.user_id.clone(),
            )
            .await
            .map_err(store_unavailable)?
        {
            ChannelPairingIdentityBindOutcome::Bound => {}
            ChannelPairingIdentityBindOutcome::AlreadyBoundToOtherUser => {
                return Ok(ChannelPairingConsumeOutcome::AlreadyBoundToOtherUser);
            }
        }
        self.complete_pairing(
            &record,
            actor_kind,
            external_actor_id,
            conversation_space_id,
            conversation_id,
        )
        .await?;
        Ok(ChannelPairingConsumeOutcome::Paired {
            user_id: record.user_id,
        })
    }

    /// Commit the idempotent completion intent shared by first-time pairing
    /// and the repair path, then publish the DM target. The product-owned
    /// interceptor dispatches and settles this durable intent before the
    /// provider ingress acknowledgement is returned.
    async fn complete_pairing(
        &self,
        record: &ChannelPairingRecord,
        actor_kind: &str,
        external_actor_id: &str,
        conversation_space_id: Option<&str>,
        conversation_id: &str,
    ) -> Result<(), ChannelPairingError> {
        let binding_epoch =
            ExternalActorBindingEpoch::new(format!("pairing-{}", uuid::Uuid::new_v4()))
                .map_err(store_unavailable)?;
        let candidate = PendingPairingCompletion {
            dispatch_id: AuthFlowId::new(),
            installation_id: record.installation_id.clone(),
            user_id: record.user_id.clone(),
            conversation_space_id: conversation_space_id.map(str::to_string),
            conversation_id: conversation_id.to_string(),
            actor_kind: actor_kind.to_string(),
            external_actor_id: external_actor_id.to_string(),
            emitted_at: Utc::now(),
            binding_epoch: Some(binding_epoch.clone()),
            lease: None,
        };
        let provider_user_id = self
            .identity
            .binding_key(&record.installation_id, external_actor_id);
        let paired_actor = PairedActorRecord {
            provider_user_id,
            installation_id: record.installation_id.clone(),
            actor_kind: actor_kind.to_string(),
            external_actor_id: external_actor_id.to_string(),
            user_id: record.user_id.clone(),
            binding_epoch: Some(binding_epoch),
        };
        let completion = self
            .store
            .update_snapshot(move |mut snapshot| {
                let completion_exists = snapshot.completions.iter().any(|existing| {
                    existing.installation_id == candidate.installation_id
                        && existing.user_id == candidate.user_id
                });
                let paired_actor_exists = snapshot
                    .paired_actors
                    .iter()
                    .any(|existing| existing.provider_user_id == paired_actor.provider_user_id);
                if (!completion_exists && snapshot.completions.len() >= PAIRING_SNAPSHOT_CAP)
                    || (!paired_actor_exists
                        && snapshot.paired_actors.len() >= PAIRING_SNAPSHOT_CAP)
                {
                    // Completion and actor records are live work/authority:
                    // reject admission rather than evicting either silently.
                    return (snapshot, None);
                }
                let completion = match snapshot.completions.iter_mut().find(|existing| {
                    existing.installation_id == candidate.installation_id
                        && existing.user_id == candidate.user_id
                }) {
                    Some(existing) => {
                        existing.conversation_space_id = candidate.conversation_space_id.clone();
                        existing.conversation_id = candidate.conversation_id.clone();
                        existing.actor_kind = candidate.actor_kind.clone();
                        existing.external_actor_id = candidate.external_actor_id.clone();
                        existing.binding_epoch = candidate.binding_epoch.clone();
                        existing.clone()
                    }
                    None => {
                        snapshot.completions.push(candidate.clone());
                        candidate.clone()
                    }
                };
                snapshot
                    .paired_actors
                    .retain(|existing| existing.provider_user_id != paired_actor.provider_user_id);
                snapshot.paired_actors.push(paired_actor.clone());
                (snapshot, Some(completion))
            })
            .await?
            .ok_or_else(|| store_unavailable("channel pairing live-record limit exceeded"))?;
        self.persist_completion_bindings(&completion).await
    }

    pub(super) async fn finish_pending_for_user(
        &self,
        user_id: &UserId,
    ) -> Result<(), ChannelPairingError> {
        self.reconcile_pending_with(
            Some(user_id),
            self.tenant_id.clone(),
            Arc::clone(&self.continuation),
        )
        .await
    }

    /// Reconcile every durable pairing-completion intent for this extension.
    /// Production composition calls this after all generic services have been
    /// registered, so an intent committed before process loss does not depend
    /// on another provider delivery to make progress.
    pub async fn reconcile_pending_completions(&self) -> Result<(), ChannelPairingError> {
        self.reconcile_pending_with(None, self.tenant_id.clone(), Arc::clone(&self.continuation))
            .await
    }

    async fn reconcile_pending_with(
        &self,
        user_id: Option<&UserId>,
        tenant_id: TenantId,
        continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    ) -> Result<(), ChannelPairingError> {
        loop {
            let Some(mut claim) = self.claim_pending_completion(user_id).await? else {
                return Ok(());
            };
            if let Err(error) = self
                .dispatch_claimed_completion(
                    &mut claim,
                    tenant_id.clone(),
                    Arc::clone(&continuation),
                )
                .await
            {
                if let Err(release_error) = self.release_completion_claim(&claim).await {
                    return Err(store_unavailable(format!(
                        "{error}; exact completion-claim release also failed: {release_error}"
                    )));
                }
                return Err(error);
            }
            self.settle_completion_claim(&claim).await?;
        }
    }

    async fn claim_pending_completion(
        &self,
        user_id: Option<&UserId>,
    ) -> Result<Option<ClaimedPairingCompletion>, ChannelPairingError> {
        let user_id = user_id.cloned();
        let owner_id = self.completion_owner_id;
        let claim_id = uuid::Uuid::new_v4();
        let lease_duration = self.completion_lease_duration();
        self.store
            .update_snapshot(move |mut snapshot| {
                let now = Utc::now();
                let lease = PairingCompletionLease {
                    owner_id,
                    claim_id,
                    expires_at: now + lease_duration,
                };
                let claimed = snapshot
                    .completions
                    .iter_mut()
                    .find(|completion| {
                        user_id
                            .as_ref()
                            .is_none_or(|expected| &completion.user_id == expected)
                            && completion
                                .lease
                                .as_ref()
                                .is_none_or(|existing| existing.expires_at <= now)
                    })
                    .map(|completion| {
                        completion.lease = Some(lease.clone());
                        ClaimedPairingCompletion {
                            completion: completion.clone(),
                            lease: lease.clone(),
                        }
                    });
                (snapshot, claimed)
            })
            .await
    }

    async fn dispatch_claimed_completion(
        &self,
        claim: &mut ClaimedPairingCompletion,
        tenant_id: TenantId,
        continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    ) -> Result<(), ChannelPairingError> {
        let completion = claim.completion.clone();
        let dispatch = async {
            // The outbox commit precedes these idempotent side effects.
            // Replaying bindings first closes the crash window between the CAS
            // commit and the original synchronous writes.
            self.persist_completion_bindings(&completion).await?;
            // Boxed: the continuation fan-out resumes parked runs through the
            // turn coordinator — a deep async subtree relative to this caller.
            Box::pin(self.dispatch_pairing_completion_with(&completion, tenant_id, continuation))
                .await
        };
        tokio::pin!(dispatch);
        let mut renewals = tokio::time::interval_at(
            tokio::time::Instant::now() + self.completion_renewal_interval(),
            self.completion_renewal_interval(),
        );
        renewals.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                result = &mut dispatch => return result,
                _ = renewals.tick() => self.renew_completion_claim(claim).await?,
            }
        }
    }

    async fn renew_completion_claim(
        &self,
        claim: &mut ClaimedPairingCompletion,
    ) -> Result<(), ChannelPairingError> {
        let dispatch_id = claim.completion.dispatch_id;
        let installation_id = claim.completion.installation_id.clone();
        let user_id = claim.completion.user_id.clone();
        let owner_id = claim.lease.owner_id;
        let claim_id = claim.lease.claim_id;
        let lease_duration = self.completion_lease_duration();
        let renewed = self
            .store
            .update_snapshot(move |mut snapshot| {
                let renewed = snapshot.completions.iter_mut().find_map(|completion| {
                    (completion.dispatch_id == dispatch_id
                        && completion.installation_id == installation_id
                        && completion.user_id == user_id
                        && completion.lease.as_ref().is_some_and(|lease| {
                            lease.owner_id == owner_id && lease.claim_id == claim_id
                        }))
                    .then(|| {
                        let renewed = PairingCompletionLease {
                            owner_id,
                            claim_id,
                            expires_at: Utc::now() + lease_duration,
                        };
                        completion.lease = Some(renewed.clone());
                        renewed
                    })
                });
                (snapshot, renewed)
            })
            .await?;
        let Some(renewed) = renewed else {
            return Err(ChannelPairingError::ContinuationDispatch {
                reason: "pairing completion outbox lease ownership was lost".to_string(),
            });
        };
        claim.lease = renewed.clone();
        claim.completion.lease = Some(renewed);
        Ok(())
    }

    async fn release_completion_claim(
        &self,
        claim: &ClaimedPairingCompletion,
    ) -> Result<(), ChannelPairingError> {
        let dispatch_id = claim.completion.dispatch_id;
        let installation_id = claim.completion.installation_id.clone();
        let user_id = claim.completion.user_id.clone();
        let lease = claim.lease.clone();
        self.store
            .update_snapshot(move |mut snapshot| {
                if let Some(completion) = snapshot.completions.iter_mut().find(|existing| {
                    existing.dispatch_id == dispatch_id
                        && existing.installation_id == installation_id
                        && existing.user_id == user_id
                        && existing.lease.as_ref() == Some(&lease)
                }) {
                    completion.lease = None;
                }
                (snapshot, ())
            })
            .await
    }

    async fn settle_completion_claim(
        &self,
        claim: &ClaimedPairingCompletion,
    ) -> Result<(), ChannelPairingError> {
        let completion = claim.completion.clone();
        let lease = claim.lease.clone();
        self.store
            .update_snapshot(move |mut snapshot| {
                if let Some(index) = snapshot
                    .completions
                    .iter()
                    .position(|existing| existing == &completion)
                {
                    snapshot.completions.remove(index);
                } else if let Some(advanced) = snapshot.completions.iter_mut().find(|existing| {
                    existing.dispatch_id == completion.dispatch_id
                        && existing.installation_id == completion.installation_id
                        && existing.user_id == completion.user_id
                        && existing.lease.as_ref() == Some(&lease)
                }) {
                    // A concurrent repair advanced the payload while this
                    // exact attempt was dispatching. Keep that newer intent,
                    // but release this claim so the loop can dispatch it next.
                    advanced.lease = None;
                }
                (snapshot, ())
            })
            .await
    }

    async fn persist_dm_target(
        &self,
        completion: &PendingPairingCompletion,
    ) -> Result<(), ChannelPairingError> {
        self.direct_targets
            .upsert(
                &self.extension_id,
                &completion.user_id,
                &completion.external_actor_id,
                completion.conversation_space_id.as_deref(),
                &completion.conversation_id,
            )
            .await
            .map_err(store_unavailable)
    }

    async fn persist_completion_bindings(
        &self,
        completion: &PendingPairingCompletion,
    ) -> Result<(), ChannelPairingError> {
        let adapter_kind =
            AdapterKind::new(self.extension_id.as_str()).map_err(store_unavailable)?;
        let installation_id =
            ironclaw_conversations::AdapterInstallationId::new(completion.installation_id.as_str())
                .map_err(store_unavailable)?;
        let actor_ref =
            ExternalActorRef::new(&completion.actor_kind, &completion.external_actor_id)
                .map_err(store_unavailable)?;
        match completion.binding_epoch.clone() {
            Some(binding_epoch) => {
                self.conversation_actor_pairings
                    .pair_external_actor_with_epoch(
                        self.tenant_id.clone(),
                        adapter_kind,
                        installation_id,
                        actor_ref,
                        completion.user_id.clone(),
                        binding_epoch,
                    )
                    .await
                    .map_err(store_unavailable)?;
            }
            None => {
                // Rollback-compatible replay for snapshots written before
                // pairing generations were persisted.
                self.conversation_actor_pairings
                    .pair_external_actor(
                        self.tenant_id.clone(),
                        adapter_kind,
                        installation_id,
                        actor_ref,
                        completion.user_id.clone(),
                    )
                    .await
                    .map_err(store_unavailable)?;
            }
        }
        self.persist_dm_target(completion).await
    }

    /// Unpair the caller: peer targets and conversation actors are removed,
    /// pending codes are invalidated, and identity bindings are deleted last.
    /// Only this user is affected; history is retained.
    ///
    /// Deliberately independent of the current installation: an admin
    /// clearing the deployment must not orphan a user's durable bindings —
    /// those would silently resurrect the connection when the same channel is
    /// reconfigured even though the user disconnected.
    pub async fn unpair(&self, caller: &UserId) -> Result<(), ChannelPairingError> {
        self.direct_targets
            .delete(&self.extension_id, caller)
            .await
            .map_err(store_unavailable)?;
        self.stage_unpair_transaction(caller).await?;
        self.finish_pending_unpairs_for_user(caller).await
    }

    async fn stage_unpair_transaction(&self, caller: &UserId) -> Result<(), ChannelPairingError> {
        let caller_owned = caller.clone();
        let transaction_id = uuid::Uuid::new_v4();
        let staged = self
            .store
            .update_snapshot(move |mut snapshot| {
                let candidates: Vec<_> = snapshot
                    .paired_actors
                    .iter()
                    .filter(|actor| actor.user_id == caller_owned)
                    .cloned()
                    .collect();
                let legacy: Vec<_> = snapshot
                    .pending_unpairs
                    .iter()
                    .filter(|actor| actor.user_id == caller_owned)
                    .cloned()
                    .collect();
                let existing_actors = snapshot
                    .pending_unpair_transactions
                    .iter()
                    .find(|transaction| transaction.user_id == caller_owned)
                    .map(|transaction| transaction.actors.as_slice())
                    .unwrap_or_default();
                let transaction_exists = snapshot
                    .pending_unpair_transactions
                    .iter()
                    .any(|transaction| transaction.user_id == caller_owned);
                let new_pending = candidates
                    .iter()
                    .chain(legacy.iter())
                    .filter(|actor| !existing_actors.iter().any(|pending| pending == *actor))
                    .count();
                let pending_actor_count = snapshot
                    .pending_unpair_transactions
                    .iter()
                    .map(|transaction| transaction.actors.len())
                    .sum::<usize>();
                if pending_actor_count.saturating_add(new_pending) > PAIRING_SNAPSHOT_CAP
                    || (!transaction_exists
                        && snapshot.pending_unpair_transactions.len() >= PAIRING_SNAPSHOT_CAP)
                {
                    return (snapshot, false);
                }
                snapshot.pairings.retain(|record| {
                    record.user_id != caller_owned || record.consumed_at.is_some()
                });
                snapshot
                    .completions
                    .retain(|completion| completion.user_id != caller_owned);
                snapshot
                    .paired_actors
                    .retain(|actor| actor.user_id != caller_owned);
                snapshot
                    .pending_unpairs
                    .retain(|actor| actor.user_id != caller_owned);
                match snapshot
                    .pending_unpair_transactions
                    .iter_mut()
                    .find(|transaction| transaction.user_id == caller_owned)
                {
                    Some(transaction) => {
                        for actor in candidates.into_iter().chain(legacy) {
                            if !transaction.actors.iter().any(|pending| pending == &actor) {
                                transaction.actors.push(actor);
                            }
                        }
                    }
                    None => {
                        let mut actors = candidates;
                        for actor in legacy {
                            if !actors.iter().any(|pending| pending == &actor) {
                                actors.push(actor);
                            }
                        }
                        snapshot
                            .pending_unpair_transactions
                            .push(PendingUnpairTransaction {
                                transaction_id,
                                user_id: caller_owned.clone(),
                                actors,
                                lease: None,
                            });
                    }
                }
                (snapshot, true)
            })
            .await?;
        if !staged {
            return Err(store_unavailable(
                "pending channel unpair cleanup limit exceeded",
            ));
        }
        Ok(())
    }

    async fn finish_pending_unpairs_for_user(
        &self,
        caller: &UserId,
    ) -> Result<(), ChannelPairingError> {
        self.migrate_legacy_pending_unpair(caller).await?;
        let snapshot = self.store.read_snapshot().await?;
        let Some(transaction) = snapshot
            .pending_unpair_transactions
            .iter()
            .find(|transaction| &transaction.user_id == caller)
            .cloned()
        else {
            return Ok(());
        };
        let adapter_kind =
            AdapterKind::new(self.extension_id.as_str()).map_err(store_unavailable)?;
        for actor in &transaction.actors {
            let actor_ref = ExternalActorRef::new(&actor.actor_kind, &actor.external_actor_id)
                .map_err(store_unavailable)?;
            let installation_id =
                ironclaw_conversations::AdapterInstallationId::new(actor.installation_id.as_str())
                    .map_err(store_unavailable)?;
            self.conversation_actor_pairings
                .unpair_external_actor_if_owned_by(
                    &self.tenant_id,
                    &adapter_kind,
                    &installation_id,
                    &actor_ref,
                    &ExpectedExternalActorOwner {
                        user_id: caller.clone(),
                        binding_epoch: actor.binding_epoch.clone(),
                    },
                )
                .await
                .map_err(store_unavailable)?;
        }
        let lease = match self
            .claim_pending_unpair(&transaction.transaction_id, caller)
            .await?
        {
            PendingUnpairClaim::Claimed(lease) => lease,
            PendingUnpairClaim::Busy => {
                return Err(store_unavailable(
                    "channel unpair identity cleanup is already in progress",
                ));
            }
            PendingUnpairClaim::Settled => return Ok(()),
        };
        if let Err(error) = self
            .identity
            .delete_user_bindings(&self.extension_id, caller)
            .await
            .map_err(store_unavailable)
        {
            self.release_pending_unpair_claim(&transaction.transaction_id, &lease)
                .await?;
            return Err(error);
        }
        if !self
            .settle_pending_unpair(&transaction.transaction_id, &lease)
            .await?
        {
            return Err(store_unavailable(
                "channel unpair identity cleanup claim was superseded",
            ));
        }
        Ok(())
    }

    async fn migrate_legacy_pending_unpair(
        &self,
        caller: &UserId,
    ) -> Result<(), ChannelPairingError> {
        let caller = caller.clone();
        let transaction_id = uuid::Uuid::new_v4();
        self.store
            .update_snapshot(move |mut snapshot| {
                if snapshot
                    .pending_unpair_transactions
                    .iter()
                    .any(|transaction| transaction.user_id == caller)
                {
                    return (snapshot, ());
                }
                let actors: Vec<_> = snapshot
                    .pending_unpairs
                    .iter()
                    .filter(|actor| actor.user_id == caller)
                    .cloned()
                    .collect();
                if actors.is_empty() {
                    return (snapshot, ());
                }
                snapshot
                    .pending_unpairs
                    .retain(|actor| actor.user_id != caller);
                snapshot
                    .pending_unpair_transactions
                    .push(PendingUnpairTransaction {
                        transaction_id,
                        user_id: caller.clone(),
                        actors,
                        lease: None,
                    });
                (snapshot, ())
            })
            .await
    }

    async fn claim_pending_unpair(
        &self,
        transaction_id: &uuid::Uuid,
        caller: &UserId,
    ) -> Result<PendingUnpairClaim, ChannelPairingError> {
        let transaction_id = *transaction_id;
        let caller = caller.clone();
        let owner_id = self.completion_owner_id;
        let lease_duration = self.completion_lease_duration();
        self.store
            .update_snapshot(move |mut snapshot| {
                let now = Utc::now();
                let Some(transaction) =
                    snapshot
                        .pending_unpair_transactions
                        .iter_mut()
                        .find(|transaction| {
                            transaction.transaction_id == transaction_id
                                && transaction.user_id == caller
                        })
                else {
                    return (snapshot, PendingUnpairClaim::Settled);
                };
                if transaction
                    .lease
                    .as_ref()
                    .is_some_and(|lease| lease.expires_at > now)
                {
                    return (snapshot, PendingUnpairClaim::Busy);
                }
                let lease = PairingCompletionLease {
                    owner_id,
                    claim_id: uuid::Uuid::new_v4(),
                    expires_at: now + lease_duration,
                };
                transaction.lease = Some(lease.clone());
                (snapshot, PendingUnpairClaim::Claimed(lease))
            })
            .await
    }

    async fn release_pending_unpair_claim(
        &self,
        transaction_id: &uuid::Uuid,
        lease: &PairingCompletionLease,
    ) -> Result<(), ChannelPairingError> {
        let transaction_id = *transaction_id;
        let lease = lease.clone();
        self.store
            .update_snapshot(move |mut snapshot| {
                if let Some(transaction) =
                    snapshot
                        .pending_unpair_transactions
                        .iter_mut()
                        .find(|transaction| {
                            transaction.transaction_id == transaction_id
                                && transaction.lease.as_ref() == Some(&lease)
                        })
                {
                    transaction.lease = None;
                }
                (snapshot, ())
            })
            .await
    }

    async fn settle_pending_unpair(
        &self,
        transaction_id: &uuid::Uuid,
        lease: &PairingCompletionLease,
    ) -> Result<bool, ChannelPairingError> {
        let transaction_id = *transaction_id;
        let lease = lease.clone();
        self.store
            .update_snapshot(move |mut snapshot| {
                let Some(index) =
                    snapshot
                        .pending_unpair_transactions
                        .iter()
                        .position(|transaction| {
                            transaction.transaction_id == transaction_id
                                && transaction.lease.as_ref() == Some(&lease)
                        })
                else {
                    return (snapshot, false);
                };
                snapshot.pending_unpair_transactions.remove(index);
                (snapshot, true)
            })
            .await
    }

    /// Atomically consume the code (single winner): the CAS snapshot update
    /// marks it consumed and returns the pre-claim record exactly once.
    async fn claim(
        &self,
        code: &ChannelPairingCode,
    ) -> Result<Option<ChannelPairingRecord>, ChannelPairingError> {
        let code = code.clone();
        self.store
            .update_snapshot(move |mut snapshot| {
                let now = Utc::now();
                let mut claimed = None;
                for record in snapshot.pairings.iter_mut() {
                    if record.code == code && record.is_live(now) {
                        let pre_claim = record.clone();
                        record.consumed_at = Some(now);
                        claimed = Some(pre_claim);
                        break;
                    }
                }
                (snapshot, claimed)
            })
            .await
    }

    async fn bound_user_for(
        &self,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
    ) -> Result<Option<UserId>, ChannelPairingError> {
        self.identity
            .resolve_user(&self.extension_id, installation_id, external_actor_id)
            .await
            .map_err(store_unavailable)
    }

    /// Emit the standard lifecycle continuation. Pairing is the final
    /// manifest-declared setup step, so activation is completed server-side
    /// before blocked runs resume; no browser or model-issued second action is
    /// part of the product state machine.
    async fn dispatch_pairing_completion_with(
        &self,
        completion: &PendingPairingCompletion,
        tenant_id: TenantId,
        continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    ) -> Result<(), ChannelPairingError> {
        let provider = AuthProviderId::new(self.extension_id.as_str()).map_err(|error| {
            ChannelPairingError::ContinuationDispatch {
                reason: error.to_string(),
            }
        })?;
        let event = AuthContinuationEvent {
            flow_id: completion.dispatch_id,
            scope: AuthProductScope::new(
                ResourceScope {
                    tenant_id,
                    user_id: completion.user_id.clone(),
                    agent_id: Some(self.agent_id.clone()),
                    project_id: self.project_id.clone(),
                    mission_id: None,
                    thread_id: None,
                    invocation_id: InvocationId::new(),
                },
                AuthSurface::Callback,
            ),
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: ironclaw_auth::LifecyclePackageRef::new(self.extension_id.as_str())
                    .map_err(|error| ChannelPairingError::ContinuationDispatch {
                        reason: error.to_string(),
                    })?,
            },
            provider,
            credential_account_id: None,
            emitted_at: completion.emitted_at,
        };
        continuation
            .dispatch_auth_continuation(event)
            .await
            .map_err(|error| ChannelPairingError::ContinuationDispatch {
                reason: error.to_string(),
            })
    }

    /// Re-dispatch pairing completion through the caller's real turn world.
    /// Integration groups execute runs in a shared turn store created after
    /// this composed service, unlike production where both use one store.
    /// Test-only: zero production bytes.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn finish_pending_for_user_with_for_test(
        &self,
        user_id: &UserId,
        tenant_id: TenantId,
        continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    ) -> Result<(), ChannelPairingError> {
        self.reconcile_pending_with(Some(user_id), tenant_id, continuation)
            .await
    }

    /// Settle pending completions through the service's configured dispatcher.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn finish_pending_for_user_for_test(
        &self,
        user_id: &UserId,
    ) -> Result<(), ChannelPairingError> {
        self.finish_pending_for_user(user_id).await
    }

    /// Inspect durable completion identities without exposing the private
    /// persistence snapshot shape to composition tests.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn pending_completion_dispatch_ids_for_test(
        &self,
    ) -> Result<Vec<AuthFlowId>, ChannelPairingError> {
        Ok(self
            .store
            .read_snapshot()
            .await?
            .completions
            .into_iter()
            .map(|completion| completion.dispatch_id)
            .collect())
    }

    /// Count completion intents currently protected by an unexpired outbox
    /// claim. Test-only visibility proves ownership, not resumed-run state.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn live_completion_lease_count_for_test(&self) -> Result<usize, ChannelPairingError> {
        let now = Utc::now();
        Ok(self
            .store
            .read_snapshot()
            .await?
            .completions
            .into_iter()
            .filter(|completion| {
                completion
                    .lease
                    .as_ref()
                    .is_some_and(|lease| lease.expires_at > now)
            })
            .count())
    }

    /// Shorten lease timing so contract tests can cover renewal past the
    /// original expiry without a production-length sleep.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_completion_lease_timing_for_test(
        &mut self,
        lease_duration: std::time::Duration,
        renewal_interval: std::time::Duration,
    ) -> Result<(), ChannelPairingError> {
        let lease_duration = Duration::from_std(lease_duration).map_err(store_unavailable)?;
        if renewal_interval.is_zero()
            || lease_duration <= Duration::zero()
            || renewal_interval >= lease_duration.to_std().map_err(store_unavailable)?
        {
            return Err(store_unavailable(
                "completion renewal interval must be positive and shorter than the lease",
            ));
        }
        self.completion_lease_duration = lease_duration;
        self.completion_renewal_interval = renewal_interval;
        Ok(())
    }

    /// Replace the continuation dispatcher in composition-level fault tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn replace_continuation_for_test(
        &mut self,
        continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    ) {
        self.continuation = continuation;
    }
}
