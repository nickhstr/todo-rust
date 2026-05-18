//! Activates the OTel `Context` for each entered tracing span.
//!
//! `tracing-opentelemetry` stores the OTel span builder (with the chosen
//! `trace_id` / `span_id`) in the tracing span's extensions, but it does
//! NOT push that data onto OTel's thread-local context stack. The OTel
//! logs SDK reads `Context::map_current(...)` inside `Logger::emit` to
//! populate `LogRecord::trace_context` — so without this layer, every
//! log record ships with an empty trace context and Loki ↔ Tempo
//! correlation is impossible.
//!
//! This layer reads `OtelData` from the entered span's extensions, builds
//! a non-recording OTel `Context` carrying that span's `SpanContext`, and
//! attaches it for the duration of the span. The resulting `ContextGuard`
//! lives on a per-thread LIFO stack; `on_exit` pops, restoring the previous
//! context. Tracing guarantees on_enter / on_exit pairs are LIFO per thread,
//! so nested spans and tokio task migrations both work correctly.

use std::cell::RefCell;

use opentelemetry::{
    trace::{SpanContext, TraceContextExt, TraceFlags, TraceState},
    Context, ContextGuard,
};
use tracing::{span, Subscriber};
use tracing_opentelemetry::OtelData;
use tracing_subscriber::{layer::Context as LayerContext, registry::LookupSpan, Layer};

thread_local! {
    static GUARD_STACK: RefCell<Vec<ContextGuard>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Default)]
pub struct OtelContextLayer;

impl<S> Layer<S> for OtelContextLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_enter(&self, id: &span::Id, ctx: LayerContext<'_, S>) {
        let Some(span_ref) = ctx.span(id) else { return };
        let extensions = span_ref.extensions();
        let Some(otel_data) = extensions.get::<OtelData>() else {
            // tracing-opentelemetry wasn't installed (or hasn't run yet) —
            // nothing to activate. Push a no-op so on_exit's pop balances.
            GUARD_STACK.with(|s| s.borrow_mut().push(Context::current().attach()));
            return;
        };

        // The PreSampledTracer populates trace_id+span_id on the builder before
        // the span starts. If they're missing, fall back to the parent context.
        let trace_id = otel_data
            .builder
            .trace_id
            .unwrap_or_else(|| otel_data.parent_cx.span().span_context().trace_id());
        let span_id = otel_data
            .builder
            .span_id
            .unwrap_or_else(|| otel_data.parent_cx.span().span_context().span_id());

        let span_context = SpanContext::new(
            trace_id,
            span_id,
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        );

        let cx = otel_data
            .parent_cx
            .clone()
            .with_remote_span_context(span_context);
        let guard = cx.attach();
        GUARD_STACK.with(|s| s.borrow_mut().push(guard));
    }

    fn on_exit(&self, _id: &span::Id, _ctx: LayerContext<'_, S>) {
        GUARD_STACK.with(|s| {
            s.borrow_mut().pop();
        });
    }
}
