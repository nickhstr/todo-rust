//! Preview HTTP handlers. Mounted by `preview::router()` and gated by
//! `state.config.dev.preview_enabled` inside each handler — a defense-in-depth
//! check beyond the `cfg(debug_assertions)` mount gate in `router.rs`.

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
    routing::get,
    Extension, Router,
};
use minijinja::context;
use serde::Deserialize;

use crate::{
    middleware::CspNonce,
    preview::{
        fixtures::{discover, load_fixture, TemplateEntry},
        shell,
    },
    AppError, AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/render/*path", get(render))
}

#[derive(Debug, Deserialize, Default)]
pub struct PreviewQuery {
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub tz: Option<String>,
}

impl PreviewQuery {
    fn locale_or_default(&self) -> &str {
        self.locale.as_deref().unwrap_or("en")
    }
    fn tz_or_default(&self) -> &str {
        self.tz.as_deref().unwrap_or("UTC")
    }
}

fn check_enabled(state: &AppState) -> Result<(), AppError> {
    if state.config.dev.preview_enabled {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

/// `GET /__preview` — list every discovered template and its stories.
pub async fn index(
    State(state): State<AppState>,
    Query(q): Query<PreviewQuery>,
    Extension(nonce): Extension<CspNonce>,
) -> Result<Response, AppError> {
    check_enabled(&state)?;
    let fixtures_dir = &state.config.dev.preview_fixtures_dir;
    if !fixtures_dir.exists() {
        // Quiet by default, but surface this loud once per request — the
        // walker treats a missing dir as "no stories" and returns nothing,
        // which renders an index where every template says [no fixtures].
        // Easy to mistake for "I haven't written fixtures yet." when the
        // real cause is a misconfigured path / unmounted volume.
        tracing::warn!(
            path = %fixtures_dir.display(),
            "preview_fixtures_dir does not exist; every template will show as having no fixtures"
        );
    }
    let entries: Vec<TemplateEntry> = discover(&state.config.templates_dir, fixtures_dir)
        .map_err(|e| AppError::Internal(format!("preview discover failed: {e}")))?;

    let html = state.templates.render(
        "_preview_index.html",
        context! {
            entries => entries.iter().map(|e| context! {
                template_path => &e.template_path,
                stories => e.stories.iter().map(|s| &s.name).collect::<Vec<_>>(),
            }).collect::<Vec<_>>(),
            _locale => q.locale_or_default(),
            _tz => q.tz_or_default(),
            csp_nonce => nonce.0.clone(),
        },
    )?;
    Ok(html.into_response())
}

/// `GET /__preview/render/*path` — render a single (template, story) pair.
/// `path` is an axum catchall; the last segment is the story name, the rest
/// is the template path (e.g. `partials/todo.html/default`).
pub async fn render(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(q): Query<PreviewQuery>,
    Extension(nonce): Extension<CspNonce>,
) -> Result<Response, AppError> {
    check_enabled(&state)?;

    let (template_path, story) = split_last_segment(&path).ok_or(AppError::NotFound)?;

    // Spec: unknown template path → 404 (not 500 from minijinja). Check
    // existence on disk before calling render(); a typo'd template name
    // would otherwise bubble up as AppError::Template -> 500.
    if !state.config.templates_dir.join(template_path).is_file() {
        return Err(AppError::NotFound);
    }

    let fixture_file = state
        .config
        .dev
        .preview_fixtures_dir
        .join(template_path.trim_end_matches(".html"))
        .join(format!("{story}.toml"));
    if !fixture_file.exists() {
        return Err(AppError::NotFound);
    }
    let loaded = load_fixture(&fixture_file)
        .map_err(|e| AppError::Internal(format!("fixture load failed: {e}")))?;

    let locale = q.locale_or_default();
    let tz = q.tz_or_default();

    // Build the render context: ambient defaults, then merge fixture ctx on top.
    // Fixture wins on key conflict (explicit override always works).
    let mut merged = serde_json::Map::new();
    merged.insert(
        "_locale".into(),
        serde_json::Value::String(locale.to_string()),
    );
    merged.insert("_tz".into(), serde_json::Value::String(tz.to_string()));
    merged.insert(
        "csp_nonce".into(),
        serde_json::Value::String(nonce.0.clone()),
    );
    if let serde_json::Value::Object(ctx) = loaded.ctx {
        for (k, v) in ctx {
            merged.insert(k, v);
        }
    }
    let merged = serde_json::Value::Object(merged);

    let rendered = state.templates.render(template_path, &merged)?;

    // Partials get wrapped in the host shell; everything else (templates that
    // extend base.html) is already a full document.
    let body = if template_path.starts_with("partials/") {
        Html(shell::wrap(
            &state.templates,
            rendered.0,
            template_path,
            story,
            locale,
            tz,
        )?)
    } else {
        rendered
    };

    Ok(body.into_response())
}

/// Split `"a/b/c.html/story"` into `("a/b/c.html", "story")`.
/// Returns `None` if there is no `/`.
fn split_last_segment(path: &str) -> Option<(&str, &str)> {
    let idx = path.rfind('/')?;
    Some((&path[..idx], &path[idx + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_last_segment_basic() {
        assert_eq!(
            split_last_segment("partials/todo.html/default"),
            Some(("partials/todo.html", "default"))
        );
        assert_eq!(
            split_last_segment("index.html/default"),
            Some(("index.html", "default"))
        );
        assert_eq!(split_last_segment("no-slash"), None);
    }
}
