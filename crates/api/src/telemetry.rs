//! Logs to stdout always; traces and logs over OTLP when `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
//! The exporters read the standard `OTEL_*` variables (endpoint, protocol, headers, service name).

use opentelemetry::propagation::TextMapCompositePropagator;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Keeps the exporters alive; flushes them on drop.
pub struct Telemetry {
    tracer: Option<SdkTracerProvider>,
    logger: Option<SdkLoggerProvider>,
}

impl Drop for Telemetry {
    fn drop(&mut self) {
        if let Some(tracer) = self.tracer.take() {
            let _ = tracer.shutdown();
        }
        if let Some(logger) = self.logger.take() {
            let _ = logger.shutdown();
        }
    }
}

fn filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=debug".into())
}

pub fn init() -> anyhow::Result<Telemetry> {
    // W3C traceparent in, so a caller's trace ID continues through our spans.
    opentelemetry::global::set_text_map_propagator(TextMapCompositePropagator::new(vec![
        Box::new(TraceContextPropagator::new()),
        Box::new(BaggagePropagator::new()),
    ]));

    let fmt = tracing_subscriber::fmt::layer().with_filter(filter());
    let enabled = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok_and(|v| !v.trim().is_empty());
    if !enabled {
        tracing_subscriber::registry().with(fmt).init();
        return Ok(Telemetry { tracer: None, logger: None });
    }

    let resource = opentelemetry_sdk::Resource::builder().build();
    let span_exporter = opentelemetry_otlp::SpanExporter::builder().with_http().build()?;
    let tracer_provider =
        SdkTracerProvider::builder().with_resource(resource.clone()).with_batch_exporter(span_exporter).build();
    let log_exporter = opentelemetry_otlp::LogExporter::builder().with_http().build()?;
    let logger_provider =
        SdkLoggerProvider::builder().with_resource(resource).with_batch_exporter(log_exporter).build();
    opentelemetry::global::set_tracer_provider(tracer_provider.clone());

    let otel_traces =
        tracing_opentelemetry::layer().with_tracer(tracer_provider.tracer("nullbox-api")).with_filter(filter());
    // The OTLP exporter logs through `tracing` itself; keep those out of the export loop.
    let otel_logs = OpenTelemetryTracingBridge::new(&logger_provider).with_filter(
        filter()
            .add_directive("opentelemetry=off".parse()?)
            .add_directive("reqwest=off".parse()?)
            .add_directive("hyper=off".parse()?)
            .add_directive("h2=off".parse()?),
    );

    tracing_subscriber::registry().with(fmt).with(otel_traces).with(otel_logs).init();
    tracing::info!("OpenTelemetry export on");
    Ok(Telemetry { tracer: Some(tracer_provider), logger: Some(logger_provider) })
}

/// The request span: named `METHOD /path`, child of the caller's `traceparent` if it sent one.
pub fn request_span<B>(request: &axum::http::Request<B>) -> tracing::Span {
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    let span = tracing::info_span!(
        "request",
        otel.name = %format!("{} {}", request.method(), request.uri().path()),
        otel.kind = "server",
        http.request.method = %request.method(),
        url.path = %request.uri().path(),
        http.response.status_code = tracing::field::Empty,
    );
    let parent = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&opentelemetry_http::HeaderExtractor(request.headers()))
    });
    let _ = span.set_parent(parent);
    span
}
