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

    let re = Regex::new(r"nonce-([A-Za-z0-9+/]+)").unwrap();
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
