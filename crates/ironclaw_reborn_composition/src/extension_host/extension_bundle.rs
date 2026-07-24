use ironclaw_product::ProductSurfaceFailure;

pub(crate) use ironclaw_extension_host::{
    MAX_EXTENSION_BUNDLE_FILES, MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES,
};

pub(crate) fn unzip_extension_bundle(
    bundle: &[u8],
) -> Result<Vec<(String, Vec<u8>)>, ProductSurfaceFailure> {
    ironclaw_extension_host::unzip_extension_bundle(bundle).map_err(|error| {
        ProductSurfaceFailure::InvalidBindingRequest {
            reason: error.reason().to_string(),
        }
    })
}
