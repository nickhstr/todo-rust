use std::{sync::Arc, time::Duration};

use axum_login::AuthManagerLayerBuilder;
use listenfd::ListenFd;
use mimalloc::MiMalloc;
use time::Duration as TimeDuration;
use todo_observability::{init_tracing, install_metrics_recorder};
use todo_storage::{pool::build_pool, run_migrations};
use tokio::{net::TcpListener, signal, task::JoinHandle};
use tower_sessions::{cookie::Key, ExpiredDeletion, Expiry, SessionManagerLayer};
use tower_sessions_sqlx_store::PostgresStore;
use tracing::info;

use todo_app::{
    auth::AuthBackend,
    build_router,
    cache::{build_redis_pool, Cache},
    templates::Templates,
    AppState, Config,
};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Best-effort: dotenv is for local dev; in container we use env vars directly.
    let _ = dotenvy::dotenv();

    let config = Config::from_env()?;
    let _obs_guard =
        init_tracing(&config.observability).map_err(|e| anyhow::anyhow!("init tracing: {e}"))?;
    let prom_handle =
        install_metrics_recorder().map_err(|e| anyhow::anyhow!("metrics recorder: {e}"))?;

    info!(
        host = %config.server.host,
        port = config.server.port,
        "todo-app starting"
    );

    let db = build_pool(&config.database).await?;
    run_migrations(&db).await?;

    let session_store = PostgresStore::new(db.clone());
    session_store
        .migrate()
        .await
        .map_err(|e| anyhow::anyhow!("session table migrate: {e}"))?;
    let session_cleanup = spawn_session_cleanup(session_store.clone());

    let redis = match build_redis_pool(&config.cache.url, config.cache.pool_size).await {
        Ok(pool) => {
            info!("redis pool ready");
            Some(pool)
        }
        Err(err) => {
            tracing::warn!(error = %err, "redis unavailable; running with cache disabled");
            None
        }
    };
    let cache = Cache::new(
        redis.clone(),
        Duration::from_secs(config.cache.default_ttl_secs),
    );

    let locales = todo_i18n::Locales::from_dir(config.locales_dir.clone())
        .map_err(|e| anyhow::anyhow!("load locales: {e}"))?;

    let assets = if config.template_autoreload {
        Arc::new(todo_i18n::Assets::dev(config.static_dir.clone()))
    } else {
        Arc::new(
            todo_i18n::Assets::production(config.static_dir.clone())
                .map_err(|e| anyhow::anyhow!("scan static dir: {e}"))?,
        )
    };

    let helpers = todo_i18n::minijinja_helpers::Helpers {
        locales: locales.clone(),
        assets: assets.clone(),
    };

    let templates = if config.template_autoreload {
        Templates::dev(config.templates_dir.clone(), helpers)
    } else {
        Templates::production(&config.templates_dir, helpers)
    };

    let state = AppState::new(
        Arc::new(config.clone()),
        Arc::new(db.clone()),
        templates,
        cache,
        redis,
        locales,
        assets,
    );

    #[cfg(debug_assertions)]
    ensure_dev_user(&config, &state).await?;

    let session_key = config.auth.decoded_session_key()?;
    let signing_key = Key::from(&session_key);
    let mut session_layer = SessionManagerLayer::new(session_store)
        .with_secure(config.auth.cookie_secure)
        .with_http_only(true)
        .with_same_site(tower_sessions::cookie::SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(TimeDuration::seconds(
            config.auth.session_ttl_secs,
        )));
    if !config.auth.cookie_domain.is_empty() {
        session_layer = session_layer.with_domain(config.auth.cookie_domain.clone());
    }
    let session_layer = session_layer.with_signed(signing_key);

    let auth_backend = AuthBackend::new(state.users.clone());
    let auth_layer = AuthManagerLayerBuilder::new(auth_backend, session_layer).build();

    let app = build_router(state.clone(), prom_handle, auth_layer);

    let addr = config.server.socket_addr()?;
    // listenfd: if running under `systemfd`, take the inherited socket for
    // zero-downtime restarts. Otherwise bind ourselves.
    let listener = match ListenFd::from_env().take_tcp_listener(0)? {
        Some(std_l) => {
            std_l.set_nonblocking(true)?;
            TcpListener::from_std(std_l)?
        }
        None => TcpListener::bind(addr).await?,
    };
    info!("listening on http://{}", listener.local_addr()?);

    let shutdown_timeout = Duration::from_secs(config.server.shutdown_timeout_secs);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // Allow in-flight tasks to flush, then abort the session cleanup loop.
    tokio::time::timeout(shutdown_timeout, async {
        session_cleanup.abort();
        let _ = session_cleanup.await;
    })
    .await
    .ok();

    info!("todo-app shut down");
    Ok(())
}

/// If `dev.auto_login_email` is set, make sure that user exists so `POST
/// /dev/login` has someone to log in as. Compiled out of `--release` along
/// with the route itself. The password is random and never surfaced — the dev
/// login endpoint bypasses verification.
#[cfg(debug_assertions)]
async fn ensure_dev_user(config: &Config, state: &todo_app::AppState) -> anyhow::Result<()> {
    use rand::Rng;
    use todo_domain::NewUser;

    let Some(email) = config.dev.enabled_email() else {
        return Ok(());
    };

    if state.users.find_by_email(email).await?.is_some() {
        tracing::warn!(email, "dev auto-login enabled (existing user)");
        return Ok(());
    }

    // Random password — never displayed; /dev/login skips verification.
    let password: String = (0..48)
        .map(|_| {
            let i = rand::thread_rng().gen_range(0..62);
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"[i] as char
        })
        .collect();
    let new = NewUser {
        email: email.to_owned(),
        password,
    };
    match state.users.create(new).await {
        Ok(_) => tracing::warn!(email, "dev auto-login enabled (new user seeded)"),
        Err(todo_storage::StorageError::Conflict(_)) => {
            // Lost the race; that's fine.
            tracing::warn!(email, "dev auto-login enabled (race-created)");
        }
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

/// Background task: prune expired session rows. Runs forever; aborted on shutdown.
fn spawn_session_cleanup(store: PostgresStore) -> JoinHandle<()> {
    tokio::spawn(async move {
        // 1h interval is plenty given typical session TTLs.
        let result = store
            .continuously_delete_expired(Duration::from_secs(60 * 60))
            .await;
        if let Err(err) = result {
            tracing::error!(error = %err, "session cleanup loop exited");
        }
    })
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install SIGINT handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("ctrl-c received; shutting down"),
        () = terminate => tracing::info!("SIGTERM received; shutting down"),
    }
}
