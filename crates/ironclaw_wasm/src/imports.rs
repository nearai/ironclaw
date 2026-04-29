fn validate_module_imports(module: &Module) -> Result<(), WasmError> {
    for import in module.imports() {
        if !is_supported_core_import(import.module(), import.name()) {
            return Err(WasmError::UnsupportedImport {
                module: import.module().to_string(),
                name: import.name().to_string(),
            });
        }
    }
    Ok(())
}

fn is_supported_core_import(module: &str, name: &str) -> bool {
    module == CORE_IMPORT_MODULE
        && matches!(
            name,
            CORE_LOG_IMPORT
                | CORE_TIME_IMPORT
                | FS_READ_IMPORT
                | FS_WRITE_IMPORT
                | FS_LIST_IMPORT
                | FS_STAT_LEN_IMPORT
                | HTTP_REQUEST_IMPORT
        )
}

fn add_core_host_imports(linker: &mut Linker<RuntimeStoreData>) -> Result<(), WasmError> {
    linker
        .func_wrap(CORE_IMPORT_MODULE, CORE_TIME_IMPORT, || -> i64 {
            unix_time_ms() as i64
        })
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            CORE_LOG_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>, level: i32, ptr: i32, len: i32| -> i32 {
                host_log_utf8(&mut caller, level, ptr, len)
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            FS_READ_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>,
             path_ptr: i32,
             path_len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                host_fs_read_utf8(&mut caller, path_ptr, path_len, out_ptr, out_cap)
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            FS_WRITE_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>,
             path_ptr: i32,
             path_len: i32,
             data_ptr: i32,
             data_len: i32|
             -> i32 {
                host_fs_write_utf8(&mut caller, path_ptr, path_len, data_ptr, data_len)
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            FS_LIST_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>,
             path_ptr: i32,
             path_len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                host_fs_list_utf8(&mut caller, path_ptr, path_len, out_ptr, out_cap)
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            FS_STAT_LEN_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>, path_ptr: i32, path_len: i32| -> i64 {
                host_fs_stat_len(&mut caller, path_ptr, path_len)
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    linker
        .func_wrap(
            CORE_IMPORT_MODULE,
            HTTP_REQUEST_IMPORT,
            |mut caller: Caller<'_, RuntimeStoreData>,
             method: i32,
             url_ptr: i32,
             url_len: i32,
             body_ptr: i32,
             body_len: i32,
             out_ptr: i32,
             out_cap: i32|
             -> i32 {
                host_http_request_utf8(
                    &mut caller,
                    HttpImportArgs {
                        method,
                        url_ptr,
                        url_len,
                        body_ptr,
                        body_len,
                        out_ptr,
                        out_cap,
                    },
                )
            },
        )
        .map_err(|error| WasmError::Engine {
            reason: error.to_string(),
        })?;
    Ok(())
}

fn host_log_utf8(caller: &mut Caller<'_, RuntimeStoreData>, level: i32, ptr: i32, len: i32) -> i32 {
    let Ok(offset) = usize::try_from(ptr) else {
        return -1;
    };
    let Ok(len) = usize::try_from(len) else {
        return -1;
    };
    if len > MAX_LOG_MESSAGE_BYTES {
        return -2;
    }
    let Some(memory) = caller
        .get_export("memory")
        .and_then(|item| item.into_memory())
    else {
        return -3;
    };
    let mut bytes = vec![0_u8; len];
    if memory.read(&*caller, offset, &mut bytes).is_err() {
        return -4;
    }
    let Ok(message) = String::from_utf8(bytes) else {
        return -5;
    };
    caller
        .data_mut()
        .push_log(WasmLogLevel::from_i32(level), message);
    0
}

enum HostImportCallError<E> {
    Operation(E),
    TimedOut,
    Panicked,
}

struct HostImportThreadLimiter {
    state: Mutex<HostImportThreadState>,
    released: Condvar,
}

#[derive(Debug)]
struct HostImportThreadState {
    active: usize,
}

struct HostImportThreadPermit {
    limiter: &'static HostImportThreadLimiter,
}

impl HostImportThreadLimiter {
    fn new() -> Self {
        Self {
            state: Mutex::new(HostImportThreadState { active: 0 }),
            released: Condvar::new(),
        }
    }

    fn acquire(&'static self, timeout: Duration) -> Option<HostImportThreadPermit> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if timeout.is_zero() {
            while state.active >= MAX_CONCURRENT_HOST_IMPORT_THREADS {
                state = self
                    .released
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            state.active += 1;
            return Some(HostImportThreadPermit { limiter: self });
        }

        let deadline = Instant::now().checked_add(timeout)?;
        while state.active >= MAX_CONCURRENT_HOST_IMPORT_THREADS {
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let (next_state, wait_result) = self
                .released
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next_state;
            if wait_result.timed_out() && state.active >= MAX_CONCURRENT_HOST_IMPORT_THREADS {
                return None;
            }
        }
        state.active += 1;
        Some(HostImportThreadPermit { limiter: self })
    }

    fn release(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.active = state.active.saturating_sub(1);
        self.released.notify_one();
    }
}

impl Drop for HostImportThreadPermit {
    fn drop(&mut self) {
        self.limiter.release();
    }
}

static HOST_IMPORT_THREAD_LIMITER: OnceLock<HostImportThreadLimiter> = OnceLock::new();

fn host_import_thread_limiter() -> &'static HostImportThreadLimiter {
    HOST_IMPORT_THREAD_LIMITER.get_or_init(HostImportThreadLimiter::new)
}

fn run_sync_host_import<T, E, F>(
    timeout: Duration,
    operation: F,
) -> Result<T, HostImportCallError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: FnOnce() -> Result<T, E> + Send + 'static,
{
    if timeout.is_zero() {
        return operation().map_err(HostImportCallError::Operation);
    }
    let Some(permit) = host_import_thread_limiter().acquire(timeout) else {
        return Err(HostImportCallError::TimedOut);
    };

    let (sender, receiver) = mpsc::sync_channel(1);
    if std::thread::Builder::new()
        .name("ironclaw-wasm-host-import".to_string())
        .spawn(move || {
            let _permit = permit;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
                .map_err(|_| HostImportCallError::Panicked)
                .and_then(|result| result.map_err(HostImportCallError::Operation));
            let _ = sender.send(result);
        })
        .is_err()
    {
        return Err(HostImportCallError::Panicked);
    }

    receiver
        .recv_timeout(timeout)
        .map_err(|_| HostImportCallError::TimedOut)?
}

fn host_import_timeout(caller: &mut Caller<'_, RuntimeStoreData>) -> Option<Duration> {
    caller.data_mut().remaining_host_import_timeout()
}

fn record_timeout_and_return_code(caller: &mut Caller<'_, RuntimeStoreData>) -> i32 {
    caller.data_mut().record_host_import_timeout();
    HOST_IMPORT_TIMEOUT_CODE
}

fn host_fs_read_utf8(
    caller: &mut Caller<'_, RuntimeStoreData>,
    path_ptr: i32,
    path_len: i32,
    out_ptr: i32,
    out_cap: i32,
) -> i32 {
    let Ok(path) = read_guest_utf8(caller, path_ptr, path_len, MAX_GUEST_PATH_BYTES) else {
        return -1;
    };
    let Some(filesystem) = caller.data().filesystem.clone() else {
        return -10;
    };
    let Some(timeout) = host_import_timeout(caller) else {
        return record_timeout_and_return_code(caller);
    };
    match run_sync_host_import(timeout, move || filesystem.read_utf8(&path)) {
        Ok(contents) => write_guest_bytes(caller, out_ptr, out_cap, contents.as_bytes()),
        Err(HostImportCallError::Operation(_)) => -11,
        Err(HostImportCallError::TimedOut) => record_timeout_and_return_code(caller),
        Err(HostImportCallError::Panicked) => -11,
    }
}

fn host_fs_write_utf8(
    caller: &mut Caller<'_, RuntimeStoreData>,
    path_ptr: i32,
    path_len: i32,
    data_ptr: i32,
    data_len: i32,
) -> i32 {
    let Ok(path) = read_guest_utf8(caller, path_ptr, path_len, MAX_GUEST_PATH_BYTES) else {
        return -1;
    };
    let Ok(contents) = read_guest_utf8(caller, data_ptr, data_len, MAX_FS_WRITE_BYTES) else {
        return -2;
    };
    let Some(filesystem) = caller.data().filesystem.clone() else {
        return -10;
    };
    let Some(timeout) = host_import_timeout(caller) else {
        return record_timeout_and_return_code(caller);
    };
    match run_sync_host_import(timeout, move || filesystem.write_utf8(&path, &contents)) {
        Ok(()) => 0,
        Err(HostImportCallError::Operation(_)) => -11,
        Err(HostImportCallError::TimedOut) => record_timeout_and_return_code(caller),
        Err(HostImportCallError::Panicked) => -11,
    }
}

fn host_fs_list_utf8(
    caller: &mut Caller<'_, RuntimeStoreData>,
    path_ptr: i32,
    path_len: i32,
    out_ptr: i32,
    out_cap: i32,
) -> i32 {
    let Ok(path) = read_guest_utf8(caller, path_ptr, path_len, MAX_GUEST_PATH_BYTES) else {
        return -1;
    };
    let Some(filesystem) = caller.data().filesystem.clone() else {
        return -10;
    };
    let Some(timeout) = host_import_timeout(caller) else {
        return record_timeout_and_return_code(caller);
    };
    match run_sync_host_import(timeout, move || filesystem.list_utf8(&path)) {
        Ok(contents) => write_guest_bytes(caller, out_ptr, out_cap, contents.as_bytes()),
        Err(HostImportCallError::Operation(_)) => -11,
        Err(HostImportCallError::TimedOut) => record_timeout_and_return_code(caller),
        Err(HostImportCallError::Panicked) => -11,
    }
}

fn host_fs_stat_len(
    caller: &mut Caller<'_, RuntimeStoreData>,
    path_ptr: i32,
    path_len: i32,
) -> i64 {
    let Ok(path) = read_guest_utf8(caller, path_ptr, path_len, MAX_GUEST_PATH_BYTES) else {
        return -1;
    };
    let Some(filesystem) = caller.data().filesystem.clone() else {
        return -10;
    };
    let Some(timeout) = host_import_timeout(caller) else {
        return record_timeout_and_return_code(caller) as i64;
    };
    match run_sync_host_import(timeout, move || filesystem.stat_len(&path)) {
        Ok(len) => len.min(i64::MAX as u64) as i64,
        Err(HostImportCallError::Operation(_)) => -11,
        Err(HostImportCallError::TimedOut) => record_timeout_and_return_code(caller) as i64,
        Err(HostImportCallError::Panicked) => -11,
    }
}

fn read_guest_utf8(
    caller: &mut Caller<'_, RuntimeStoreData>,
    ptr: i32,
    len: i32,
    max_len: usize,
) -> Result<String, i32> {
    let offset = usize::try_from(ptr).map_err(|_| -1)?;
    let len = usize::try_from(len).map_err(|_| -1)?;
    if len > max_len {
        return Err(-6);
    }
    let Some(memory) = caller
        .get_export("memory")
        .and_then(|item| item.into_memory())
    else {
        return Err(-3);
    };
    let mut bytes = vec![0_u8; len];
    memory.read(&*caller, offset, &mut bytes).map_err(|_| -4)?;
    String::from_utf8(bytes).map_err(|_| -5)
}

fn write_guest_bytes(
    caller: &mut Caller<'_, RuntimeStoreData>,
    out_ptr: i32,
    out_cap: i32,
    bytes: &[u8],
) -> i32 {
    let Ok(offset) = usize::try_from(out_ptr) else {
        return -1;
    };
    let Ok(capacity) = usize::try_from(out_cap) else {
        return -1;
    };
    if bytes.len() > capacity {
        return -6;
    }
    let Some(memory) = caller
        .get_export("memory")
        .and_then(|item| item.into_memory())
    else {
        return -3;
    };
    if memory.write(caller, offset, bytes).is_err() {
        return -4;
    }
    i32::try_from(bytes.len()).unwrap_or(-6)
}

struct HttpImportArgs {
    method: i32,
    url_ptr: i32,
    url_len: i32,
    body_ptr: i32,
    body_len: i32,
    out_ptr: i32,
    out_cap: i32,
}

fn host_http_request_utf8(caller: &mut Caller<'_, RuntimeStoreData>, args: HttpImportArgs) -> i32 {
    let Some(method) = network_method_from_i32(args.method) else {
        return -1;
    };
    let Ok(url) = read_guest_utf8(caller, args.url_ptr, args.url_len, MAX_HTTP_URL_BYTES) else {
        return -2;
    };
    let body = if args.body_len == 0 {
        String::new()
    } else {
        match read_guest_utf8(caller, args.body_ptr, args.body_len, MAX_HTTP_BODY_BYTES) {
            Ok(body) => body,
            Err(_) => return -3,
        }
    };
    let Some(http) = caller.data().http.clone() else {
        return -10;
    };
    let request_body_bytes = body.len() as u64;
    let Some(timeout) = host_import_timeout(caller) else {
        return record_timeout_and_return_code(caller);
    };
    let request = WasmHttpRequest {
        method,
        url,
        body,
        resolved_ip: None,
        max_response_bytes: None,
    };
    match run_sync_host_import(timeout, move || http.request_utf8(request)) {
        Ok(response) => {
            caller.data_mut().record_network_bytes(request_body_bytes);
            write_guest_bytes(caller, args.out_ptr, args.out_cap, response.body.as_bytes())
        }
        Err(HostImportCallError::Operation(error)) => {
            if error.bytes_received > 0 {
                caller.data_mut().record_network_bytes(request_body_bytes);
            }
            -11
        }
        Err(HostImportCallError::TimedOut) => record_timeout_and_return_code(caller),
        Err(HostImportCallError::Panicked) => -11,
    }
}

fn network_method_from_i32(value: i32) -> Option<NetworkMethod> {
    Some(match value {
        0 => NetworkMethod::Get,
        1 => NetworkMethod::Post,
        2 => NetworkMethod::Put,
        3 => NetworkMethod::Patch,
        4 => NetworkMethod::Delete,
        5 => NetworkMethod::Head,
        _ => return None,
    })
}

fn network_target_for_url(raw: &str) -> Result<NetworkTarget, String> {
    let url = url::Url::parse(raw).map_err(|error| error.to_string())?;
    let scheme = match url.scheme() {
        "http" => NetworkScheme::Http,
        "https" => NetworkScheme::Https,
        other => return Err(format!("unsupported URL scheme {other}")),
    };
    let host = url
        .host_str()
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| "URL host is required".to_string())?
        .to_ascii_lowercase();
    Ok(NetworkTarget {
        scheme,
        host,
        port: url.port(),
    })
}

fn validate_network_target<R>(
    policy: &NetworkPolicy,
    target: &NetworkTarget,
    resolver: &R,
) -> Result<IpAddr, String>
where
    R: WasmNetworkResolver,
{
    if !network_policy_allows(policy, target) {
        return Err("network target denied by policy".to_string());
    }
    validate_private_ip_policy(policy, target, resolver)
}

fn network_policy_allows(policy: &NetworkPolicy, target: &NetworkTarget) -> bool {
    if policy.allowed_targets.is_empty() {
        return false;
    }
    if policy.deny_private_ip_ranges
        && let Ok(ip) = target.host.parse::<IpAddr>()
        && is_private_or_loopback_ip(ip)
    {
        return false;
    }
    policy
        .allowed_targets
        .iter()
        .any(|pattern| target_matches_pattern(target, pattern))
}

fn validate_private_ip_policy<R>(
    policy: &NetworkPolicy,
    target: &NetworkTarget,
    resolver: &R,
) -> Result<IpAddr, String>
where
    R: WasmNetworkResolver,
{
    let resolved_ips = if let Ok(ip) = target.host.parse::<IpAddr>() {
        vec![ip]
    } else {
        let port = Some(target.port.unwrap_or_else(|| default_port(target.scheme)));
        resolver.resolve_ips(&target.host, port)?
    };
    if resolved_ips.is_empty() {
        return Err("network target did not resolve to any IP addresses".to_string());
    }
    if policy.deny_private_ip_ranges && resolved_ips.iter().copied().any(is_private_or_loopback_ip)
    {
        return Err("network target resolves to a private or loopback IP".to_string());
    }
    resolved_ips
        .into_iter()
        .next()
        .ok_or_else(|| "network target did not resolve to any IP addresses".to_string())
}

fn default_port(scheme: NetworkScheme) -> u16 {
    match scheme {
        NetworkScheme::Http => 80,
        NetworkScheme::Https => 443,
    }
}

fn target_matches_pattern(target: &NetworkTarget, pattern: &NetworkTargetPattern) -> bool {
    if let Some(scheme) = pattern.scheme
        && scheme != target.scheme
    {
        return false;
    }
    if let Some(port) = pattern.port
        && Some(port) != target.port
    {
        return false;
    }
    host_matches_pattern(&target.host, &pattern.host_pattern.to_ascii_lowercase())
}

fn host_matches_pattern(host: &str, pattern: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host.ends_with(&format!(".{suffix}")) && host != suffix
    } else {
        host == pattern
    }
}

fn is_private_or_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.octets()[0] == 0
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}
