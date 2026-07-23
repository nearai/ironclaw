use std::io::Read;

use ironclaw_extensions::ExtensionAssetPath;
use ironclaw_product_workflow::ProductWorkflowError;

/// Zip-bomb guards for uploaded extension bundles. The HTTP route caps only
/// the compressed body, so these bounds the archive entries and decompressed
/// bytes held during validation.
pub(crate) const MAX_EXTENSION_BUNDLE_FILES: usize = 512;
pub(crate) const MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

/// Decode an uploaded extension bundle while enforcing the complete archive
/// boundary: entry count, safe relative paths, duplicate names, and the
/// decompressed byte budget. The returned entries are the exact bytes later
/// validated and materialized by the import path.
pub(crate) fn unzip_extension_bundle(
    bundle: &[u8],
) -> Result<Vec<(String, Vec<u8>)>, ProductWorkflowError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bundle)).map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("uploaded tool bundle is not a valid zip: {error}"),
        }
    })?;
    if archive.len() > MAX_EXTENSION_BUNDLE_FILES {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!(
                "uploaded tool bundle contains too many archive entries (limit {MAX_EXTENSION_BUNDLE_FILES})"
            ),
        });
    }
    let mut files = Vec::new();
    let mut seen_names = std::collections::HashSet::new();
    let mut total_bytes = 0usize;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("uploaded tool bundle has a corrupt entry: {error}"),
            }
        })?;
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().to_string();
        ExtensionAssetPath::new(name.clone()).map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("uploaded tool bundle contains an unsafe path: {name}: {error}"),
            }
        })?;
        // Zip archives may legally repeat an entry name. Rejecting duplicates
        // keeps validation and materialization over the same byte set.
        if !seen_names.insert(name.clone()) {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!("uploaded tool bundle contains a duplicate entry: {name}"),
            });
        }
        // Declared zip sizes are attacker-controlled, so bound the actual
        // decompressed stream rather than trusting entry metadata.
        let allowance = MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES - total_bytes;
        let mut bytes = Vec::new();
        entry
            .take(allowance as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
                reason: format!("failed to read `{name}` from the uploaded bundle: {error}"),
            })?;
        if bytes.len() > allowance {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "uploaded tool bundle expands past the {MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES}-byte decompressed limit"
                ),
            });
        }
        total_bytes += bytes.len();
        files.push((name, bytes));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an in-memory zip from `(entry_name, bytes)` pairs for the decoder
    /// boundary tests.
    fn zip_bundle(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;

        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            writer.start_file(*name, options).expect("start zip entry");
            writer.write_all(bytes).expect("write zip entry");
        }
        writer.finish().expect("finish zip").into_inner()
    }

    fn zip_bundle_with_directories(directory_count: usize) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for index in 0..directory_count {
            writer
                .add_directory(format!("assets/directory-{index}/"), options)
                .expect("start zip directory");
        }
        writer.finish().expect("finish zip").into_inner()
    }

    /// The path contract rejects backslash separators instead of normalizing
    /// them into an accepted path shape.
    #[test]
    fn unzip_extension_bundle_rejects_backslash_entry_names() {
        let bundle = zip_bundle(&[("wasm\\module.wasm", b"x".as_slice())]);
        let error = unzip_extension_bundle(&bundle)
            .expect_err("backslash separators must be rejected, not normalized");
        assert!(
            format!("{error}").contains("unsafe path"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn unzip_extension_bundle_rejects_empty_and_dot_path_segments() {
        for name in ["assets//module.wasm", "assets/./module.wasm"] {
            let bundle = zip_bundle(&[(name, b"x".as_slice())]);
            let error = unzip_extension_bundle(&bundle)
                .expect_err("empty and dot path segments must be rejected");
            assert!(
                format!("{error}").contains("unsafe path"),
                "unexpected error for {name}: {error}"
            );
        }
    }

    /// A small compressed upload must not expand past the decompressed cap.
    #[test]
    fn unzip_extension_bundle_caps_total_decompressed_bytes() {
        let oversized = vec![0u8; MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES + 1];
        let bundle = zip_bundle(&[("payload.bin", oversized.as_slice())]);
        assert!(
            bundle.len() < MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES,
            "test premise: the bomb must be small compressed"
        );
        let error = unzip_extension_bundle(&bundle)
            .expect_err("expansion past the decompressed cap must be rejected");
        assert!(
            format!("{error}").contains("expands past"),
            "unexpected error: {error}"
        );
    }

    /// Duplicate archive entries must never reach validation and materialize
    /// as two different byte values for one path. `ZipWriter` refuses a
    /// duplicate, so patch a same-length placeholder into both ZIP headers.
    #[test]
    fn unzip_extension_bundle_never_returns_duplicate_entry_names() {
        let placeholder = zip_bundle(&[
            ("manifest.toml", b"validated".as_slice()),
            ("manifest.tomX", b"materialized".as_slice()),
        ]);
        let needle = b"manifest.tomX";
        let replacement = b"manifest.toml";
        let mut bundle = placeholder;
        let mut patched = 0;
        let mut index = 0;
        while index + needle.len() <= bundle.len() {
            if &bundle[index..index + needle.len()] == needle {
                bundle[index..index + needle.len()].copy_from_slice(replacement);
                patched += 1;
            }
            index += 1;
        }
        assert!(
            patched >= 2,
            "test premise: placeholder name must appear in both ZIP headers; patched {patched} occurrence(s)"
        );
        match unzip_extension_bundle(&bundle) {
            Err(error) => {
                assert!(
                    format!("{error}").contains("duplicate"),
                    "unexpected error: {error}"
                );
            }
            Ok(files) => {
                let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
                assert_eq!(names, vec!["manifest.toml"]);
            }
        }
    }

    /// Directory entries count toward the archive-entry limit even though
    /// only regular files are returned to the importer.
    #[test]
    fn unzip_extension_bundle_caps_total_entry_count() {
        let bundle = zip_bundle_with_directories(MAX_EXTENSION_BUNDLE_FILES + 1);
        let error =
            unzip_extension_bundle(&bundle).expect_err("entry-count flooding must be rejected");
        assert!(
            format!("{error}").contains("too many archive entries"),
            "unexpected error: {error}"
        );
    }
}
