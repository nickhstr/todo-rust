//! Renders the host shell for partial-template previews.

use minijinja::context;

use crate::{templates::Templates, AppError};

/// Wrap an already-rendered partial in `_preview_shell.html`.
/// Top-level templates that extend `base.html` skip this entirely.
pub fn wrap(
    templates: &Templates,
    rendered_partial: String,
    template_path: &str,
    story: &str,
    locale: &str,
    tz: &str,
) -> Result<String, AppError> {
    let html = templates.render(
        "_preview_shell.html",
        context! {
            rendered_partial,
            template_path,
            story,
            _locale => locale,
            _tz => tz,
        },
    )?;
    Ok(html.0)
}
