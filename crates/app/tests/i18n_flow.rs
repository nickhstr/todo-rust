mod common;

#[tokio::test]
async fn accept_language_es_returns_spanish() {
    let server = common::spawn().await;
    let res = server
        .client
        .get(format!("{}/login", server.base_url))
        .header(reqwest::header::ACCEPT_LANGUAGE, "es")
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());
    let body = res.text().await.unwrap();
    // "Bienvenido de nuevo" from es/auth.ftl login-heading
    assert!(
        body.contains("Bienvenido de nuevo"),
        "body should be Spanish; got snippet: {}",
        &body[..body.len().min(500)]
    );
    assert!(body.contains("lang=\"es\""));
}

#[tokio::test]
async fn cookie_overrides_accept_language() {
    let server = common::spawn().await;
    let res = server
        .client
        .get(format!("{}/login", server.base_url))
        .header(reqwest::header::ACCEPT_LANGUAGE, "es")
        .header(reqwest::header::COOKIE, "locale=fr")
        .send()
        .await
        .unwrap();
    let body = res.text().await.unwrap();
    assert!(body.contains("Bienvenue"), "body should be French");
    assert!(body.contains("lang=\"fr\""));
}

#[tokio::test]
async fn switcher_sets_cookie_and_refreshes() {
    let server = common::spawn().await;
    let res = server
        .client
        .post(format!("{}/preferences/locale", server.base_url))
        .form(&[("locale", "de")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    let set_cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(set_cookie.starts_with("locale=de"));
    let hx_refresh = res.headers().get("hx-refresh");
    assert_eq!(hx_refresh.map(|v| v.to_str().unwrap()), Some("true"));
}

#[tokio::test]
async fn unsupported_locale_rejected() {
    let server = common::spawn().await;
    let res = server
        .client
        .post(format!("{}/preferences/locale", server.base_url))
        .form(&[("locale", "xx")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 422);
}
