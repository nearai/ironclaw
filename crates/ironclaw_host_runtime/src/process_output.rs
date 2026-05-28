use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ironclaw_host_api::ResourceScope;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::{RuntimeProcessError, sandbox_process::RebornSandboxScopeKey};

/// Maximum model-facing process output preview before middle truncation.
pub(crate) const COMMAND_MAX_OUTPUT_SIZE: usize = 64 * 1024;
const COMMAND_PREVIEW_HALF_SIZE: usize = COMMAND_MAX_OUTPUT_SIZE / 2;
const COMMAND_MAX_SAVED_STREAM_SIZE: usize = 16 * 1024 * 1024;
const COMMAND_OUTPUT_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const COMMAND_OUTPUT_TEMP_PREFIX: &str = "ironclaw-command-output-";
const COMMAND_OUTPUT_SCRATCH_PREFIX: &str = "ironclaw-command-output-scratch-";
/// Root sub-directory under the OS temp dir that holds per-scope saved-output
/// directories. Each principal gets its own immediate child directory (named
/// by the [`RebornSandboxScopeKey`] digest) created with owner-only `0o700`
/// permissions, so saved output files from different tenants/users/projects
/// live in disjoint, non-enumerable directories.
const COMMAND_OUTPUT_ROOT_DIRNAME: &str = "ironclaw-command-outputs";
const COMMAND_OUTPUT_BLOCKED_MARKER: &str =
    "[Full command output blocked due to potential secret leakage]\n";
const STREAM_READ_BUF_SIZE: usize = 16 * 1024;

/// Metadata for full process output persisted outside the model preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCommandOutput {
    pub path: PathBuf,
    pub sanitization: SavedCommandOutputSanitization,
    pub stream_was_capped: bool,
    pub max_saved_stream_size: usize,
    pub expires_at_unix_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedCommandOutputSanitization {
    Clean,
    Redacted,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedCommandOutput {
    pub(crate) preview: String,
    /// Metadata exists only when full output was persisted behind a saved-output
    /// ref. Small inline output still passes through the same sanitizer, but
    /// its sanitization state is represented by `preview` alone.
    pub(crate) saved_output: Option<SavedCommandOutput>,
}

#[derive(Debug, Default)]
pub(crate) struct StreamCapture {
    storage: StreamStorage,
    pub(crate) was_capped: bool,
}

impl StreamCapture {
    fn has_output(&self) -> bool {
        match &self.storage {
            StreamStorage::Inline(output) => !output.is_empty(),
            StreamStorage::Saved(_) => true,
        }
    }

    fn inline_output(&self) -> &[u8] {
        match &self.storage {
            StreamStorage::Inline(output) => output,
            StreamStorage::Saved(_) => &[],
        }
    }

    fn saved_path(&self) -> Option<&Path> {
        match &self.storage {
            StreamStorage::Inline(_) => None,
            StreamStorage::Saved(path) => Some(path.as_path()),
        }
    }
}

#[derive(Debug)]
enum StreamStorage {
    Inline(Vec<u8>),
    Saved(ScratchOutputPath),
}

impl Default for StreamStorage {
    fn default() -> Self {
        Self::Inline(Vec::new())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ScratchOutputPath(PathBuf);

impl ScratchOutputPath {
    fn as_path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchOutputPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl From<PathBuf> for ScratchOutputPath {
    fn from(path: PathBuf) -> Self {
        Self(path)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PreviewBytes {
    total_len: usize,
    head: Vec<u8>,
    tail: Vec<u8>,
}

impl PreviewBytes {
    fn push(&mut self, bytes: &[u8]) {
        self.total_len = self.total_len.saturating_add(bytes.len());
        if self.head.len() < COMMAND_PREVIEW_HALF_SIZE {
            let remaining = COMMAND_PREVIEW_HALF_SIZE - self.head.len();
            self.head
                .extend_from_slice(&bytes[..bytes.len().min(remaining)]);
        }
        self.tail.extend_from_slice(bytes);
        if self.tail.len() > COMMAND_PREVIEW_HALF_SIZE {
            let excess = self.tail.len() - COMMAND_PREVIEW_HALF_SIZE;
            self.tail.drain(..excess);
        }
    }

    fn render(&self) -> String {
        if self.total_len <= COMMAND_MAX_OUTPUT_SIZE {
            return String::from_utf8_lossy(&self.head).to_string();
        }
        format!(
            "{}\n\n... [truncated {} bytes] ...\n\n{}",
            String::from_utf8_lossy(&self.head),
            self.total_len - COMMAND_MAX_OUTPUT_SIZE,
            String::from_utf8_lossy(&self.tail)
        )
    }
}

pub(crate) async fn read_stream_capped<R>(
    scope: &ResourceScope,
    mut stream: R,
) -> Result<StreamCapture, RuntimeProcessError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut capture = StreamCapture::default();
    let mut saved_file: Option<tokio::fs::File> = None;
    let mut saved_bytes = 0usize;
    let mut buf = [0u8; STREAM_READ_BUF_SIZE];

    loop {
        let read = stream
            .read(&mut buf)
            .await
            .map_err(file_error("read command output stream"))?;
        if read == 0 {
            break;
        }
        let chunk = &buf[..read];

        if let StreamStorage::Inline(output) = &mut capture.storage
            && output.len().saturating_add(chunk.len()) <= COMMAND_MAX_OUTPUT_SIZE
        {
            output.extend_from_slice(chunk);
            continue;
        }

        if saved_file.is_none() {
            cleanup_stale_command_outputs_async(scope).await;
            let (path, file) = create_private_temp_file(scope, COMMAND_OUTPUT_SCRATCH_PREFIX)?;
            let mut file = tokio::fs::File::from_std(file);
            if let StreamStorage::Inline(output) = &capture.storage {
                if let Err(error) = file
                    .write_all(output)
                    .await
                    .map_err(file_error("write saved stream output"))
                {
                    let _ = fs::remove_file(path);
                    return Err(error);
                }
                saved_bytes = output.len();
            }
            capture.storage = StreamStorage::Saved(ScratchOutputPath(path));
            saved_file = Some(file);
        }

        let Some(file) = saved_file.as_mut() else {
            continue;
        };
        if saved_bytes >= COMMAND_MAX_SAVED_STREAM_SIZE {
            capture.was_capped = true;
            continue;
        }
        let remaining = COMMAND_MAX_SAVED_STREAM_SIZE - saved_bytes;
        let to_write = chunk.len().min(remaining);
        file.write_all(&chunk[..to_write])
            .await
            .map_err(file_error("write saved stream output"))?;
        saved_bytes += to_write;
        if to_write < chunk.len() {
            capture.was_capped = true;
        }
    }

    if let Some(mut file) = saved_file {
        file.flush()
            .await
            .map_err(file_error("flush saved stream output"))?;
    }
    Ok(capture)
}

pub(crate) fn capture_command_output(
    scope: &ResourceScope,
    stdout: StreamCapture,
    stderr: StreamCapture,
) -> Result<CapturedCommandOutput, RuntimeProcessError> {
    if !requires_saved_output(&stdout, &stderr) {
        let preview = sanitize_preview_bytes(&combined_inline_output(&stdout, &stderr));
        return Ok(CapturedCommandOutput {
            preview,
            saved_output: None,
        });
    }

    let mut preview = PreviewBytes::default();
    (|| {
        let raw_output = materialize_combined_output(scope, &stdout, &stderr, &mut preview)?;
        finalize_saved_output(
            scope,
            &raw_output,
            stdout.was_capped || stderr.was_capped,
            preview,
        )
        .map(|(saved_output, preview)| CapturedCommandOutput {
            preview,
            saved_output: Some(saved_output),
        })
    })()
}

fn requires_saved_output(stdout: &StreamCapture, stderr: &StreamCapture) -> bool {
    stdout.saved_path().is_some()
        || stderr.saved_path().is_some()
        || stdout.was_capped
        || stderr.was_capped
        || combined_inline_output_len(stdout, stderr) > COMMAND_MAX_OUTPUT_SIZE
}

fn combined_inline_output(stdout: &StreamCapture, stderr: &StreamCapture) -> Vec<u8> {
    let mut output = Vec::with_capacity(combined_inline_output_len(stdout, stderr));
    output.extend_from_slice(stdout.inline_output());
    if !stdout.inline_output().is_empty() && !stderr.inline_output().is_empty() {
        output.extend_from_slice(b"\n\n--- stderr ---\n");
    }
    output.extend_from_slice(stderr.inline_output());
    output
}

fn combined_inline_output_len(stdout: &StreamCapture, stderr: &StreamCapture) -> usize {
    stdout.inline_output().len()
        + stderr.inline_output().len()
        + if !stdout.inline_output().is_empty() && !stderr.inline_output().is_empty() {
            b"\n\n--- stderr ---\n".len()
        } else {
            0
        }
}

fn materialize_combined_output(
    scope: &ResourceScope,
    stdout: &StreamCapture,
    stderr: &StreamCapture,
    preview: &mut PreviewBytes,
) -> Result<ScratchOutputPath, RuntimeProcessError> {
    let (path, mut file) = create_private_temp_file(scope, COMMAND_OUTPUT_SCRATCH_PREFIX)?;
    let scratch = ScratchOutputPath::from(path);
    let result = (|| {
        append_stream(stdout, &mut file, preview)?;
        if stdout.has_output() && stderr.has_output() {
            append_bytes(b"\n\n--- stderr ---\n", &mut file, preview)?;
        }
        append_stream(stderr, &mut file, preview)
    })();
    result?;
    Ok(scratch)
}

fn append_stream(
    stream: &StreamCapture,
    output: &mut File,
    preview: &mut PreviewBytes,
) -> Result<(), RuntimeProcessError> {
    if let Some(path) = stream.saved_path() {
        let mut file = File::open(path).map_err(file_error("open saved stream output"))?;
        let mut buf = [0u8; STREAM_READ_BUF_SIZE];
        loop {
            let read = file
                .read(&mut buf)
                .map_err(file_error("read saved stream output"))?;
            if read == 0 {
                break;
            }
            append_bytes(&buf[..read], output, preview)?;
        }
        return Ok(());
    }
    append_bytes(stream.inline_output(), output, preview)
}

fn append_bytes(
    bytes: &[u8],
    output: &mut File,
    preview: &mut PreviewBytes,
) -> Result<(), RuntimeProcessError> {
    output
        .write_all(bytes)
        .map_err(file_error("write saved command output"))?;
    preview.push(bytes);
    Ok(())
}

fn finalize_saved_output(
    scope: &ResourceScope,
    raw_path: &ScratchOutputPath,
    stream_was_capped: bool,
    raw_preview: PreviewBytes,
) -> Result<(SavedCommandOutput, String), RuntimeProcessError> {
    // Each stream is capped before materialization, so this read is bounded to
    // stdout + stderr plus the separator: about 2 * COMMAND_MAX_SAVED_STREAM_SIZE.
    // LeakDetector is string-oriented today; keep the bound explicit until it
    // supports streaming or byte-span sanitization.
    let content = fs::read(raw_path.as_path())
        .map_err(file_error("read saved command output for sanitization"))?;
    let sanitized = sanitize_command_output_bytes(&content, raw_preview.render());
    let final_path = if let Some(saved_replacement) = sanitized.saved_replacement.as_deref() {
        write_final_saved_output(scope, saved_replacement.as_bytes())?
    } else {
        write_final_saved_output(scope, &content)?
    };
    let saved_output = SavedCommandOutput {
        path: final_path,
        sanitization: sanitized.sanitization,
        stream_was_capped,
        max_saved_stream_size: COMMAND_MAX_SAVED_STREAM_SIZE,
        expires_at_unix_secs: expires_at_unix_secs(),
    };
    Ok((saved_output, sanitized.preview))
}

pub(crate) fn saved_output_filename(saved_output: &SavedCommandOutput) -> String {
    saved_output
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unavailable-saved-output")
        .to_string()
}

fn write_final_saved_output(
    scope: &ResourceScope,
    content: &[u8],
) -> Result<PathBuf, RuntimeProcessError> {
    let (path, mut file) = create_private_temp_file(scope, COMMAND_OUTPUT_TEMP_PREFIX)?;
    if let Err(error) = file.write_all(content) {
        let _ = fs::remove_file(&path);
        return Err(file_error("write sanitized command output")(error));
    }
    Ok(path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SanitizedCommandOutput {
    preview: String,
    sanitization: SavedCommandOutputSanitization,
    saved_replacement: Option<String>,
}

fn sanitize_preview_bytes(bytes: &[u8]) -> String {
    sanitize_command_output_bytes(bytes, String::from_utf8_lossy(bytes).to_string()).preview
}

fn sanitize_command_output_bytes(content: &[u8], raw_preview: String) -> SanitizedCommandOutput {
    let content_text = String::from_utf8_lossy(content);
    let detector = ironclaw_safety::LeakDetector::new();
    let (preview, saved_replacement, sanitization) = match detector.scan_and_clean(&content_text) {
        Ok(cleaned) => {
            if cleaned == content_text {
                (raw_preview, None, SavedCommandOutputSanitization::Clean)
            } else {
                let mut preview = PreviewBytes::default();
                preview.push(cleaned.as_bytes());
                (
                    preview.render(),
                    Some(cleaned),
                    SavedCommandOutputSanitization::Redacted,
                )
            }
        }
        Err(_) => (
            COMMAND_OUTPUT_BLOCKED_MARKER.to_string(),
            Some(COMMAND_OUTPUT_BLOCKED_MARKER.to_string()),
            SavedCommandOutputSanitization::Blocked,
        ),
    };
    SanitizedCommandOutput {
        preview,
        sanitization,
        saved_replacement,
    }
}

async fn cleanup_stale_command_outputs_async(scope: &ResourceScope) {
    let scope = scope.clone();
    let _ = tokio::task::spawn_blocking(move || cleanup_stale_command_outputs(&scope)).await;
}

/// Walk only the current scope's saved-output directory and remove files older
/// than [`COMMAND_OUTPUT_RETENTION`]. Scoping the scan to the per-tenant
/// directory (created with owner-only `0o700`) means a command running under
/// scope A cannot enumerate or unlink any file belonging to scope B, even on
/// a shared temp-dir host — the cross-principal-delete surface closes by
/// construction.
fn cleanup_stale_command_outputs(scope: &ResourceScope) {
    let scope_dir = scoped_output_dir_path(scope);
    let Ok(entries) = fs::read_dir(&scope_dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with(COMMAND_OUTPUT_TEMP_PREFIX)
            && !name.starts_with(COMMAND_OUTPUT_SCRATCH_PREFIX)
        {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if now
            .duration_since(modified)
            .is_ok_and(|age| age > COMMAND_OUTPUT_RETENTION)
        {
            let _ = fs::remove_file(path);
        }
    }
}

/// Return the tenant/user/(agent)/project-scoped directory that holds this
/// scope's saved-output files. The path layout — `<tempdir>/ironclaw-command-
/// outputs/<scope_digest>/` — matches the [`RebornSandboxScopeKey`] convention
/// already used by `RebornScopedSandboxCommandTransport::prepare_workspace`
/// for sandbox workspaces (`<root>/scopes/<digest>`), so isolation guarantees
/// for two distinct (tenant, user, agent, project) tuples are inherited from
/// the SHA-256 digest's collision properties: distinct scope tuples produce
/// distinct digests and therefore disjoint, non-overlapping directories.
pub(crate) fn scoped_output_dir_path(scope: &ResourceScope) -> PathBuf {
    let digest = RebornSandboxScopeKey::from_scope(scope).workspace_path(Path::new(""));
    // `workspace_path` returns `scopes/<digest>`; the digest is the final
    // component. Pull it out directly to avoid leaking the `scopes/` prefix
    // into the unrelated saved-output namespace.
    let digest_component = digest
        .file_name()
        .map(|name| name.to_owned())
        .unwrap_or_else(|| std::ffi::OsString::from("unknown-scope"));
    std::env::temp_dir()
        .join(COMMAND_OUTPUT_ROOT_DIRNAME)
        .join(digest_component)
}

/// Create the per-scope output directory if it does not yet exist and tighten
/// permissions to owner-only on unix. The directory itself becomes the tenant
/// isolation boundary: another principal on the same host cannot list, read,
/// or unlink files inside it.
fn ensure_scoped_output_dir(scope: &ResourceScope) -> Result<PathBuf, RuntimeProcessError> {
    let dir = scoped_output_dir_path(scope);
    fs::create_dir_all(&dir).map_err(file_error("create saved command output directory"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best-effort tightening; if metadata/set_permissions fails the file
        // open below still enforces 0o600 on the file itself.
        if let Ok(metadata) = fs::metadata(&dir) {
            let mut perms = metadata.permissions();
            if perms.mode() & 0o777 != 0o700 {
                perms.set_mode(0o700);
                let _ = fs::set_permissions(&dir, perms);
            }
        }
    }
    Ok(dir)
}

fn create_private_temp_file(
    scope: &ResourceScope,
    prefix: &str,
) -> Result<(PathBuf, File), RuntimeProcessError> {
    let dir = ensure_scoped_output_dir(scope)?;
    let path = dir.join(format!("{prefix}{}.log", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(file_error("create saved command output"))?;
    Ok((path, file))
}

fn file_error(action: &'static str) -> impl Fn(std::io::Error) -> RuntimeProcessError {
    move |error| RuntimeProcessError::ExecutionFailed(format!("Failed to {action}: {error}"))
}

fn expires_at_unix_secs() -> u64 {
    SystemTime::now()
        .checked_add(COMMAND_OUTPUT_RETENTION)
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs())
}

pub(crate) fn truncate_output(s: &str) -> String {
    if s.len() <= COMMAND_MAX_OUTPUT_SIZE {
        s.to_string()
    } else {
        let half = COMMAND_MAX_OUTPUT_SIZE / 2;
        let head_end = floor_char_boundary(s, half);
        let tail_start = floor_char_boundary(s, s.len() - half);
        format!(
            "{}\n\n... [truncated {} bytes] ...\n\n{}",
            &s[..head_end],
            s.len() - COMMAND_MAX_OUTPUT_SIZE,
            &s[tail_start..]
        )
    }
}

fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{InvocationId, ResourceScope, UserId};

    fn test_scope() -> ResourceScope {
        ResourceScope::local_default(UserId::new("test-user").unwrap(), InvocationId::new())
            .expect("local_default scope")
    }

    fn test_scope_with(tenant: &str, user: &str, project: Option<&str>) -> ResourceScope {
        use ironclaw_host_api::{AgentId, ProjectId, TenantId};
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: Some(AgentId::new("agent").unwrap()),
            project_id: project.map(|p| ProjectId::new(p).unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    #[test]
    fn truncate_output_preserves_exact_limit() {
        let output = "x".repeat(COMMAND_MAX_OUTPUT_SIZE);

        assert_eq!(truncate_output(&output), output);
    }

    #[test]
    fn truncate_output_respects_utf8_boundaries() {
        let output = format!(
            "{}{}{}",
            "a".repeat(COMMAND_MAX_OUTPUT_SIZE / 2 - 1),
            "é",
            "b".repeat(COMMAND_MAX_OUTPUT_SIZE)
        );

        let truncated = truncate_output(&output);

        assert!(truncated.is_char_boundary(COMMAND_MAX_OUTPUT_SIZE / 2 - 1));
        assert!(truncated.contains("... [truncated "));
        assert!(truncated.starts_with(&"a".repeat(COMMAND_MAX_OUTPUT_SIZE / 2 - 1)));
        assert!(truncated.ends_with(&"b".repeat(COMMAND_MAX_OUTPUT_SIZE / 2)));
    }

    #[tokio::test]
    async fn read_stream_capped_keeps_large_output_out_of_inline_buffer() {
        let input = "x".repeat(COMMAND_MAX_OUTPUT_SIZE + 1);

        let output = read_stream_capped(&test_scope(), input.as_bytes())
            .await
            .expect("stream capture succeeds");
        let saved_path = output
            .saved_path()
            .expect("saved stream path")
            .to_path_buf();
        let saved = fs::read_to_string(&saved_path).expect("saved stream readable");
        let _ = fs::remove_file(&saved_path);

        assert!(output.inline_output().is_empty());
        assert_eq!(saved.len(), COMMAND_MAX_OUTPUT_SIZE + 1);
        assert!(!output.was_capped);
    }

    #[test]
    fn capture_command_output_preserves_small_output() {
        let stdout = StreamCapture {
            storage: StreamStorage::Inline(b"small output".to_vec()),
            ..StreamCapture::default()
        };
        let output = capture_command_output(&test_scope(), stdout, StreamCapture::default())
            .expect("capture succeeds");

        assert_eq!(output.preview, "small output");
        assert_eq!(output.saved_output, None);
    }

    #[test]
    fn capture_command_output_blocks_secret_like_small_preview() {
        let secret = "sk-proj-test1234567890abcdefghij";
        let stdout = StreamCapture {
            storage: StreamStorage::Inline(secret.as_bytes().to_vec()),
            ..StreamCapture::default()
        };

        let output = capture_command_output(&test_scope(), stdout, StreamCapture::default())
            .expect("capture succeeds");

        assert_eq!(output.preview, COMMAND_OUTPUT_BLOCKED_MARKER);
        assert_eq!(output.saved_output, None);
    }

    #[test]
    fn capture_command_output_saves_large_output_file() {
        let middle = "middle content that current preview would omit";
        let raw = format!(
            "{}{middle}{}",
            "a".repeat(COMMAND_MAX_OUTPUT_SIZE),
            "z".repeat(COMMAND_MAX_OUTPUT_SIZE)
        );
        let stdout = saved_stream_capture(raw.as_bytes());

        let output = capture_command_output(&test_scope(), stdout, StreamCapture::default())
            .expect("capture succeeds");
        let saved_output = output.saved_output.expect("saved output metadata");
        let saved = fs::read_to_string(&saved_output.path).expect("saved output readable");
        let _ = fs::remove_file(&saved_output.path);

        assert!(output.preview.contains("... [truncated "));
        assert!(!output.preview.contains(middle));
        assert_eq!(
            saved_output.sanitization,
            SavedCommandOutputSanitization::Clean
        );
        assert!(!saved_output.stream_was_capped);
        assert_eq!(saved, raw);
    }

    #[test]
    fn capture_command_output_blocks_secret_like_saved_output() {
        let secret = "sk-proj-test1234567890abcdefghij";
        let raw = format!("{}{}", "x".repeat(COMMAND_MAX_OUTPUT_SIZE + 1), secret);
        let stdout = saved_stream_capture(raw.as_bytes());

        let output = capture_command_output(&test_scope(), stdout, StreamCapture::default())
            .expect("capture succeeds");
        let saved_output = output.saved_output.expect("saved output metadata");
        let saved = fs::read_to_string(&saved_output.path).expect("saved output readable");
        let _ = fs::remove_file(&saved_output.path);

        assert_eq!(
            saved_output.sanitization,
            SavedCommandOutputSanitization::Blocked
        );
        assert_eq!(saved, COMMAND_OUTPUT_BLOCKED_MARKER);
        assert!(!saved.contains(secret));
        assert_eq!(output.preview, saved);
    }

    #[test]
    fn capture_command_output_preserves_binary_saved_output() {
        let mut raw = vec![0xff; COMMAND_MAX_OUTPUT_SIZE + 1];
        raw.extend_from_slice(b" clean tail");
        let stdout = saved_stream_capture(&raw);

        let output = capture_command_output(&test_scope(), stdout, StreamCapture::default())
            .expect("capture succeeds");
        let saved_output = output.saved_output.expect("saved output metadata");
        let saved = fs::read(&saved_output.path).expect("saved output readable");
        let _ = fs::remove_file(&saved_output.path);

        assert_eq!(saved, raw);
        assert_eq!(
            saved_output.sanitization,
            SavedCommandOutputSanitization::Clean
        );
    }

    #[cfg(unix)]
    #[test]
    fn saved_output_uses_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let stdout = saved_stream_capture(b"large clean output");
        let output = capture_command_output(&test_scope(), stdout, StreamCapture::default())
            .expect("capture succeeds");
        let saved_output = output.saved_output.expect("saved output metadata");
        let mode = fs::metadata(&saved_output.path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        let _ = fs::remove_file(&saved_output.path);

        assert_eq!(mode, 0o600);
    }

    fn saved_stream_capture(bytes: &[u8]) -> StreamCapture {
        saved_stream_capture_for_scope(&test_scope(), bytes)
    }

    fn saved_stream_capture_for_scope(scope: &ResourceScope, bytes: &[u8]) -> StreamCapture {
        let (path, mut file) =
            create_private_temp_file(scope, COMMAND_OUTPUT_SCRATCH_PREFIX).expect("stream file");
        file.write_all(bytes).expect("write stream file");
        StreamCapture {
            storage: StreamStorage::Saved(ScratchOutputPath(path)),
            was_capped: false,
        }
    }

    #[test]
    fn scoped_output_dir_isolates_distinct_user_project_tuples() {
        // Tenant-isolation regression: two distinct (user, project) tuples
        // (and any change to tenant or agent) must produce disjoint,
        // non-overlapping save directories under the OS temp dir.
        let a = scoped_output_dir_path(&test_scope_with("tenant", "alice", Some("proj-a")));
        let b = scoped_output_dir_path(&test_scope_with("tenant", "bob", Some("proj-a")));
        let c = scoped_output_dir_path(&test_scope_with("tenant", "alice", Some("proj-b")));
        let d = scoped_output_dir_path(&test_scope_with("tenant-other", "alice", Some("proj-a")));

        assert_ne!(a, b, "different users must not share a save dir");
        assert_ne!(a, c, "different projects must not share a save dir");
        assert_ne!(a, d, "different tenants must not share a save dir");
        assert_ne!(b, c);
        assert_ne!(b, d);
        assert_ne!(c, d);

        // And the path stays under the scoped root, never directly in temp.
        let root = std::env::temp_dir().join(COMMAND_OUTPUT_ROOT_DIRNAME);
        assert!(a.starts_with(&root));
        assert!(b.starts_with(&root));
        assert!(c.starts_with(&root));
        assert!(d.starts_with(&root));
        assert_ne!(a.parent(), Some(std::env::temp_dir().as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_scoped_output_dir_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        // Use a unique scope so the directory is fresh.
        let scope = test_scope_with("tenant-perm-test", "u-perm-test", Some("p-perm-test"));
        let dir = ensure_scoped_output_dir(&scope).expect("ensure dir");
        let mode = fs::metadata(&dir).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "scoped save dir must be owner-only");
        let _ = fs::remove_dir_all(&dir);
    }
}
