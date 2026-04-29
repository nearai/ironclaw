fn unix_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn reserve_or_use_existing<G>(
    governor: &G,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    reservation: Option<ResourceReservation>,
) -> Result<ResourceReservation, WasmError>
where
    G: ResourceGovernor + ?Sized,
{
    if let Some(reservation) = reservation {
        if reservation.scope != scope || reservation.estimate != estimate {
            return Err(WasmError::Resource(Box::new(
                ResourceError::ReservationMismatch { id: reservation.id },
            )));
        }
        return Ok(reservation);
    }
    governor
        .reserve(scope, estimate)
        .map_err(|error| WasmError::Resource(Box::new(error)))
}

fn release_after_failure<G>(
    governor: &G,
    reservation_id: ResourceReservationId,
    original: WasmError,
) -> WasmError
where
    G: ResourceGovernor,
{
    match governor.release(reservation_id) {
        Ok(_) => original,
        Err(error) => WasmError::Resource(Box::new(error)),
    }
}

fn capability_export_name(
    package_id: &ExtensionId,
    capability_id: &CapabilityId,
) -> Result<String, WasmError> {
    let expected_prefix = format!("{}.", package_id.as_str());
    capability_id
        .as_str()
        .strip_prefix(&expected_prefix)
        .filter(|suffix| !suffix.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| WasmError::DescriptorMismatch {
            reason: format!(
                "capability {} is not prefixed by package {}",
                capability_id, package_id
            ),
        })
}

fn enable_compilation_cache(config: &mut Config, cache_dir: &Path) -> Result<(), WasmError> {
    std::fs::create_dir_all(cache_dir).map_err(|error| WasmError::Cache {
        reason: error.to_string(),
    })?;
    let toml_path = cache_dir.join("wasmtime-cache.toml");
    let escaped = cache_dir
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    std::fs::write(&toml_path, format!("[cache]\ndirectory = \"{escaped}\"\n")).map_err(
        |error| WasmError::Cache {
            reason: error.to_string(),
        },
    )?;
    let cache = Cache::from_file(Some(&toml_path)).map_err(|error| WasmError::Cache {
        reason: error.to_string(),
    })?;
    config.cache(Some(cache));
    Ok(())
}

#[derive(Debug, Clone)]
struct EpochTicker {
    _state: Arc<EpochTickerState>,
}

#[derive(Debug)]
struct EpochTickerState {
    stop: AtomicBool,
}

impl Drop for EpochTickerState {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn spawn_epoch_ticker(
    engine: Engine,
    tick_interval: Duration,
) -> Result<Option<EpochTicker>, WasmError> {
    if tick_interval.is_zero() {
        return Ok(None);
    }
    let state = Arc::new(EpochTickerState {
        stop: AtomicBool::new(false),
    });
    let weak_state: Weak<EpochTickerState> = Arc::downgrade(&state);
    std::thread::Builder::new()
        .name("ironclaw-wasm-epoch-ticker".to_string())
        .spawn(move || {
            while let Some(state) = weak_state.upgrade() {
                if state.stop.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(tick_interval);
                if state.stop.load(Ordering::SeqCst) {
                    break;
                }
                engine.increment_epoch();
            }
        })
        .map(|_| Some(EpochTicker { _state: state }))
        .map_err(|error| WasmError::Engine {
            reason: format!("failed to spawn epoch ticker thread: {error}"),
        })
}

fn epoch_deadline_ticks(config: &WasmRuntimeConfig) -> u64 {
    if config.timeout.is_zero() || config.epoch_tick_interval.is_zero() {
        return u64::MAX;
    }
    let timeout_ms = config.timeout.as_millis();
    let interval_ms = config.epoch_tick_interval.as_millis().max(1);
    (timeout_ms / interval_ms).max(1) as u64
}

fn wasm_content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn parse_marker_u64(message: &str, marker: &str) -> Option<u64> {
    let start = message.find(marker)? + marker.len();
    let digits: String = message[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn positive_offset(value: i32, reason: &str) -> Result<usize, WasmError> {
    usize::try_from(value).map_err(|_| WasmError::InvalidGuestOutput {
        reason: reason.to_string(),
    })
}

fn positive_len(value: i32) -> Result<usize, WasmError> {
    usize::try_from(value).map_err(|_| WasmError::InvalidGuestOutput {
        reason: "output_len returned a negative length".to_string(),
    })
}

fn guest_error(status: i32, output_bytes: &[u8]) -> WasmError {
    let message = serde_json::from_slice::<Value>(output_bytes)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "guest returned an error without a valid message".to_string());
    WasmError::GuestError { status, message }
}

fn validate_invocation_schema(schema: &Value, input: &Value) -> Result<(), WasmError> {
    if schema.is_null() {
        return Ok(());
    }

    let validator =
        jsonschema::validator_for(schema).map_err(|error| WasmError::InvalidInvocation {
            reason: format!("invalid parameter schema: {error}"),
        })?;
    let errors = validator
        .iter_errors(input)
        .take(5)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();

    if errors.is_empty() {
        Ok(())
    } else {
        Err(WasmError::InvalidInvocation {
            reason: format!(
                "input failed parameter schema validation: {}",
                errors.join("; ")
            ),
        })
    }
}

fn resource_usage(start: Instant, output_bytes: u64, network_egress_bytes: u64) -> ResourceUsage {
    ResourceUsage {
        usd: Decimal::ZERO,
        input_tokens: 0,
        output_tokens: 0,
        wall_clock_ms: start.elapsed().as_millis().max(1) as u64,
        output_bytes,
        network_egress_bytes,
        process_count: 1,
    }
}
