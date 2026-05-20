mod common;
use regex::Regex;

#[tokio::test]
async fn nonce_in_csp_matches_inline_script() {
    let server = common::spawn().await;
    let res = server
        .client
        .get(format!("{}/login", server.base_url))
        .send()
        .await
        .unwrap();
    let csp = res
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let body = res.text().await.unwrap();

    // URL-safe base64 alphabet (see middleware/security.rs)
    let re = Regex::new(r"nonce-([A-Za-z0-9_-]+)").unwrap();
    let nonce = re
        .captures(&csp)
        .expect("nonce in CSP")
        .get(1)
        .unwrap()
        .as_str();

    assert!(
        body.contains(&format!("nonce=\"{nonce}\"")),
        "page lacks matching nonce"
    );
}

/// `'unsafe-eval'` is required because Alpine.js compiles each directive
/// expression with `Function(...)`, and htmx 4's `hx-on::*` attributes
/// do the same. Removing it silently breaks every Alpine interaction.
/// This test exists so the next maintainer who reads the CSP and thinks
/// "we're not using eval" doesn't quietly drop it.
#[tokio::test]
async fn csp_keeps_unsafe_eval_for_alpine_compatibility() {
    let server = common::spawn().await;
    let res = server
        .client
        .get(format!("{}/login", server.base_url))
        .send()
        .await
        .unwrap();
    let csp = res
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        csp.contains("'unsafe-eval'"),
        "CSP must keep 'unsafe-eval' for Alpine/htmx — got: {csp}"
    );
}

#[tokio::test]
async fn two_requests_get_two_nonces() {
    let server = common::spawn().await;
    let csp_a = server
        .client
        .get(format!("{}/login", server.base_url))
        .send()
        .await
        .unwrap()
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let csp_b = server
        .client
        .get(format!("{}/login", server.base_url))
        .send()
        .await
        .unwrap()
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_ne!(csp_a, csp_b);
}
