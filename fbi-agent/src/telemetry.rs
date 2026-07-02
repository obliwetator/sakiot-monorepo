use opentelemetry_sdk::Resource;
use std::error::Error;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{
    Layer, Registry,
    filter::{EnvFilter, LevelFilter},
    layer::SubscriberExt,
};

const SUPPRESSED_SONGBIRD_UDP_RX_LOGS: [&str; 2] = [
    "songbird::driver::tasks::udp_rx=off",
    "songbird::driver::tasks::udp_rx::ssrc_state=off",
];

pub fn init_telemetry() -> Result<(), Box<dyn Error + Send + Sync>> {
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()?;

    let metrics_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .build()?;

    let instance_id = std::env::var("BOT_INSTANCE_ID")
        .unwrap_or_else(|_| format!("{}-{}", crate::config::SERVICE_NAME, std::process::id()));
    let resource = Resource::builder_empty()
        .with_attributes([
            opentelemetry::KeyValue::new("service.name", crate::config::SERVICE_NAME),
            opentelemetry::KeyValue::new("service.instance.id", instance_id),
        ])
        .build();

    let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(otlp_exporter)
        .with_resource(resource.clone())
        .build();

    let meter_provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(opentelemetry_sdk::metrics::PeriodicReader::builder(metrics_exporter).build())
        .with_resource(resource)
        .build();

    opentelemetry::global::set_tracer_provider(tracer_provider);
    opentelemetry::global::set_meter_provider(meter_provider);

    let tracer = opentelemetry::global::tracer(crate::config::SERVICE_NAME);
    let log_filter = log_filter()?;

    let telemetry = tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_filter(log_filter.clone());

    let file_appender = tracing_appender::rolling::daily("logs", "fbi-agent.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // We intentionally leak the guard so the background writer stays alive.
    // Ideally this would be returned and kept in main(), but leaking it works
    // for global long-running daemons.
    std::mem::forget(_guard);

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(non_blocking)
        .with_span_events(FmtSpan::NONE)
        .with_filter(log_filter.clone());

    let subscriber = Registry::default().with(telemetry).with(file_layer).with(
        tracing_subscriber::fmt::layer()
            .pretty()
            .with_span_events(FmtSpan::NONE)
            .with_filter(log_filter),
    );

    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}

fn log_filter() -> Result<EnvFilter, Box<dyn Error + Send + Sync>> {
    let mut filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    for directive in SUPPRESSED_SONGBIRD_UDP_RX_LOGS {
        filter = filter.add_directive(directive.parse()?);
    }

    Ok(filter)
}
