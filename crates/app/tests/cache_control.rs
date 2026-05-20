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
    // static/css/app.css is the Tailwind-compiled stylesheet. It is committed
    // to the repo, so this test will pass as long as the file exists on disk.
    // If the file is absent (e.g. in a clean checkout without running Tailwind),
    // the endpoint returns 404 and the test fails with a clear message.
    let server = common::spawn().await;
    let res = server
        .client
        .get(format!("{}/static/css/app.css", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        200,
        "static/css/app.css must exist on disk; run `tailwindcss` to generate it"
    );
    let cc = res
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cc.contains("public"), "got: {cc}");
    assert!(cc.contains("max-age=300"), "got: {cc}");
}
