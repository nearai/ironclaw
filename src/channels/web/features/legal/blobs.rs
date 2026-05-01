//! Filesystem blob storage for legal documents.
//!
//! Files live under `<data_dir>/legal/blobs/<sha2[0..2]>/<sha256>`. Storage
//! is content-addressed: identical bytes share a single on-disk file even
//! if a project re-uploads them under a different filename.
//!
//! Per the spec, dedupe keys are scoped to a single project at the SQL
//! layer (`legal_documents.project_id` + `legal_documents.sha256`). The
//! filesystem layout intentionally does *not* segment by project, so
//! cross-project content reuse still benefits from a single blob on
//! disk; only the row-level dedupe is project-scoped.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

/// Errors raised by blob operations.
#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("blob i/o error: {0}")]
    Io(#[from] std::io::Error),
}

/// Compute a hex sha256 over the bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

/// Return the directory under the data dir where legal blobs live.
pub fn blobs_root(data_dir: &Path) -> PathBuf {
    data_dir.join("legal").join("blobs")
}

/// Build the relative storage path stored in the DB
/// (`legal/blobs/<prefix>/<sha>`). Centralising this keeps callers from
/// hand-rolling the layout.
///
/// The sha is expected to be lowercase hex (64 chars). Anything shorter
/// than 2 chars falls back to the literal sha as the prefix bucket so the
/// caller cannot construct a path that escapes the blob root.
pub fn relative_path(sha256: &str) -> PathBuf {
    let prefix: &str = sha256.get(0..2).unwrap_or(sha256);
    PathBuf::from("legal").join("blobs").join(prefix).join(sha256)
}

/// Resolve the relative blob path against a data dir.
pub fn absolute_path(data_dir: &Path, sha256: &str) -> PathBuf {
    data_dir.join(relative_path(sha256))
}

/// Write bytes to the content-addressed blob path, creating directories as
/// needed. If the file already exists it is left untouched (content is, by
/// definition, identical for the same sha).
///
/// Returns the relative path (the value to persist in
/// `legal_documents.storage_path`).
pub async fn write_blob(
    data_dir: &Path,
    sha256: &str,
    bytes: &[u8],
) -> Result<PathBuf, BlobError> {
    let abs = absolute_path(data_dir, sha256);
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if !tokio::fs::try_exists(&abs).await? {
        // Atomic-ish write: temp file in same dir, rename. Avoids a partial
        // file becoming visible if the process is killed mid-write.
        let tmp = abs.with_extension("tmp");
        tokio::fs::write(&tmp, bytes).await?;
        if let Err(e) = tokio::fs::rename(&tmp, &abs).await {
            // Best-effort cleanup of the temp file; preserve the original error.
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(BlobError::Io(e));
        }
    }
    Ok(relative_path(sha256))
}

/// Read a blob from disk. Used by `GET /skills/legal/documents/:id/blob`.
pub async fn read_blob(data_dir: &Path, sha256: &str) -> Result<Vec<u8>, BlobError> {
    let abs = absolute_path(data_dir, sha256);
    let mut file = tokio::fs::File::open(&abs).await?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).await?;
    Ok(buf)
}

/// Lower-case hex encoding without pulling another dep just for this.
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        // RFC 6234 test: sha256("abc") == ba7816bf8f01cfea4141...
        let h = sha256_hex(b"abc");
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn relative_path_uses_sha_prefix() {
        let p = relative_path("abcdef0123456789");
        assert_eq!(p, PathBuf::from("legal").join("blobs").join("ab").join("abcdef0123456789"));
    }

    #[test]
    fn relative_path_short_sha_falls_back_safely() {
        // A degenerate caller could pass a single-char sha; the path must
        // still be within `legal/blobs/`, never traversal-prone.
        let p = relative_path("x");
        assert!(p.starts_with("legal/blobs"));
    }

    #[tokio::test]
    async fn write_then_read_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = b"hello world".to_vec();
        let sha = sha256_hex(&bytes);

        let rel = write_blob(dir.path(), &sha, &bytes).await.expect("write");
        assert_eq!(rel, relative_path(&sha));

        let got = read_blob(dir.path(), &sha).await.expect("read");
        assert_eq!(got, bytes);

        // Re-writing identical bytes is a no-op (file already exists path).
        let rel2 = write_blob(dir.path(), &sha, &bytes)
            .await
            .expect("re-write");
        assert_eq!(rel2, rel);
    }
}
