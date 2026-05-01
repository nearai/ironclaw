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
    /// The supplied sha256 is not a 64-character lowercase hex string —
    /// rejecting these closes a path-traversal channel via a tainted
    /// `legal_documents.sha256` row.
    #[error("invalid sha256 format (expected 64 lowercase hex chars)")]
    InvalidSha256,
}

/// Returns true iff `s` is exactly 64 lowercase hex characters.
///
/// We use this on every blob path construction so a malicious or
/// corrupted DB row can't direct a read at a relative or absolute path
/// outside `<data_dir>/legal/blobs/`.
fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Compute a hex sha256 over the bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

/// Return the directory under the data dir where legal blobs live.
#[allow(dead_code)]
pub fn blobs_root(data_dir: &Path) -> PathBuf {
    data_dir.join("legal").join("blobs")
}

/// Build the relative storage path stored in the DB
/// (`legal/blobs/<prefix>/<sha>`). Centralising this keeps callers from
/// hand-rolling the layout.
///
/// The sha must be exactly 64 lowercase hex characters; anything else
/// returns `BlobError::InvalidSha256`. Validating here is the
/// path-traversal choke point.
pub fn relative_path(sha256: &str) -> Result<PathBuf, BlobError> {
    if !is_valid_sha256_hex(sha256) {
        return Err(BlobError::InvalidSha256);
    }
    let prefix = &sha256[0..2];
    Ok(PathBuf::from("legal")
        .join("blobs")
        .join(prefix)
        .join(sha256))
}

/// Resolve the relative blob path against a data dir.
pub fn absolute_path(data_dir: &Path, sha256: &str) -> Result<PathBuf, BlobError> {
    Ok(data_dir.join(relative_path(sha256)?))
}

/// Write bytes to the content-addressed blob path, creating directories as
/// needed. If the file already exists it is left untouched (content is, by
/// definition, identical for the same sha).
///
/// Returns the relative path (the value to persist in
/// `legal_documents.storage_path`).
///
/// Race-safe: two concurrent uploads of the same sha use unique tempfile
/// names (random suffix + pid), so they never clobber each other's
/// in-flight write. A second `rename` may fail because the destination
/// already exists from the first writer; we treat that as success because
/// content-addressing means both writers had byte-identical data.
pub async fn write_blob(data_dir: &Path, sha256: &str, bytes: &[u8]) -> Result<PathBuf, BlobError> {
    let abs = absolute_path(data_dir, sha256)?;
    let rel = relative_path(sha256)?;
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if tokio::fs::try_exists(&abs).await? {
        return Ok(rel);
    }

    // Unique tempfile per writer: random 8 bytes hex + pid, in the same
    // directory as the destination so `rename` is atomic on POSIX.
    let mut rand_suffix = [0u8; 8];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut rand_suffix);
    let tmp_name = format!(
        "{}.{}.{}.tmp",
        sha256,
        std::process::id(),
        hex_short(&rand_suffix),
    );
    let tmp = abs
        .parent()
        .map(|p| p.join(&tmp_name))
        .unwrap_or_else(|| PathBuf::from(&tmp_name));

    tokio::fs::write(&tmp, bytes).await?;
    match tokio::fs::rename(&tmp, &abs).await {
        Ok(()) => Ok(rel),
        Err(e) => {
            // If the destination now exists (a concurrent writer beat
            // us), treat that as success — the bytes are the same.
            if tokio::fs::try_exists(&abs).await.unwrap_or(false) {
                let _ = tokio::fs::remove_file(&tmp).await;
                Ok(rel)
            } else {
                let _ = tokio::fs::remove_file(&tmp).await;
                Err(BlobError::Io(e))
            }
        }
    }
}

fn hex_short(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Read a blob from disk. Used by `GET /skills/legal/documents/:id/blob`.
///
/// The sha is validated as 64 lowercase hex chars before any path
/// construction; an invalid sha (e.g. an attacker who managed to write a
/// crafted row into `legal_documents.sha256` with a relative-traversal
/// payload) returns `BlobError::InvalidSha256` rather than reaching the
/// filesystem.
pub async fn read_blob(data_dir: &Path, sha256: &str) -> Result<Vec<u8>, BlobError> {
    let abs = absolute_path(data_dir, sha256)?;
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
    fn relative_path_uses_sha_prefix_for_valid_input() {
        let valid = "ab".repeat(32); // 64 hex chars
        let p = relative_path(&valid).expect("valid sha");
        assert_eq!(
            p,
            PathBuf::from("legal").join("blobs").join("ab").join(&valid)
        );
    }

    #[test]
    fn relative_path_rejects_short_sha() {
        let err = relative_path("abc").unwrap_err();
        assert!(matches!(err, BlobError::InvalidSha256));
    }

    #[test]
    fn relative_path_rejects_uppercase_hex() {
        let bad = "AB".repeat(32);
        let err = relative_path(&bad).unwrap_err();
        assert!(matches!(err, BlobError::InvalidSha256));
    }

    #[test]
    fn relative_path_rejects_path_traversal_in_sha() {
        // A tainted DB row containing `../../etc/passwd` must not be
        // turned into a real path.
        let err = relative_path("../../etc/passwd").unwrap_err();
        assert!(matches!(err, BlobError::InvalidSha256));
    }

    #[test]
    fn relative_path_rejects_non_hex_chars() {
        let bad = "z".repeat(64);
        let err = relative_path(&bad).unwrap_err();
        assert!(matches!(err, BlobError::InvalidSha256));
    }

    #[tokio::test]
    async fn write_then_read_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = b"hello world".to_vec();
        let sha = sha256_hex(&bytes);

        let rel = write_blob(dir.path(), &sha, &bytes).await.expect("write");
        assert_eq!(rel, relative_path(&sha).expect("valid sha"));

        let got = read_blob(dir.path(), &sha).await.expect("read");
        assert_eq!(got, bytes);

        // Re-writing identical bytes is a no-op (file already exists path).
        let rel2 = write_blob(dir.path(), &sha, &bytes)
            .await
            .expect("re-write");
        assert_eq!(rel2, rel);
    }

    #[tokio::test]
    async fn read_blob_rejects_traversal_sha() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = read_blob(dir.path(), "../../etc/passwd").await.unwrap_err();
        assert!(matches!(err, BlobError::InvalidSha256));
    }
}
