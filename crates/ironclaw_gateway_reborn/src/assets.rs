//! Embedded Reborn gateway static assets.

include!(concat!(env!("OUT_DIR"), "/reborn_assets.rs"));

pub fn asset(path: &str) -> Option<(&'static str, &'static [u8])> {
    let path = match path {
        "" => "index.html",
        other => other,
    };
    asset_bytes(path).map(|body| (content_type_for_path(path), body))
}

pub fn content_type_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "css" => "text/css; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "wasm" => "application/wasm",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
