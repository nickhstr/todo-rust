use axum::http::{HeaderName, HeaderValue, Request};
use tower_http::request_id::{
    MakeRequestId, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use uuid::Uuid;

const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

#[derive(Clone, Default)]
pub struct UuidV7;

impl MakeRequestId for UuidV7 {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        let id = Uuid::now_v7().to_string();
        HeaderValue::from_str(&id).ok().map(RequestId::new)
    }
}

pub fn make_request_id_layers() -> (SetRequestIdLayer<UuidV7>, PropagateRequestIdLayer) {
    (
        SetRequestIdLayer::new(X_REQUEST_ID, UuidV7),
        PropagateRequestIdLayer::new(X_REQUEST_ID),
    )
}
