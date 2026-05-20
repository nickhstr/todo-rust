mod common;

#[tokio::test]
async fn html_responses_have_private_no_cache() {
    let server = common::spawn().await;
    let res = server
        .client
        .get(format!("{}/login", server.base_url))
        .send()
        .await
        .unwrap();
    let cc = res
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cc.contains("private"), "got: {cc}");
    assert!(cc.contains("no-cache"), "got: {cc}");
}

#[tokio::test]
async fn unhashed_static_assets_are_cacheable() {
    // A vendored JS file is tracked in git, so this works in any clean
    // checkout (unlike static/css/app.css, which Tailwind generates and is
    // gitignored — CI doesn't run Tailwind before the Rust test step).
    let server = common::spawn().await;
    let res = server
        .client
        .get(format!(
            "{}/static/vendor/htmx-4.0.0-beta3.min.js",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let cc = res
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cc.contains("public"), "got: {cc}");
    assert!(cc.contains("max-age=300"), "got: {cc}");
}
