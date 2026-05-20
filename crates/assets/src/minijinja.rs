//! Minijinja global: `asset(logical)`. Resolves a logical path
//! (e.g. `css/app.css`) through the `Assets` manifest and returns the
//! served URL (e.g. `/static/css/app.<hash>.css` in prod,
//! `/static/css/app.css` in dev).

use std::sync::Arc;

use minijinja::{value::Value, Environment, Error as JinjaError};

use crate::manifest::Assets;

pub fn register(env: &mut Environment<'static>, assets: Arc<Assets>) {
    env.add_function("asset", move |logical: String| {
        let resolved = assets.resolve(&logical);
        Ok::<_, JinjaError>(Value::from(format!("/static/{}", resolved)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use minijinja::context;
    use std::path::PathBuf;

    #[test]
    fn asset_helper_prepends_static_prefix() {
        let assets = Arc::new(Assets::dev(PathBuf::from(".")));
        let mut env = Environment::new();
        register(&mut env, assets);
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
