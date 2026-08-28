//! Private filesystem persistence and ordered read implementation.

use super::*;
use ironclaw_host_api::ids::TenantId;

trait TenantScoped {
    fn tenant_id(&self) -> &TenantId;
}

impl TenantScoped for HourlyRunFailure {
    fn tenant_id(&self) -> &TenantId {
        HourlyRunFailure::tenant_id(self)
    }
}

impl TenantScoped for HourlyAutomationUsage {
    fn tenant_id(&self) -> &TenantId {
        HourlyAutomationUsage::tenant_id(self)
    }
}

impl TenantScoped for CollectorCoverage {
    fn tenant_id(&self) -> &TenantId {
        CollectorCoverage::tenant_id(self)
    }
}

#[derive(Debug, Clone, Copy)]
enum QueryShape {
    Activity,
    Model,
    Failure,
    Automation,
    Lifecycle,
    Coverage,
}

impl QueryShape {
    fn family(self) -> &'static str {
        match self {
            Self::Activity => FAMILY_ACTIVITY,
            Self::Model => FAMILY_MODEL,
            Self::Failure => FAMILY_FAILURE,
            Self::Automation => FAMILY_AUTOMATION,
            Self::Lifecycle => FAMILY_LIFECYCLE,
            Self::Coverage => FAMILY_COVERAGE,
        }
    }

    fn time_key(self) -> &'static str {
        match self {
            Self::Lifecycle => "occurred_at",
            Self::Activity | Self::Model | Self::Failure | Self::Automation | Self::Coverage => {
                "window_start"
            }
        }
    }
}

impl<F> FilesystemTelemetryRepository<F>
where
    F: ironclaw_filesystem::RootFilesystem + ?Sized,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    pub async fn ensure_indexes(
        &self,
        scope: &ResourceScope,
    ) -> Result<(), TelemetryRepositoryError> {
        let prefix = scoped_path(TELEMETRY_PREFIX.to_owned())?;
        for (name, keys) in [
            (
                INDEX_FAMILY_TIME,
                &["tenant_id", "record_family", "window_start", "tie_breaker"][..],
            ),
            (
                INDEX_PROVIDER_TIME,
                &[
                    "tenant_id",
                    "record_family",
                    "provider_id",
                    "window_start",
                    "tie_breaker",
                ][..],
            ),
            (
                INDEX_MODEL_TIME,
                &[
                    "tenant_id",
                    "record_family",
                    "effective_model_id",
                    "window_start",
                    "tie_breaker",
                ][..],
            ),
            (
                INDEX_PROVIDER_MODEL_TIME,
                &[
                    "tenant_id",
                    "record_family",
                    "provider_id",
                    "effective_model_id",
                    "window_start",
                    "tie_breaker",
                ][..],
            ),
        ] {
            let spec = exact_index(name, keys)?;
            self.filesystem
                .ensure_index(scope, &prefix, &spec)
                .await
                .map_err(|source| TelemetryRepositoryError::StorageOperation {
                    operation: "declaring telemetry ordered index",
                    source: Box::new(source),
                })?;
        }
        let lifecycle = exact_index(
            INDEX_LIFECYCLE_TIME,
            &["tenant_id", "record_family", "occurred_at", "tie_breaker"],
        )?;
        self.filesystem
            .ensure_index(scope, &prefix, &lifecycle)
            .await
            .map_err(|source| TelemetryRepositoryError::StorageOperation {
                operation: "declaring telemetry lifecycle index",
                source: Box::new(source),
            })
    }

    pub async fn apply_batch(
        &self,
        batch: ScopedTelemetryBatch,
    ) -> Result<BatchApplyReport, TelemetryRepositoryError> {
        self.apply_scoped_batch(batch).await
    }

    pub(super) async fn apply_scoped_batch(
        &self,
        batch: ScopedTelemetryBatch,
    ) -> Result<BatchApplyReport, TelemetryRepositoryError> {
        batch.validate()?;
        let (scope, batch) = batch.into_parts();
        self.ensure_indexes(&scope).await?;
        let mut applied = 0;
        for row in batch.activity() {
            let path = path_for(
                FAMILY_ACTIVITY,
                row.window_start(),
                &[row.user_id().as_str(), origin_text(row.origin_kind())],
            )?;
            let indexed = projection(
                FAMILY_ACTIVITY,
                row.tenant_id().as_str(),
                row.window_start(),
                path.as_str(),
                None,
                None,
            )?;
            additive_update(
                &self.filesystem,
                &scope,
                &path,
                "telemetry_hourly_activity_v0",
                indexed,
                row.clone(),
                add_activity,
                activity_wire,
            )
            .await?;
            applied += 1;
        }
        for row in batch.model_usage() {
            let path = path_for(
                FAMILY_MODEL,
                row.window_start(),
                &[
                    row.user_id().as_str(),
                    row.provider_id().as_str(),
                    row.effective_model_id().as_str(),
                ],
            )?;
            let indexed = projection(
                FAMILY_MODEL,
                row.tenant_id().as_str(),
                row.window_start(),
                path.as_str(),
                Some(row.provider_id().as_str()),
                Some(row.effective_model_id().as_str()),
            )?;
            additive_update(
                &self.filesystem,
                &scope,
                &path,
                "telemetry_hourly_model_v0",
                indexed,
                row.clone(),
                add_model,
                model_wire,
            )
            .await?;
            applied += 1;
        }
        for row in batch.run_failures() {
            let path = path_for(
                FAMILY_FAILURE,
                row.window_start(),
                &[row.user_id().as_str(), row.failure_category().as_str()],
            )?;
            let indexed = projection(
                FAMILY_FAILURE,
                row.tenant_id().as_str(),
                row.window_start(),
                path.as_str(),
                None,
                None,
            )?;
            additive_update(
                &self.filesystem,
                &scope,
                &path,
                "telemetry_hourly_failure_v0",
                indexed,
                row.clone(),
                add_failure,
                failure_wire,
            )
            .await?;
            applied += 1;
        }
        for row in batch.automation_usage() {
            let path = path_for(
                FAMILY_AUTOMATION,
                row.window_start(),
                &[
                    row.user_id().as_str(),
                    automation_text(row.automation_kind()),
                ],
            )?;
            let indexed = projection(
                FAMILY_AUTOMATION,
                row.tenant_id().as_str(),
                row.window_start(),
                path.as_str(),
                None,
                None,
            )?;
            additive_update(
                &self.filesystem,
                &scope,
                &path,
                "telemetry_hourly_automation_v0",
                indexed,
                row.clone(),
                add_automation,
                automation_wire,
            )
            .await?;
            applied += 1;
        }
        for row in batch.lifecycle_events() {
            let path = lifecycle_path(row.event_id().as_str())?;
            let indexed = lifecycle_projection(
                row.tenant_id().as_str(),
                row.occurred_at(),
                row.event_id().as_str(),
            )?;
            self.apply_lifecycle(&scope, row, &path, indexed).await?;
            applied += 1;
        }
        for row in batch.collector_coverage() {
            let path = coverage_path(row)?;
            let indexed = projection(
                FAMILY_COVERAGE,
                row.tenant_id().as_str(),
                row.window_start(),
                path.as_str(),
                None,
                None,
            )?;
            additive_update(
                &self.filesystem,
                &scope,
                &path,
                "telemetry_collector_coverage_v0",
                indexed,
                row.clone(),
                add_coverage,
                coverage_wire,
            )
            .await?;
            applied += 1;
        }
        Ok(BatchApplyReport::complete(applied))
    }

    async fn apply_lifecycle(
        &self,
        scope: &ResourceScope,
        row: &LifecycleEvent,
        path: &ScopedPath,
        indexed: BTreeMap<IndexKey, IndexValue>,
    ) -> Result<(), TelemetryRepositoryError> {
        let incoming = lifecycle_wire(row);
        let decode = |body: &[u8]| {
            serde_json::from_slice::<WireLifecycle>(body)
                .map_err(|source| json_error("decoding telemetry lifecycle", source))
        };
        let encode =
            move |wire: &WireLifecycle| entry("telemetry_lifecycle_v0", wire, indexed.clone());
        cas_update(
            &self.filesystem,
            scope,
            path,
            decode,
            encode,
            move |current| {
                let incoming = incoming.clone();
                async move {
                    match current {
                        None => Ok(CasApply::new(incoming, ())),
                        Some(current) => {
                            let existing = lifecycle_from_wire(current)?;
                            let incoming_row = lifecycle_from_wire(incoming.clone())?;
                            if existing == incoming_row {
                                Ok(CasApply::no_op(incoming, ()))
                            } else {
                                Err(TelemetryRepositoryError::InvalidProjection)
                            }
                        }
                    }
                }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn query_entries(
        &self,
        scope: &ResourceScope,
        request: &TelemetryPageRequest,
        shape: QueryShape,
    ) -> Result<Vec<VersionedEntry>, TelemetryRepositoryError> {
        let family = shape.family();
        let to = request.effective_to();
        if request.from >= to {
            return Ok(Vec::new());
        }
        self.ensure_indexes(scope).await?;
        let prefix = scoped_path(TELEMETRY_PREFIX.to_owned())?;
        let (index_name_value, key_name, tie_name) = match shape {
            QueryShape::Lifecycle => (INDEX_LIFECYCLE_TIME, "occurred_at", "tie_breaker"),
            QueryShape::Model => match (
                request.provider_id.is_some(),
                request.effective_model_id.is_some(),
            ) {
                (true, true) => (INDEX_PROVIDER_MODEL_TIME, "window_start", "tie_breaker"),
                (true, false) => (INDEX_PROVIDER_TIME, "window_start", "tie_breaker"),
                (false, true) => (INDEX_MODEL_TIME, "window_start", "tie_breaker"),
                (false, false) => (INDEX_FAMILY_TIME, "window_start", "tie_breaker"),
            },
            QueryShape::Activity
            | QueryShape::Failure
            | QueryShape::Automation
            | QueryShape::Coverage => (INDEX_FAMILY_TIME, "window_start", "tie_breaker"),
        };
        let index = index_name(index_name_value)?;
        let mut filters = vec![
            Filter::Eq {
                key: index_key("tenant_id")?,
                value: IndexValue::Text(scope.tenant_id.as_str().to_owned()),
            },
            Filter::Eq {
                key: index_key("record_family")?,
                value: IndexValue::Text(family.to_owned()),
            },
        ];
        if let QueryShape::Model = shape {
            if let Some(provider) = request.provider_id.as_ref() {
                filters.push(Filter::Eq {
                    key: index_key("provider_id")?,
                    value: IndexValue::Text(provider.as_str().to_owned()),
                });
            }
            if let Some(model) = request.effective_model_id.as_ref() {
                filters.push(Filter::Eq {
                    key: index_key("effective_model_id")?,
                    value: IndexValue::Text(model.as_str().to_owned()),
                });
            }
        }
        let filter = Filter::And(filters);
        let (after_time, after_tie) = match request.after.as_deref() {
            Some(cursor) => {
                let (time, fields) = decode_cursor(cursor, 1)?;
                (
                    time,
                    fields
                        .into_iter()
                        .next()
                        .ok_or(TelemetryRepositoryError::InvalidCursor)?,
                )
            }
            None => (request.from, String::new()),
        };
        let mut entries = Vec::with_capacity(request.page_size.saturating_add(1));
        let mut cursor = Some(OrderedQueryCursor {
            value: IndexValue::Text(timestamp_text(after_time)),
            tie_breaker: IndexValue::Text(after_tie),
        });
        while entries.len() <= request.page_size {
            let remaining = request
                .page_size
                .saturating_add(1)
                .saturating_sub(entries.len());
            let limit = remaining.min(ironclaw_filesystem::Page::MAX_LIMIT as usize) as u32;
            let page = OrderedPage {
                index: index.clone(),
                key: index_key(key_name)?,
                tie_breaker: index_key(tie_name)?,
                direction: SortDirection::Ascending,
                after: cursor.take(),
                limit,
            };
            let result = self
                .filesystem
                .query_ordered(scope, &prefix, &filter, &page)
                .await
                .map_err(|source| TelemetryRepositoryError::StorageOperation {
                    operation: "reading ordered telemetry records",
                    source: Box::new(source),
                })?;
            if result.is_empty() {
                break;
            }
            let result_len = result.len();
            let mut last = None;
            for item in result {
                let time_key = shape.time_key();
                let Some(IndexValue::Text(value)) = item.entry.indexed.get(&index_key(time_key)?)
                else {
                    return Err(TelemetryRepositoryError::InvalidProjection);
                };
                let timestamp = parse_timestamp(value, time_key)?;
                let Some(IndexValue::Text(tie)) =
                    item.entry.indexed.get(&index_key("tie_breaker")?)
                else {
                    return Err(TelemetryRepositoryError::InvalidProjection);
                };
                if timestamp < request.from {
                    last = Some((timestamp, tie.clone()));
                    continue;
                }
                if timestamp >= to {
                    last = None;
                    break;
                }
                last = Some((timestamp, tie.clone()));
                entries.push(item);
                if entries.len() > request.page_size {
                    break;
                }
            }
            if entries.len() > request.page_size || last.is_none() {
                break;
            }
            let (time, tie) = last.ok_or(TelemetryRepositoryError::InvalidCursor)?;
            cursor = Some(OrderedQueryCursor {
                value: IndexValue::Text(timestamp_text(time)),
                tie_breaker: IndexValue::Text(tie),
            });
            if result_len < limit as usize {
                break;
            }
        }
        Ok(entries)
    }

    pub async fn read_activity_page(
        &self,
        scope: &ResourceScope,
        request: &TelemetryPageRequest,
    ) -> Result<TelemetryPage<HourlyUserActivity>, TelemetryRepositoryError> {
        let entries = self
            .query_entries(scope, request, QueryShape::Activity)
            .await?;
        let has_more = entries.len() > request.page_size;
        let mut rows = Vec::with_capacity(entries.len().min(request.page_size));
        for entry in entries.into_iter().take(request.page_size) {
            let wire = serde_json::from_slice::<WireActivity>(&entry.entry.body)
                .map_err(|source| json_error("decoding activity", source))?;
            let row = activity_from_wire(wire)?;
            if row.tenant_id() != &scope.tenant_id {
                return Err(TelemetryRepositoryError::ScopeMismatch);
            }
            let path = path_for(
                FAMILY_ACTIVITY,
                row.window_start(),
                &[row.user_id().as_str(), origin_text(row.origin_kind())],
            )?;
            validate_entry_shape(
                &entry,
                "telemetry_hourly_activity_v0",
                projection(
                    FAMILY_ACTIVITY,
                    row.tenant_id().as_str(),
                    row.window_start(),
                    path.as_str(),
                    None,
                    None,
                )?,
            )?;
            rows.push(row);
        }
        let next = if has_more {
            rows.last()
                .map(|row| {
                    path_for(
                        FAMILY_ACTIVITY,
                        row.window_start(),
                        &[row.user_id().as_str(), origin_text(row.origin_kind())],
                    )
                    .map(|path| encode_cursor(row.window_start(), &[path.as_str()]))
                })
                .transpose()?
        } else {
            None
        };
        Ok(TelemetryPage::new(rows, next))
    }

    pub async fn read_model_page(
        &self,
        scope: &ResourceScope,
        request: &TelemetryPageRequest,
    ) -> Result<TelemetryPage<HourlyModelUsage>, TelemetryRepositoryError> {
        let entries = self
            .query_entries(scope, request, QueryShape::Model)
            .await?;
        let has_more = entries.len() > request.page_size;
        let mut rows = Vec::with_capacity(entries.len().min(request.page_size));
        for entry in entries.into_iter().take(request.page_size) {
            let row = model_from_wire(
                serde_json::from_slice(&entry.entry.body)
                    .map_err(|source| json_error("decoding model", source))?,
            )?;
            if row.tenant_id() != &scope.tenant_id {
                return Err(TelemetryRepositoryError::ScopeMismatch);
            }
            let path = path_for(
                FAMILY_MODEL,
                row.window_start(),
                &[
                    row.user_id().as_str(),
                    row.provider_id().as_str(),
                    row.effective_model_id().as_str(),
                ],
            )?;
            validate_entry_shape(
                &entry,
                "telemetry_hourly_model_v0",
                projection(
                    FAMILY_MODEL,
                    row.tenant_id().as_str(),
                    row.window_start(),
                    path.as_str(),
                    Some(row.provider_id().as_str()),
                    Some(row.effective_model_id().as_str()),
                )?,
            )?;
            rows.push(row);
        }
        let next = if has_more {
            rows.last()
                .map(|row| {
                    path_for(
                        FAMILY_MODEL,
                        row.window_start(),
                        &[
                            row.user_id().as_str(),
                            row.provider_id().as_str(),
                            row.effective_model_id().as_str(),
                        ],
                    )
                    .map(|path| encode_cursor(row.window_start(), &[path.as_str()]))
                })
                .transpose()?
        } else {
            None
        };
        Ok(TelemetryPage::new(rows, next))
    }

    pub async fn read_failure_page(
        &self,
        scope: &ResourceScope,
        request: &TelemetryPageRequest,
    ) -> Result<TelemetryPage<HourlyRunFailure>, TelemetryRepositoryError> {
        self.read_simple(
            scope,
            request,
            QueryShape::Failure,
            |entry| {
                let row = failure_from_wire(
                    serde_json::from_slice(&entry.entry.body)
                        .map_err(|source| json_error("decoding failure", source))?,
                )?;
                if row.tenant_id() != &scope.tenant_id {
                    return Err(TelemetryRepositoryError::ScopeMismatch);
                }
                let path = path_for(
                    FAMILY_FAILURE,
                    row.window_start(),
                    &[row.user_id().as_str(), row.failure_category().as_str()],
                )?;
                let shape = projection(
                    FAMILY_FAILURE,
                    row.tenant_id().as_str(),
                    row.window_start(),
                    path.as_str(),
                    None,
                    None,
                )?;
                validate_entry_shape(entry, "telemetry_hourly_failure_v0", shape)?;
                Ok(row)
            },
            |row: &HourlyRunFailure| {
                path_for(
                    FAMILY_FAILURE,
                    row.window_start(),
                    &[row.user_id().as_str(), row.failure_category().as_str()],
                )
                .map(|path| (row.window_start(), path.as_str().to_owned()))
            },
        )
        .await
    }

    pub async fn read_automation_page(
        &self,
        scope: &ResourceScope,
        request: &TelemetryPageRequest,
    ) -> Result<TelemetryPage<HourlyAutomationUsage>, TelemetryRepositoryError> {
        self.read_simple(
            scope,
            request,
            QueryShape::Automation,
            |entry| {
                let row = automation_from_wire(
                    serde_json::from_slice(&entry.entry.body)
                        .map_err(|source| json_error("decoding automation", source))?,
                )?;
                if row.tenant_id() != &scope.tenant_id {
                    return Err(TelemetryRepositoryError::ScopeMismatch);
                }
                let path = path_for(
                    FAMILY_AUTOMATION,
                    row.window_start(),
                    &[
                        row.user_id().as_str(),
                        automation_text(row.automation_kind()),
                    ],
                )?;
                let shape = projection(
                    FAMILY_AUTOMATION,
                    row.tenant_id().as_str(),
                    row.window_start(),
                    path.as_str(),
                    None,
                    None,
                )?;
                validate_entry_shape(entry, "telemetry_hourly_automation_v0", shape)?;
                Ok(row)
            },
            |row: &HourlyAutomationUsage| {
                path_for(
                    FAMILY_AUTOMATION,
                    row.window_start(),
                    &[
                        row.user_id().as_str(),
                        automation_text(row.automation_kind()),
                    ],
                )
                .map(|path| (row.window_start(), path.as_str().to_owned()))
            },
        )
        .await
    }

    pub async fn read_lifecycle_page(
        &self,
        scope: &ResourceScope,
        request: &TelemetryPageRequest,
    ) -> Result<TelemetryPage<LifecycleEvent>, TelemetryRepositoryError> {
        self.read_simple_lifecycle(scope, request).await
    }

    pub async fn read_coverage_page(
        &self,
        scope: &ResourceScope,
        request: &TelemetryPageRequest,
    ) -> Result<TelemetryPage<CollectorCoverage>, TelemetryRepositoryError> {
        self.read_simple(
            scope,
            request,
            QueryShape::Coverage,
            |entry| {
                let row = coverage_from_wire(
                    serde_json::from_slice(&entry.entry.body)
                        .map_err(|source| json_error("decoding coverage", source))?,
                )?;
                if row.tenant_id() != &scope.tenant_id {
                    return Err(TelemetryRepositoryError::ScopeMismatch);
                }
                let path = coverage_path(&row)?;
                let shape = projection(
                    FAMILY_COVERAGE,
                    row.tenant_id().as_str(),
                    row.window_start(),
                    path.as_str(),
                    None,
                    None,
                )?;
                validate_entry_shape(entry, "telemetry_collector_coverage_v0", shape)?;
                Ok(row)
            },
            |row: &CollectorCoverage| {
                coverage_path(row).map(|path| (row.window_start(), path.as_str().to_owned()))
            },
        )
        .await
    }

    async fn read_simple<T, D, C>(
        &self,
        scope: &ResourceScope,
        request: &TelemetryPageRequest,
        shape: QueryShape,
        decode: D,
        cursor: C,
    ) -> Result<TelemetryPage<T>, TelemetryRepositoryError>
    where
        T: TenantScoped,
        D: Fn(&VersionedEntry) -> Result<T, TelemetryRepositoryError>,
        C: Fn(&T) -> Result<(DateTime<Utc>, String), TelemetryRepositoryError>,
    {
        let entries = self.query_entries(scope, request, shape).await?;
        let has_more = entries.len() > request.page_size;
        let rows = entries
            .into_iter()
            .take(request.page_size)
            .map(|entry| decode(&entry))
            .collect::<Result<Vec<_>, _>>()?;
        if rows.iter().any(|row| row.tenant_id() != &scope.tenant_id) {
            return Err(TelemetryRepositoryError::ScopeMismatch);
        }
        let next = has_more
            .then(|| rows.last().map(&cursor).transpose())
            .transpose()?
            .flatten()
            .map(|(time, tie)| encode_cursor(time, &[&tie]));
        Ok(TelemetryPage::new(rows, next))
    }

    async fn read_simple_lifecycle(
        &self,
        scope: &ResourceScope,
        request: &TelemetryPageRequest,
    ) -> Result<TelemetryPage<LifecycleEvent>, TelemetryRepositoryError> {
        let entries = self
            .query_entries(scope, request, QueryShape::Lifecycle)
            .await?;
        let has_more = entries.len() > request.page_size;
        let mut rows = Vec::with_capacity(entries.len().min(request.page_size));
        for entry in entries.into_iter().take(request.page_size) {
            let row = lifecycle_from_wire(
                serde_json::from_slice(&entry.entry.body)
                    .map_err(|source| json_error("decoding lifecycle", source))?,
            )?;
            if row.tenant_id() != &scope.tenant_id {
                return Err(TelemetryRepositoryError::ScopeMismatch);
            }
            validate_entry_shape(
                &entry,
                "telemetry_lifecycle_v0",
                lifecycle_projection(
                    row.tenant_id().as_str(),
                    row.occurred_at(),
                    row.event_id().as_str(),
                )?,
            )?;
            rows.push(row);
        }
        let next = has_more
            .then(|| {
                rows.last()
                    .map(|row| encode_cursor(row.occurred_at(), &[row.event_id().as_str()]))
            })
            .flatten();
        Ok(TelemetryPage::new(rows, next))
    }
}
