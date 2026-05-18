//! End-to-end auth flow tests: signup, login, logout, cross-user isolation.

mod common;

use common::spawn;

#[tokio::test]
async fn signup_redirects_and_sets_cookie() {
    let app = spawn().await;

    let res = app
        .client
        .post(format!("{}/signup", app.base_url))
        .form(&[
            ("email", "eve@example.com"),
            ("password", "verylongsecret123"),
        ])
        .send()
        .await
        .unwrap();
    assert!(res.status().is_redirection(), "got {}", res.status());

    // Reuse the cookie jar: GET / should now succeed (no redirect to /login).
    let index = app
        .client
        .get(format!("{}/", app.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(index.status(), 200);
    let body = index.text().await.unwrap();
    assert!(body.contains("Quiet Ledger") || body.contains("intentions"));
}

#[tokio::test]
async fn signup_duplicate_email_conflicts() {
    let app = spawn().await;
    let _ = app
        .client
        .post(format!("{}/signup", app.base_url))
        .form(&[
            ("email", "frank@example.com"),
            ("password", "verylongsecret123"),
        ])
        .send()
        .await
        .unwrap();

    // Fresh client (no cookies) to avoid the auto-login session.
    let bare = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let res = bare
        .post(format!("{}/signup", app.base_url))
        .form(&[
            ("email", "frank@example.com"),
            ("password", "another-long-pw"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 409);
}

#[tokio::test]
async fn login_wrong_password_returns_401() {
    let app = spawn().await;
    let _ = app
        .client
        .post(format!("{}/signup", app.base_url))
        .form(&[
            ("email", "gary@example.com"),
            ("password", "verylongsecret123"),
        ])
        .send()
        .await
        .unwrap();

    let bare = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let res = bare
        .post(format!("{}/login", app.base_url))
        .form(&[
            ("email", "gary@example.com"),
            ("password", "wrong-password"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn unauthenticated_index_redirects_to_login() {
    let app = spawn().await;
    let bare = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let res = bare.get(format!("{}/", app.base_url)).send().await.unwrap();
    assert!(res.status().is_redirection(), "got {}", res.status());
    let loc = res.headers().get("location").unwrap().to_str().unwrap();
    assert_eq!(loc, "/login");
}

#[tokio::test]
async fn responses_carry_x_app_version() {
    let app = spawn().await;
    let res = app
        .client
        .get(format!("{}/healthz", app.base_url))
        .send()
        .await
        .unwrap();
    let v = res
        .headers()
        .get("x-app-version")
        .expect("x-app-version present")
        .to_str()
        .unwrap();
    assert!(!v.is_empty());
    // Build-time SHA: either "unknown" (no git, no $GIT_SHA) or a 40-char hex.
    assert!(
        v == "unknown" || (v.len() == 40 && v.chars().all(|c| c.is_ascii_hexdigit())),
        "unexpected x-app-version: {v:?}"
    );
}
