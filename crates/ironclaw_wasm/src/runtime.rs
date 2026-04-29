/// Minimal Wasmtime-backed runtime.
pub struct WasmRuntime {
    engine: Engine,
    config: WasmRuntimeConfig,
    prepared_modules: Arc<Mutex<HashMap<ModuleCacheKey, Arc<PreparedWasmModule>>>>,
    _epoch_ticker: Option<EpochTicker>,
}

impl WasmRuntime {
    pub fn new(config: WasmRuntimeConfig) -> Result<Self, WasmError> {
        let mut wasmtime_config = Config::new();
        wasmtime_config.consume_fuel(true);
        wasmtime_config.epoch_interruption(true);
        wasmtime_config.wasm_threads(false);
        wasmtime_config.debug_info(false);
        if let Some(cache_dir) = &config.cache_dir {
            enable_compilation_cache(&mut wasmtime_config, cache_dir)?;
        }
        let engine = Engine::new(&wasmtime_config).map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
        let epoch_ticker = spawn_epoch_ticker(engine.clone(), config.epoch_tick_interval)?;
        Ok(Self {
            engine,
            config,
            prepared_modules: Arc::new(Mutex::new(HashMap::new())),
            _epoch_ticker: epoch_ticker,
        })
    }

    pub fn for_testing() -> Result<Self, WasmError> {
        Self::new(WasmRuntimeConfig::for_testing())
    }

    pub fn config(&self) -> &WasmRuntimeConfig {
        &self.config
    }

    pub fn prepared_module_count(&self) -> usize {
        self.prepared_modules
            .lock()
            .map(|cache| cache.len())
            .unwrap_or(0)
    }

    pub fn prepare(&self, spec: WasmModuleSpec) -> Result<PreparedWasmModule, WasmError> {
        self.prepare_uncached(spec)
    }

    pub fn prepare_cached(
        &self,
        spec: WasmModuleSpec,
    ) -> Result<Arc<PreparedWasmModule>, WasmError> {
        if !self.config.cache_compiled_modules {
            return self.prepare_uncached(spec).map(Arc::new);
        }

        let key = ModuleCacheKey::new(&spec);
        {
            let cache = self.prepared_modules.lock().map_err(|_| WasmError::Cache {
                reason: "prepared module cache lock poisoned".to_string(),
            })?;
            if let Some(module) = cache.get(&key) {
                return Ok(Arc::clone(module));
            }
        }

        let prepared = Arc::new(self.prepare_uncached(spec)?);
        let mut cache = self.prepared_modules.lock().map_err(|_| WasmError::Cache {
            reason: "prepared module cache lock poisoned".to_string(),
        })?;
        Ok(Arc::clone(cache.entry(key).or_insert(prepared)))
    }

    fn prepare_uncached(&self, spec: WasmModuleSpec) -> Result<PreparedWasmModule, WasmError> {
        let content_hash = wasm_content_hash(&spec.bytes);
        let module = Module::from_binary(&self.engine, &spec.bytes).map_err(|error| {
            WasmError::InvalidModule {
                reason: error.to_string(),
            }
        })?;

        validate_module_imports(&module)?;

        if spec.export.trim().is_empty()
            || !module.exports().any(|export| export.name() == spec.export)
        {
            return Err(WasmError::MissingExport {
                export: spec.export,
            });
        }

        Ok(PreparedWasmModule {
            provider: spec.provider,
            capability: spec.capability,
            export: spec.export,
            content_hash,
            module,
        })
    }

    /// Prepare a WASM capability from a validated extension package manifest.
    pub async fn prepare_extension_capability<F>(
        &self,
        fs: &F,
        package: &ExtensionPackage,
        capability_id: &CapabilityId,
    ) -> Result<PreparedWasmCapability, WasmError>
    where
        F: RootFilesystem,
    {
        let descriptor = package
            .capabilities
            .iter()
            .find(|descriptor| &descriptor.id == capability_id)
            .cloned()
            .ok_or_else(|| WasmError::CapabilityNotDeclared {
                capability: capability_id.clone(),
            })?;

        if descriptor.runtime != RuntimeKind::Wasm {
            return Err(WasmError::ExtensionRuntimeMismatch {
                extension: package.id.clone(),
                actual: descriptor.runtime,
            });
        }
        if descriptor.provider != package.id {
            return Err(WasmError::DescriptorMismatch {
                reason: format!(
                    "descriptor {} provider {} does not match package {}",
                    descriptor.id, descriptor.provider, package.id
                ),
            });
        }

        let module_asset = match &package.manifest.runtime {
            ExtensionRuntime::Wasm { module } => module,
            other => {
                return Err(WasmError::ExtensionRuntimeMismatch {
                    extension: package.id.clone(),
                    actual: other.kind(),
                });
            }
        };
        let module_path = module_asset
            .resolve_under(&package.root)
            .map_err(|error| WasmError::Extension(Box::new(error)))?;
        let bytes = fs
            .read_file(&module_path)
            .await
            .map_err(|error| WasmError::Filesystem(Box::new(error)))?;
        let export = capability_export_name(&package.id, capability_id)?;
        let module = self.prepare_cached(WasmModuleSpec {
            provider: package.id.clone(),
            capability: capability_id.clone(),
            export,
            bytes,
        })?;

        Ok(PreparedWasmCapability {
            descriptor,
            module,
            module_path,
        })
    }

    /// Execute a WASM extension capability with resource reserve/reconcile semantics.
    pub async fn execute_extension_json<F, G>(
        &self,
        fs: &F,
        governor: &G,
        request: WasmExecutionRequest<'_>,
    ) -> Result<WasmExecutionResult, WasmError>
    where
        F: RootFilesystem,
        G: ResourceGovernor,
    {
        self.execute_extension_json_with_host_context(
            fs,
            governor,
            request,
            WasmHostImportContext::new(),
        )
        .await
    }

    /// Execute a WASM extension capability with host-mediated network imports.
    pub async fn execute_extension_json_with_network<F, G>(
        &self,
        fs: &F,
        governor: &G,
        request: WasmExecutionRequest<'_>,
        http: Arc<dyn WasmHostHttp>,
    ) -> Result<WasmExecutionResult, WasmError>
    where
        F: RootFilesystem,
        G: ResourceGovernor,
    {
        self.execute_extension_json_with_host_context(
            fs,
            governor,
            request,
            WasmHostImportContext::new().with_http(http),
        )
        .await
    }

    /// Execute a WASM extension capability with an explicit host-import context.
    pub async fn execute_extension_json_with_host_context<F, G>(
        &self,
        fs: &F,
        governor: &G,
        request: WasmExecutionRequest<'_>,
        host_context: WasmHostImportContext,
    ) -> Result<WasmExecutionResult, WasmError>
    where
        F: RootFilesystem,
        G: ResourceGovernor,
    {
        let reservation = reserve_or_use_existing(
            governor,
            request.scope.clone(),
            request.estimate.clone(),
            request.resource_reservation,
        )?;

        let prepared = match self
            .prepare_extension_capability(fs, request.package, request.capability_id)
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => return Err(release_after_failure(governor, reservation.id, error)),
        };

        let result = match self.invoke_json_with_host_context(
            prepared.module.as_ref(),
            &prepared.descriptor,
            Some(&reservation),
            request.invocation,
            host_context,
        ) {
            Ok(result) => result,
            Err(error) => return Err(release_after_failure(governor, reservation.id, error)),
        };

        let receipt = governor
            .reconcile(reservation.id, result.usage.clone())
            .map_err(|error| WasmError::Resource(Box::new(error)))?;
        Ok(WasmExecutionResult { result, receipt })
    }

    /// Invoke a capability through the initial JSON pointer/length ABI.
    ///
    /// The guest module must export:
    ///
    /// - `memory`
    /// - `alloc(len: i32) -> i32`
    /// - the module's configured capability export as `(ptr: i32, len: i32) -> i32`
    /// - `output_ptr() -> i32`
    /// - `output_len() -> i32`
    ///
    /// A zero status means the output buffer contains JSON success output. Any
    /// non-zero status means the output buffer contains a JSON error object and
    /// is surfaced as [`WasmError::GuestError`].
    pub fn invoke_json(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ResourceReservation>,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResult, WasmError> {
        self.invoke_json_inner(module, descriptor, reservation, invocation, None, None)
    }

    pub fn invoke_json_with_filesystem(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ResourceReservation>,
        invocation: CapabilityInvocation,
        filesystem: Arc<dyn WasmHostFilesystem>,
    ) -> Result<CapabilityResult, WasmError> {
        self.invoke_json_inner(
            module,
            descriptor,
            reservation,
            invocation,
            Some(filesystem),
            None,
        )
    }

    pub fn invoke_json_with_network(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ResourceReservation>,
        invocation: CapabilityInvocation,
        http: Arc<dyn WasmHostHttp>,
    ) -> Result<CapabilityResult, WasmError> {
        self.invoke_json_inner(
            module,
            descriptor,
            reservation,
            invocation,
            None,
            Some(http),
        )
    }

    pub fn invoke_json_with_host_context(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ResourceReservation>,
        invocation: CapabilityInvocation,
        host_context: WasmHostImportContext,
    ) -> Result<CapabilityResult, WasmError> {
        self.invoke_json_inner(
            module,
            descriptor,
            reservation,
            invocation,
            host_context.filesystem.clone(),
            host_context.http.clone(),
        )
    }

    fn invoke_json_inner(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ResourceReservation>,
        invocation: CapabilityInvocation,
        filesystem: Option<Arc<dyn WasmHostFilesystem>>,
        http: Option<Arc<dyn WasmHostHttp>>,
    ) -> Result<CapabilityResult, WasmError> {
        let reservation = reservation.ok_or(WasmError::MissingReservation)?;
        self.validate_descriptor(module, descriptor)?;
        validate_invocation_schema(&descriptor.parameters_schema, &invocation.input)?;

        let input_bytes = serde_json::to_vec(&invocation.input).map_err(|error| {
            WasmError::InvalidInvocation {
                reason: error.to_string(),
            }
        })?;
        let input_len =
            i32::try_from(input_bytes.len()).map_err(|_| WasmError::InvalidInvocation {
                reason: "input JSON is too large for the V1 WASM ABI".to_string(),
            })?;

        let start = Instant::now();
        let mut store = self.fueled_store_with_context(filesystem, http)?;
        let instance = self.instantiate_module(&mut store, module)?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or(WasmError::MissingMemory)?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&mut store, "alloc")
            .map_err(|_| WasmError::MissingExport {
                export: "alloc".to_string(),
            })?;
        let run = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, module.export())
            .map_err(|_| WasmError::MissingExport {
                export: module.export().to_string(),
            })?;
        let output_ptr = instance
            .get_typed_func::<(), i32>(&mut store, "output_ptr")
            .map_err(|_| WasmError::MissingExport {
                export: "output_ptr".to_string(),
            })?;
        let output_len = instance
            .get_typed_func::<(), i32>(&mut store, "output_len")
            .map_err(|_| WasmError::MissingExport {
                export: "output_len".to_string(),
            })?;

        let input_ptr = alloc
            .call(&mut store, input_len)
            .map_err(|error| self.classify_wasmtime_error(error))?;
        let input_offset = positive_offset(input_ptr, "alloc returned a negative input pointer")?;
        memory
            .write(&mut store, input_offset, &input_bytes)
            .map_err(|error| WasmError::GuestAllocation {
                reason: error.to_string(),
            })?;

        let status = run
            .call(&mut store, (input_ptr, input_len))
            .map_err(|error| self.classify_wasmtime_error(error))?;
        self.ensure_no_host_import_timeout(&store)?;
        self.ensure_no_memory_denial(&store)?;
        let output_ptr_value = output_ptr
            .call(&mut store, ())
            .map_err(|error| self.classify_wasmtime_error(error))?;
        let output_len_value = output_len
            .call(&mut store, ())
            .map_err(|error| self.classify_wasmtime_error(error))?;
        let output_offset =
            positive_offset(output_ptr_value, "output_ptr returned a negative pointer")?;
        let output_len = positive_len(output_len_value)?;
        if output_len as u64 > self.config.max_output_bytes {
            return Err(WasmError::OutputLimitExceeded {
                limit: self.config.max_output_bytes,
                actual: output_len as u64,
            });
        }

        let mut output_bytes = vec![0_u8; output_len];
        memory
            .read(&store, output_offset, &mut output_bytes)
            .map_err(|error| WasmError::InvalidGuestOutput {
                reason: error.to_string(),
            })?;

        if status != 0 {
            return Err(guest_error(status, &output_bytes));
        }

        let output = serde_json::from_slice(&output_bytes).map_err(|error| {
            WasmError::InvalidGuestOutput {
                reason: error.to_string(),
            }
        })?;
        let output_byte_count = output_bytes.len() as u64;
        let fuel_consumed = self.fuel_consumed(&store);
        let usage = resource_usage(start, output_byte_count, store.data().network_egress_bytes);

        let logs = store.data().logs.clone();

        Ok(CapabilityResult {
            output,
            reservation_id: reservation.id,
            usage,
            fuel_consumed,
            output_bytes: output_byte_count,
            logs,
        })
    }

    pub fn invoke_i32(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ResourceReservation>,
        input: i32,
    ) -> Result<WasmInvocationResult<i32>, WasmError> {
        let reservation = reservation.ok_or(WasmError::MissingReservation)?;
        self.validate_descriptor(module, descriptor)?;

        let start = Instant::now();
        let mut store = self.fueled_store()?;

        let instance = self.instantiate_module(&mut store, module)?;
        let func = instance
            .get_typed_func::<i32, i32>(&mut store, module.export())
            .map_err(|_| WasmError::MissingExport {
                export: module.export().to_string(),
            })?;

        let value = func
            .call(&mut store, input)
            .map_err(|error| self.classify_wasmtime_error(error))?;
        self.ensure_no_host_import_timeout(&store)?;
        self.ensure_no_memory_denial(&store)?;

        let output_bytes = value.to_string().len() as u64;
        if output_bytes > self.config.max_output_bytes {
            return Err(WasmError::OutputLimitExceeded {
                limit: self.config.max_output_bytes,
                actual: output_bytes,
            });
        }

        Ok(WasmInvocationResult {
            value,
            reservation_id: reservation.id,
            usage: resource_usage(start, output_bytes, 0),
            fuel_consumed: self.fuel_consumed(&store),
            output_bytes,
        })
    }

    fn validate_descriptor(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
    ) -> Result<(), WasmError> {
        if descriptor.runtime != RuntimeKind::Wasm {
            return Err(WasmError::DescriptorMismatch {
                reason: "descriptor runtime must be RuntimeKind::Wasm".to_string(),
            });
        }
        if descriptor.provider != module.provider {
            return Err(WasmError::DescriptorMismatch {
                reason: format!(
                    "descriptor provider {} does not match module provider {}",
                    descriptor.provider, module.provider
                ),
            });
        }
        if descriptor.id != module.capability {
            return Err(WasmError::DescriptorMismatch {
                reason: format!(
                    "descriptor capability {} does not match module capability {}",
                    descriptor.id, module.capability
                ),
            });
        }
        Ok(())
    }

    fn instantiate_module(
        &self,
        store: &mut Store<RuntimeStoreData>,
        module: &PreparedWasmModule,
    ) -> Result<Instance, WasmError> {
        let mut linker = Linker::new(&self.engine);
        add_core_host_imports(&mut linker)?;
        linker
            .instantiate(store, &module.module)
            .map_err(|error| self.classify_wasmtime_error(error))
    }

    fn fueled_store(&self) -> Result<Store<RuntimeStoreData>, WasmError> {
        self.fueled_store_with_context(None, None)
    }

    fn fueled_store_with_context(
        &self,
        filesystem: Option<Arc<dyn WasmHostFilesystem>>,
        http: Option<Arc<dyn WasmHostHttp>>,
    ) -> Result<Store<RuntimeStoreData>, WasmError> {
        let mut store = Store::new(
            &self.engine,
            RuntimeStoreData::new(
                self.config.max_memory_bytes,
                self.config.timeout,
                filesystem,
                http,
            ),
        );
        store.limiter(|data| &mut data.limiter);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(epoch_deadline_ticks(&self.config));
        store
            .set_fuel(self.config.fuel)
            .map_err(|error| WasmError::Trap {
                reason: error.to_string(),
            })?;
        Ok(store)
    }

    fn fuel_consumed(&self, store: &Store<RuntimeStoreData>) -> u64 {
        self.config
            .fuel
            .saturating_sub(store.get_fuel().unwrap_or(0))
    }

    fn ensure_no_memory_denial(&self, store: &Store<RuntimeStoreData>) -> Result<(), WasmError> {
        if let Some((used, limit)) = store.data().limiter.denied_memory_growth {
            Err(WasmError::MemoryExceeded { used, limit })
        } else {
            Ok(())
        }
    }

    fn ensure_no_host_import_timeout(
        &self,
        store: &Store<RuntimeStoreData>,
    ) -> Result<(), WasmError> {
        if store.data().host_import_timed_out {
            Err(WasmError::Timeout {
                timeout: self.config.timeout,
            })
        } else {
            Ok(())
        }
    }

    fn classify_wasmtime_error(&self, error: wasmtime::Error) -> WasmError {
        if matches!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(wasmtime::Trap::OutOfFuel)
        ) {
            return WasmError::FuelExhausted {
                limit: self.config.fuel,
            };
        }
        if matches!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(wasmtime::Trap::Interrupt)
        ) {
            return WasmError::Timeout {
                timeout: self.config.timeout,
            };
        }
        let message = error.to_string();
        if message.contains("ironclaw memory limit exceeded") {
            return WasmError::MemoryExceeded {
                used: parse_marker_u64(&message, "desired=")
                    .unwrap_or(self.config.max_memory_bytes.saturating_add(1)),
                limit: parse_marker_u64(&message, "limit=").unwrap_or(self.config.max_memory_bytes),
            };
        }
        if message.contains("all fuel consumed") || message.contains("out of fuel") {
            WasmError::FuelExhausted {
                limit: self.config.fuel,
            }
        } else if message.contains("interrupt") {
            WasmError::Timeout {
                timeout: self.config.timeout,
            }
        } else {
            WasmError::Trap { reason: message }
        }
    }
}

struct RuntimeStoreData {
    limiter: WasmRuntimeLimiter,
    logs: Vec<WasmLogEntry>,
    network_egress_bytes: u64,
    host_import_deadline: Option<Instant>,
    host_import_timed_out: bool,
    filesystem: Option<Arc<dyn WasmHostFilesystem>>,
    http: Option<Arc<dyn WasmHostHttp>>,
}

impl RuntimeStoreData {
    fn new(
        memory_limit: u64,
        host_import_timeout: Duration,
        filesystem: Option<Arc<dyn WasmHostFilesystem>>,
        http: Option<Arc<dyn WasmHostHttp>>,
    ) -> Self {
        Self {
            limiter: WasmRuntimeLimiter::new(memory_limit),
            logs: Vec::new(),
            network_egress_bytes: 0,
            host_import_deadline: if host_import_timeout.is_zero() {
                None
            } else {
                Instant::now().checked_add(host_import_timeout)
            },
            host_import_timed_out: false,
            filesystem,
            http,
        }
    }

    fn record_network_bytes(&mut self, bytes: u64) {
        self.network_egress_bytes = self.network_egress_bytes.saturating_add(bytes);
    }

    fn record_host_import_timeout(&mut self) {
        self.host_import_timed_out = true;
    }

    fn remaining_host_import_timeout(&mut self) -> Option<Duration> {
        if self.host_import_timed_out {
            return None;
        }
        let Some(deadline) = self.host_import_deadline else {
            return Some(Duration::ZERO);
        };
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            self.record_host_import_timeout();
            return None;
        };
        if remaining.is_zero() {
            self.record_host_import_timeout();
            None
        } else {
            Some(remaining)
        }
    }

    fn push_log(&mut self, level: WasmLogLevel, message: String) {
        if self.logs.len() >= MAX_LOG_ENTRIES {
            return;
        }
        self.logs.push(WasmLogEntry {
            level,
            message,
            timestamp_unix_ms: unix_time_ms(),
        });
    }
}

#[derive(Debug)]
struct WasmRuntimeLimiter {
    memory_limit: u64,
    memory_used: u64,
    denied_memory_growth: Option<(u64, u64)>,
}

impl WasmRuntimeLimiter {
    fn new(memory_limit: u64) -> Self {
        Self {
            memory_limit,
            memory_used: 0,
            denied_memory_growth: None,
        }
    }
}

impl ResourceLimiter for WasmRuntimeLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        let desired = desired as u64;
        if desired > self.memory_limit {
            self.denied_memory_growth = Some((desired, self.memory_limit));
            return Ok(false);
        }
        self.memory_used = desired;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        Ok(desired <= 10_000)
    }

    fn instances(&self) -> usize {
        10
    }

    fn tables(&self) -> usize {
        10
    }

    fn memories(&self) -> usize {
        10
    }
}
