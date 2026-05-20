//! Minijinja globals: `t(id, **kwargs)` and `datetime(value, style)`.
//! The globals read shared state via closures so the Environment only has
//! to be set up once.

use std::borrow::Cow;

use fluent_templates::fluent_bundle::FluentValue;
use minijinja::{value::Value, Environment, Error as JinjaError, ErrorKind};
use time::OffsetDateTime;
use unic_langid::{langid, LanguageIdentifier};

use crate::{
    datetime::{format_datetime, DateTimeStyle},
    messages::{FluentArgs, Locales},
    tz::{Tz, UTC},
};

pub fn register(env: &mut Environment<'static>, locales: Locales) {
    let locales_for_t = locales.clone();
    env.add_function(
        "t",
        move |state: &minijinja::State<'_, '_>, id: String, kwargs: minijinja::value::Kwargs| {
            let locale = current_locale(state);
            let args = kwargs_to_args(&kwargs);
            Ok::<_, JinjaError>(Value::from(locales_for_t.lookup(&locale, &id, args.as_ref())))
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

/// Full HTML attribute-value escape: handles `&`, `<`, `>`, `"`, `'`.
/// Used for the `datetime` attribute and `data-style` attribute of the
/// generated `<time>` element. Today's inputs (RFC3339 timestamp,
/// `DateTimeStyle::parse` output) are known-safe — this is defense in
/// depth so future callers can't accidentally smuggle markup through.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// HTML text-node escape: handles `&`, `<`, `>`. ICU's formatted output
/// for the four shipped locales is always safe ASCII/printable Unicode
/// without markup-significant characters, but this is defense in depth
/// for future locale additions.
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
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
        let mut env = Environment::new();
        register(&mut env, locales);
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
    fn escape_attr_handles_all_html_significant_chars() {
        let out = escape_attr(r#"&<>"'"#);
        assert_eq!(out, "&amp;&lt;&gt;&quot;&#39;");
    }

    #[test]
    fn escape_text_handles_markup_chars() {
        let out = escape_text("a<script>&amp;");
        assert_eq!(out, "a&lt;script&gt;&amp;amp;");
    }
}
