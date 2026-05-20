//! Helpers for the hashed-asset handler: open a file with an immutable
//! Cache-Control, and a 404 builder. The actual routing decision —
//! "manifest hit vs. fall through to ServeDir" — lives in `router.rs`
//! because it needs access to the `ServeDir` service for the fallback.

use std::path::{Path, PathBuf};

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode},
    response::Response,
};
use tokio::fs::File;
use tokio_util::io::ReaderStream;

/// Open the file at `path` and stream it back with the immutable
/// `Cache-Control`. The `accept_encoding` argument is the raw
/// `Accept-Encoding` header value; if it advertises `br` or `gzip` AND
/// a `<path>.br` / `<path>.gz` sibling exists on disk, the sibling is
/// served with the matching `Content-Encoding`. The advertised
/// `Content-Type` is always derived from the unhashed path's extension,
/// not the compressed sibling's.
pub async fn serve_immutable_file(
    path: &Path,
    accept_encoding: Option<&str>,
) -> std::io::Result<Response> {
    let mime = mime_guess::from_path(path)
        .first()
        .unwrap_or(mime::APPLICATION_OCTET_STREAM);

    let (open_path, encoding) = pick_precompressed(path, accept_encoding);
    let f = File::open(&open_path).await?;
    let stream = ReaderStream::new(f);
    let body = Body::from_stream(stream);

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        )
        // Caches that share this URL across encodings (rare with our
        // hashed-URL scheme, but cheap correctness insurance).
        .header(header::VARY, "Accept-Encoding");
    if let Some(enc) = encoding {
        builder = builder.header(header::CONTENT_ENCODING, enc);
    }
    Ok(builder.body(body).expect("response builder"))
}

/// Prefer `.br` if the client accepts it, then `.gz`, then the raw file.
fn pick_precompressed(
    path: &Path,
    accept_encoding: Option<&str>,
) -> (PathBuf, Option<&'static str>) {
    let accepts = |needle: &str| {
        accept_encoding
            .map(|h| {
                h.split(',')
                    .any(|t| t.trim().split(';').next() == Some(needle))
            })
            .unwrap_or(false)
    };
    if accepts("br") {
        let br = with_extra_ext(path, "br");
        if br.is_file() {
            return (br, Some("br"));
        }
    }
    if accepts("gzip") {
        let gz = with_extra_ext(path, "gz");
        if gz.is_file() {
            return (gz, Some("gzip"));
        }
    }
    (path.to_path_buf(), None)
}

fn with_extra_ext(path: &Path, ext: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

pub fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::empty())
        .expect("response builder")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(p: &Path, body: &[u8]) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }

    #[test]
    fn picks_brotli_when_accepted_and_present() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("app.js");
        write(&raw, b"raw");
        write(&dir.path().join("app.js.br"), b"compressed-br");
        write(&dir.path().join("app.js.gz"), b"compressed-gz");
        let (p, enc) = pick_precompressed(&raw, Some("br, gzip"));
        assert_eq!(enc, Some("br"));
        assert!(p.ends_with("app.js.br"));
    }

    #[test]
    fn picks_gzip_when_brotli_missing() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("app.js");
        write(&raw, b"raw");
        write(&dir.path().join("app.js.gz"), b"compressed-gz");
        let (p, enc) = pick_precompressed(&raw, Some("br, gzip"));
        assert_eq!(enc, Some("gzip"));
        assert!(p.ends_with("app.js.gz"));
    }

    #[test]
    fn falls_back_to_raw_when_no_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("app.js");
        write(&raw, b"raw");
        let (p, enc) = pick_precompressed(&raw, Some("br, gzip"));
        assert_eq!(enc, None);
        assert_eq!(p, raw);
    }

    #[test]
    fn ignores_unaccepted_encodings() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("app.js");
        write(&raw, b"raw");
        write(&dir.path().join("app.js.br"), b"br");
        let (p, enc) = pick_precompressed(&raw, Some("identity"));
        assert_eq!(enc, None);
        assert_eq!(p, raw);
    }
}
