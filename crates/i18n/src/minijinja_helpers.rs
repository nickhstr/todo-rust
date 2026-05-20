//! Minijinja globals: `t(id, **kwargs)`, `datetime(value, style)`, and
//! `asset(logical)`. The globals read shared state via closures so the
//! Environment only has to be set up once.

use std::{borrow::Cow, sync::Arc};

use fluent_templates::fluent_bundle::FluentValue;
use minijinja::{value::Value, Environment, Error as JinjaError, ErrorKind};
use time::OffsetDateTime;
use unic_langid::{langid, LanguageIdentifier};

use crate::{
    assets::Assets,
    datetime::{format_datetime, DateTimeStyle},
    messages::{FluentArgs, Locales},
    tz::{Tz, UTC},
};

#[derive(Clone)]
pub struct Helpers {
    pub locales: Locales,
    pub assets: Arc<Assets>,
}

pub fn register(env: &mut Environment<'static>, helpers: Helpers) {
    let locales = helpers.locales.clone();
    env.add_function(
        "t",
        move |state: &minijinja::State<'_, '_>, id: String, kwargs: minijinja::value::Kwargs| {
            let locale = current_locale(state);
            let args = kwargs_to_args(&kwargs);
            Ok::<_, JinjaError>(Value::from(locales.lookup(&locale, &id, args.as_ref())))
        },
    );

    env.add_function(
        "datetime",
        move |state: &minijinja::State<'_, '_>,
              value: Value,
              kwargs: minijinja::value::Kwargs|
              -> Result<Value, JinjaError> {
            let locale = current_locale(state);
            let tz = current_tz(state);
            let style_str: String = kwargs.get("style").unwrap_or_else(|_| "medium".into());
            let style = DateTimeStyle::parse(&style_str);
            let dt = value_to_offset_datetime(&value)?;
            let iso = dt
                .format(&time::format_description::well_known::Rfc3339)
                .map_err(|e| JinjaError::new(ErrorKind::InvalidOperation, e.to_string()))?;
            let inner = format_datetime(dt, &locale, tz, style);
            let html = format!(
                "<time datetime=\"{}\" data-style=\"{}\">{}</time>",
                escape_attr(&iso),
                escape_attr(&style_str),
                escape_text(&inner),
            );
            Ok(Value::from_safe_string(html))
        },
    );

    let assets = helpers.assets.clone();
    env.add_function("asset", move |logical: String| {
        let resolved = assets.resolve(&logical);
        Ok::<_, JinjaError>(Value::from(format!("/static/{}", resolved)))
    });
}

fn current_locale(state: &minijinja::State<'_, '_>) -> LanguageIdentifier {
    state
        .lookup("_locale")
        .and_then(|v| v.as_str().map(str::to_owned))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| langid!("en"))
}

fn current_tz(state: &minijinja::State<'_, '_>) -> Tz {
    state
        .lookup("_tz")
        .and_then(|v| v.as_str().map(str::to_owned))
        .and_then(|s| crate::tz::parse_tz(&s))
        .unwrap_or(UTC)
}

fn kwargs_to_args(kwargs: &minijinja::value::Kwargs) -> Option<FluentArgs<'static>> {
    let names: Vec<String> = kwargs.args().map(str::to_owned).collect();
    if names.is_empty() {
        return None;
    }
    let mut args: FluentArgs<'static> = std::collections::HashMap::new();
    for name in names {
        if let Ok(v) = kwargs.get::<Value>(&name) {
            // Convert minijinja Value into FluentValue. We accept strings and
            // integers; everything else gets stringified.
            let fv: FluentValue<'static> = if let Some(s) = v.as_str() {
                FluentValue::from(s.to_owned())
            } else if let Some(n) = v.as_i64() {
                FluentValue::from(n)
            } else {
                FluentValue::from(v.to_string())
            };
            args.insert(Cow::Owned(name), fv);
        }
    }
    Some(args)
}

fn value_to_offset_datetime(value: &Value) -> Result<OffsetDateTime, JinjaError> {
    // Templates render the OffsetDateTime via Value::from_serialize, which
    // currently produces an RFC 3339 string. Accept that string here.
    let s = value
        .as_str()
        .ok_or_else(|| JinjaError::new(ErrorKind::InvalidOperation, "datetime expects a string"))?;
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .map_err(|e| JinjaError::new(ErrorKind::InvalidOperation, e.to_string()))
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use minijinja::context;
    use std::path::PathBuf;

    fn locales_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("locales")
    }

    fn build_env() -> Environment<'static> {
        let locales = Locales::from_dir(locales_dir()).unwrap();
        let assets = Arc::new(Assets::dev(PathBuf::from(".")));
        let mut env = Environment::new();
        register(&mut env, Helpers { locales, assets });
        env
    }

    #[test]
    fn t_helper_renders_message() {
        let mut env = build_env();
        env.add_template("test.txt", "{{ t('app-name') }}").unwrap();
        let out = env
            .get_template("test.txt")
            .unwrap()
            .render(context! { _locale => "en" })
            .unwrap();
        assert_eq!(out, "Quiet Ledger");
    }

    #[test]
    fn datetime_helper_emits_time_element() {
        let mut env = build_env();
        env.add_template("test.txt", "{{ datetime(value, style='medium') }}")
            .unwrap();
        let out = env
            .get_template("test.txt")
            .unwrap()
            .render(context! {
                _locale => "en",
                _tz => "America/Los_Angeles",
                value => "2026-05-19T20:00:00Z"
            })
            .unwrap();
        assert!(out.starts_with("<time datetime=\"2026-05-19T20:00:00Z\""));
        assert!(out.contains("data-style=\"medium\""));
        assert!(out.contains("2026"));
    }

    #[test]
    fn asset_helper_prepends_static_prefix() {
        let mut env = build_env();
        env.add_template("test.txt", "{{ asset('css/app.css') }}")
            .unwrap();
        let out = env
            .get_template("test.txt")
            .unwrap()
            .render(context! {})
            .unwrap();
        assert_eq!(out, "/static/css/app.css");
    }
}
