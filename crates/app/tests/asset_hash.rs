mod common;

#[tokio::test]
async fn login_page_references_static_css() {
    // The test harness uses Assets::dev() so asset() returns raw paths.
    // Production hashing is exercised in i18n::assets::tests unit tests.
    let server = common::spawn().await;
    let body = server
        .client
        .get(format!("{}/login", server.base_url))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    // Minijinja auto-escapes the `/` characters in the path, so the href
    // appears as `&#x2f;static&#x2f;css&#x2f;app.css` in the raw HTML.
    // We check for the unescaped suffix that always appears regardless of slash encoding.
    assert!(
        body.contains("css/app.css") || body.contains("css&#x2f;app.css"),
        "asset('css/app.css') should resolve to raw path in dev mode; body head: {}",
        &body[..body.len().min(500)]
    );
}

#[tokio::test]
async fn unknown_static_path_404s() {
    let server = common::spawn().await;
    let res = server
        .client
        .get(format!("{}/static/does-not-exist.png", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}
