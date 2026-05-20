//! Shared test plumbing: spins up a real Postgres in a container, wires up the
//! app, and exposes a client + base URL the tests can drive.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum_login::AuthManagerLayerBuilder;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use time::Duration as TimeDuration;
use todo_app::{
    auth::AuthBackend, build_router, cache::Cache, templates::Templates, AppState, Config,
};
use todo_i18n::minijinja_helpers::Helpers;
use todo_observability::install_metrics_recorder;
use todo_storage::{pool::build_pool, run_migrations};
use tokio::{net::TcpListener, task::JoinHandle};
use tower_sessions::{cookie::Key, Expiry, SessionManagerLayer};
use tower_sessions_sqlx_store::PostgresStore;

pub struct TestServer {
    pub base_url: String,
    pub client: reqwest::Client,
    _server: JoinHandle<()>,
    _pg: testcontainers::ContainerAsync<Postgres>,
}

pub async fn spawn() -> TestServer {
    spawn_with(|_| {}).await
}

pub async fn spawn_with(configure: impl FnOnce(&mut Config)) -> TestServer {
    let pg = Postgres::default()
        .with_db_name("todo")
        .with_user("todo")
        .with_password("todo")
        .start()
        .await
        .expect("start postgres");
    let host_port = pg.get_host_port_ipv4(5432).await.expect("host port");
    let db_url = format!("postgres://todo:todo@127.0.0.1:{host_port}/todo");

    let mut cfg = Config::default();
    cfg.database.url = db_url.clone();
    cfg.database.max_connections = 4;
    cfg.database.min_connections = 1;
    cfg.template_autoreload = false;
    cfg.templates_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("templates");
    cfg.static_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("static");
    cfg.auth.session_key = hex::encode([7u8; 64]); // 64 bytes, deterministic
    cfg.auth.cookie_secure = false;
    configure(&mut cfg);

    let pool = build_pool(&cfg.database).await.expect("pool");
    run_migrations(&pool).await.expect("migrate");

    let session_store = PostgresStore::new(pool.clone());
    session_store.migrate().await.expect("session migrate");

    let locales_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("locales");
    let locales = todo_i18n::Locales::from_dir(locales_dir).expect("load locales");
    let assets = std::sync::Arc::new(todo_assets::Assets::dev(cfg.static_dir.clone()));
    let helpers = Helpers {
        locales: locales.clone(),
        assets: assets.clone(),
    };

    let templates = Templates::production(&cfg.templates_dir, helpers);
    let cache = Cache::disabled();
    let state = AppState::new(
        Arc::new(cfg.clone()),
        Arc::new(pool),
        templates,
        cache,
        None,
        locales,
        assets,
    );

    // Per-test independent Prometheus recorder. install_recorder() is global, so
    // tests that call this twice in the same process will get an error on the
    // second call; we ignore that since tests only need the handle for /metrics.
    let prom = install_metrics_recorder().unwrap_or_else(|_| {
        metrics_exporter_prometheus::PrometheusBuilder::new()
            .build_recorder()
            .handle()
    });

    let session_key = cfg.auth.decoded_session_key().unwrap();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_http_only(true)
        .with_same_site(tower_sessions::cookie::SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(TimeDuration::seconds(60 * 60)))
        .with_signed(Key::from(&session_key));
    let auth_layer =
        AuthManagerLayerBuilder::new(AuthBackend::new(state.users.clone()), session_layer).build();

    let app = build_router(state, prom, auth_layer);
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind");
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .expect("build client");

    TestServer {
        base_url,
        client,
        _server: server,
        _pg: pg,
    }
}
