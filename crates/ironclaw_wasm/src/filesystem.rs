pub trait WasmHostFilesystem: Send + Sync {
    fn read_utf8(&self, path: &str) -> Result<String, String>;
    fn write_utf8(&self, path: &str, contents: &str) -> Result<(), String>;
    fn list_utf8(&self, path: &str) -> Result<String, String>;
    fn stat_len(&self, path: &str) -> Result<u64, String>;
}

/// Scoped filesystem adapter for WASM filesystem imports.
#[derive(Debug)]
pub struct WasmScopedFilesystem<F> {
    root: Arc<F>,
    mounts: MountView,
    scoped: ScopedFilesystem<F>,
}

impl<F> WasmScopedFilesystem<F>
where
    F: RootFilesystem,
{
    pub fn new(root: Arc<F>, mounts: MountView) -> Self {
        Self {
            scoped: ScopedFilesystem::new(Arc::clone(&root), mounts.clone()),
            root,
            mounts,
        }
    }

    pub fn scoped(&self) -> &ScopedFilesystem<F> {
        &self.scoped
    }

    fn scoped_for_bridge(&self) -> ScopedFilesystem<F> {
        ScopedFilesystem::new(Arc::clone(&self.root), self.mounts.clone())
    }
}

impl<F> WasmHostFilesystem for WasmScopedFilesystem<F>
where
    F: RootFilesystem + 'static,
{
    fn read_utf8(&self, path: &str) -> Result<String, String> {
        let path = ScopedPath::new(path.to_string()).map_err(|error| error.to_string())?;
        let scoped = self.scoped_for_bridge();
        let bytes = run_filesystem_bridge(async move { scoped.read_file(&path).await })?;
        String::from_utf8(bytes).map_err(|error| error.to_string())
    }

    fn write_utf8(&self, path: &str, contents: &str) -> Result<(), String> {
        let path = ScopedPath::new(path.to_string()).map_err(|error| error.to_string())?;
        let contents = contents.as_bytes().to_vec();
        let scoped = self.scoped_for_bridge();
        run_filesystem_bridge(async move { scoped.write_file(&path, &contents).await })
    }

    fn list_utf8(&self, path: &str) -> Result<String, String> {
        let path = ScopedPath::new(path.to_string()).map_err(|error| error.to_string())?;
        let scoped = self.scoped_for_bridge();
        let entries = run_filesystem_bridge(async move { scoped.list_dir(&path).await })?;
        let names = entries
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        serde_json::to_string(&names).map_err(|error| error.to_string())
    }

    fn stat_len(&self, path: &str) -> Result<u64, String> {
        let path = ScopedPath::new(path.to_string()).map_err(|error| error.to_string())?;
        let scoped = self.scoped_for_bridge();
        run_filesystem_bridge(async move { scoped.stat(&path).await }).map(|stat| stat.len)
    }
}

fn run_filesystem_bridge<T, Fut>(future: Fut) -> Result<T, String>
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, FilesystemError>> + Send + 'static,
{
    let Some(permit) = host_import_thread_limiter().acquire(DEFAULT_FILESYSTEM_BRIDGE_TIMEOUT)
    else {
        return Err("filesystem bridge thread budget exhausted".to_string());
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("ironclaw-wasm-fs-bridge".to_string())
        .spawn(move || {
            let _permit = permit;
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("failed to create filesystem bridge runtime: {error}"))
                .and_then(|runtime| runtime.block_on(future).map_err(|error| error.to_string()));
            let _ = sender.send(result);
        })
        .map_err(|error| format!("failed to spawn filesystem bridge thread: {error}"))?;
    receiver
        .recv_timeout(DEFAULT_FILESYSTEM_BRIDGE_TIMEOUT)
        .map_err(|_| "filesystem bridge timed out".to_string())?
}
