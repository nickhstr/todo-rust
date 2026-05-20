use std::{path::PathBuf, sync::Arc};

use axum::response::Html;
use minijinja::{path_loader, value::Value, Environment};
use minijinja_autoreload::AutoReloader;
use serde::Serialize;
use todo_i18n::minijinja_helpers::{register, Helpers};

use crate::error::AppError;

/// Templating env. Production uses a one-shot `Environment`; dev uses
/// `AutoReloader` so file edits show up on the next request.
#[derive(Clone)]
pub enum Templates {
    Static(Arc<Environment<'static>>),
    Reloading(Arc<AutoReloader>),
}

impl Templates {
    pub fn production(dir: &PathBuf, helpers: Helpers) -> Self {
        let mut env = Environment::new();
        env.set_loader(path_loader(dir));
        register(&mut env, helpers);
        Self::Static(Arc::new(env))
    }

    pub fn dev(dir: PathBuf, helpers: Helpers) -> Self {
        let helpers_for_reload = helpers.clone();
        let reloader = AutoReloader::new(move |notifier| {
            let dir = dir.clone();
            let helpers = helpers_for_reload.clone();
            let mut env = Environment::new();
            env.set_loader(path_loader(&dir));
            register(&mut env, helpers);
            notifier.watch_path(&dir, true);
            Ok(env)
        });
        // notify-rs (the inotify-based watcher minijinja-autoreload uses by
        // default) doesn't receive events through podman/Docker bind mounts on
        // macOS. Force the env to rebuild on every render so template edits
        // show up without a server restart. ~ms overhead per request; fine
        // for dev. Production uses Templates::Static and never pays this.
        reloader.notifier().set_fast_reload(false);
        Self::Reloading(Arc::new(reloader))
    }

    pub fn render<S: Serialize>(&self, name: &str, ctx: S) -> Result<Html<String>, AppError> {
        let value = Value::from_serialize(&ctx);
        let out = match self {
            Self::Static(env) => env.get_template(name)?.render(value)?,
            Self::Reloading(rl) => {
                let env = rl.acquire_env().map_err(AppError::from)?;
                env.get_template(name)?.render(value)?
            }
        };
        Ok(Html(out))
    }
}
