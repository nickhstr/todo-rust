//! Integration tests for the dev preview tool. Routes are gated on
//! `cfg(debug_assertions)`; `cargo test` defaults to the debug profile so
//! the gate is always satisfied here.

#![cfg(debug_assertions)]

mod common;

use common::{spawn, spawn_with};

#[tokio::test]
async fn preview_index_lists_templates() {
    let app = spawn_with(|cfg| cfg.dev.preview_enabled = true).await;
    let res = app
        .client
        .get(format!("{}/__preview", app.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "expected 200, got {}", res.status());
    let body = res.text().await.unwrap();
    assert!(body.contains("partials/todo.html"), "body missing template path. body:\n{body}");
    assert!(body.contains("index.html"), "body missing index.html");
    assert!(
        !body.contains("_preview_shell.html"),
        "underscore-prefixed templates must be hidden from the index"
    );
}

#[tokio::test]
async fn preview_render_partial_shows_fixture_data() {
    let app = spawn_with(|cfg| cfg.dev.preview_enabled = true).await;
    let res = app
        .client
        .get(format!(
            "{}/__preview/render/partials/todo.html/default",
            app.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "expected 200, got {}", res.status());
    let body = res.text().await.unwrap();
    // The default fixture has title "Buy milk".
    assert!(body.contains("Buy milk"), "body missing fixture title. body:\n{body}");
    // Host shell should be present for partials.
    assert!(body.contains("PREVIEW"), "host shell PREVIEW bar missing. body:\n{body}");
}

#[tokio::test]
async fn preview_render_respects_locale_query_param() {
    let app = spawn_with(|cfg| cfg.dev.preview_enabled = true).await;
    let res = app
        .client
        .get(format!(
            "{}/__preview/render/partials/todo.html/default?locale=es",
            app.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    // partials/todo.html uses `t("todo-mark-done")` or `t("todo-mark-open")`.
    // The Spanish catalog at locales/es/main.ftl has whichever of those keys.
    // We assert SOME Spanish-only marker shows up — `lang="es"` is set by the
    // host shell from _locale, which is a stable, low-flake assertion.
    assert!(
        body.contains(r#"lang="es""#),
        "expected lang=\"es\" in shell. body:\n{body}"
    );
}

#[tokio::test]
async fn preview_render_full_page_skips_shell() {
    let app = spawn_with(|cfg| cfg.dev.preview_enabled = true).await;
    let res = app
        .client
        .get(format!(
            "{}/__preview/render/login.html/default",
            app.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    // login.html extends base.html, so the host shell's "PREVIEW" banner must
    // NOT be present. We expect the real base.html `<title>` containing the
    // localized page title instead.
    assert!(!body.contains("__preview_bar"),
        "host shell must be skipped for full-page templates");
    // And the page should be a complete document (base.html applied).
    assert!(body.contains("<title>"), "missing base.html <title>");
}

#[tokio::test]
async fn preview_render_missing_fixture_is_404() {
    let app = spawn_with(|cfg| cfg.dev.preview_enabled = true).await;
    let res = app
        .client
        .get(format!(
            "{}/__preview/render/partials/todo.html/does-not-exist",
            app.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn preview_disabled_returns_404() {
    let app = spawn().await; // default config — preview_enabled = false
    let res = app
        .client
        .get(format!("{}/__preview", app.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);

    let res = app
        .client
        .get(format!(
            "{}/__preview/render/partials/todo.html/default",
            app.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}
