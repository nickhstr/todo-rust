//! End-to-end test for `POST /dev/login`. The route only exists in debug
//! builds (`cfg(debug_assertions)`); `cargo test` defaults to debug, so this
//! always compiles. If anyone flips test profile to release-with-overrides,
//! this file's behavior changes — that's intentional.

mod common;

use common::{spawn, spawn_with};

#[tokio::test]
async fn dev_login_drops_into_session_for_seeded_user() {
    let email = "dev@local";
    let app = spawn_with(|cfg| cfg.dev.auto_login_email = email.into()).await;

    // The test harness doesn't run main::ensure_dev_user, so seed the account
    // through the normal signup flow then log out to drop the session.
    let signup = app
        .client
        .post(format!("{}/signup", app.base_url))
        .form(&[("email", email), ("password", "doesntmatterforthistest")])
        .send()
        .await
        .unwrap();
    assert!(signup.status().is_redirection(), "signup got {}", signup.status());

    let logout = app
        .client
        .post(format!("{}/logout", app.base_url))
        .send()
        .await
        .unwrap();
    assert!(logout.status().is_redirection(), "logout got {}", logout.status());

    // Now exercise the dev login.
    let dev = app
        .client
        .post(format!("{}/dev/login", app.base_url))
        .send()
        .await
        .unwrap();
    assert!(dev.status().is_redirection(), "dev login got {}", dev.status());
    assert_eq!(dev.headers().get("location").unwrap(), "/");

    // Cookie jar should now hold a valid session.
    let index = app
        .client
        .get(format!("{}/", app.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(index.status(), 200, "expected 200, got {}", index.status());
}

#[tokio::test]
async fn dev_login_404s_when_disabled() {
    let app = spawn().await; // default config — dev.auto_login_email empty
    let res = app
        .client
        .post(format!("{}/dev/login", app.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}
