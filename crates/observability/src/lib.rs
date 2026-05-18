//! Observability wiring: tracing subscriber + Prometheus recorder + optional OTLP.
//!
//! When `otel_enabled` is true, both traces and logs flow over OTLP/gRPC to
//! the configured collector. The OTel log record carries the current span's
//! `trace_id` and `span_id` natively, which Grafana/Loki uses to link a log
//! line back to its trace in Tempo.

mod otel_context_layer;

use std::time::Duration;

use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use opentelemetry::{global, trace::TracerProvider as _, KeyValue};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    logs::LoggerProvider,
    propagation::TraceContextPropagator,
    runtime::Tokio,
    trace::{self as sdktrace, TracerProvider},
    Resource,
};
use opentelemetry_semantic_conventions::resource::SERVICE_NAME;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    pub service_name: String,
    pub otel_endpoint: String,
    pub otel_enabled: bool,
    pub log_format: LogFormat,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Pretty,
    Json,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            service_name: "todo-app".into(),
            otel_endpoint: "http://localhost:4317".into(),
            otel_enabled: false,
            log_format: LogFormat::Pretty,
        }
    }
}

#[derive(Debug, Error)]
pub enum ObservabilityError {
    #[error("invalid log filter: {0}")]
    Filter(#[from] tracing_subscriber::filter::FromEnvError),
    #[error("metrics recorder install: {0}")]
    Metrics(#[from] metrics_exporter_prometheus::BuildError),
    #[error("OTLP exporter: {0}")]
    Otlp(String),
    #[error("subscriber already set")]
    AlreadySet,
}

/// Drop guard: flushes and shuts down both the OTLP tracer provider and the
/// OTLP logger provider on drop. Held by `main` for the process lifetime.
pub struct ObservabilityGuard {
    tracer_provider: Option<TracerProvider>,
    logger_provider: Option<LoggerProvider>,
}

impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.tracer_provider.take() {
            let _ = provider.shutdown();
        }
        if let Some(provider) = self.logger_provider.take() {
            let _ = provider.shutdown();
        }
    }
}

/// Initialize `tracing-subscriber` (plus OTLP trace + log bridges when enabled).
///
/// Branch on `log_format` so the layer stack stays statically typed; `Option<L>`
/// makes the OTel layers optional without boxing.
pub fn init_tracing(cfg: &ObservabilityConfig) -> Result<ObservabilityGuard, ObservabilityError> {
    global::set_text_map_propagator(TraceContextPropagator::new());

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let (tracer_provider, otel_trace_layer) = if cfg.otel_enabled {
        let provider = build_otlp_tracer_provider(cfg)?;
        let tracer = provider.tracer(cfg.service_name.clone());
        let layer = tracing_opentelemetry::layer().with_tracer(tracer);
        (Some(provider), Some(layer))
    } else {
        (None, None)
    };

    // The logs bridge converts `tracing` events into OTel `LogRecord`s. Each
    // record automatically picks up `trace_id`/`span_id` from the current
    // OTel context, which is what Grafana uses to link Loki ↔ Tempo.
    let (logger_provider, otel_log_layer) = if cfg.otel_enabled {
        let provider = build_otlp_logger_provider(cfg)?;
        let layer = OpenTelemetryTracingBridge::new(&provider);
        (Some(provider), Some(layer))
    } else {
        (None, None)
    };

    // Activator must run AFTER otel_trace_layer (which populates OtelData in
    // span extensions on new_span) so that on_enter sees the data, and BEFORE
    // otel_log_layer so the log emitter sees the active OTel context.
    let activator = otel_context_layer::OtelContextLayer;

    match cfg.log_format {
        LogFormat::Json => Registry::default()
            .with(env_filter)
            .with(otel_trace_layer)
            .with(activator)
            .with(otel_log_layer)
            .with(
                fmt::layer()
                    .json()
                    .with_target(true)
                    .with_current_span(true),
            )
            .try_init()
            .map_err(|_| ObservabilityError::AlreadySet)?,
        LogFormat::Pretty => Registry::default()
            .with(env_filter)
            .with(otel_trace_layer)
            .with(activator)
            .with(otel_log_layer)
            .with(fmt::layer().with_target(true).with_ansi(true))
            .try_init()
            .map_err(|_| ObservabilityError::AlreadySet)?,
    }

    Ok(ObservabilityGuard {
        tracer_provider,
        logger_provider,
    })
}

fn resource(cfg: &ObservabilityConfig) -> Resource {
    Resource::new(vec![KeyValue::new(SERVICE_NAME, cfg.service_name.clone())])
}

fn build_otlp_tracer_provider(
    cfg: &ObservabilityConfig,
) -> Result<TracerProvider, ObservabilityError> {
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(&cfg.otel_endpoint)
        .with_timeout(Duration::from_secs(5));

    let trace_config = sdktrace::Config::default().with_resource(resource(cfg));

    opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(trace_config)
        .install_batch(Tokio)
        .map_err(|e| ObservabilityError::Otlp(format!("traces: {e}")))
}

fn build_otlp_logger_provider(
    cfg: &ObservabilityConfig,
) -> Result<LoggerProvider, ObservabilityError> {
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(&cfg.otel_endpoint)
        .with_timeout(Duration::from_secs(5));

    opentelemetry_otlp::new_pipeline()
        .logging()
        .with_exporter(exporter)
        .with_resource(resource(cfg))
        .install_batch(Tokio)
        .map_err(|e| ObservabilityError::Otlp(format!("logs: {e}")))
}

/// Install the global Prometheus recorder and return its handle for `/metrics`.
pub fn install_metrics_recorder() -> Result<PrometheusHandle, ObservabilityError> {
    let handle = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Suffix("_seconds".to_owned()),
            &[
                0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ],
        )?
        .install_recorder()?;
    Ok(handle)
}
