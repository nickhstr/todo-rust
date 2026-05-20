//! Helpers for the hashed-asset handler: open a file with an immutable
//! Cache-Control, and a 404 builder. The actual routing decision —
//! "manifest hit vs. fall through to ServeDir" — lives in `router.rs`
//! because it needs access to the `ServeDir` service for the fallback.

use std::path::Path;

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode},
    response::Response,
};
use tokio::fs::File;
use tokio_util::io::ReaderStream;

pub async fn serve_immutable_file(path: &Path) -> std::io::Result<Response> {
    let f = File::open(path).await?;
    let mime = mime_guess::from_path(path)
        .first()
        .unwrap_or(mime::APPLICATION_OCTET_STREAM);
    let stream = ReaderStream::new(f);
    let body = Body::from_stream(stream);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        )
        .body(body)
        .expect("response builder"))
}

pub fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::empty())
        .expect("response builder")
}
